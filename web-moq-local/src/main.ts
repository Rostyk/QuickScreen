// MOQ Video Client - Official @kixelated/moq Library Implementation
// Based on kixelated/moq samples

import * as Moq from "@kixelated/moq";

        console.log('🎥 MOQ Video Client - VERSION 3.20 - FIXED avcC DETECTION!');

interface Stats {
    startTime: number;
    framesReceived: number;
    framesDecoded: number;
    bytesReceived: number;
    fps: number;
}

class MOQVideoClient {
    private connection: Moq.Connection.Established | null = null;
    private broadcast: Moq.Broadcast | null = null;
    private videoTrack: Moq.Track | null = null;
    private decoder: VideoDecoder | null = null;
    private canvas: HTMLCanvasElement;
    private ctx: CanvasRenderingContext2D;
    private stats: Stats;
    private statsInterval: number | null = null;
    private avcConfig: Uint8Array | null = null;

    constructor(canvas: HTMLCanvasElement) {
        this.canvas = canvas;
        this.ctx = canvas.getContext('2d')!;
        this.stats = {
            startTime: 0,
            framesReceived: 0,
            framesDecoded: 0,
            bytesReceived: 0,
            fps: 0
        };
        
        this.initializeDecoder();
        this.log('✅ MOQ Video Client initialized');
    }

    private initializeDecoder() {
        this.log('🔧 Initializing WebCodecs H.264 decoder...');
        
        this.decoder = new VideoDecoder({
            output: (frame: VideoFrame) => {
                this.renderFrame(frame);
                this.stats.framesDecoded++;
                this.log(`✅ Decoded frame: ${frame.codedWidth}x${frame.codedHeight}, timestamp=${frame.timestamp}`);
                frame.close();
            },
            error: (error: Error) => {
                console.error('❌ Decoder error:', error);
                this.log(`❌ DECODER CRASH: ${error.name}: ${error.message}`);
                
                // Log decoder state when it crashes
                this.log(`❌ Decoder state at crash: ${this.decoder?.state || 'unknown'}`);
                
                // Try to get more error details
                if ('code' in error) {
                    this.log(`❌ Error code: ${(error as any).code}`);
                }
                
                this.log(`❌ This crash happened after ${this.stats.framesDecoded} successfully decoded frames`);
            }
        });
        
        this.log('✅ WebCodecs decoder initialized');
    }

    private renderFrame(frame: VideoFrame) {
        // Resize canvas if needed (exactly like official samples)
        const w = frame.displayWidth;
        const h = frame.displayHeight;
        if (this.canvas.width !== w || this.canvas.height !== h) {
            this.canvas.width = w;
            this.canvas.height = h;
            this.log(`📐 Canvas resized: ${w}x${h}`);
        }

        // Render frame to canvas (EXACTLY like official samples)
        this.ctx.save();
        this.ctx.fillStyle = "#000";
        this.ctx.fillRect(0, 0, this.canvas.width, this.canvas.height);
        this.ctx.drawImage(frame, 0, 0, this.canvas.width, this.canvas.height);
        this.ctx.restore();
    }

    private async fetchCertificateFingerprint(): Promise<Uint8Array> {
        try {
            const response = await fetch('https://50-19-36-193.nip.io/certificate.sha256');
            const fingerprint = await response.text();
            this.log(`🔒 Certificate fingerprint: ${fingerprint.trim()}`);
            return this.hexToBytes(fingerprint.trim());
        } catch (error) {
            console.error('❌ Failed to fetch certificate fingerprint:', error);
            throw error;
        }
    }

    private hexToBytes(hex: string): Uint8Array {
        const bytes = new Uint8Array(hex.length / 2);
        for (let i = 0; i < hex.length; i += 2) {
            bytes[i / 2] = parseInt(hex.substr(i, 2), 16);
        }
        return bytes;
    }

