import Foundation
import Network
import CoreVideo
import VideoToolbox
import CoreMedia
import QuartzCore

final class QuicTransportPersistent {
    private let queue = DispatchQueue(label: "quic.video.client")
    private var connection: NWConnection?
    private var isConnected = false
    
    // Video compression
    private var compressionSession: VTCompressionSession?
    private var frameCounter: UInt64 = 0
    private var lastFrameTime: CFTimeInterval = 0
    private var droppedFrameCount: Int = 0
    
    // Frame sending queue
    private let sendingQueue = DispatchQueue(label: "quic.sending", qos: .userInitiated)
    
    // Dedicated encoding queue to prevent blocking SCStream output queue
    private let encodeQueue = DispatchQueue(label: "quic.encoding", qos: .userInitiated)
    
    // In-flight bytes tracking for flow control
    private var inFlightBytes: Int = 0
    private let inFlightLock = DispatchQueue(label: "quic.inflight.lock")
    private let maxInFlightBytes = 2 * 1024 * 1024 // 2 MB threshold
    
    // Encode queue depth tracking to prevent encoder backpressure
    private var pendingEncodes: Int = 0
    private let maxPendingEncodes = 3 // Limit concurrent encodes
    
    // Frame header structure (matches Rust server)
    private struct StreamFrameHeader {
        let magic: UInt32 = 0x53545246        // "STRF" - Stream Frame
        let frameNumber: UInt64
        let timestamp: UInt64                 // Timestamp in milliseconds
        let frameType: UInt8                  // 0x01 = keyframe, 0x00 = delta
        let dataSize: UInt32                  // Frame data size in bytes
    }
    
    init() {
        setupVideoCompression()
        // Don't auto-connect in init - wait for explicit connection
    }
    
    deinit {
        disconnect()
        cleanupVideoCompression()
    }
    
    // MARK: - Simple Direct QUIC Connection
    
    func connect() {
        print("🔗 Setting up direct QUIC connection...")
        
        // Create QUIC parameters WITHOUT TLS wrapping
        let quicOptions = NWProtocolQUIC.Options(alpn: ["hq-interop"])
        
        // Disable certificate verification for AWS testing (localhost cert on remote server)
        sec_protocol_options_set_verify_block(quicOptions.securityProtocolOptions, { _, _, completion in
            print("🔐 QUIC verification - accepting AWS certificate (testing mode)")
            completion(true)
        }, DispatchQueue.main)
        
        // Create parameters with QUIC directly, no TLS layer
        let params = NWParameters(quic: quicOptions)
        
        // Create direct connection to AWS server
        self.connection = NWConnection(
            host: .name("51.21.152.112", nil),
            port: .init(rawValue: 8443)!,
            using: params
        )
        
        connection?.stateUpdateHandler = { [weak self] state in
            print("🔗 Connection state: \(state)")
            switch state {
            case .ready:
                print("🔗 ✅ QUIC connection established!")
                self?.isConnected = true
                
            case .waiting(let error):
                print("⏳ Connection waiting: \(error)")
                
            case .failed(let error):
                print("❌ Connection failed: \(error)")
                self?.isConnected = false
                
            case .cancelled:
                print("🔌 Connection cancelled")
                self?.isConnected = false
                
            default:
                print("🔄 Connection state: \(state)")
            }
        }
        
        connection?.start(queue: self.queue)
    }
    
    // MARK: - Public Interface
    
    func disconnect() {
        connection?.cancel()
        connection = nil
        isConnected = false
    }
    
    // MARK: - Video Frame Sending (Using NWConnection from group)
    
