#!/usr/bin/env swift

import Network
import Foundation
import CoreGraphics
import QuartzCore

// 🎛️ CONFIGURATION
struct NetworkSimulation {
    static let enabled = false          // DISABLED to see complete frames first
    static let reorderProbability = 0.15  // 15% chance of frame reordering
    static let minDelay = 0.1           // Minimum delay in seconds
    static let maxDelay = 0.5           // Maximum delay in seconds
}

struct FragmentationConfig {
    static let enabled = true           // Enable frame fragmentation
    static let maxChunkSize = 1150      // QUIC datagram limit (~1200 - headers)
    static let largeFrameThreshold = 1400  // Fragment frames larger than this
    static let chunkReorderProbability = 0.05  // 5% chance each chunk gets reordered (realistic)
    static let maxChunkDelay = 0.1      // Max 100ms delay for chunks (realistic)
    
    // 🖥️ 16-inch MacBook Pro Retina optimized frame sizes
    static let keyframeSize = 150000    // 150KB typical keyframe
    static let deltaFrameSize = 25000   // 25KB typical delta frame
}

class SimpleVideoClient {
    private var connection: NWConnection?
    private var frameCounter: Int64 = 0
    
    // 🧪 Network Simulation Settings (using global config)
    private var pendingFrames: [(Data, TimeInterval)] = []  // Delayed frames buffer
    
    func startStreaming() {
        print("🎥 Starting simple video streaming client...")
        
        if NetworkSimulation.enabled {
            print("🧪 Network simulation ENABLED:")
            print("   📊 Reorder probability: \(Int(NetworkSimulation.reorderProbability * 100))%")
            print("   ⏱️  Delay range: \(Int(NetworkSimulation.minDelay*1000))-\(Int(NetworkSimulation.maxDelay*1000))ms")
        } else {
            print("✨ Perfect network delivery (simulation disabled)")
        }
        
        if FragmentationConfig.enabled {
            print("🧩 Frame fragmentation ENABLED:")
            print("   📦 Max chunk size: \(FragmentationConfig.maxChunkSize) bytes")
            print("   🔄 Fragment threshold: \(FragmentationConfig.largeFrameThreshold) bytes")
            print("   🔀 Chunk reorder probability: \(Int(FragmentationConfig.chunkReorderProbability * 100))%")
            print("   ⏱️  Max chunk delay: \(Int(FragmentationConfig.maxChunkDelay * 1000))ms")
            print("🖥️ 16-inch MacBook Pro Retina optimized:")
            print("   🔑 Keyframe size: ~\(FragmentationConfig.keyframeSize/1000)KB (±50KB)")
            print("   📊 Delta frame size: ~\(FragmentationConfig.deltaFrameSize/1000)KB (±15KB)")
        } else {
            print("📦 Single datagram mode (fragmentation disabled)")
        }
        
        setupQUICConnection()
        startMockVideoStream()
    }
    
    // MARK: - QUIC Connection Setup
    private func setupQUICConnection() {
        print("🔗 Setting up QUIC connection...")
        
        let quicOptions = NWProtocolQUIC.Options()
        quicOptions.isDatagram = true
        quicOptions.alpn = ["video-stream"]
        
        // Disable certificate verification for self-signed certs
        sec_protocol_options_set_verify_block(quicOptions.securityProtocolOptions, { _, _, complete in
            complete(true)
        }, .main)
        
        let parameters = NWParameters(quic: quicOptions)
        connection = NWConnection(host: "localhost", port: 8443, using: parameters)
        
        connection?.stateUpdateHandler = { [weak self] state in
            switch state {
            case .ready:
                print("✅ QUIC connection established!")
                self?.sendControlMessage(type: "client_ready", data: [:])
            case .waiting(let error):
                print("⏳ Connection waiting: \(error)")
            case .failed(let error):
                print("❌ Connection failed: \(error)")
            default:
                print("🔄 Connection state: \(state)")
            }
        }
        
        connection?.start(queue: .main)
    }
    
