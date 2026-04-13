import './libs/bootstrap.min.css';
import './style.css';
import * as wglt from 'wglt';
import * as modplayer from '../pkg/modplayer_wasm';
import { memory } from '../pkg/modplayer_wasm_bg.wasm';
let font = require("./8x16 Font.png");

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

function set_screen_colors() {
    term.fillRect(0, 0, 200, 50, 0, wglt.Colors.LIGHT_GRAY, wglt.Colors.BLACK);
}

set_screen_colors();

document.querySelector('#play').addEventListener('click', async function () {
    try { await initPlayer(); } catch (e) { return; }
    if (player.IsPlaying()) {
        player.Pause();
    } else {
        player.Play();
    }
});

document.querySelector('#prev').addEventListener('click', async function () {
    try { await initPlayer(); } catch (e) { return; }
    Prev()
});

document.querySelector('#next').addEventListener('click', async function () {
    try { await initPlayer(); } catch (e) { return; }
    Next()
});

document.querySelector('#file').addEventListener('change', async function () {
    try { await initPlayer(); } catch (e) { return; }
    loadFilesInput(document.querySelector('#file'));
});

window.addEventListener('dragover', function (e) { e.preventDefault(); }, false);
window.addEventListener('drop', async function (e) {
    e.preventDefault();
    e.stopPropagation();
    
    // Resume/Create AudioContext synchronously to satisfy browser user gesture policy
    if (!audioCtx) {
        audioCtx = new (window.AudioContext || window.webkitAudioContext)({ sampleRate: 48000 });
    }
    if (audioCtx.state === 'suspended') audioCtx.resume();

    // Extract File objects into a real Array synchronously.
    if (!e.dataTransfer || !e.dataTransfer.files || e.dataTransfer.files.length === 0) return;
    var fileArray = [];
    for (var i = 0; i < e.dataTransfer.files.length; i++) {
        fileArray.push(e.dataTransfer.files[i]);
    }
    
    try { await initPlayer(); } catch (err) { return; }
    
    // Build file list from captured File objects
    filesList = [];
    for (var i = 0; i < fileArray.length; i++) {
        filesList.push({name: fileArray[i].name, url: window.URL.createObjectURL(fileArray[i])});
    }
    filesListPosition = 0;
    loadFileInput(filesList[0]);
}, false);

let audioCtx = null;
let player = null;
let analyzer = null;
let channelCount = 0;
let filesList = [];
let filesListPosition = 0;
let viewMode = 0; // 0: Pattern, 1: Instruments, 2: Message, 3: Help
let themeMode = 2; // 0: Pro, 1: Vibrant, 2: Obsidian, 3: Mono
let visualMode = 1; // 0: Both(S), 1: Both(A), 2: Scope(S), 3: Scope(A), 4: FFT, 5: Off
let scopeMode = 'multi'; // 'global' or 'multi'
let _initPlayerPromise = null;

function unlockAudio() {
    if (audioCtx && audioCtx.state === 'suspended') {
        audioCtx.resume().catch(() => {});
    }
}
document.addEventListener('click', unlockAudio, { capture: true });
document.addEventListener('keydown', unlockAudio, { capture: true });
document.addEventListener('dragenter', unlockAudio, { capture: true });
document.addEventListener('dragover', unlockAudio, { capture: true });
document.addEventListener('drop', unlockAudio, { capture: true });

async function initAudio() {
    if (!audioCtx) {
        audioCtx = new (window.AudioContext || window.webkitAudioContext)({ sampleRate: 48000 });
    }
    await audioCtx.resume();
    
    await audioCtx.audioWorklet.addModule('audio-worklet.js');
    const modplayerNode = new AudioWorkletNode(audioCtx, 'modplayer-worklet', {
        outputChannelCount: [2]
    });

    const analyzerNode = audioCtx.createAnalyser();
    analyzerNode.fftSize = 512;
    modplayerNode.connect(analyzerNode);
    analyzerNode.connect(audioCtx.destination);
    
    const processor = new ModPlayerProcessor(audioCtx.sampleRate, modplayerNode.port,
        function (self) {
            if (self.IsPlaying()) {
                document.querySelector('#play').value = "⏸";
            } else {
                document.querySelector('#play').value = "▶️";
            }
        },
        function () {
            if (!Next()) {
                document.querySelector('#play').value = "▶️";
            }
        });
        
    return { processor, analyzer: analyzerNode };
}

