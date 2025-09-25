/*
 * WebTransport Video Relay Server
 * Following WebTransport Best Practices (Based on Research)
 * 
 * Key Principles:
 * 1. Use WebTransport's natural flow control instead of simulated buffers
 * 2. Monitor actual send queue length, not artificial buffer utilization
 * 3. Let write_all() blocking behavior indicate congestion
 * 4. Prioritize keyframes over delta frames during congestion
 */

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{RwLock, Semaphore};
use tokio::time::sleep;
use tracing::{info, warn, error};
use wtransport::{Endpoint, ServerConfig, Connection};
use wtransport::stream::SendStream;
use async_trait::async_trait;

const MAGIC_STREAM_FRAME: u32 = 0x53545246; // "STRF"
const FRAME_HEADER_SIZE: usize = 25;

// WebTransport flow control constants (based on research)
const MAX_PENDING_FRAMES: usize = 10; // Maximum frames in send queue per client
const CONGESTION_THRESHOLD: usize = 8; // Start dropping frames when queue > 8
const SEND_TIMEOUT: Duration = Duration::from_millis(100); // Max time to wait for send

#[derive(Debug, Clone)]
struct VideoFrame {
    frame_number: u64,
    timestamp: u64,
    data: Vec<u8>,
    frame_type: u8, // 0x01 = keyframe, 0x41 = delta, 0xFF = avcC
    priority: u8,   // 0 = high (keyframes), 1 = medium (delta), 2 = low
}

impl VideoFrame {
    fn new(frame_number: u64, timestamp: u64, data: Vec<u8>, frame_type: u8) -> Self {
        let priority = match frame_type {
            0x01 | 0xFF => 0, // Keyframes and avcC - highest priority
            0x41 => if data.len() > 10000 { 2 } else { 1 }, // Large delta = low priority
            _ => 1, // Default medium priority
        };
        
        Self {
            frame_number,
            timestamp,
            data,
            frame_type,
            priority,
        }
    }
}

// WebTransport-based client management (following best practices)
#[derive(Debug)]
struct WebTransportClient {
    id: String,
    pending_frames: usize, // Actual frames in WebTransport send queue
    dropped_frames: u64,
    last_keyframe: Option<u64>,
    connection_quality: f32, // Based on actual queue congestion
}

impl WebTransportClient {
    fn new(id: String) -> Self {
        Self {
            id,
            pending_frames: 0,
            dropped_frames: 0,
            last_keyframe: None,
            connection_quality: 1.0,
        }
    }

    // WebTransport congestion-aware frame acceptance
    fn should_accept_frame(&self, frame: &VideoFrame) -> bool {
        // Always accept critical frames
        if frame.frame_type == 0x01 || frame.frame_type == 0xFF {
            return true;
        }
        
        // Use actual WebTransport queue state for decisions
        if self.pending_frames >= MAX_PENDING_FRAMES {
            // Queue full - only accept keyframes
            false
        } else if self.pending_frames >= CONGESTION_THRESHOLD {
            // Congestion detected - be selective
            match frame.priority {
                0 => true,  // Keyframes - always accept
                1 => self.pending_frames <= CONGESTION_THRESHOLD, // Small deltas
                _ => false, // Large deltas - drop first
            }
        } else {
            // No congestion - accept all
            true
        }
    }

    fn frame_queued(&mut self) {
        self.pending_frames = self.pending_frames.saturating_add(1);
    }

    fn frame_sent(&mut self) {
        self.pending_frames = self.pending_frames.saturating_sub(1);
        
        // Update connection quality based on actual queue state
        let congestion_ratio = self.pending_frames as f32 / MAX_PENDING_FRAMES as f32;
        self.connection_quality = (1.0 - congestion_ratio).max(0.1);
    }

    fn frame_dropped(&mut self) {
        self.dropped_frames += 1;
        self.connection_quality = (self.connection_quality * 0.95).max(0.1);
    }
}

