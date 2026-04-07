class ModPlayerWorklet extends AudioWorkletProcessor {
    constructor() {
        super();
        this.bufferLeft = new Float32Array(96000); // 2 second buffer at 48kHz
        this.bufferRight = new Float32Array(96000);
        this.writePos = 0;
        this.readPos = 0;
        
        this.port.onmessage = (e) => {
            if (e.data.type === 'audio') {
                const left = e.data.left;
                const right = e.data.right;
                const available = (this.writePos - this.readPos + this.bufferLeft.length) % this.bufferLeft.length;
                
                // If adding this chunk would overflow, drop it (or we could truncate)
                if (available + left.length >= this.bufferLeft.length - 100) {
                    return;
                }

                for (let i = 0; i < left.length; i++) {
                    this.bufferLeft[this.writePos] = left[i];
                    this.bufferRight[this.writePos] = right[i];
                    this.writePos = (this.writePos + 1) % this.bufferLeft.length;
                }
            } else if (e.data.type === 'stop') {
                this.writePos = 0;
                this.readPos = 0;
            }
        };
    }
    
    process(inputs, outputs, parameters) {
        const output = outputs[0];
        if (!output || !output[0] || !output[1]) return true;

        const channelLeft = output[0];
        const channelRight = output[1];
        const frames = channelLeft.length;
        
        let framesAvailable = (this.writePos - this.readPos + this.bufferLeft.length) % this.bufferLeft.length;
        
        if (framesAvailable < frames) {
            // Buffer underrun
            channelLeft.fill(0);
            channelRight.fill(0);
            this.port.postMessage({ type: 'starve' });
            return true;
        }

        for (let i = 0; i < frames; i++) {
            channelLeft[i] = this.bufferLeft[this.readPos];
            channelRight[i] = this.bufferRight[this.readPos];
            this.readPos = (this.readPos + 1) % this.bufferLeft.length;
        }
        
        // Notify main thread if we drop below 200ms of audio (9600 frames)
        framesAvailable = (this.writePos - this.readPos + this.bufferLeft.length) % this.bufferLeft.length;
        if (framesAvailable < 9600) {
            this.port.postMessage({ type: 'needData' });
        }
        
        return true;
    }
}
registerProcessor('modplayer-worklet', ModPlayerWorklet);