    func sendTestVideoFrame() {
        print("🚨 DEBUG: sendTestVideoFrame() CALLED!")
           
           guard isConnected else {
               print("❌ Cannot send frame - not connected")
               return
           }
           
           print("🚨 DEBUG: Passed guard check, creating frame...")
        
        frameCounter += 1
        print("🎥 Sending test video frame #\(frameCounter) using NWConnection from group...")
        
        // Create test video data
        let testVideoData = Data([
            0x00, 0x00, 0x00, 0x01, // Start code
            0x67, 0x42, 0x00, 0x1E, // SPS
            0x00, 0x00, 0x00, 0x01, // Start code
            0x68, 0xCE, 0x3C, 0x80, // PPS
            0x00, 0x00, 0x00, 0x01, // Start code
            0x65, 0x88, 0x84, 0x00, // IDR frame
            0xFF, 0xE1, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD,
            0xEF, 0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD
        ])
        
        // Create frame header
        let header = StreamFrameHeader(
            frameNumber: frameCounter,
            timestamp: UInt64(Date().timeIntervalSince1970 * 1000),
            frameType: frameCounter % 30 == 1 ? 0x01 : 0x00,
            dataSize: UInt32(testVideoData.count)
        )
        
        // Serialize header
        var headerData = Data()
        headerData.append(withUnsafeBytes(of: header.magic.littleEndian) { Data($0) })
        headerData.append(withUnsafeBytes(of: header.frameNumber.littleEndian) { Data($0) })
        headerData.append(withUnsafeBytes(of: header.timestamp.littleEndian) { Data($0) })
        headerData.append(withUnsafeBytes(of: header.frameType) { Data($0) })
        headerData.append(withUnsafeBytes(of: header.dataSize.littleEndian) { Data($0) })
        
        // Combine header + frame data
        var completeFrame = headerData
        completeFrame.append(testVideoData)
        
        let frameType = header.frameType == 0x01 ? "KEYFRAME" : "delta"
        print("🎥 Frame #\(frameCounter): \(completeFrame.count) bytes (\(frameType))")
        print("🎥 Magic: 0x\(String(format: "%08X", header.magic))")
        print("🎥 First 16 bytes: \(completeFrame.prefix(16).map { String(format: "%02X", $0) }.joined(separator: " "))")
        
        // Use the same stream ID for all frames (persistent stream approach)
        guard let conn = connection, isConnected else {
            print("❌ No connection available")
            return
        }
        
        // Capture the frame data to avoid closure issues
        let frameDataToSend = completeFrame
        let currentFrameNumber = frameCounter
        
        print("🔗 📤 About to send \(frameDataToSend.count) bytes on stream 0")
        print("🔗 📤 First 16 bytes: \(frameDataToSend.prefix(16).map { String(format: "%02X", $0) }.joined(separator: " "))")
        
        // Always use the same stream context (stream 0) for all frames
        let streamContext = NWConnection.ContentContext(identifier: "VideoStream0")
        
        // Send frame data without closing the stream (isComplete: false)
        conn.send(content: frameDataToSend, contentContext: streamContext, isComplete: false, completion: .contentProcessed { error in
            if let error = error {
                print("❌ Failed to send frame: \(error)")
            } else {
                print("✅ Frame #\(currentFrameNumber) sent successfully (\(frameDataToSend.count) bytes)")
            }
        })
    }
    
    private func handleIncomingStream(_ streamConn: NWConnection) {
        print("📥 Received incoming stream from server")
        
        streamConn.stateUpdateHandler = { state in
            print("📥 Incoming stream state: \(state)")
        }
        
        streamConn.start(queue: queue)
        
        streamConn.receive(minimumIncompleteLength: 1, maximumLength: 64 * 1024) { data, _, isComplete, error in
            if let data = data {
                print("📥 Incoming stream data: \(data.count) bytes")
            }
            if isComplete || error != nil {
                print("📥 Incoming stream completed")
            }
        }
    }
    
    // MARK: - Video Compression Setup
    
