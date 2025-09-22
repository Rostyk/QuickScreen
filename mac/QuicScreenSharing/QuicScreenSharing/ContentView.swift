//
//  ContentView.swift
//  QuicScreenSharing
//
//  Created by Rostyslav Stepanyak on 9/19/25.
//

import SwiftUI

struct ContentView: View {
    @StateObject private var vm = ScreenCaptureViewModel()

    var body: some View {
        VStack(spacing: 16) {
            Text("ScreenCaptureKit Demo")
                .font(.title)
            Button(vm.isCapturing ? "Stop Capture" : "Start Capture") {
                vm.toggleCapture()
            }
            .keyboardShortcut(.space, modifiers: [])
            .padding(.horizontal, 20)
            .padding(.vertical, 10)
            .background(vm.isCapturing ? Color.red.opacity(0.2) : Color.blue.opacity(0.2))
            .clipShape(RoundedRectangle(cornerRadius: 10))
            .animation(.default, value: vm.isCapturing)

            Text(vm.isCapturing ? "Capturing… sending CVPixelBuffer frames." : "Not capturing.")
                .foregroundStyle(.secondary)
        }
        .padding(24)
        .frame(minWidth: 360)
    }
}

#Preview {
    ContentView()
}