async function initPlayer() {
    if (!player) {
        if (!_initPlayerPromise) {
            _initPlayerPromise = initAudio().catch(err => {
                _initPlayerPromise = null;
                throw err;
            });
        }
        const res = await _initPlayerPromise;
        player = res.processor;
        analyzer = res.analyzer;
    }
}

class ModPlayerProcessor {
    constructor(sampleRate, port, state_change_cb, finished_cb) {
        this.sampleRate = sampleRate;
        this.port = port;
        this.playing = false;
        this.leftBuf = new Float32Array(1024);
        this.rightBuf = new Float32Array(1024);
        if (state_change_cb) this.state_change_cb = state_change_cb;
        if (finished_cb) this.finished_cb = finished_cb;
        this.port.onmessage = (e) => {
            if (e.data.type === 'needData' || e.data.type === 'starve') {
                this.pumpAudio();
            }
        };
    }

    pumpAudio() {
        if (!this.playing || !this.song) return;
        if (!this.song.get_next_tick(this.leftBuf, this.rightBuf, this.sampleRate)) {
            this.playing = false;
            this.finished_cb();
            return;
        }
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
        for (let i = 0; i < 6; i++) {
            this.pumpAudio();
        }
    }

    Start(data) {
        this.Stop();
        this.lastBuffer = null; // Force full re-render on first frame
        term.clear();
        try {
            this.song = modplayer.SongJs.new(this.sampleRate, data);
            channelCount = this.song.get_channel_count();
        } catch (e) {
            console.error("WASM Song Load Failed:", e);
        }
    }

    IsPlaying() {
        return this.playing && !!this.song;
    }

    Display() {
        if (this && this.song) {
            // Check for audio suspension
            if (audioCtx && audioCtx.state === 'suspended') {
                term.drawString(80, 25, " [ CLICK TO ACTIVATE AUDIO ] ", wglt.fromRgb(255,255,255), wglt.fromRgb(200,0,0));
            }

            this.song.display(viewMode, themeMode);
            const gridPtr = this.song.get_grid_ptr();
            const gridLen = this.song.get_grid_size();
            
            // ZERO-COPY: Create a view directly into WASM memory
            const wasmMemory = memory.buffer;
            const gridData = new Uint8Array(wasmMemory, gridPtr, gridLen);

            const width = 200;
            const height = 50;
            const strideBytes = 12; // char(4) + fg(3) + bg(3) + align(2)
            
            if (!this.lastBuffer || this.lastBuffer.length !== gridLen) {
                this.lastBuffer = new Uint8Array(gridLen);
                this.lastBuffer.fill(0); // Force initial render
            }
            
            for (let y = 0; y < height; y++) {
                for (let x = 0; x < width; x++) {
                    const offset = (y * width + x) * strideBytes;
                    
                    // Fast dirty-check across 10 relevant bytes
                    let dirty = false;
                    for (let i = 0; i < 10; i++) {
                        if (this.lastBuffer[offset + i] !== gridData[offset + i]) {
                            dirty = true;
                            break;
                        }
                    }
                    
                    if (dirty) {
                        const charCode = gridData[offset]; // Lower 8 bits of u32 (ASCII)
                        const fr = gridData[offset + 4];
                        const fg = gridData[offset + 5];
                        const fb = gridData[offset + 6];
                        const br = gridData[offset + 7];
                        const bg = gridData[offset + 8];
                        const bb = gridData[offset + 9];

                        const fgColor = wglt.fromRgb(fr, fg, fb);
                        const bgColor = wglt.fromRgb(br, bg, bb);

                        const cell = term.getCell(x, y);
                        cell.setValue(charCode, fgColor, bgColor);
                        
                        // Update cache
                        for (let i = 0; i < 10; i++) {
                            this.lastBuffer[offset + i] = gridData[offset + i];
                        }
                    }
                }
            }
        }
    }

    HandleKeyboardEvents(events) {
        if (this && this.song) {
            if (true === this.song.handle_input(events)) {
                if (this.playing) {
                    this.Pause();
                } else {
                    this.Play();
                }
            }
        }
    }
}