    func setupVideoCompression(width: Int32 = 1920, height: Int32 = 1080) {
        print("🎥 Setting up compression session: \(width)x\(height)")
        let status = VTCompressionSessionCreate(
            allocator: nil,
            width: width,
            height: height,
            codecType: kCMVideoCodecType_H264,
            encoderSpecification: nil,
            imageBufferAttributes: nil,
            compressedDataAllocator: nil,
            outputCallback: compressionOutputCallback,
            refcon: Unmanaged.passUnretained(self).toOpaque(),
            compressionSessionOut: &compressionSession
        )
        
        // Reset frame counter and avcC flag when creating new session
        frameCounter = 0
        avccSent = false
        
        guard status == noErr, let session = compressionSession else {
            print("❌ Failed to create compression session: \(status)")
            return
        }
        
        // Anti-stutter settings optimized to prevent keyframe encoding delays
        VTSessionSetProperty(session, key: kVTCompressionPropertyKey_RealTime, value: kCFBooleanTrue)
        VTSessionSetProperty(session, key: kVTCompressionPropertyKey_ExpectedFrameRate, value: 30 as CFNumber)
        
        // CRITICAL: Reduce keyframe frequency to minimize encoding spikes
        // Large keyframes (150-190KB) are causing 200ms encoding delays
        VTSessionSetProperty(session, key: kVTCompressionPropertyKey_MaxKeyFrameInterval, value: 120 as CFNumber) // 4 seconds - balance startup vs encoding load
        
        // Higher bitrate for 1080p resolution and better text readability
        VTSessionSetProperty(session, key: kVTCompressionPropertyKey_AverageBitRate, value: 3_000_000 as CFNumber) // 3 Mbps for 1080p
        
        // Higher data rate limits for better quality
        let dataRateLimits: [NSNumber] = [4_000_000, 1] // 4 Mbps max, 1 second window
        VTSessionSetProperty(session, key: kVTCompressionPropertyKey_DataRateLimits, value: dataRateLimits as CFArray)
        
        // Higher quality setting for better text clarity
        VTSessionSetProperty(session, key: kVTCompressionPropertyKey_Quality, value: 0.8 as CFNumber) // 0.8 = higher quality for text
        
        // Use baseline profile for consistent, predictable encoding
        VTSessionSetProperty(session, key: kVTCompressionPropertyKey_ProfileLevel, value: kVTProfileLevel_H264_Baseline_AutoLevel)
        
        // Minimize keyframe generation and disable frame reordering for low latency
        VTSessionSetProperty(session, key: kVTCompressionPropertyKey_AllowFrameReordering, value: kCFBooleanFalse)
        
        // CRITICAL: Limit maximum frame delay to prevent encoding queue buildup
        VTSessionSetProperty(session, key: kVTCompressionPropertyKey_MaxFrameDelayCount, value: 0 as CFNumber) // No delay buffering
        
        VTCompressionSessionPrepareToEncodeFrames(session)
        print("✅ Video compression session initialized - ready for keyframe-first streaming")
    }
    
    // MARK: - Compression Callback
    
    private let compressionOutputCallback: VTCompressionOutputCallback = { refcon, sourceFrameRefcon, status, infoFlags, sampleBuffer in
        guard let refcon = refcon else {
            print("❌ Compression callback: No refcon")
            return
        }
        
        let transport = Unmanaged<QuicTransportPersistent>.fromOpaque(refcon).takeUnretainedValue()
        
        // CRITICAL FIX: Release the retained pixel buffer passed in sourceFrameRefcon
        if let sourceRef = sourceFrameRefcon {
            Unmanaged<CVPixelBuffer>.fromOpaque(sourceRef).release()
        }
        
        guard status == noErr else {
            let errorName: String
            switch status {
            case -12902: errorName = "kVTVideoEncoderMalfunctionErr (encoder malfunction)"
            case -12903: errorName = "kVTVideoEncoderNotAvailableErr (encoder not available)"
            case -12904: errorName = "kVTCouldNotFindVideoEncoderErr (encoder not found)"
            case -12905: errorName = "kVTVideoEncoderAuthorizationErr (authorization error)"
            case -12210: errorName = "kVTFrameSiloInvalidTimeStampErr (invalid timestamp)"
            case -12211: errorName = "kVTFrameSiloInvalidTimeRangeErr (invalid time range)"
            default: errorName = "Unknown error"
            }
            print("❌ Compression callback error: \(status) (\(errorName))")
            return
        }
        
        guard let sampleBuffer = sampleBuffer else {
            print("❌ Compression callback: No sample buffer")
            return
        }
        
        transport.handleCompressedFrame(sampleBuffer)
    }
    
    private var avccSent = false
    
