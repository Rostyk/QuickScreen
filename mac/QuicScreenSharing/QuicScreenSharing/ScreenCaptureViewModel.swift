//
//  Untitled.swift
//  QuicScreenSharing
//
//  Created by Rostyslav Stepanyak on 9/19/25.
//

import SwiftUI
import AVFoundation

@MainActor
final class ScreenCaptureViewModel: ObservableObject {
    @Published var isCapturing = false
    private let manager = ScreenCaptureManager()

    func toggleCapture() {
        print("🎯 toggleCapture() called - isCapturing: \(isCapturing)")
        if isCapturing {
            print("🛑 Stopping capture...")
            manager.stop()
            isCapturing = false
        } else {
            print("▶️ Starting capture...")
            Task {
                do {
                    try await manager.start()
                    print("✅ Capture started successfully")
                    isCapturing = true
                } catch {
                    print("❌ Failed to start capture: \(error)")
                    isCapturing = false
                }
            }
        }
    }
}