async function loadFileInput(file) {
    if (!file) return;
    try {
        let buf;
        if (file instanceof File) {
            buf = await file.arrayBuffer();
        } else if (file.url) {
            const response = await fetch(file.url);
            buf = await response.arrayBuffer();
        } else {
            console.error("Unknown file format:", file);
            return;
        }
        
        var dataarr = new Uint8Array(buf);
        document.getElementById('filename').innerText = file.name;
        
        if (audioCtx && audioCtx.state === 'suspended') await audioCtx.resume();
        
        player.Start(dataarr);
        player.Play();
    } catch (err) {
        console.error("Failed to load module:", err);
    }
}

function loadFilesInput(fileInput) {
    let files = (fileInput.target && fileInput.target.files) ? fileInput.target.files : fileInput.files;
    if (files.length === 0) return;
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

function fileHandlerCallback() {
    if (!filesList) return;
    if (filesListPosition < filesList.length) {
        var file = filesList[filesListPosition];
        if (!file) return;
        loadFileInput(file);
    }
}

function fileHandler(data) {
    if (!data) return;
    
    // Support both DataTransfer (data.files) and FileList (data as list)
    var files = data.files || data;
    if (!files || files.length === 0) {
        console.warn("fileHandler: No files found in data source");
        return;
    }
    
    filesList = [];
    console.log("Processing files in handler:", files.length);
    for (var i = 0; i < files.length; ++i) {
        var file = files[i];
        if (!file) continue;
        filesList.push({name: file.name, url: window.URL.createObjectURL(file)});
    }
    filesListPosition = 0;
    fileHandlerCallback();
}

function dragOverHandler(ev) {
    ev.preventDefault();
}

let posy = 0;
top.term_writeln = function(str) {
    if (posy >= 50) return;
    for (let i = 0; i < 200; i++) {
        let cell = term.getCell(i, posy);
        cell.setBackground(wglt.Colors.BLACK);
        cell.setForeground(wglt.Colors.LIGHT_GRAY);
    }
    term.drawString(0, posy, str.padEnd(200, ' '));
    posy = posy + 1;
}

top.term_writeln_with_background = function(str, c) {
    if (posy >= 50) return;
    let bg = wglt.fromRgb(c.r, c.g, c.b);
    for (let i = 0; i < 200; i++) {
        let cell = term.getCell(i, posy);
        cell.setBackground(bg);
        cell.setForeground(wglt.Colors.LIGHT_GRAY);
    }
    term.drawString(0, posy, str.padEnd(200, ' '));
    posy = posy + 1;
}

let events = [];
window.addEventListener('keydown', handleKeyboardEvents, true);
function handleKeyboardEvents(e) {
    if (e.key === 'F1') { viewMode = 0; e.preventDefault(); }
    if (e.key === 'F2') { viewMode = 1; e.preventDefault(); }
    if (e.key === 'F3') { viewMode = 2; e.preventDefault(); }
    if (e.key === 'F4') { viewMode = 3; e.preventDefault(); }
    
    switch (e.code) {
        case 'KeyT':
            if (e.shiftKey) {
                cycleTheme();
                e.preventDefault();
            }
            break;
        case 'KeyP':
            if (e.shiftKey) {
                togglePanning();
                e.preventDefault();
            }
            break;
        case 'KeyV':
            if (e.shiftKey) {
                cycleVisuals();
                e.preventDefault();
            }
            break;
        case 'KeyS':
            if (e.shiftKey) {
                if (player && player.song) player.song.handle_input(['S']);
                e.preventDefault();
            }
            break;
    }

    if (e.key === 'ArrowUp') {
        if (player && player.song) player.song.scroll(-1);
        e.preventDefault();
    }
    if (e.key === 'ArrowDown') {
        if (player && player.song) player.song.scroll(1);
        e.preventDefault();
    }
    if (e.key === 'PageUp') {
        if (player && player.song) player.song.scroll(-20);
        e.preventDefault();
    }
    if (e.key === 'PageDown') {
        if (player && player.song) player.song.scroll(20);
        e.preventDefault();
    }

    if (e.key === 'ArrowRight') {
        if (player && player.song) player.song.scroll_x(10);
        e.preventDefault();
    }
    if (e.key === 'ArrowLeft') {
        if (player && player.song) player.song.scroll_x(-10);
        e.preventDefault();
    }

    events.push(e.key);
}

window.addEventListener('wheel', (e) => {
    if (player && player.song) {
        if (Math.abs(e.deltaX) > Math.abs(e.deltaY)) {
            const delta = Math.sign(e.deltaX) * 4;
            player.song.scroll_x(delta);
        } else {
            const delta = Math.sign(e.deltaY);
            player.song.scroll(delta);
        }
    }
}, { passive: true });

function cycleTheme() {
    themeMode = (themeMode + 1) % 4;
}

function cycleVisuals() {
    visualMode = (visualMode + 1) % 6;
    updateVisualizerLayout();
}

function togglePanning() {
    if (player && player.song) player.song.toggle_panning();
}

document.querySelector('#cycle-theme').addEventListener('click', cycleTheme);
document.querySelector('#cycle-visuals').addEventListener('click', cycleVisuals);
document.querySelector('#toggle-panning').addEventListener('click', togglePanning);

function updateVisualizerLayout() {
    const container = document.querySelector('.visualizers-container');
    const scope = document.querySelector('#oscilloscope');
    const spectrum = document.querySelector('#spectrum');
    
    if (visualMode === 5) {
        container.style.display = 'none';
        return;
    }
    
    container.style.display = 'flex';
    const showScope = [0, 1, 2, 3].includes(visualMode);
    const showFFT = [0, 1, 4].includes(visualMode);
    
    scope.style.display = showScope ? 'block' : 'none';
    spectrum.style.display = showFFT ? 'block' : 'none';
    
    if (showScope && showFFT) {
        container.style.justifyContent = 'space-between';
        scope.style.width = '49%';
        spectrum.style.width = '49%';
    } else {
        container.style.justifyContent = 'center';
        if (showScope) scope.style.width = '100%';
        if (showFFT) spectrum.style.width = '100%';
    }
}

function drawOscilloscope(song) {
    if (![0, 2].includes(visualMode)) return;
    const canvas = document.querySelector('#oscilloscope');
    const oscCtx = canvas.getContext('2d');
    const width = canvas.width;
    const height = canvas.height;
    oscCtx.fillStyle = '#000';
    oscCtx.fillRect(0, 0, width, height);

    oscCtx.lineWidth = 2;
    const colors = [
        ['#00ff00', '#ffff00', '#ff0000'], // 0: Pro
        ['#00f2fe', '#7b27ff'],            // 1: Cyberpunk
        ['#a6e22e', '#fd971f', '#f92672', '#ae81ff', '#66d9ef'], // 2: Obsidian (Monokai-ish)
        ['#ff8c00', '#404040']             // 3: Monochrome
    ];
    const theme = colors[themeMode] || colors[0];
    const grad = oscCtx.createLinearGradient(0, height, 0, 0); 
    if (themeMode === 0) {
        grad.addColorStop(0, '#ff0000');   
        grad.addColorStop(0.15, '#ffff00'); 
        grad.addColorStop(0.5, '#00ff00'); 
        grad.addColorStop(0.85, '#ffff00'); 
        grad.addColorStop(1, '#ff0000');   
    } else {
        grad.addColorStop(0, theme[0]);
        grad.addColorStop(0.5, theme[theme.length - 1]);
        grad.addColorStop(1, theme[0]);
    }
    oscCtx.strokeStyle = grad;
    oscCtx.beginPath();

    const ptr = song.get_scopes_ptr();
    const len = song.get_scopes_len();
    const scopes = new Float32Array(memory.buffer, ptr, len);

    const bufferLength = 128; // Downsampled
    const sliceWidth = width / bufferLength;
    let x = 0;
    for (let i = 0; i < bufferLength; i++) {
        let v = 0;
        let activeChannels = 0;
        for (let c = 0; c < channelCount; c++) {
            const sample = scopes[c * 128 + i];
            if (sample !== 0.0) {
                v += sample;
                activeChannels++;
            }
        }
        if (activeChannels > 0) v /= Math.sqrt(activeChannels);
        const y = (v * 1.0 * height * 0.8 / 2) + height / 2;
        if (i === 0) oscCtx.moveTo(x, y);
        else oscCtx.lineTo(x, y);
        x += sliceWidth;
    }
    oscCtx.stroke();
}

function drawMultiOscilloscope(song) {
    if (![1, 3].includes(visualMode)) return;
    const canvas = document.querySelector('#oscilloscope');
    const oscCtx = canvas.getContext('2d');
    const width = canvas.width;
    const height = canvas.height;
    oscCtx.fillStyle = '#000';
    oscCtx.fillRect(0, 0, width, height);

    const n = channelCount;
    const cols = Math.ceil(Math.sqrt(n));
    const rows = Math.ceil(n / cols);
    const cellW = width / cols;
    const cellH = height / rows;
    
    const colors = [['#00ff00', '#ffff00', '#ff0000'], ['#00f2fe', '#7b27ff'], ['#ffff00', '#ff00ff'], ['#ff8c00', '#404040']];
    const theme = colors[themeMode] || colors[0];
    oscCtx.lineWidth = 1;
    oscCtx.font = '8px Share Tech Mono';
    oscCtx.fillStyle = '#a0a0c0';

    const ptr = song.get_scopes_ptr();
    const len = song.get_scopes_len();
    const scopes = new Float32Array(memory.buffer, ptr, len);

    for (let i = 0; i < Math.min(channelCount, cols * rows); i++) {
        const xBase = (i % cols) * cellW;
        const yBase = Math.floor(i / cols) * cellH;
        oscCtx.strokeStyle = 'rgba(255,255,255,0.05)';
        oscCtx.strokeRect(xBase, yBase, cellW, cellH);
        
        oscCtx.fillText((i + 1).toString(), xBase + 2, yBase + 8);
        const grad = oscCtx.createLinearGradient(0, yBase + cellH, 0, yBase);
        if (themeMode === 0) {
            grad.addColorStop(0, '#ff0000'); grad.addColorStop(0.15, '#ffff00'); grad.addColorStop(0.5, '#00ff00'); grad.addColorStop(0.85, '#ffff00'); grad.addColorStop(1, '#ff0000');
        } else {
            grad.addColorStop(0, theme[0]); grad.addColorStop(0.5, theme[theme.length - 1]); grad.addColorStop(1, theme[0]);
        }
        oscCtx.strokeStyle = grad;
        oscCtx.beginPath();
        
        const sliceW = cellW / 128;
        for (let s = 0; s < 128; s++) {
            const v = scopes[i * 128 + s];
            const y = (v * 1.0 * cellH * 0.7) + yBase + cellH / 2;
            if (s === 0) oscCtx.moveTo(xBase, y);
            else oscCtx.lineTo(xBase + s * sliceW, y);
        }
        oscCtx.stroke();
    }
}

function drawSpectrum() {
    if (![0, 1, 4].includes(visualMode)) return;
    if (!analyzer) return;
    const canvas = document.querySelector('#spectrum');
    const ctx = canvas.getContext('2d');
    const width = canvas.width;
    const height = canvas.height;
    const bufferLength = analyzer.frequencyBinCount;
    const dataArray = new Uint8Array(bufferLength);
    analyzer.getByteFrequencyData(dataArray);
    ctx.fillStyle = '#000';
    ctx.fillRect(0, 0, width, height);
    const barWidth = (width / bufferLength) * 2.5;
    
    const colors = [
        ['#00ff00', '#ffff00', '#ff0000'],
        ['#00f2fe', '#7b27ff'], 
        ['#ffff00', '#ff00ff'], 
        ['#ff8c00', '#404040'],
         
    ];
    const theme = colors[themeMode] || colors[0];
    
    const grad = ctx.createLinearGradient(0, height, 0, 0);
    if (themeMode === 0) {
        grad.addColorStop(0, '#00ff00');
        grad.addColorStop(0.5, '#ffff00');
        grad.addColorStop(1, '#ff0000');
    } else {
        grad.addColorStop(0, theme[0]);
        grad.addColorStop(1, theme[theme.length - 1]);
    }
    
    let x = 0;
    for (let i = 0; i < bufferLength; i++) {
        const barHeight = (dataArray[i] / 255) * height;
        ctx.fillStyle = grad;
        ctx.fillRect(x, height - barHeight, barWidth - 1, barHeight);
        x += barWidth;
    }
}

let lastTimestamp = 0;
const fps = 60;
const timestep = 1000 / fps; 
function render(timestamp) {
    window.requestAnimationFrame(render);
    if (events.length !== 0) {
        if (player) player.HandleKeyboardEvents(events);
        events = [];
    }
    if (timestamp - lastTimestamp < timestep) return;
    lastTimestamp = timestamp;
    posy = 0;
    if (player && player.IsPlaying() && player.song) {
        try {
            player.Display();
            drawOscilloscope(player.song);
            drawMultiOscilloscope(player.song);
            drawSpectrum();
        } catch (e) {
            console.error("Display Loop Error:", e);
        }
    }
}

updateVisualizerLayout();
render();