    private func sendAvccConfiguration(_ formatDescription: CMFormatDescription) {
        print("🔧 sendAvccConfiguration called")
        
        // Try to get avcC directly from format description
        let extensions = CMFormatDescriptionGetExtensions(formatDescription) as? [String: Any]
        print("🔧 Extensions: \(extensions?.keys.joined(separator: ", ") ?? "none")")
        
        if let extensions = extensions,
           let atoms = extensions[kCMFormatDescriptionExtension_SampleDescriptionExtensionAtoms as String] as? [String: Any] {
            print("🔧 Sample description atoms: \(atoms.keys.joined(separator: ", "))")
            
            if let avccData = atoms["avcC"] as? Data {
                print("🔧 Found avcC directly from format description: \(avccData.count) bytes")
                sendAvccToServer(avccData)
                return
            } else {
                print("🔧 No avcC found in atoms, trying fallback...")
            }
        } else {
            print("🔧 No sample description atoms found, trying fallback...")
        }
        
        // Fallback: construct avcC from SPS/PPS
        print("🔧 Trying to construct avcC from SPS/PPS...")
        var parameterSetCount: Int = 0
        var nalUnitHeaderLength: Int32 = 0
        
        let status = CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
            formatDescription, parameterSetIndex: 0, parameterSetPointerOut: nil,
            parameterSetSizeOut: nil, parameterSetCountOut: &parameterSetCount,
            nalUnitHeaderLengthOut: &nalUnitHeaderLength
        )
        
        print("🔧 Parameter set query status: \(status), count: \(parameterSetCount), NAL header length: \(nalUnitHeaderLength)")
        
        guard status == noErr && parameterSetCount >= 2 else {
            print("❌ Failed to get parameter sets for avcC construction: status=\(status), count=\(parameterSetCount)")
            return
        }
        
        // Extract SPS (index 0) and PPS (index 1)
        var spsPointer: UnsafePointer<UInt8>?
        var spsSize: Int = 0
        var ppsPointer: UnsafePointer<UInt8>?
        var ppsSize: Int = 0
        
