//
//  Untitled.swift
//  QuicScreenSharing
//
//  Created by Rostyslav Stepanyak on 9/19/25.
//
import SwiftUI
import ScreenCaptureKit
import AVFoundation
import CoreMedia
import CoreVideo

final class ScreenCaptureManager: NSObject, SCStreamOutput {
    private var stream: SCStream?
    private var filter: SCContentFilter?
    private var config = SCStreamConfiguration()
    private let outputQueue = DispatchQueue(label: "ScreenCapture.OutputQueue")
    private let quicTransport = QuicTransportPersistent()

    private(set) var isCapturing = false

    // Configure and start capture of the main display.
    func start() async throws {
        print("🚀 ScreenCaptureManager.start() called")
        guard !isCapturing else { 
            print("⚠️ Already capturing, returning early")
            return 
        }
        
        // 1) Discover shareable content and pick the main (first) display.
        let content = try await SCShareableContent.current
        guard let display = content.displays.first else {
            throw NSError(domain: "ScreenCapture", code: 1, userInfo: [NSLocalizedDescriptionKey: "No displays available to capture."])
        }
        
        print("📺 Display found: \(display.width)x\(display.height)")

        // Connect to QUIC server first
        print("🔗 Connecting to QUIC server...")
        quicTransport.connect()

        // 2) Build a content filter that captures the chosen display.
        //    You can exclude individual windows/apps if desired.
        let filter = SCContentFilter(display: display,
                                     excludingApplications: [],
                                     exceptingWindows: [])
        self.filter = filter

        // 3) Configure streaming parameters.
        let cfg = SCStreamConfiguration()
        cfg.capturesAudio = false
        cfg.pixelFormat = kCVPixelFormatType_32BGRA
        // Target 15 fps for stable real-time streaming (more time for encoding)
        cfg.minimumFrameInterval = CMTime(value: 1, timescale: 30) // 30 FPS for smooth video
        
        
        // Scale to 1080p for better text readability - good balance of quality and performance
        let targetWidth = 1920
        let targetHeight = 1080
        let aspectRatio = Double(display.width) / Double(display.height)
        
        if aspectRatio > (Double(targetWidth) / Double(targetHeight)) {
            cfg.width = targetWidth
            cfg.height = Int(Double(targetWidth) / aspectRatio)
        } else {
            cfg.width = Int(Double(targetHeight) * aspectRatio)
            cfg.height = targetHeight
        }
        
        // Ensure even dimensions for H.264
        cfg.width = (cfg.width / 2) * 2
        cfg.height = (cfg.height / 2) * 2
        
        print("📏 Scaled resolution: \(display.width)x\(display.height) → \(cfg.width)x\(cfg.height) (optimized for encoding)")
        
        // CRITICAL FIX: Setup compression with the SAME resolution as ScreenCaptureKit
        // This ensures VideoToolbox and ScreenCaptureKit use matching dimensions
        quicTransport.setupVideoCompression(width: Int32(cfg.width), height: Int32(cfg.height))
        
        // If you want to capture the cursor:
        cfg.showsCursor = true
        
        // OPTIMIZATION: Small buffer for smooth flow while preventing excessive batching
        // Balance between frame drops (queueDepth=1) and burst accumulation (queueDepth=3+)
        cfg.queueDepth = 4 // Small buffer - prevents drops while minimizing batching

        self.config = cfg

        // 4) Create the stream and attach ourselves as the output.
        let stream = SCStream(filter: filter, configuration: cfg, delegate: self)
        self.stream = stream

        // Add a video output. (Use .audio too if you also capture audio.)
        try stream.addStreamOutput(self, type: .screen, sampleHandlerQueue: outputQueue)

        // 5) Start!
        try await stream.startCapture()
        isCapturing = true
    }

    func stop() {
        guard isCapturing else { return }
        Task {
            do {
                try await stream?.stopCapture()
            } catch {
                // Handle stop errors if needed
            }
            if let stream = stream {
                try? stream.removeStreamOutput(self, type: .screen)
            }
            self.stream = nil
            self.filter = nil
            self.isCapturing = false
            
            // Disconnect QUIC transport
            self.quicTransport.disconnect()
        }
    }

    // MARK: - SCStreamOutput

    func stream(_ stream: SCStream,
                didOutputSampleBuffer sampleBuffer: CMSampleBuffer,
                of type: SCStreamOutputType) {
        guard type == .screen else { return }

        // Validate and extract CVPixelBuffer from CMSampleBuffer.
        guard CMSampleBufferIsValid(sampleBuffer),
              let pixelBuffer = CMSampleBufferGetImageBuffer(sampleBuffer) else {
            return
        }

        // Hand off the frame for QUIC transport directly (VideoToolbox is already async)
        let timestamp = CACurrentMediaTime()
        //print("📹 [\(String(format: "%.3f", timestamp))] Captured frame: \(CVPixelBufferGetWidth(pixelBuffer))x\(CVPixelBufferGetHeight(pixelBuffer))")
        quicTransport.sendFrame(pixelBuffer)
    }

    // Optional: observe stream errors.
    func stream(_ stream: SCStream, didStopWithError error: Error) {
        // You might want to surface this to the UI.
        // print("Stream stopped with error: \(error)")
        isCapturing = false
    }
}

extension ScreenCaptureManager: SCStreamDelegate {

}