    async connect() {
        try {
            this.updateStatus('🔗 Connecting to MOQ server...', 'connecting');
            this.log('🔗 Starting official MOQ connection...');

            // Fetch certificate fingerprint for self-signed certificate
            const certificateHash = await this.fetchCertificateFingerprint();

            // Use official MOQ API: URL as first param, options as second param
            const serverUrl = new URL('https://50-19-36-193.nip.io:4433/');
            const connectProps = {
                webtransport: {
                    enabled: true,
                    congestionControl: 'low-latency',
                    serverCertificateHashes: [{
                        algorithm: 'sha-256',
                        value: certificateHash
                    }]
                },
                websocket: {
                    enabled: false
                }
            };
            
            console.log('🔧 VERSION 3.10 Connection parameters:', { url: serverUrl.toString(), props: connectProps });
            this.connection = await Moq.Connection.connect(serverUrl, connectProps);
            console.log('🔧 Connection result:', this.connection);

            if (!this.connection) {
                throw new Error('Connection is null after Moq.Connection.connect');
            }

            this.log('✅ Official MOQ connection established');

            // Handle connection close
            this.connection.closed.then(() => {
                this.log('❌ MOQ connection closed');
                this.updateStatus('Disconnected', 'disconnected');
                this.resetUI();
            });

            // Consume the broadcast from empty path (like official samples)
            this.log('📡 Consuming MOQ broadcast from empty path...');
            this.broadcast = this.connection.consume('');
            this.log('✅ MOQ broadcast consumed');

            // Subscribe to the "video" track with priority 1 (like official samples)
            this.log('📺 Subscribing to video track...');
            this.videoTrack = this.broadcast.subscribe('video', 1);
            this.log('✅ Subscribed to video track');

            this.updateStatus('✅ Connected to MOQ server', 'connected');
            this.stats.startTime = Date.now();
            this.startStatsTracking();

            // Handle incoming video frames using official pattern
            this.handleMOQVideoTrack();

        } catch (error: any) {
            console.error("❌ Detailed MOQ connection error:", error);
            if (error.errors) {
                console.error("❌ Individual errors:", error.errors);
                error.errors.forEach((err: any, index: number) => {
                    console.error(`❌ Error ${index + 1}:`, err);
                });
            }
            console.error('❌ MOQ connection failed:', error);
            this.log(`❌ MOQ connection failed: ${error.message}`);
            this.updateStatus('❌ Connection failed', 'disconnected');
            this.resetUI();
        }
    }

    async handleMOQVideoTrack() {
        try {
            this.log('🎬 Processing MOQ video track...');
            
            if (!this.videoTrack) {
                throw new Error('Video track is null');
            }
            
            // REVERT TO WORKING v3.9 PATTERN - Use simple Track.readFrame() loop
            while (true) {
                try {
                    const frameData = await this.videoTrack.readFrame();
                    
                    if (!frameData) {
                        this.log('📭 No more frames from MOQ track');
                        break;
                    }
                    
                    this.log(`📦 Received MOQ frame: ${frameData.length} bytes`);
                
                // Debug: Check if this could be avcC (26 bytes expected)
                if (frameData.length <= 50) {
                    const fullHex = Array.from(frameData.slice(0, Math.min(frameData.length, 32))).map(b => b.toString(16).padStart(2, '0')).join(' ');
                    this.log(`🔍 Small frame (${frameData.length}b): ${fullHex}`);
                }
                    this.stats.framesReceived++;
                    this.stats.bytesReceived += frameData.length;
                    
                    // NOW ADD SIMPLE H.264 DECODING
                    this.decodeH264Frame(frameData);
                    
                } catch (error: any) {
                    if (error.name === 'AbortError') {
                        this.log('🔌 MOQ track connection closed');
                        break;
                    }
                    console.error('❌ Error reading MOQ frame:', error);
                    this.log(`❌ Frame read error: ${error.message}`);
                    await new Promise(resolve => setTimeout(resolve, 100));
                }
            }
            
        } catch (error: any) {
            console.error('❌ MOQ track error:', error);
            this.log(`❌ Track error: ${error.message}`);
        }
    }

