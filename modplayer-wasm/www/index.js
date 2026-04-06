import './libs/bootstrap.min.css';
import * as wglt from 'wglt';
import * as modplayer from '../pkg/modplayer_wasm';
// import * as Terminal from "terminal";
let font = require("./8x16 Font.png");
//import './libs/xterm.css';
//import * as xterm from './libs/xterm';

//let term = new xterm.Terminal({cols: 200, rows: 50, disableStdin: true});
//term.open(document.getElementById('terminal'));

class NonResizeableTerminal extends wglt.Terminal {
    constructor(canvas, width, height, options) {
        super(canvas, width, height, options);
    }
    handleResize() {}
}

const term = new NonResizeableTerminal(
    document.querySelector('#terminal'),
    200, 50,
    { font: new wglt.Font(font.default || font, 8, 16) });

function set_line_colors(x, y, term) {
    const colors = [
        wglt.fromRgb(0, 120, 0),
        wglt.fromRgb(0, 140, 0),
        wglt.fromRgb(0, 160, 0),
        wglt.fromRgb(0, 180, 0),
        wglt.fromRgb(180, 180, 0),
        wglt.fromRgb(195, 195, 0),
        wglt.fromRgb(210, 210, 0),
        wglt.fromRgb(225, 225, 0),
        wglt.fromRgb(225, 64, 0),
        wglt.fromRgb(225, 64, 0),
        wglt.fromRgb(225, 64, 0),
        wglt.fromRgb(225, 64, 0),
    ];

    for (let i = 0; i < 12; i++) {
        term.getCell(x + i, y).setForeground(colors[i]);
    }
}
function set_screen_colors() {
    term.fillRect(0, 0, 200, 50, 0, wglt.Colors.LIGHT_GRAY, wglt.Colors.BLACK);
}

set_screen_colors();

// function install_input_handler() {
//     // let textarea = document.getElementsByClassName('xterm-helper-textarea')[0];
//     // textarea.addEventListener('keydown', function(){
//     //     this.outerHTML = this.outerHTML;
//     // }, false);
//     // textarea.addEventListener('keypress', function(){
//     //     this.outerHTML = this.outerHTML;
//     // }, false);
//     // textarea.addEventListener('keyup', function(){
//     //     this.outerHTML = this.outerHTML;
//     // }, false);
//
// //     var keyboardEvent = document.createEvent('KeyboardEvent');
// //     var initMethod = typeof keyboardEvent.initKeyboardEvent !== 'undefined' ? 'initKeyboardEvent' : 'initKeyEvent';
// //
// //     keyboardEvent[initMethod](
// //         'keyup', // event type: keydown, keyup, keypress
// //         true, // bubbles
// //         true, // cancelable
// //         window, // view: should be window
// //         false, // ctrlKey
// //         false, // altKey
// //         false, // shiftKey
// //         false, // metaKey
// //         40, // keyCode: unsigned long - the virtual key code, else 0
// //         0, // charCode: unsigned long - the Unicode character associated with the depressed key, else 0
// //     );
// //     textarea.dispatchEvent(keyboardEvent);
// //     window.addEventListener('resize', function(e) {
// //                 e.stopPropagation();
// //              }, false);
// }
//
// // install_input_handler();

// const terminal_canvas = document.querySelector('#terminal');
// terminal_canvas.style.width = "1600px";
// terminal_canvas.style.height = "800px";

document.querySelector('#play').addEventListener('click', async function () {
    await initPlayer();
    if (player.IsPlaying()) {
        player.Pause();
    } else {
        player.Play();
    }
});

document.querySelector('#prev').addEventListener('click', async function () {
    await initPlayer();
    Prev()
});

document.querySelector('#next').addEventListener('click', async function () {
    await initPlayer();
    Next()
});

document.querySelector('#file').addEventListener('change', async function () {
    await initPlayer();
    loadFilesInput(document.querySelector('#file'));
});

document.querySelector('#terminal').addEventListener('drop', async function (e) {
    await initPlayer();
    dropHandler(e);
});

