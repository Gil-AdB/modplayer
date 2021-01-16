import './libs/bootstrap.min.css';
import './libs/xterm.css';
import * as xterm from './libs/xterm';

let term = new xterm.Terminal({cols: 200, rows: 50, disableStdin: true});
term.open(document.getElementById('terminal'));

function remove_xterm_input_handler() {
    let textarea = document.getElementsByClassName('xterm-helper-textarea')[0];
    textarea.addEventListener('keydown', function(){
        this.outerHTML = this.outerHTML;
    }, false);
    textarea.addEventListener('keypress', function(){
        this.outerHTML = this.outerHTML;
    }, false);
    textarea.addEventListener('keyup', function(){
        this.outerHTML = this.outerHTML;
    }, false);

    var keyboardEvent = document.createEvent('KeyboardEvent');
    var initMethod = typeof keyboardEvent.initKeyboardEvent !== 'undefined' ? 'initKeyboardEvent' : 'initKeyEvent';

    keyboardEvent[initMethod](
        'keyup', // event type: keydown, keyup, keypress
        true, // bubbles
        true, // cancelable
        window, // view: should be window
        false, // ctrlKey
        false, // altKey
        false, // shiftKey
        false, // metaKey
        40, // keyCode: unsigned long - the virtual key code, else 0
        0, // charCode: unsigned long - the Unicode character associated with the depressed key, else 0
    );
    textarea.dispatchEvent(keyboardEvent);
}

remove_xterm_input_handler();

document.querySelector('#play').addEventListener('click', function () {
    initPlayer();
    if (player.IsPlaying()) {
        player.Pause();
    } else {
        player.Play();
    }
});

document.querySelector('#prev').addEventListener('click', function () {
    initPlayer();
    Prev()
});

document.querySelector('#next').addEventListener('click', function () {
    initPlayer();
    Next()
});

document.querySelector('#file').addEventListener('change', function () {
    initPlayer();
    loadFilesInput(document.querySelector('#file'));
});

document.querySelector('#terminal').addEventListener('drop', function (e) {
    initPlayer();
    dropHandler(e);
});

document.querySelector('#terminal').addEventListener('dragover', function (e) {
    dragOverHandler(e);
});

import * as modplayer from '../pkg/modplayer_wasm';

function initAudio() {
    const audioContext = new (window.AudioContext || window.webkitAudioContext)();
    const modplayerNode = audioContext.createScriptProcessor(4096, 0, 2);
    const processor = new ModPlayerProcessor(audioContext.sampleRate,
        function (self) {
            if (self.IsPlaying()) {
                document.querySelector('#play').value = "Pause";
                //render();
            } else {
                document.querySelector('#play').value = "Play";
            }
        },
        function () {
            if (!Next()) {
                document.querySelector('#play').value = "Play";
            }
        });
    modplayerNode.onaudioprocess = processor.process.bind(processor);
    modplayerNode.connect(audioContext.destination)
    return processor;
}

let player = null;

function initPlayer() {
    if (!player) {
        player = initAudio();
    }
}

class ModPlayerProcessor {
    constructor(sampleRate, state_change_cb, finished_cb) {
        this.sampleRate = sampleRate;
        this.playing = false;
        if (state_change_cb) {
            this.state_change_cb = state_change_cb;
        }
        if (finished_cb) {
            this.finished_cb = finished_cb;
        }
    }

    process(event) {
        let rate = event.outputBuffer.sampleRate;
        let left = event.outputBuffer.getChannelData(0);
        let right = event.outputBuffer.getChannelData(1);

        if (!this.playing) {
            left.fill(0.0);
            right.fill(0.0);
            return true;
        }

        if (this && this.song) {
            if (!this.song.get_next_tick(left, right, rate)) {
                this.playing = false;
                this.finished_cb();
            }
        }
        return true;
    }

    Stop() {
        this.Pause();
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
    }

    Start(data) {
        this.Stop();
        term.clear()
        this.song = modplayer.SongJs.new(this.sampleRate, data);
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
        if (true === this.song.handle_input(events)) {
            this.Stop();
        }
    }
}

function loadFileInput(file) {
    var fr = new FileReader();
    fr.onload = function () {
        var dataarr = new Uint8Array(fr.result);
        document.getElementById('filename').innerText = file.name;
        player.Start(dataarr);
        player.Play();
    };
    fr.readAsArrayBuffer(file);
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
                filesList.push(file);
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
            filesList.push(file);
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

top.term_writeln = function(str) {
    term.writeln(str);
}

let events = [];
window.onkeyup = handleKeyboardEvents;
function handleKeyboardEvents(e) {
    events.push(e.key);
}

let lastTimestamp = 0;
const fps = 30;
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

    if (player && player.IsPlaying()) {
        player.Display();
    }

}

render();