    private decodeH264Frame(frameData: Uint8Array) {
        try {
            // Parse hang format frame (like official samples)
            const { data, timestamp } = this.parseHangFrame(frameData);
            
            // Debug: show hex dump of first 16 bytes
            const hexDump = Array.from(data.slice(0, 16)).map(b => b.toString(16).padStart(2, '0')).join(' ');
            
            // ⏳ WAIT FOR REAL avcC: Use actual configuration from macOS VideoToolbox encoder
            if (!this.avcConfig) {
                this.log(`⏳ Waiting for real avcC configuration from macOS stream (no hardcoded fallback)...`);
            }
            
            // Check if this is avcC configuration data (SPS/PPS) from server
            if (this.isAvcConfig(data)) {
                this.log(`🔧 Server sent avcC config: ${data.length} bytes, hex: ${hexDump}`);
                
                // If this is the stripped version (starts with 0x42), restore the 0x01 prefix
                if (data[0] === 0x42) {
                    const restoredAvcC = new Uint8Array(data.length + 1);
                    restoredAvcC[0] = 0x01; // avcC version
                    restoredAvcC.set(data, 1); // Copy the rest
                    this.avcConfig = restoredAvcC;
                    this.log(`🔧 Restored avcC with version prefix: ${restoredAvcC.length} bytes`);
                } else {
                    this.avcConfig = data; // Use as-is if already complete
                }
                
                this.log(`✅ Real avcC configuration received from macOS stream!`);
                return; // Don't decode config frames
            }
            
            // Debug: Log first frame details to see what we're missing
            if (data.length > 20) {
                this.log(`🔍 Frame analysis: length=${data.length}, first 20 bytes: ${Array.from(data.slice(0, 20)).map(b => b.toString(16).padStart(2, '0')).join(' ')}`);
            }
            
            // Check if this is a keyframe (look for SPS/PPS/IDR)
            const isKeyframe = this.isH264Keyframe(data);
            
            this.log(`🎬 Frame: ${data.length} bytes, keyframe: ${isKeyframe}, timestamp: ${timestamp}, hex: ${hexDump}`);
            
            // Configure decoder on first keyframe (now with avcC config)
            if (!this.decoder || this.decoder.state !== 'configured') {
                if (!isKeyframe) {
                    this.log('⚠️ Waiting for keyframe to configure decoder...');
                    return;
                }
                if (!this.avcConfig) {
                    this.log('⚠️ Waiting for avcC configuration...');
                    return;
                }
                this.configureDecoderWithConfig();
            }

            // 🔍 DETAILED DEBUGGING: Analyze the H.264 data before decoding
            this.log(`🔍 About to decode: ${isKeyframe ? 'KEYFRAME' : 'delta'} frame, ${data.length} bytes`);
            
            // Check NAL unit structure in annexb format
            if (data.length >= 4) {
                const nalHeader = Array.from(data.slice(0, 8)).map(b => b.toString(16).padStart(2, '0')).join(' ');
                this.log(`🔍 NAL header: ${nalHeader}`);
                
                // Look for start codes and NAL unit types
                for (let i = 0; i < Math.min(data.length - 4, 100); i++) {
                    if (data[i] === 0x00 && data[i+1] === 0x00 && data[i+2] === 0x00 && data[i+3] === 0x01) {
                        const nalType = data[i+4] & 0x1F;
                        const nalRefIdc = (data[i+4] >> 5) & 0x03;
                        this.log(`🔍 Found NAL at offset ${i}: type=${nalType}, ref_idc=${nalRefIdc}`);
                        
                        // Decode NAL types
                        const nalTypeNames: {[key: number]: string} = {
                            1: 'Non-IDR slice', 2: 'Data partition A', 3: 'Data partition B',
                            4: 'Data partition C', 5: 'IDR slice', 6: 'SEI',
                            7: 'SPS', 8: 'PPS', 9: 'Access unit delimiter'
                        };
                        this.log(`🔍 NAL type ${nalType}: ${nalTypeNames[nalType] || 'Unknown'}`);
                    }
                }
            }

            // Create EncodedVideoChunk with parsed data
            const chunk = new EncodedVideoChunk({
                type: isKeyframe ? 'key' : 'delta',
                timestamp: timestamp,
                data: data
            });

            // Decode the frame with detailed error handling
            if (this.decoder && this.decoder.state === 'configured') {
                try {
                    this.log(`🔍 Decoder state before decode: ${this.decoder.state}`);
                    this.decoder.decode(chunk);
                    this.log(`🔍 Decoder state after decode: ${this.decoder.state}`);
                } catch (decodeError: any) {
                    console.error('❌ Decode call failed:', decodeError);
                    this.log(`❌ Decode call error: ${decodeError.message}`);
                    
                    // Log the problematic frame data
                    const problemData = Array.from(data.slice(0, 32)).map(b => b.toString(16).padStart(2, '0')).join(' ');
                    this.log(`❌ Problematic frame data (first 32 bytes): ${problemData}`);
                }
            } else {
                this.log(`⚠️ Decoder not ready: state=${this.decoder?.state || 'null'}`);
            }

        } catch (error: any) {
            console.error('❌ Error decoding H.264 frame:', error);
            this.log(`❌ Frame decoding error: ${error.message}`);
        }
    }

