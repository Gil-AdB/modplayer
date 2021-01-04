import './libs/bootstrap.min.css';
import './libs/xterm.css';
import * as xterm from './libs/xterm';

let term = new xterm.Terminal({cols: 200, rows: 50, disableStdin: true});
term.open(document.getElementById('terminal'));

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

// async function run() {
//     await init();
// }
//
// run();


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
//var file = fileInput.files[0];
//loadFileInput(file)
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

let lastTimestamp = 0;
const fps = 30;
const timestep = 1000 / fps; // ms for each frame
function render(timestamp) {
    window.requestAnimationFrame(render);

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