struct WebTransportVideoRelay {
    clients: Arc<RwLock<HashMap<String, WebTransportClient>>>,
    frame_buffer: Arc<RwLock<Vec<VideoFrame>>>,
    global_semaphore: Arc<Semaphore>,
    stats: Arc<RwLock<RelayStats>>,
}

#[derive(Debug, Default)]
struct RelayStats {
    start_time: Option<Instant>,
    total_frames: u64,
    keyframes: u64,
    delta_frames: u64,
    total_data: u64,
    global_drops: u64,
    client_drops: u64,
}

impl WebTransportVideoRelay {
    fn new() -> Self {
        Self {
            clients: Arc::new(RwLock::new(HashMap::new())),
            frame_buffer: Arc::new(RwLock::new(Vec::new())),
            global_semaphore: Arc::new(Semaphore::new(100)), // Prevent server overload
            stats: Arc::new(RwLock::new(RelayStats::default())),
        }
    }

    // WebTransport natural flow control (following best practices)
    async fn send_frame_to_client(
        &self, 
        client_id: &str, 
        frame: &VideoFrame, 
        mut send_stream: SendStream
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        
        // Check if client can accept frame based on actual queue state
        let should_send = {
            let mut clients = self.clients.write().await;
            if let Some(client) = clients.get_mut(client_id) {
                if client.should_accept_frame(frame) {
                    client.frame_queued(); // Track that we're queuing a frame
                    true
                } else {
                    client.frame_dropped();
                    false
                }
            } else {
                false
            }
        };

        if !should_send {
            return Ok(()); // Frame dropped due to congestion
        }

        // Prepare frame data
        let frame_buffer = self.prepare_frame_data(frame).await;

        // Use WebTransport's natural backpressure - write_all() will block if congested
        let send_result = tokio::time::timeout(SEND_TIMEOUT, async {
            send_stream.write_all(&frame_buffer).await?;
            send_stream.flush().await
        }).await;

        // Update client state based on actual send result
        {
            let mut clients = self.clients.write().await;
            if let Some(client) = clients.get_mut(client_id) {
                match send_result {
                    Ok(Ok(())) => {
                        client.frame_sent(); // Successfully sent
                    }
                    Ok(Err(_)) | Err(_) => {
                        // Send failed or timed out - indicates real congestion
                        client.frame_dropped();
                        client.pending_frames = client.pending_frames.saturating_sub(1);
                        return Err("Send timeout or error - network congestion detected".into());
                    }
                }
            }
        }

        Ok(())
    }

    async fn prepare_frame_data(&self, frame: &VideoFrame) -> Vec<u8> {
        let mut frame_buffer = Vec::with_capacity(FRAME_HEADER_SIZE + frame.data.len());
        
        // Frame header
        frame_buffer.extend_from_slice(&MAGIC_STREAM_FRAME.to_le_bytes());
        frame_buffer.extend_from_slice(&frame.frame_number.to_le_bytes());
        frame_buffer.extend_from_slice(&frame.timestamp.to_le_bytes());
        frame_buffer.extend_from_slice(&(frame.data.len() as u32).to_le_bytes());
        frame_buffer.push(frame.frame_type);
        
        // Frame data
        frame_buffer.extend_from_slice(&frame.data);
        
        frame_buffer
    }

    async fn handle_webtransport_session(&self, connection: Connection) {
        let client_id = format!("wt_{}", connection.remote_address());
        info!("🚀 WebTransport client connected: {}", client_id);

        // Register client
        {
            let mut clients = self.clients.write().await;
            clients.insert(client_id.clone(), WebTransportClient::new(client_id.clone()));
        }

        // Handle client session
        loop {
            match connection.accept_uni().await {
                Ok(send_stream) => {
                    let relay = self.clone();
                    let client_id = client_id.clone();
                    
                    tokio::spawn(async move {
                        relay.process_client_stream(client_id, send_stream).await;
                    });
                }
                Err(e) => {
                    warn!("WebTransport stream error: {}", e);
                    break;
                }
            }
        }

        // Cleanup client
        {
            let mut clients = self.clients.write().await;
            clients.remove(&client_id);
        }
        
        info!("🏁 WebTransport client disconnected: {}", client_id);
    }