    private parseHangFrame(buffer: Uint8Array): { data: Uint8Array; timestamp: number } {
        // Parse hang frame format (following official samples)
        const [timestamp, data] = this.getVarInt53(buffer);
        return { timestamp, data };
    }

    private getVarInt53(buf: Uint8Array): [number, Uint8Array] {
        const size = 1 << ((buf[0] & 0xc0) >> 6);
        const view = new DataView(buf.buffer, buf.byteOffset, size);
        const remain = new Uint8Array(buf.buffer, buf.byteOffset + size, buf.byteLength - size);
        let v: number;

        if (size === 1) {
            v = buf[0] & 0x3f;
        } else if (size === 2) {
            v = view.getUint16(0) & 0x3fff;
        } else if (size === 4) {
            v = view.getUint32(0) & 0x3fffffff;
        } else if (size === 8) {
            v = Number(view.getBigUint64(0) & 0x3fffffffffffffffn);
        } else {
            throw new Error("impossible");
        }

        return [v, remain];
    }

    private isH264Keyframe(data: Uint8Array): boolean {
        // Handle both AVCC (length-prefixed) and annexb (start code) formats
        let i = 0;
        while (i < data.length - 4) {
            // Try annexb format first (start codes: 00 00 01 or 00 00 00 01)
            if (data[i] === 0x00 && data[i+1] === 0x00 && data[i+2] === 0x01) {
                // 3-byte start code (00 00 01)
                const nalType = data[i+3] & 0x1F;
                if (nalType === 0x07 || nalType === 0x08 || nalType === 0x05) { // SPS, PPS, or IDR
                    return true;
                }
                i += 3;
                continue;
            }
            if (data[i] === 0x00 && data[i+1] === 0x00 && 
                data[i+2] === 0x00 && data[i+3] === 0x01) {
                // 4-byte start code (00 00 00 01)
                const nalType = data[i+4] & 0x1F;
                if (nalType === 0x07 || nalType === 0x08 || nalType === 0x05) { // SPS, PPS, or IDR
                    return true;
                }
                i += 4;
                continue;
            }
            
            // Try AVCC format (length-prefixed NAL units)
            if (i + 4 < data.length) {
                const nalLength = (data[i] << 24) | (data[i+1] << 16) | (data[i+2] << 8) | data[i+3];
                if (nalLength > 0 && nalLength < data.length - i - 4) {
                    const nalType = data[i+4] & 0x1F;
                    if (nalType === 0x07 || nalType === 0x08 || nalType === 0x05) { // SPS, PPS, or IDR
                        return true;
                    }
                    i += 4 + nalLength;
                    continue;
                }
            }
            
            i++;
        }
        return false;
    }