document.querySelector('#terminal').addEventListener('dragover', function (e) {
    dragOverHandler(e);
});

async function initAudio() {
    const audioContext = new (window.AudioContext || window.webkitAudioContext)({ sampleRate: 48000 });
    await audioContext.audioWorklet.addModule('audio-worklet.js');
    const modplayerNode = new AudioWorkletNode(audioContext, 'modplayer-worklet', {
        outputChannelCount: [2]
    });
    
    const processor = new ModPlayerProcessor(audioContext.sampleRate, modplayerNode.port,
        function (self) {
            if (self.IsPlaying()) {
                document.querySelector('#play').value = "⏸";
                //render();
            } else {
                document.querySelector('#play').value = "▶️";
            }
        },
        function () {
            if (!Next()) {
                document.querySelector('#play').value = "▶️";
            }
        });
        
    modplayerNode.connect(audioContext.destination);
    return processor;
}

let player = null;
let channelCount = 0;
let _initPlayerPromise = null;

async function initPlayer() {
    if (!player) {
        if (!_initPlayerPromise) {
            _initPlayerPromise = initAudio();
        }
        player = await _initPlayerPromise;
    }
}

class ModPlayerProcessor {
    constructor(sampleRate, port, state_change_cb, finished_cb) {
        this.sampleRate = sampleRate;
        this.port = port;
        this.playing = false;
        
        // Setup chunk rendering arrays
        this.leftBuf = new Float32Array(4096);
        this.rightBuf = new Float32Array(4096);

        if (state_change_cb) {
            this.state_change_cb = state_change_cb;
        }
        if (finished_cb) {
            this.finished_cb = finished_cb;
        }
        
        this.port.onmessage = (e) => {
            if (e.data.type === 'needData' || e.data.type === 'starve') {
                this.pumpAudio();
            }
        };
        
        // Fail-safe interval if worklet is busy
        setInterval(() => this.pumpAudio(), 50);
    }

    pumpAudio() {
        if (!this.playing || !this.song) return;
        
        // Generate a chunk of audio
        if (!this.song.get_next_tick(this.leftBuf, this.rightBuf, this.sampleRate)) {
            this.playing = false;
            this.finished_cb();
            return;
        }
        
        // Send chunk to worklet queue
        this.port.postMessage({
            type: 'audio',
            left: this.leftBuf,
            right: this.rightBuf,
            length: this.leftBuf.length
        });
    }

    Stop() {
        this.Pause();
        this.port.postMessage({ type: 'stop' });
        if (this && this.song) {
            let song = this.song;
            this.song = null;
            song.free();
        }
    }

    Pause() {
        this.playing = false;
        this.state_change_cb(this);
    }

    Play() {
        this.playing = true;
        this.state_change_cb(this);
        this.pumpAudio();
    }

    Start(data) {
        this.Stop();
        term.clear();
        this.song = modplayer.SongJs.new(this.sampleRate, data);
        channelCount = this.song.get_channel_count();
    }

    IsPlaying() {
        return this.playing;
    }

    Display() {
        if (this && this.song) {
            this.song.display();
        }
    }

    HandleKeyboardEvents(events) {
        if (this && this.song) {
            if (true === this.song.handle_input(events)) {
                this.Stop();
            }
        }
    }
}

function loadFileInput(file) {
    fetch(file.url).then(function (response) {
        response.arrayBuffer().then(function (buf) {
            var dataarr = new Uint8Array(buf);
            document.getElementById('filename').innerText = file.name;
            player.Start(dataarr);
            player.Play();
        });
    });
}

function loadFilesInput(fileInput) {
    let files;
    if (fileInput.target && fileInput.target.files) {
        files = fileInput.target.files;
    } else {
        files = fileInput.files;
    }
    if (files.length === 0) {
        return;
    }
    fileHandler(files);
}

function Prev() {
    player.Stop();
    if (filesList !== undefined && filesListPosition >= 1) {
        filesListPosition--;
        fileHandlerCallback();
    }
}