    // MARK: - Mock Video Stream
    private func startMockVideoStream() {
        print("📺 Starting mock video stream (30fps)...")
        
        Timer.scheduledTimer(withTimeInterval: 1.0 / 30.0, repeats: true) { [weak self] _ in
            self?.sendMockVideoFrame()
        }
    }
    
    private func sendMockVideoFrame() {
        frameCounter += 1
        
        // Create mock video data (simulating H.264 compressed frame)
        let isKeyframe = (frameCounter % 60 == 1) // Keyframe every 2 seconds at 30fps
        
        // 🖥️ Realistic 16-inch MacBook Pro Retina screen capture frame sizes
        let frameSize = isKeyframe ? 
            FragmentationConfig.keyframeSize + Int.random(in: -50000...50000) :  // 100-200KB keyframes
            FragmentationConfig.deltaFrameSize + Int.random(in: -15000...25000)  // 10-50KB delta frames
        let mockVideoData = Data(repeating: UInt8.random(in: 0...255), count: frameSize)
        
        let frameType = isKeyframe ? "KEYFRAME" : "delta"
        let timestamp = UInt64(CACurrentMediaTime() * 1000)
        
        // 🧩 Use fragmentation for large frames
        if FragmentationConfig.enabled && frameSize > FragmentationConfig.largeFrameThreshold {
            print("🧩 Mock frame #\(frameCounter): \(frameSize) bytes (\(frameType)) → FRAGMENTED")
            sendFragmentedFrame(data: mockVideoData, frameNumber: UInt64(frameCounter), timestamp: timestamp, isKeyframe: isKeyframe)
        } else {
            print("📦 Mock frame #\(frameCounter): \(frameSize) bytes (\(frameType)) → DATAGRAM")
            sendViaDatagram(data: mockVideoData, frameNumber: UInt64(frameCounter), timestamp: timestamp, isKeyframe: isKeyframe)
        }
        
        // Simulate occasional keyframe requests
        if frameCounter % 150 == 0 { // Every 5 seconds
            sendControlMessage(type: "keyframe_request", data: ["reason": "periodic"])
        }
    }
    
    private func sendViaStream(data: Data, frameNumber: UInt64, timestamp: UInt64, isKeyframe: Bool) {
        // Create stream frame header with magic number
        var header = StreamFrameHeader(
            magic: 0x4B455946, // "KEYF" magic
            frameNumber: frameNumber,
            timestamp: timestamp,
            dataSize: UInt32(data.count),
            isKeyframe: isKeyframe ? 1 : 0
        )
        
        var packet = Data()
        packet.append(Data(bytes: &header, count: MemoryLayout<StreamFrameHeader>.size))
        packet.append(data)
        
        // Send via QUIC stream (reliable, ordered) 
        print("🔍 DEBUG: Attempting to send STREAM frame: \(packet.count) bytes")
        connection?.send(content: packet, completion: .contentProcessed { error in
            if let error = error {
                print("❌ Stream frame failed: \(error)")
            } else {
                print("✅ Frame #\(frameNumber) sent via STREAM (\(packet.count) bytes)")
            }
        })
    }
    