    private isAvcConfig(data: Uint8Array): boolean {
        // Check if this is avcC configuration data (container format from macOS)
        // avcC format starts with: [version][profile][compatibility][level][reserved+lengthSizeMinusOne][reserved+numOfSequenceParameterSets]
        // Example: 01 42 00 28 ff e1 ... (version=1, profile=0x42 Baseline, level=0x28)
        if (data.length >= 6 && 
            data[0] === 0x01 &&           // avcC version = 1
            data[4] === 0xFF &&           // reserved (6 bits) + lengthSizeMinusOne (2 bits) = 0xFF 
            data[5] === 0xE1) {           // reserved (3 bits) + numOfSequenceParameterSets (5 bits) = 0xE1 (1 SPS)
            return true;
        }
        
        // HANG FRAME STRIPPED VERSION: After parseHangFrame, the leading 0x01 is removed
        // So we see: 42 00 28 ff e1 ... instead of 01 42 00 28 ff e1 ...
        if (data.length >= 5 && 
            data[0] === 0x42 &&           // profile = 0x42 Baseline (was at index 1, now at 0)
            data[3] === 0xFF &&           // reserved + lengthSizeMinusOne (was at index 4, now at 3)
            data[4] === 0xE1) {           // reserved + numOfSequenceParameterSets (was at index 5, now at 4)
            return true;
        }
        
        // Fallback: Check if this looks like annexb format with SPS + PPS
        let hasSPS = false;
        let hasPPS = false;
        
        for (let i = 0; i < data.length - 4; i++) {
            if (data[i] === 0x00 && data[i+1] === 0x00 && 
                data[i+2] === 0x00 && data[i+3] === 0x01) {
                const nalType = data[i+4] & 0x1F;
                if (nalType === 0x07) hasSPS = true; // SPS
                if (nalType === 0x08) hasPPS = true; // PPS
            }
        }
        
        return hasSPS && hasPPS;
    }

    private configureDecoder() {
        if (!this.decoder) return;
        
        try {
            const config: VideoDecoderConfig = {
                codec: 'avc1.64001f', // H.264 Baseline Profile Level 3.1
                codedWidth: 1670,
                codedHeight: 1080,
                optimizeForLatency: true
            };
            
            this.decoder.configure(config);
            this.log('✅ H.264 decoder configured (basic)');
            
        } catch (error: any) {
            console.error('❌ Decoder configuration error:', error);
            this.log(`❌ Decoder config error: ${error.message}`);
        }
    }

    private configureDecoderWithConfig() {
        if (!this.decoder || !this.avcConfig) return;
        
        try {
            const config: VideoDecoderConfig = {
                codec: 'avc1.64001f', // H.264 Baseline Profile Level 3.1
                codedWidth: 1670,
                codedHeight: 1080,
                optimizeForLatency: true,
                description: this.avcConfig // This is the key fix!
            };
            
            this.decoder.configure(config);
            this.log('✅ H.264 decoder configured with avcC description');
            
        } catch (error: any) {
            console.error('❌ Decoder configuration error:', error);
            this.log(`❌ Decoder config error: ${error.message}`);
        }
    }


    private updateStatus(message: string, status: string) {
        const statusElement = document.getElementById('status');
        if (statusElement) {
            statusElement.textContent = message;
            statusElement.className = status;
        }
    }

    private log(message: string) {
        console.log(message);
        const logElement = document.getElementById('log');
        if (logElement) {
            logElement.textContent += message + '\n';
            logElement.scrollTop = logElement.scrollHeight;
        }
    }

    private resetUI() {
        if (this.statsInterval) {
            clearInterval(this.statsInterval);
            this.statsInterval = null;
        }
    }

    private startStatsTracking() {
        this.statsInterval = setInterval(() => {
            const elapsed = (Date.now() - this.stats.startTime) / 1000;
            this.stats.fps = this.stats.framesReceived / elapsed;
            
            this.log(`📊 Stats: ${this.stats.framesReceived} frames, ${this.stats.fps.toFixed(1)} fps, ${(this.stats.bytesReceived / 1024 / 1024).toFixed(2)} MB`);
        }, 5000);
    }
}


// Initialize when DOM is loaded
document.addEventListener('DOMContentLoaded', () => {
    const canvas = document.getElementById('view') as HTMLCanvasElement;
    const connectButton = document.getElementById('connect') as HTMLButtonElement;
    
    if (!canvas) {
        console.error('❌ Canvas element with id="view" not found');
        return;
    }
    
    if (!connectButton) {
        console.error('❌ Connect button with id="connect" not found');
        return;
    }
    
    const client = new MOQVideoClient(canvas);
    
    connectButton.addEventListener('click', async () => {
        await client.connect();
    });
});