    async fn process_client_stream(&self, client_id: String, send_stream: SendStream) {
        // Send frames from buffer to client using WebTransport flow control
        let frame_buffer = self.frame_buffer.read().await;
        
        for frame in frame_buffer.iter() {
            if let Err(e) = self.send_frame_to_client(&client_id, frame, send_stream).await {
                warn!("Failed to send frame to {}: {}", client_id, e);
                break;
            }
        }
    }

    async fn print_stats(&self) {
        let clients = self.clients.read().await;
        let stats = self.stats.read().await;
        
        if let Some(start_time) = stats.start_time {
            let runtime = start_time.elapsed().as_secs_f32();
            let fps = stats.total_frames as f32 / runtime;
            let mbps = (stats.total_data as f32 * 8.0) / (runtime * 1_000_000.0);
            
            let avg_quality: f32 = if clients.is_empty() {
                1.0
            } else {
                clients.values().map(|c| c.connection_quality).sum::<f32>() / clients.len() as f32
            };
            
            let avg_pending: f32 = if clients.is_empty() {
                0.0
            } else {
                clients.values().map(|c| c.pending_frames).sum::<usize>() as f32 / clients.len() as f32
            };

            info!("📊 === WEBTRANSPORT RELAY STATS ===");
            info!("⏱️  Runtime: {:.1}s", runtime);
            info!("🎬 Frames: {} ({:.1} fps)", stats.total_frames, fps);
            info!("🔑 Keyframes: {} | 📦 Delta: {}", stats.keyframes, stats.delta_frames);
            info!("💾 Data: {:.2} MB ({:.2} Mbps)", stats.total_data as f32 / 1_000_000.0, mbps);
            info!("👥 Clients: {}", clients.len());
            info!("🎯 Global Drops: {} | Client Drops: {}", stats.global_drops, stats.client_drops);
            info!("🔗 Avg Connection Quality: {:.2}", avg_quality);
            info!("📊 Avg Pending Frames: {:.1}", avg_pending);
            info!("===============================");
        }
    }
}

impl Clone for WebTransportVideoRelay {
    fn clone(&self) -> Self {
        Self {
            clients: Arc::clone(&self.clients),
            frame_buffer: Arc::clone(&self.frame_buffer),
            global_semaphore: Arc::clone(&self.global_semaphore),
            stats: Arc::clone(&self.stats),
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    info!("🚀 Starting WebTransport Video Relay (Following Best Practices)");
    info!("🌟 ================================");
    info!("🔗 QUIC (macOS) on: localhost:8443");
    info!("🚀 WebTransport on: localhost:4433");
    info!("🌐 HTTP server on: localhost:3000");
    info!("🎯 Natural flow control enabled");
    info!("⚡ WebTransport congestion awareness");
    info!("🧠 Queue-based backpressure");
    info!("🌟 ================================");

    let relay = WebTransportVideoRelay::new();

    // Initialize stats
    {
        let mut stats = relay.stats.write().await;
        stats.start_time = Some(Instant::now());
    }

    // Start stats reporting
    let stats_relay = relay.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        loop {
            interval.tick().await;
            stats_relay.print_stats().await;
        }
    });

    // Start WebTransport server
    let config = ServerConfig::builder()
        .with_bind_default(4433)
        .with_certificate_chain_and_key_pem(
            include_str!("../certs/cert.pem"),
            include_str!("../certs/key.pem")
        )?
        .build();

    let server = Endpoint::server(config)?;
    info!("🚀 ✅ WebTransport server listening on: 127.0.0.1:4433");

    // Accept connections
    while let Some(incoming) = server.accept().await {
        let relay = relay.clone();
        tokio::spawn(async move {
            match incoming.await {
                Ok(connection) => {
                    relay.handle_webtransport_session(connection).await;
                }
                Err(e) => {
                    error!("Connection failed: {}", e);
                }
            }
        });
    }

    Ok(())
}