    private func sendViaDatagram(data: Data, frameNumber: UInt64, timestamp: UInt64, isKeyframe: Bool) {
        // Create datagram frame header with magic number
        var header = DatagramFrameHeader(
            magic: 0x44415441, // "DATA" magic
            frameNumber: frameNumber,
            timestamp: timestamp,
            isKeyframe: isKeyframe ? 1 : 0,
            dataSize: UInt32(data.count)
        )
        
        // Convert header to little-endian bytes (to match Go server)
        var packet = Data()
        packet.append(withUnsafeBytes(of: header.magic.littleEndian) { Data($0) })
        packet.append(withUnsafeBytes(of: header.frameNumber.littleEndian) { Data($0) })
        packet.append(withUnsafeBytes(of: header.timestamp.littleEndian) { Data($0) })
        packet.append(withUnsafeBytes(of: header.isKeyframe) { Data($0) })
        packet.append(withUnsafeBytes(of: header.dataSize.littleEndian) { Data($0) })
        packet.append(data)
        
        // 🧪 Network Simulation: Randomly reorder frames
        if NetworkSimulation.enabled && Double.random(in: 0...1) < NetworkSimulation.reorderProbability {
            let delay = Double.random(in: NetworkSimulation.minDelay...NetworkSimulation.maxDelay)
            print("🔀 REORDER: Frame #\(frameNumber) delayed by \(Int(delay*1000))ms")
            
            DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in
                self?.sendDatagramNow(packet: packet, frameNumber: frameNumber)
            }
        } else {
            // Send immediately (normal case)
            sendDatagramNow(packet: packet, frameNumber: frameNumber)
        }
    }
    
    // 🧩 Fragment large frames into multiple datagrams
    private func sendFragmentedFrame(data: Data, frameNumber: UInt64, timestamp: UInt64, isKeyframe: Bool) {
        let frameId = UInt32(frameNumber)
        let timestampMs = UInt32(timestamp)
        let flags: UInt8 = isKeyframe ? 0x01 : 0x00
        
        let chunkHeaderSize = MemoryLayout<ChunkHeader>.size // 15 bytes
        let maxPayloadPerChunk = FragmentationConfig.maxChunkSize - chunkHeaderSize
        
        let totalChunks = (data.count + maxPayloadPerChunk - 1) / maxPayloadPerChunk
        
        print("🧩 Fragmenting frame #\(frameNumber): \(data.count) bytes → \(totalChunks) chunks")
        
        // 📦 Send chunks in order (realistic network behavior)
        var chunkOrder = Array(0..<totalChunks)
        if NetworkSimulation.enabled {
            // Only shuffle if we want extreme chaos - normally chunks are sent sequentially
            // chunkOrder.shuffle()
            print("📦 Sending \(totalChunks) chunks in sequential order (realistic)")
        }
        
        for chunkIndex in chunkOrder {
            let startOffset = chunkIndex * maxPayloadPerChunk
            let endOffset = min(startOffset + maxPayloadPerChunk, data.count)
            let chunkData = data.subdata(in: startOffset..<endOffset)
            
            // Create chunk header
            var header = ChunkHeader(
                magic: 0x4348554E,  // "CHUN"
                frameId: frameId,
                chunkIndex: UInt16(chunkIndex),
                chunkCount: UInt16(totalChunks),
                flags: flags,
                timestampMs: timestampMs
            )
            
            // Build packet with little-endian header + chunk data
            var packet = Data()
            packet.append(withUnsafeBytes(of: header.magic.littleEndian) { Data($0) })
            packet.append(withUnsafeBytes(of: header.frameId.littleEndian) { Data($0) })
            packet.append(withUnsafeBytes(of: header.chunkIndex.littleEndian) { Data($0) })
            packet.append(withUnsafeBytes(of: header.chunkCount.littleEndian) { Data($0) })
            packet.append(withUnsafeBytes(of: header.flags) { Data($0) })
            packet.append(withUnsafeBytes(of: header.timestampMs.littleEndian) { Data($0) })
            packet.append(chunkData)
            
            // Send chunk with aggressive reordering simulation
            if NetworkSimulation.enabled && Double.random(in: 0...1) < FragmentationConfig.chunkReorderProbability {
                let delay = Double.random(in: 0.1...FragmentationConfig.maxChunkDelay)
                print("🔀 CHUNK REORDER: Frame #\(frameId) chunk \(chunkIndex+1)/\(totalChunks) delayed by \(Int(delay*1000))ms")
                
                DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in
                    self?.sendChunkNow(packet: packet, frameId: frameId, chunkIndex: chunkIndex, totalChunks: totalChunks)
                }
            } else {
                print("📤 Sending chunk \(chunkIndex+1)/\(totalChunks) for frame #\(frameId)")
                sendChunkNow(packet: packet, frameId: frameId, chunkIndex: chunkIndex, totalChunks: totalChunks)
            }
        }
    }
    
    // 🧩 Helper function to send a chunk
    private func sendChunkNow(packet: Data, frameId: UInt32, chunkIndex: Int, totalChunks: Int) {
        connection?.send(content: packet, completion: .contentProcessed { error in
            if let error = error {
                print("❌ Chunk failed: Frame #\(frameId) chunk \(chunkIndex+1)/\(totalChunks) - \(error)")
            } else {
                print("✅ Chunk sent: Frame #\(frameId) chunk \(chunkIndex+1)/\(totalChunks) (\(packet.count) bytes)")
            }
        })
    }
    
    // 🧪 Helper function to actually send the datagram
    private func sendDatagramNow(packet: Data, frameNumber: UInt64) {
        print("🔍 DEBUG: Attempting to send DATAGRAM frame: \(packet.count) bytes")
        connection?.send(content: packet, completion: .contentProcessed { error in
            if let error = error {
                print("❌ Datagram failed: \(error)")
            } else {
                print("✅ Frame #\(frameNumber) sent via DATAGRAM (\(packet.count) bytes)")
            }
        })
    }
    
    private func sendControlMessage(type: String, data: [String: Any]) {
        let message: [String: Any] = [
            "type": type,
            "timestamp": CACurrentMediaTime() * 1000,
            "data": data
        ]
        
        guard let jsonData = try? JSONSerialization.data(withJSONObject: message) else {
            print("❌ Failed to serialize control message")
            return
        }
        
        // Send as control datagram with marker
        var controlPacket = Data([0xFF]) // Control message marker
        controlPacket.append(jsonData)
        
        connection?.send(content: controlPacket, completion: .contentProcessed { error in
            if let error = error {
                print("❌ Failed to send control message: \(error)")
            } else {
                print("📤 Sent control message: \(type)")
            }
        })
    }
}