function Next() {
    player.Stop();
    if (filesList !== undefined && filesListPosition < filesList.length - 1) {
        filesListPosition++;
        fileHandlerCallback();
        return true;
    }
    return false;
}

var filesList;
var filesListPosition;

function fileHandlerCallback() {
    if (!filesList) {
        return;
    }

    if (filesListPosition < filesList.length) {
        var file = filesList[filesListPosition];
        if (!file) {
            return;
        }
        console.log('... file[' + filesListPosition + '].name = ' + file.name);
        loadFileInput(file);
    }
}

function fileHandler(data) {
    filesList = [];
    if (data.items) {
        for (var i = 0; i < data.items.length; ++i) {
            if (data.items[i].kind === 'file') {
                var file = data.items[i].getAsFile();

                console.log('... file[' + i + '].name = ' + file.name);
                filesList.push({name: file.name, url: window.URL.createObjectURL(file)});
            }
        }
    } else {
        let files;
        if (data.files) {
            files = data.files;
        } else {
            files = data;
        }
        for (var i = 0; i < files.length; ++i) {
            var file = files[i];
            if (!file) {
                continue;
            }
            console.log('... file[' + i + '].name = ' + file.name);
            filesList.push({name: file.name, url: window.URL.createObjectURL(file)});
        }
    }

    filesListPosition = 0;
    fileHandlerCallback();
}

function dropHandler(ev) {
    console.log('File(s) dropped');

// Prevent default behavior (Prevent file from being opened)
    ev.preventDefault();

    fileHandler(ev.dataTransfer);
}

function dragOverHandler(ev) {
    console.log('File(s) in drop zone');

// Prevent default behavior (Prevent file from being opened)
    ev.preventDefault();
}

let posy = 0;
top.term_writeln = function(str) {
    if (posy >= 50) return;
    let isChannelRow = (posy >= 2 && posy < 2 + channelCount);
    for (let i = 0; i < 200; i++) {
        let cell = term.getCell(i, posy);
        cell.setBackground(wglt.Colors.BLACK);
        cell.setForeground(wglt.Colors.LIGHT_GRAY);
    }
    term.drawString(0, posy, str.padEnd(200, ' '));
    if (isChannelRow) {
        set_line_colors(57, posy, term);
        set_line_colors(102, posy, term);
        set_line_colors(115, posy, term);
        set_line_colors(128, posy, term);
        set_line_colors(141, posy, term);
    }
    posy = posy + 1;
}

top.term_writeln_with_background = function(str, c) {
    if (posy >= 50) return;
    let bg = wglt.fromRgb(c.r, c.g, c.b);
    let isChannelRow = (posy >= 2 && posy < 2 + channelCount);
    for (let i = 0; i < 200; i++) {
        let cell = term.getCell(i, posy);
        cell.setBackground(bg);
        cell.setForeground(wglt.Colors.LIGHT_GRAY);
    }
    term.drawString(0, posy, str.padEnd(200, ' '));
    if (isChannelRow) {
        set_line_colors(57, posy, term);
        set_line_colors(102, posy, term);
        set_line_colors(115, posy, term);
        set_line_colors(128, posy, term);
        set_line_colors(141, posy, term);
    }
    posy = posy + 1;
}


let events = [];
window.onkeyup = handleKeyboardEvents;
function handleKeyboardEvents(e) {
    events.push(e.key);
}

let lastTimestamp = 0;
const fps = 60;
const timestep = 1000 / fps; // ms for each frame
function render(timestamp) {
    window.requestAnimationFrame(render);

    if (events.length !== 0) {
        if (player) {
            player.HandleKeyboardEvents(events);
        }
        events = [];
    }

    // skip if timestep ms hasn't passed since last frame
    if (timestamp - lastTimestamp < timestep) {
        return;
    }
    lastTimestamp = timestamp;

    posy = 0;
    if (player && player.IsPlaying()) {
        player.Display();
    }

}

render();