        let spsStatus = CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
            formatDescription, parameterSetIndex: 0,
            parameterSetPointerOut: &spsPointer, parameterSetSizeOut: &spsSize,
            parameterSetCountOut: nil, nalUnitHeaderLengthOut: nil
        )
        
        let ppsStatus = CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
            formatDescription, parameterSetIndex: 1,
            parameterSetPointerOut: &ppsPointer, parameterSetSizeOut: &ppsSize,
            parameterSetCountOut: nil, nalUnitHeaderLengthOut: nil
        )
        
        guard spsStatus == noErr && ppsStatus == noErr,
              let sps = spsPointer, let pps = ppsPointer,
              spsSize > 3, ppsSize > 0 else {
            print("❌ Failed to extract SPS/PPS for avcC construction")
            return
        }
        
        // Construct avcC manually
        let spsData = Data(bytes: sps, count: spsSize)
        let ppsData = Data(bytes: pps, count: ppsSize)
        
        // Build AVCDecoderConfigurationRecord
        var avccData = Data()
        avccData.append(1) // configurationVersion
        avccData.append(spsData[1]) // AVCProfileIndication (profile_idc)
        avccData.append(spsData[2]) // profile_compatibility
        avccData.append(spsData[3]) // AVCLevelIndication (level_idc)
        avccData.append(0xFF) // lengthSizeMinusOne (3 = 4-byte lengths)
        avccData.append(0xE1) // numOfSequenceParameterSets (1 with reserved bits)
        avccData.append(contentsOf: withUnsafeBytes(of: UInt16(spsSize).bigEndian) { Data($0) })
        avccData.append(spsData)
        avccData.append(1) // numOfPictureParameterSets
        avccData.append(contentsOf: withUnsafeBytes(of: UInt16(ppsSize).bigEndian) { Data($0) })
        avccData.append(ppsData)
        
        print("🔧 Constructed avcC from SPS/PPS: \(avccData.count) bytes")
        sendAvccToServer(avccData)
    }
    
    private func sendAvccToServer(_ avccData: Data) {
        guard let connection = connection, isConnected else {
            print("❌ No connection available for avcC")
            return
        }
        
        // Create special header for avcC configuration
        let header = StreamFrameHeader(
            frameNumber: 0, // Special frame number for config
            timestamp: UInt64(Date().timeIntervalSince1970 * 1000),
            frameType: 0xFF, // Special type for avcC config
            dataSize: UInt32(avccData.count)
        )
        
        // Serialize header
        var headerData = Data()
        headerData.append(withUnsafeBytes(of: header.magic.littleEndian) { Data($0) })
        headerData.append(withUnsafeBytes(of: header.frameNumber.littleEndian) { Data($0) })
        headerData.append(withUnsafeBytes(of: header.timestamp.littleEndian) { Data($0) })
        headerData.append(withUnsafeBytes(of: header.frameType) { Data($0) })
        headerData.append(withUnsafeBytes(of: header.dataSize.littleEndian) { Data($0) })
        
        // Combine header + avcC data
        var completeConfig = headerData
        completeConfig.append(avccData)
        
        print("🔧 Sending avcC configuration: \(completeConfig.count) bytes")
        
        // Send configuration
        sendingQueue.async { [weak self] in
            guard let self = self else { return }
            
            connection.send(content: completeConfig, completion: .contentProcessed { error in
                if let error = error {
                    print("❌ Failed to send avcC: \(error)")
                } else {
                    print("✅ avcC configuration sent successfully")
                }
            })
        }
    }
    
    // Generate proper AVCC AU with verified 4-byte big-endian length prefixes
    private func generateAvccAU(from sampleBuffer: CMSampleBuffer) -> Data? {
        guard let dataBuffer = CMSampleBufferGetDataBuffer(sampleBuffer) else { return nil }
        
        var lengthAtOffset = 0
        var totalLength = 0
        var dataPointer: UnsafeMutablePointer<Int8>?
        
        guard CMBlockBufferGetDataPointer(dataBuffer, atOffset: 0, lengthAtOffsetOut: &lengthAtOffset,
                                          totalLengthOut: &totalLength, dataPointerOut: &dataPointer) == kCMBlockBufferNoErr,
              let basePointer = dataPointer else { return nil }
        
        var outputData = Data()
        var cursor = 0
        
        // Parse existing AVCC format and regenerate with verified big-endian lengths
        while cursor + 4 <= totalLength {
            // Read existing length (should be big-endian)
            var beLength: UInt32 = 0
            memcpy(&beLength, basePointer + cursor, 4)
            let nalLength = Int(CFSwapInt32BigToHost(beLength))
            cursor += 4
            
            guard cursor + nalLength <= totalLength else {
                print("❌ Invalid NAL length in AVCC: \(nalLength)")
                return nil
            }
            
            // Write verified big-endian length
            var verifiedBELength = CFSwapInt32HostToBig(UInt32(nalLength))
            outputData.append(Data(bytes: &verifiedBELength, count: 4))
            
            // Copy NAL unit data
            outputData.append(Data(bytes: basePointer + cursor, count: nalLength))
            cursor += nalLength
        }
        
        if cursor != totalLength {
            print("❌ AVCC parsing mismatch: cursor=\(cursor), total=\(totalLength)")
            return nil
        }
        
        return outputData
    }
    
    private func handleCompressedFrame(_ sampleBuffer: CMSampleBuffer) {
        // Move heavy frame processing off main thread
        queue.async { [weak self] in
            self?.processCompressedFrame(sampleBuffer)
        }
    }
    
    private func processCompressedFrame(_ sampleBuffer: CMSampleBuffer) {
        guard let connection = connection, isConnected else {
            print("❌ No connection available for compressed frame")
            return
        }
        
        // Check connection state
        if connection.state != .ready {
            print("❌ Connection not ready: \(connection.state)")
            return
        }
        
        // Send avcC configuration once at the beginning
        if !avccSent, let formatDescription = CMSampleBufferGetFormatDescription(sampleBuffer) {
            print("🔧 Attempting to send avcC configuration...")
            sendAvccConfiguration(formatDescription)
            avccSent = true
        } else if !avccSent {
            print("❌ No format description available for avcC")
        }
        
        // Extract compressed data from the sample buffer (this is already in AVCC format)
        guard let dataBuffer = CMSampleBufferGetDataBuffer(sampleBuffer) else {
            print("❌ Failed to get data buffer from sample")
            return
        }
        
        // Get the compressed data
        var length: Int = 0
        var dataPointer: UnsafeMutablePointer<Int8>?
        let status = CMBlockBufferGetDataPointer(dataBuffer, atOffset: 0, lengthAtOffsetOut: nil, totalLengthOut: &length, dataPointerOut: &dataPointer)
        
        guard status == noErr, let pointer = dataPointer, length > 0 else {
            print("❌ Failed to get compressed data pointer")
            return
        }
        
        // Generate proper AVCC format with verified 4-byte big-endian length prefixes
        let compressedData = generateAvccAU(from: sampleBuffer) ?? Data(bytes: pointer, count: length)
        
        // Debug: Log first few bytes of AVCC data for validation
        if frameCounter <= 3 || frameCounter % 30 == 0 {
            let preview = compressedData.prefix(16).map { String(format: "%02X", $0) }.joined(separator: " ")
            print("🔍 AVCC frame #\(frameCounter) preview: \(preview)")
        }
        
        // Increment frame counter
        frameCounter += 1
        
        // Determine if this is a keyframe by checking NAL unit types in AVCC format
        var actuallyKeyframe = false
        var offset = 0
        
        while offset < compressedData.count - 4 {
            // Read 4-byte length prefix (big-endian)
            let nalLength = Int(compressedData[offset]) << 24 |
                           Int(compressedData[offset + 1]) << 16 |
                           Int(compressedData[offset + 2]) << 8 |
                           Int(compressedData[offset + 3])
            
            if nalLength <= 0 || offset + 4 + nalLength > compressedData.count {
                break
            }
            
            // Check NAL unit type (first byte after length prefix)
            if offset + 4 < compressedData.count {
                let nalType = compressedData[offset + 4] & 0x1F
                if nalType == 5 { // IDR frame
                    actuallyKeyframe = true
                    break
                }
            }
            
            offset += 4 + nalLength
        }
        
        // Send all frames - browser can handle starting with P-frames
        
        // Create frame header
        let header = StreamFrameHeader(
            frameNumber: frameCounter,
            timestamp: UInt64(Date().timeIntervalSince1970 * 1000),
            frameType: actuallyKeyframe ? 0x01 : 0x00,
            dataSize: UInt32(compressedData.count)
        )
        
        // Serialize header
        var headerData = Data()
        headerData.append(withUnsafeBytes(of: header.magic.littleEndian) { Data($0) })
        headerData.append(withUnsafeBytes(of: header.frameNumber.littleEndian) { Data($0) })
        headerData.append(withUnsafeBytes(of: header.timestamp.littleEndian) { Data($0) })
        headerData.append(withUnsafeBytes(of: header.frameType) { Data($0) })
        headerData.append(withUnsafeBytes(of: header.dataSize.littleEndian) { Data($0) })
        
        // Combine header + AVCC frame data (length-prefixed NALs)
        var completeFrame = headerData
        completeFrame.append(compressedData)
        
        let frameType = header.frameType == 0x01 ? "KEYFRAME" : "delta"
        let timestamp = CACurrentMediaTime()
        
        // Only log keyframes and every 30th frame to reduce spam
        if header.frameType == 0x01 || frameCounter % 30 == 0 {
            print("🎥 [\(String(format: "%.3f", timestamp))] Compressed Frame #\(frameCounter): \(completeFrame.count) bytes (\(frameType))")
        }
        
        // Send frame with in-flight bytes tracking and flow control
        sendingQueue.async { [weak self] in
            guard let self = self else { return }
            
            // CRITICAL FIX: Drop frame early if we have too many bytes outstanding
            var shouldDrop = false
            self.inFlightLock.sync {
                if self.inFlightBytes + completeFrame.count > self.maxInFlightBytes {
                    shouldDrop = true
                } else {
                    self.inFlightBytes += completeFrame.count
                }
            }
            
            if shouldDrop {
                print("⚠️ Dropping frame #\(header.frameNumber) - in-flight bytes exceed \(self.maxInFlightBytes / (1024*1024))MB threshold")
                return
            }
            
            // Let NWConnection manage the stream automatically
            let streamContext = NWConnection.ContentContext.defaultMessage
            
            connection.send(content: completeFrame, contentContext: streamContext, isComplete: false, completion: .contentProcessed { [weak self] error in
                guard let self = self else { return }
                
                // Decrement in-flight bytes regardless of success or failure
                self.inFlightLock.sync {
                    self.inFlightBytes = max(0, self.inFlightBytes - completeFrame.count)
                }
                
                if let error = error {
                    print("❌ Failed to send compressed frame: \(error)")
                } else {
                    // Success - frame delivered
                    let timestamp = CACurrentMediaTime()
                   // print("✅ [\(String(format: "%.3f", timestamp))] Compressed Frame sent successfully (\(completeFrame.count) bytes)")
                }
            })
        }
    }
    
    func sendFrame(_ pixelBuffer: CVPixelBuffer) {
        guard let session = compressionSession else {
            print("❌ No compression session available")
            return
        }
        
        guard isConnected else {
            print("❌ Not connected - skipping frame")
            return
        }
        
        // Frame timing analysis - be more lenient to allow natural frame timing variations
        let currentTime = CACurrentMediaTime()
        if lastFrameTime > 0 {
            let timeDelta = currentTime - lastFrameTime
            // Allow for natural variations and encoding complexity - 200ms threshold
            // At 30fps, normal frame time is ~33ms, but encoding can cause natural delays
            if timeDelta > 0.2 { // More than 200ms gap - likely a real issue
                droppedFrameCount += 1
                print("⚠️ Significant frame gap: \(Int(timeDelta * 1000))ms (expected ~33ms) - possible issue #\(droppedFrameCount)")
            }
        }
        lastFrameTime = currentTime
        
        frameCounter += 1
        let currentFrameNumber = frameCounter
        
        // Only log every 30th frame to reduce spam
        if currentFrameNumber % 30 == 0 {
            print("🎥 Frame #\(currentFrameNumber) (dropped: \(droppedFrameCount))")
        }
        
        // CRITICAL FIX: Check encoder backpressure before queuing more work
        if pendingEncodes >= maxPendingEncodes {
            print("⚠️ Dropping frame #\(currentFrameNumber) - encoder backpressure (\(pendingEncodes) pending)")
            return
        }
        
        // CRITICAL FIX: Retain the pixelBuffer for asynchronous encode
        // This prevents blocking SCStream's output queue
        let retained = Unmanaged.passRetained(pixelBuffer).toOpaque()
        
        // Create presentation timestamp
        let timestamp = CMTime(seconds: CACurrentMediaTime(), preferredTimescale: 600)
        
        // Track pending encodes to prevent backpressure
        pendingEncodes += 1
        
        // Dispatch encode to encodeQueue so SCStream outputQueue isn't blocked
        encodeQueue.async { [weak self] in
            guard let self = self else {
                // release retained buffer if we can't access self
                Unmanaged<CVPixelBuffer>.fromOpaque(retained).release()
                return
            }
            
            // Let VideoToolbox naturally decide when to generate keyframes
            // No forced keyframes - this reduces encoding complexity significantly
            
            // Compress the frame
            let status = VTCompressionSessionEncodeFrame(
                session,
                imageBuffer: pixelBuffer,
                presentationTimeStamp: timestamp,
                duration: .invalid,
                frameProperties: nil, // No forced properties
                sourceFrameRefcon: retained, // pass retained pointer to callback
                infoFlagsOut: nil
            )
            
            if status != noErr {
                print("❌ Failed to encode frame: \(status)")
                // release retained buffer on encode failure
                Unmanaged<CVPixelBuffer>.fromOpaque(retained).release()
            } else {
                // Only log every 20 frames to see if frames are being submitted
                if currentFrameNumber % 20 == 0 {
                    print("📹 Frame #\(currentFrameNumber) submitted for compression")
                }
            }
            
            // Decrement pending encodes count (done in encode queue)
            self.pendingEncodes = max(0, self.pendingEncodes - 1)
        }
    }
    
    private func cleanupVideoCompression() {
        if let session = compressionSession {
            VTCompressionSessionCompleteFrames(session, untilPresentationTimeStamp: .invalid)
            VTCompressionSessionInvalidate(session)
            compressionSession = nil
        }
    }
}