// MARK: - Data Structures
struct VideoFrameHeader {
    let frameNumber: UInt64
    let timestamp: UInt64
    let isKeyframe: UInt8
    let dataSize: UInt32
}

struct StreamFrameHeader {
    let magic: UInt32        // 0x4B455946 = "KEYF"
    let frameNumber: UInt64
    let timestamp: UInt64
    let dataSize: UInt32
    let isKeyframe: UInt8
}

struct DatagramFrameHeader {
    let magic: UInt32        // 0x44415441 = "DATA"
    let frameNumber: UInt64
    let timestamp: UInt64
    let isKeyframe: UInt8
    let dataSize: UInt32
}

// 🧩 Fragmentation Header (15 bytes)
struct ChunkHeader {
    let magic: UInt32        // 0x4348554E = "CHUN"
    let frameId: UInt32      // Unique frame identifier
    let chunkIndex: UInt16   // 0-based chunk index
    let chunkCount: UInt16   // Total chunks for this frame
    let flags: UInt8         // Keyframe bit (0x01), future use
    let timestampMs: UInt32  // Timestamp in milliseconds
}

// MARK: - Main
print("🌟 Simple Video Streaming Client")
print("================================")
print("This client sends mock video frames to test the QUIC video streaming system")
print("📡 30fps mock H.264 frames")
print("🔑 Keyframes every 2 seconds") 
print("📊 Realistic 16-inch MacBook Pro frame sizes (keyframes: ~150KB, deltas: ~25KB)")
print("🧪 Optional network simulation (reordering, delays)")
print("🧩 Smart fragmentation for large frames (1400-byte chunks)")
print("")

let client = SimpleVideoClient()
client.startStreaming()

// Keep running
RunLoop.main.run()
