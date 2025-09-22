# 🦀 QUIC Video Relay Server

High-performance video streaming using **QUIC** and **WebTransport**.

## 🏗️ Architecture

```
macOS App → [QUIC Streams] → Rust Server → [WebTransport] → Web Browser
```

- **macOS Client**: Captures screen, compresses to H.264, sends via QUIC streams
- **Rust Server**: Receives QUIC streams, relays via WebTransport datagrams  
- **Web Player**: Receives WebTransport datagrams, plays via MediaSource API

## 🚀 Quick Start

### 1. Start the Rust Server
```bash
cargo run --release
```
Server will listen on:
- **Port 8443**: QUIC (for macOS app)
- **Port 8444**: WebTransport (for web browser)

### 2. Start the Web Player Server
```bash
python3 serve-player.py
```
Then open: **https://localhost:3000/webtransport-player.html**

### 3. Run your macOS App
```bash
cd ~/Work/QUIC/mac/QuicScreenSharing
swift run
```

## 🔧 Components

### Rust Server (`src/main.rs`)
- **QUIC Server**: Receives H.264 frames from macOS
- **WebTransport Server**: Relays frames to web browsers
- **Frame Processing**: Parses headers, manages statistics

### Web Player (`webtransport-player.html`)
- **WebTransport Client**: Connects to Rust server
- **MediaSource API**: Plays H.264 video streams
- **Real-time Stats**: Frame rate, data usage, connection status

### macOS Client (`QuicTransport.swift`)
- **ScreenCaptureKit**: Captures screen content
- **VideoToolbox**: H.264 compression
- **Network.framework**: QUIC transport

## 📦 Frame Format

Each frame includes a 25-byte header:
```
[4 bytes] Magic: 0x53545246 ("FRTS")
[8 bytes] Frame Number (little-endian)
[8 bytes] Timestamp (little-endian)  
[1 byte]  Frame Type (1=keyframe, 0=delta)
[4 bytes] Data Size (little-endian)
[N bytes] H.264 Data
```

## 🌐 WebTransport Requirements

- **Chrome 97+** or **Edge 97+**
- **HTTPS required** (self-signed certs provided)
- **Experimental features** may need to be enabled

## 🔍 Troubleshooting

### WebTransport Not Supported
Enable in Chrome: `chrome://flags/#enable-experimental-web-platform-features`

### Certificate Errors
Accept self-signed certificates in both:
- Rust server (port 8444)
- Python server (port 3000)

### No Video Frames
Check that:
1. Rust server is running and shows "QUIC connection"
2. macOS app successfully connects
3. Web player shows "Connected" status

## 📊 Performance

- **Low Latency**: QUIC datagrams for real-time delivery
- **High Throughput**: Optimized for screen capture
- **Reliable**: Automatic reconnection and error handling

## 🛠️ Development

### Build Server
```bash
cargo build --release
```

### Run Tests
```bash
cargo test
```

### Debug Mode
```bash
RUST_LOG=debug cargo run
```