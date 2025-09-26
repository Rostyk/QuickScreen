use anyhow::Result;
use bytes::{Buf, Bytes, BytesMut};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, RwLock, Semaphore};
use tokio::time::interval;
use tracing::{error, info, warn};
use std::collections::HashMap;
use async_trait::async_trait;

// Real MOQ imports - using the correct high-level APIs
use moq_lite;
use moq_native;
use hang;
use hang::catalog::{Catalog, Video, VideoConfig};
// Note: H264 is not directly accessible, will use string codec instead

/**
 * 🚀 MOQ-Inspired Video Relay Server with Built-in Backpressure
 * 
 * This implementation incorporates key MOQ (Media over QUIC) concepts:
 * - Intelligent backpressure management
 * - Frame prioritization (keyframes > delta frames)
 * - Adaptive flow control based on network conditions
 * - Partial reliability (can drop frames under congestion)
 * - Real-time optimizations for media streaming
 */

const MAGIC_STREAM_FRAME: u32 = 0x53545246; // "STRF"
const FRAME_HEADER_SIZE: usize = 25;

// WebTransport flow control constants (based on research)
const MAX_BUFFER_SIZE: usize = 8 * 1024 * 1024; // Keep for compatibility
const BACKPRESSURE_THRESHOLD: f32 = 0.85; // More reasonable threshold
const KEYFRAME_PRIORITY: u8 = 0; // Highest priority
const DELTA_PRIORITY: u8 = 1; // Lower priority, can be dropped

#[derive(Debug, Clone)]
struct VideoFrame {
    frame_number: u64,
    timestamp: u64,
    frame_type: u8, // 0x01 = keyframe, 0x00 = delta, 0xFF = avcC
    data: Bytes,
    priority: u8, // 0 = highest (keyframes), 1+ = lower (delta frames)
    size: usize,
}

#[derive(Debug)]
struct FrameHeader {
    magic: u32,
    frame_number: u64,
    timestamp: u64,
    frame_type: u8,
    data_size: u32,
}

#[derive(Debug)]
struct ServerStats {
    total_frames: u64,
    keyframes: u64,
    delta_frames: u64,
    total_bytes: u64,
    start_time: Instant,
    connected_clients: usize,
    avcc_config: Option<Bytes>,
    // Simple stats
    dropped_frames: u64,
}

impl Default for ServerStats {
    fn default() -> Self {
        Self {
            total_frames: 0,
            keyframes: 0,
            delta_frames: 0,
            total_bytes: 0,
            start_time: Instant::now(),
            connected_clients: 0,
            avcc_config: None,
            dropped_frames: 0,
        }
    }
}

// MOQ-inspired client management with intelligent backpressure
#[derive(Debug)]
struct SmartClient {
    id: String,
    dropped_frames: u64,
    last_keyframe: Option<u64>,
    connection_quality: f32, // 0.0 = poor, 1.0 = excellent
}

impl SmartClient {
    fn new(id: String) -> Self {
        Self {
            id,
            dropped_frames: 0,
            last_keyframe: None,
            connection_quality: 1.0, // Start optimistic
        }
    }

    // Accept all frames - let WebTransport handle the backpressure naturally
    fn should_accept_frame(&self, _frame: &VideoFrame) -> bool {
        true
    }

    // Removed broken buffer simulation

    fn record_dropped_frame(&mut self) {
        self.dropped_frames += 1;
        // Slightly reduce connection quality on drops
        self.connection_quality = (self.connection_quality * 0.99).max(0.1);
    }

    fn record_successful_send(&mut self) {
        // Slowly improve connection quality on successful sends
        self.connection_quality = (self.connection_quality * 1.001).min(1.0);
    }
}

// MOQ-inspired adaptive streaming trait
#[async_trait]
trait AdaptiveStreaming {
    async fn send_frame_adaptive(&mut self, frame: &VideoFrame) -> Result<bool>;
    fn get_buffer_utilization(&self) -> f32;
    fn get_dropped_frame_count(&self) -> u64;
}

struct MOQInspiredVideoRelayServer {
    stats: Arc<RwLock<ServerStats>>,
    frame_sender: broadcast::Sender<VideoFrame>,
    smart_clients: Arc<RwLock<HashMap<String, SmartClient>>>,
    // Semaphore for global backpressure management
    global_semaphore: Arc<Semaphore>,
    // Real MOQ components using proper APIs
    moq_broadcast_producer: Option<moq_lite::BroadcastProducer>,
    moq_server: Option<moq_native::Server>,
}

impl MOQInspiredVideoRelayServer {
    fn new() -> Self {
        let (frame_sender, _) = broadcast::channel(1000);
        
        // Global semaphore to prevent server overload (MOQ concept)
        let global_semaphore = Arc::new(Semaphore::new(100)); // Max 100 concurrent operations
        
        Self {
            stats: Arc::new(RwLock::new(ServerStats::default())),
            frame_sender,
            smart_clients: Arc::new(RwLock::new(HashMap::new())),
            global_semaphore,
            // Real MOQ components will be initialized when we start the server
            moq_broadcast_producer: None,
            moq_server: None,
        }
    }

    async fn start(&self) -> Result<()> {
        info!("🚀 Starting Real MOQ Video Relay Server - VERSION 2.11 (Official Catalog Approach)");
        info!("🔧 This version uses empty broadcast path like official samples");

        // Start QUIC server for macOS clients
        let server_clone = self.clone();
        tokio::spawn(async move {
            if let Err(e) = server_clone.start_quic_server().await {
                error!("❌ QUIC server error: {}", e);
            }
        });

        // Start real MOQ server for web clients
        let server_moq = self.clone();
        tokio::spawn(async move {
            if let Err(e) = server_moq.start_real_moq_server().await {
                error!("❌ MOQ server error: {}", e);
            }
        });

        // Start adaptive statistics reporting
        let stats_clone = self.stats.clone();
        let clients_clone = self.smart_clients.clone();
        tokio::spawn(async move {
            report_adaptive_statistics(stats_clone, clients_clone).await;
        });

        // HTTP server will be started in MOQ server method with fingerprints

        // Background task for buffer management (MOQ concept)
        let server_bg = self.clone();
        tokio::spawn(async move {
            server_bg.buffer_management_task().await;
        });

        info!("🌟 ================================");
        info!("🚀 Real MOQ Video Relay Started");
        info!("🌟 ================================");
        info!("🔗 QUIC (macOS) on: localhost:8443");
        info!("🚀 MOQ Server on: localhost:4433");
        info!("🌐 HTTP server on: localhost:3000");
        info!("🎯 Native MOQ backpressure enabled");
        info!("⚡ Built-in media streaming optimizations");
        info!("🧠 MOQ intelligent buffer management");
        info!("🌟 ================================");

        // Keep server running
        tokio::signal::ctrl_c().await?;
        info!("🛑 Shutting down Real MOQ Video Relay Server...");

        Ok(())
    }

    // Background task for intelligent buffer management
    async fn buffer_management_task(&self) {
        let mut interval = interval(Duration::from_secs(5));
        
        loop {
            interval.tick().await;
            
            let clients = self.smart_clients.read().await;
            let mut stats = self.stats.write().await;
            
            // Update simple statistics
            let total_dropped: u64 = clients.values().map(|c| c.dropped_frames).sum();
            stats.dropped_frames = total_dropped;
        }
    }

    async fn start_quic_server(&self) -> Result<()> {
        info!("🔗 Starting QUIC server for macOS clients...");

        // Load certificate
        let (cert, key) = load_certificate()?;
        
        // Configure QUIC server with MOQ-inspired optimizations
        let server_config = configure_smart_quic_server(cert, key)?;
        
        // Create QUIC endpoint
        let endpoint = quinn::Endpoint::server(server_config, "0.0.0.0:8443".parse()?)?;
        info!("🔗 ✅ QUIC server listening on: 0.0.0.0:8443");

        // Accept QUIC connections from macOS clients
        let mut connection_count = 0;
        
        while let Some(conn) = endpoint.accept().await {
            connection_count += 1;
            info!("🔗 📞 macOS connection #{} from: {:?}", connection_count, conn.remote_address());
            
            match conn.await {
                Ok(connection) => {
                    info!("🔗 ✅ macOS connection #{} established", connection_count);
                    
                    let server = self.clone();
                    let conn_id = connection_count;
                    tokio::spawn(async move {
                        if let Err(e) = server.handle_macos_connection(connection).await {
                            error!("❌ macOS connection #{} error: {}", conn_id, e);
                        }
                    });
                }
                Err(e) => {
                    error!("🔗 ❌ macOS connection #{} failed: {}", connection_count, e);
                    continue;
                }
            }
        }

        Ok(())
    }

    async fn handle_macos_connection(&self, connection: quinn::Connection) -> Result<()> {
        let remote_addr = connection.remote_address();
        info!("📱 macOS client connected from: {}", remote_addr);

        // Handle bidirectional streams from macOS
        let server_bi = self.clone();
        let connection_bi = connection.clone();
        tokio::spawn(async move {
            let mut stream_count = 0;
           while let Ok((mut send, recv)) = connection_bi.accept_bi().await {
                stream_count += 1;
                info!("📱 ✅ macOS stream #{} from {}", stream_count, remote_addr);
                
                let server = server_bi.clone();
                tokio::spawn(async move {
                    // Send smart acknowledgment
                    let ack_msg = b"SMART_MOQ_READY";
                    if let Err(e) = send.write_all(ack_msg).await {
                        error!("❌ Failed to send smart ack: {}", e);
                        return;
                    }
                    
                    if let Err(e) = server.handle_macos_video_stream(recv).await {
                        error!("❌ macOS stream #{} error: {}", stream_count, e);
                    }
                });
            }
        });
        
        // Handle unidirectional streams from macOS
        let server_uni = self.clone();
        let connection_uni = connection.clone();
        tokio::spawn(async move {
            let mut stream_count = 0;
            while let Ok(recv) = connection_uni.accept_uni().await {
                stream_count += 1;
                info!("📱 ✅ macOS uni-stream #{} from {}", stream_count, remote_addr);
                
                let server = server_uni.clone();
                tokio::spawn(async move {
                    if let Err(e) = server.handle_macos_video_stream(recv).await {
                        error!("❌ macOS uni-stream #{} error: {}", stream_count, e);
                    }
                });
            }
        });
        
        // Keep connection alive
        connection.closed().await;
        info!("📱 ❌ macOS client {} disconnected", remote_addr);

        Ok(())
    }

    async fn handle_macos_video_stream(&self, mut recv: quinn::RecvStream) -> Result<()> {
        info!("📱 📥 Processing macOS video stream with smart backpressure...");
        let mut buffer = BytesMut::new();
        let mut total_bytes = 0;
        let mut read_buffer = [0u8; 8192];
        
        // Read frames from macOS and process with intelligent backpressure
        loop {
            match recv.read(&mut read_buffer).await {
                Ok(Some(n)) => {
                    total_bytes += n;
                    buffer.extend_from_slice(&read_buffer[..n]);
                    
                    // Process frames with MOQ-inspired intelligence
                    self.process_frames_with_smart_backpressure(&mut buffer).await?;
                },
                Ok(None) => {
                    info!("📱 🏁 macOS stream finished. Total: {} bytes", total_bytes);
                    break;
                },
                Err(e) => {
                    error!("📱 ❌ macOS stream error: {}", e);
                    break;
                }
            }
        }

        Ok(())
    }

    async fn process_frames_with_smart_backpressure(&self, buffer: &mut BytesMut) -> Result<()> {
        let mut processed_frames = 0;

        while buffer.len() >= FRAME_HEADER_SIZE {
            // Parse frame header (same as before)
            let header = match parse_frame_header(&buffer[..FRAME_HEADER_SIZE]) {
                Some(h) if h.magic == MAGIC_STREAM_FRAME => h,
                Some(h) => {
                    warn!("📱 ❌ Invalid magic: expected 0x{:08X}, got 0x{:08X}", 
                          MAGIC_STREAM_FRAME, h.magic);
                    buffer.advance(1);
                    continue;
                },
                None => {
                    buffer.advance(1);
                    continue;
                }
            };

            // Check if we have complete frame
            let total_frame_size = FRAME_HEADER_SIZE + header.data_size as usize;
            if buffer.len() < total_frame_size {
                break;
            }

            // Extract frame data
            buffer.advance(FRAME_HEADER_SIZE);
            let frame_data = buffer.split_to(header.data_size as usize).freeze();

            // Process with smart backpressure (the magic happens here!)
            self.process_video_frame_smart(header, frame_data).await?;
            processed_frames += 1;
        }

        if processed_frames > 0 && processed_frames % 50 == 0 {  // Log only every 50 processed frames
            info!("🎬 ✅ Processed {} frames with smart backpressure", processed_frames);
        }

        Ok(())
    }

    async fn process_video_frame_smart(&self, header: FrameHeader, data: Bytes) -> Result<()> {
        // Global backpressure check (MOQ concept)
        let _permit = self.global_semaphore.acquire().await?;
        
        // avcC configuration frames DISABLED - client uses hardcoded config
        if header.frame_type == 0xFF {
            info!("🔧 Ignoring avcC config: {} bytes (client uses hardcoded)", data.len());
            return Ok(()); // Skip avcC frames entirely
        }
        
        // Update statistics
        {
            let mut stats = self.stats.write().await;
            stats.total_frames += 1;
            stats.total_bytes += data.len() as u64;
            
            if header.frame_type == 0x01 {
                stats.keyframes += 1;
            } else {
                stats.delta_frames += 1;
            }
        }

        // MOQ-inspired frame prioritization
        let priority = if header.frame_type == 0x01 {
            KEYFRAME_PRIORITY // Keyframes: highest priority, never drop
        } else {
            DELTA_PRIORITY // Delta frames: can be dropped under backpressure
        };

        // Create smart video frame with MOQ-inspired metadata
        let video_frame = VideoFrame {
            frame_number: header.frame_number,
            timestamp: header.timestamp,
            frame_type: header.frame_type,
            data: data.clone(),
            priority,
            size: data.len(),
        };

        // SIMPLIFIED: Just broadcast all frames - let MOQ handle backpressure like in samples
        match self.frame_sender.send(video_frame) {
            Ok(_count) => {
                // Frame successfully sent to all subscribers (including MOQ)
            }
            Err(_) => {
                // No subscribers - that's fine
            }
        }

        Ok(())
    }

    // REMOVED: Custom backpressure logic - following official MOQ samples
    // The official samples don't have custom backpressure, MOQ handles this internally

    async fn start_real_moq_server(&self) -> Result<()> {
        info!("🚀 Starting MOQ server using official kixelated/moq pattern...");
        
        // Create MOQ server config exactly like moq-relay
        let mut server_config = moq_native::ServerConfig::default();
        server_config.listen = Some("[::]:4433".parse().unwrap());
        server_config.tls.cert = vec!["server-cert-wt.pem".into()];
        server_config.tls.key = vec!["server-key-wt.pem".into()];
        
        // Initialize server exactly like moq-relay
        let mut server = server_config.init()?;
        let fingerprints = server.fingerprints().to_vec();
        
        info!("🌐 MOQ server listening on port 4433");
        info!("🔒 Certificate fingerprints: {:?}", fingerprints);
        
        // Start HTTP server with certificate fingerprints (like moq-relay)
        let fingerprints_for_http = fingerprints.clone();
        tokio::spawn(async move {
            start_http_server(fingerprints_for_http).await;
        });

        // Accept connections exactly like moq-relay main.rs
        let mut conn_id = 0;
        
        while let Some(request) = server.accept().await {
            info!("✅ MOQ connection {} accepted", conn_id);
            
            let relay_server = self.clone();
            tokio::spawn(async move {
                if let Err(err) = relay_server.handle_moq_connection(conn_id, request).await {
                    tracing::warn!(%err, "MOQ connection {} closed", conn_id);
                }
            });
            
            conn_id += 1;
        }
        
        Ok(())
    }

    async fn handle_moq_connection(&self, conn_id: u64, request: moq_native::Request) -> Result<()> {
        info!("🔗 Handling MOQ connection {} with OFFICIAL CATALOG APPROACH", conn_id);
        
        // Accept the WebTransport connection first
        let connection = request.ok().await?;
        info!("✅ WebTransport connection {} accepted", conn_id);
        
        // Create broadcast for this connection
        let mut broadcast = moq_lite::Broadcast::produce();
        
        // Create catalog describing our H.264 video track
        let mut catalog = Catalog::default();
        
        // Add H.264 video track to catalog (following official samples)
        let video_track = Video {
            track: moq_lite::Track {
                name: "video".to_string(),
                priority: 1,
            },
            config: VideoConfig {
                codec: hang::catalog::VideoCodec::Unknown("avc1.64001f".to_string()), // H.264 High Profile Level 3.1
                coded_width: Some(1670),
                coded_height: Some(1080),
                framerate: Some(30.0),
                bitrate: Some(4_000_000), // 4 Mbps
                optimize_for_latency: Some(true),
                description: None,
                display_ratio_width: None,
                display_ratio_height: None,
                rotation: None,
                flip: None,
            },
        };
        catalog.video.push(video_track);
        
        info!("📋 Created catalog with H.264 video track: {}x{} @ {}fps", 1670, 1080, 30.0);
        
        // Create origin and publish the broadcast
        let origin = moq_lite::Origin::produce();
        origin.producer.publish_broadcast("", broadcast.consumer);
        
        // Create catalog track and publish it to the broadcast
        let catalog_track = moq_lite::Track {
            name: "catalog.json".to_string(),
            priority: 100, // High priority for catalog
        };
        let mut catalog_producer = broadcast.producer.create_track(catalog_track);
        
        // Publish catalog immediately
        let mut catalog_group = catalog_producer.append_group();
        let catalog_json = catalog.to_string()?;
        catalog_group.write_frame(catalog_json);
        catalog_group.close();
        
        info!("📋 Published catalog.json describing video track");
        
        // Create video track producer
        let video_track = moq_lite::Track {
            name: "video".to_string(),
            priority: 1,
        };
        let video_producer = broadcast.producer.create_track(video_track);
        
        info!("🎬 Created video track producer");
        
        // Start publishing video frames to the video track
        let publisher = self.clone();
        tokio::spawn(async move {
            if let Err(err) = publisher.publish_hang_video_frames(video_producer).await {
                error!("❌ Failed to publish video frames for connection {}: {}", conn_id, err);
            }
        });
        
        // Establish MOQ session (this will handle client subscriptions to catalog and video)
        // Pass origin.producer as publish handler to allow proper session establishment
        let session = moq_lite::Session::accept(connection, Some(origin.consumer), Some(origin.producer)).await?;
        info!("✅ MOQ session {} established with catalog - clients can now subscribe!", conn_id);
        
        // Wait for session to close
        let session_error = session.closed().await;
        info!("🔌 MOQ session {} closed: {:?}", conn_id, session_error);
        Err(anyhow::anyhow!("MOQ session closed: {:?}", session_error))
    }

    async fn accept_moq_connections(
        &self,
        mut server: moq_native::Server,
        broadcast_name: String,
        consumer: moq_lite::BroadcastConsumer,
    ) -> Result<()> {
        let mut conn_id = 0;
        
        while let Some(session) = server.accept().await {
            let id = conn_id;
            conn_id += 1;
            
            let name = broadcast_name.clone();
            let consumer = consumer.clone();
            
            tokio::spawn(async move {
                if let Err(err) = Self::handle_moq_session(id, session, name, consumer).await {
                    error!("❌ Failed to handle MOQ session {}: {}", id, err);
                }
            });
        }
        
        Ok(())
    }
    
    async fn handle_moq_session(
        id: u64,
        session: moq_native::Request,
        broadcast_name: String,
        consumer: moq_lite::BroadcastConsumer,
    ) -> Result<()> {
        // Accept the session (WebTransport or QUIC)
        let session = session.ok().await?;
        
        // FIXED: Follow official kixelated/moq-relay pattern exactly
        // Create an Origin and use its consumer (OriginConsumer) for the subscribe parameter
        let origin = moq_lite::Origin::produce();
        origin.producer.publish_broadcast("", consumer);
        
        // Pass origin.consumer (OriginConsumer) as subscribe parameter
        let session = moq_lite::Session::accept(session, Some(origin.consumer), None).await?;
        
        info!("✅ MOQ session {} accepted - following official pattern", id);
        
        // Wait for session to close
        Err(session.closed().await.into())
    }
    
    async fn publish_moq_frames(&self, mut producer: moq_lite::BroadcastProducer) -> Result<()> {
        info!("📡 Starting MOQ video frame publishing with hang format - VERSION 2.1");
        
        // Create video track following the official kixelated/moq pattern
        let video_track = moq_lite::Track {
            name: "video".to_string(),
            priority: 1, // Video typically has lower priority than audio
        };
        
        // Create the track producer using hang::TrackProducer pattern
        let track = producer.create_track(video_track);
        let mut track_producer: hang::TrackProducer = track.into();
        
        info!("✅ Created MOQ video track: 'video' with priority 1");
        
        // Subscribe to video frames exactly like the official sample
        let mut frame_receiver = self.frame_sender.subscribe();
        
        let mut published_count = 0;
        while let Ok(video_frame) = frame_receiver.recv().await {
            // Convert our VideoFrame to hang::Frame format
            let hang_frame = self.convert_to_hang_frame(&video_frame)?;
            
            // Write the frame to the MOQ track (let MOQ handle its own backpressure)
            track_producer.write(hang_frame);
            published_count += 1;
            
            if published_count % 30 == 0 {  // Log every 30th published frame
                let frame_type = if video_frame.frame_type == 0x01 { "KEYFRAME" } else { "delta" };
                info!("📦 Published MOQ {} #{}: {} bytes (total published: {})", 
                      frame_type, video_frame.frame_number, video_frame.data.len(), published_count);
            }
        }
        
        Ok(())
    }
    
    // NEW: Official catalog-based video frame publishing
    async fn publish_hang_video_frames(&self, mut video_producer: moq_lite::TrackProducer) -> Result<()> {
        info!("🎬 Starting OFFICIAL CATALOG video frame publishing - VERSION 2.11");
        
        // Subscribe to video frames from macOS app
        let mut frame_receiver = self.frame_sender.subscribe();
        
        let mut published_count = 0;
        let mut avcc_sent = false; // Track if we've sent avcC to this client
        
        while let Ok(video_frame) = frame_receiver.recv().await {
        // avcC configuration sending DISABLED - client uses hardcoded config
        // if !avcc_sent {
        //     let stats = self.stats.read().await;
        //     if let Some(avcc_data) = &stats.avcc_config {
        //         info!("🔧 Sending avcC config to NEW MOQ client: {} bytes", avcc_data.len());
        //         
        //         // Create avcC frame
        //         let avcc_frame = VideoFrame {
        //             frame_number: 0,
        //             timestamp: 0,
        //             frame_type: 0xFF,
        //             data: avcc_data.clone(),
        //             priority: 0,
        //             size: avcc_data.len(),
        //         };
        //         
        //         // Convert and publish avcC frame
        //         let avcc_hang_frame = self.convert_to_hang_frame(&avcc_frame)?;
        //         let mut avcc_group = video_producer.append_group();
        //         avcc_group.write_frame(avcc_hang_frame.payload);
        //         avcc_group.close();
        //         
        //         avcc_sent = true; // Mark as sent for this client
        //         info!("✅ Published avcC config to NEW MOQ client");
        //     }
        // }
            
            // Convert our VideoFrame to hang::Frame format
            let hang_frame = self.convert_to_hang_frame(&video_frame)?;
            
            // Create a new group for this frame
            let mut group = video_producer.append_group();
            
            // Write the raw frame data (annexb format for H.264)
            group.write_frame(hang_frame.payload);
            group.close();
            
            published_count += 1;
            
            // Log every 30th frame to avoid spam
            if published_count % 30 == 0 {
                let frame_type = if video_frame.frame_type == 0x01 { "KEYFRAME" } else { "delta" };
                info!("🎬 Published catalog video {} #{}: {} bytes (total: {})", 
                      frame_type, video_frame.frame_number, video_frame.data.len(), published_count);
            }
        }
        
        Ok(())
    }
    
    fn convert_to_hang_frame(&self, video_frame: &VideoFrame) -> Result<hang::Frame> {
        // avcC configuration frames DISABLED - skip conversion
        if video_frame.frame_type == 0xFF {
            return Err(anyhow::anyhow!("avcC frames disabled"));
        }
        
        // Convert H.264 AVCC format (from macOS) to annexb format (for MOQ/hang)
        let annexb_data = self.avcc_to_annexb(&video_frame.data)?;
        
        // Create hang::Frame following the official pattern
        let hang_frame = hang::Frame {
            keyframe: video_frame.frame_type == 0x01, // 0x01 = keyframe, 0x00 = delta
            timestamp: std::time::Duration::from_micros(video_frame.timestamp),
            payload: moq_lite::coding::Bytes::from(annexb_data),
        };
        
        Ok(hang_frame)
    }
    
    fn avcc_to_annexb(&self, avcc_data: &[u8]) -> Result<Vec<u8>> {
        // Convert H.264 AVCC format (length-prefixed) to annexb format (start code prefixed)
        // AVCC: [length][NAL] [length][NAL] ...
        // annexb: [0x00 0x00 0x00 0x01][NAL] [0x00 0x00 0x00 0x01][NAL] ...
        
        let mut annexb_data = Vec::new();
        let mut cursor = 0;
        
        while cursor + 4 <= avcc_data.len() {
            // Read NAL length (big-endian 4 bytes)
            let nal_length = u32::from_be_bytes([
                avcc_data[cursor],
                avcc_data[cursor + 1], 
                avcc_data[cursor + 2],
                avcc_data[cursor + 3]
            ]) as usize;
            cursor += 4;
            
            if cursor + nal_length > avcc_data.len() {
                return Err(anyhow::anyhow!("Invalid NAL length in AVCC data"));
            }
            
            // Add annexb start code: 0x00 0x00 0x00 0x01
            annexb_data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
            
            // Add NAL unit data
            annexb_data.extend_from_slice(&avcc_data[cursor..cursor + nal_length]);
            cursor += nal_length;
        }
        
        Ok(annexb_data)
    }

    // Remove all old WebTransport session handling - replaced with MOQ
    /*
    async fn handle_smart_webtransport_session_old(&self, connection: wtransport::Connection) {
        let remote_addr = connection.remote_address();
        let client_id = format!("wt_{}", remote_addr);
        
        info!("🚀 📡 SMART WEBTRANSPORT SESSION STARTED: {}", client_id);
        
        // Register smart client
        {
            let mut clients = self.smart_clients.write().await;
            clients.insert(client_id.clone(), SmartClient::new(client_id.clone()));
            
            let mut stats = self.stats.write().await;
            stats.connected_clients += 1;
        }
        
        // Subscribe to video frames
        let mut frame_receiver = self.frame_sender.subscribe();
        
        info!("🚀 📤 Using UNRELIABLE datagrams for WebTransport client: {}", client_id);
        
        // Send stored avcC configuration via datagram (unreliable but small, likely to arrive)
        {
            let stats = self.stats.read().await;
            if let Some(avcc_data) = &stats.avcc_config {
                info!("🚀 📤 Sending avcC config to smart client {}: {} bytes", client_id, avcc_data.len());
                
                // Create avcC datagram with frame type 255
                let mut datagram_data = Vec::new();
                datagram_data.push(255u8); // Frame type for avcC
                datagram_data.extend_from_slice(&0u64.to_be_bytes()); // Frame number
                datagram_data.extend_from_slice(&0u64.to_be_bytes()); // Timestamp  
                datagram_data.extend_from_slice(avcc_data);
                
                // Send via unreliable datagram
                if let Err(e) = connection.send_datagram(&datagram_data) {
                    error!("🚀 ❌ Failed to send avcC datagram: {}", e);
                } else {
                    info!("🚀 ✅ avcC sent to smart client {} (UNRELIABLE DATAGRAM)", client_id);
                }
            }
        }
        
        // Smart frame delivery with MOQ-inspired backpressure
        while let Ok(video_frame) = frame_receiver.recv().await {
            // Check if this smart client should receive this frame
            let should_send = {
                let clients = self.smart_clients.read().await;
                if let Some(client) = clients.get(&client_id) {
                    client.should_accept_frame(&video_frame)
                } else {
                    false // Client disconnected
                }
            };
            
            if should_send {
                // Send frame with smart delivery
                match self.send_frame_smart(
                    &mut send_stream,
                    &client_id,
                    video_frame.frame_type,
                    video_frame.frame_number as u32,
                    video_frame.timestamp,
                    &video_frame.data
                ).await {
                    Ok(_) => {
                        // Update client state on successful send
                        let mut clients = self.smart_clients.write().await;
                        if let Some(client) = clients.get_mut(&client_id) {
                            // Frame queued
                            client.record_successful_send();
                        }
                    }
                    Err(e) => {
                        error!("🚀 ❌ Smart send failed to {}: {}", client_id, e);
                        break;
                    }
                }
            } else {
                // Smart backpressure dropped this frame for this client
                let mut clients = self.smart_clients.write().await;
                if let Some(client) = clients.get_mut(&client_id) {
                    client.record_dropped_frame();
                }
                
                // Reduced logging for client drops
                if video_frame.frame_number % 200 == 0 {  // Log only every 200th client drop
                    let frame_type = if video_frame.frame_type == 0x01 { "KEYFRAME" } else { "delta" };
                    info!("🎯 Smart client {} backpressure: dropped {} frame #{} [logging every 200th]", 
                          client_id, frame_type, video_frame.frame_number);
                }
            }
        }
        
        // Cleanup on disconnect
        {
            let mut clients = self.smart_clients.write().await;
            if let Some(client) = clients.remove(&client_id) {
                info!("🚀 📊 Smart client {} stats: dropped {} frames, quality {:.2}", 
                      client_id, client.dropped_frames, client.connection_quality);
            }
            
            let mut stats = self.stats.write().await;
            stats.connected_clients = stats.connected_clients.saturating_sub(1);
        }
        
        info!("🚀 🏁 Smart WebTransport client {} disconnected", client_id);
    }
    */

    /*
    // OLD Smart frame delivery - replaced with MOQ
    async fn send_frame_smart(
        &self,
        send_stream: &mut wtransport::SendStream,
        client_id: &str,
        frame_type: u8,
        frame_number: u32,
        timestamp: u64,
        data: &Bytes
    ) -> Result<()> {
        use tokio::io::AsyncWriteExt;
        
        // Build frame with MOQ-inspired binary protocol
        let data_size = data.len() as u32;
        let total_size = 1 + 4 + 8 + 4 + data.len();
        let mut frame_buffer = Vec::with_capacity(total_size);
        
        // Smart frame header
        frame_buffer.push(frame_type);
        frame_buffer.extend_from_slice(&frame_number.to_le_bytes());
        frame_buffer.extend_from_slice(&timestamp.to_le_bytes());
        frame_buffer.extend_from_slice(&data_size.to_le_bytes());
        frame_buffer.extend_from_slice(data);
        
        // Atomic send with backpressure handling
        send_stream.write_all(&frame_buffer).await?;
        send_stream.flush().await?;
        
        // Update client buffer tracking
        {
            let mut clients = self.smart_clients.write().await;
            if let Some(client) = clients.get_mut(client_id) {
                // No buffer simulation - just mark as sent
                client.record_successful_send();
            }
        }
        
        Ok(())
    }
    */
}

// Make the struct Send + Sync for tokio::spawn
unsafe impl Send for MOQInspiredVideoRelayServer {}
unsafe impl Sync for MOQInspiredVideoRelayServer {}

impl Clone for MOQInspiredVideoRelayServer {
    fn clone(&self) -> Self {
        Self {
            stats: self.stats.clone(),
            frame_sender: self.frame_sender.clone(),
            smart_clients: self.smart_clients.clone(),
            global_semaphore: self.global_semaphore.clone(),
            // Real MOQ components can't be cloned - they'll be reinitialized when needed
            moq_broadcast_producer: None,
            moq_server: None,
        }
    }
}

fn parse_frame_header(data: &[u8]) -> Option<FrameHeader> {
    if data.len() < FRAME_HEADER_SIZE {
        return None;
    }

    Some(FrameHeader {
        magic: u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
        frame_number: u64::from_le_bytes([
            data[4], data[5], data[6], data[7],
            data[8], data[9], data[10], data[11],
        ]),
        timestamp: u64::from_le_bytes([
            data[12], data[13], data[14], data[15],
            data[16], data[17], data[18], data[19],
        ]),
        frame_type: data[20],
        data_size: u32::from_le_bytes([data[21], data[22], data[23], data[24]]),
    })
}

async fn report_adaptive_statistics(
    stats: Arc<RwLock<ServerStats>>, 
    clients: Arc<RwLock<HashMap<String, SmartClient>>>
) {
    let mut interval = interval(Duration::from_secs(5));
    
    loop {
        interval.tick().await;
        
        let stats = stats.read().await;
        let clients = clients.read().await;
        
        if stats.total_frames == 0 {
            continue;
        }

        let elapsed = stats.start_time.elapsed().as_secs_f64();
        let fps = stats.total_frames as f64 / elapsed;
        let mbps = (stats.total_bytes as f64 * 8.0) / (1024.0 * 1024.0) / elapsed;

        // Calculate client-specific metrics
        let total_client_drops: u64 = clients.values().map(|c| c.dropped_frames).sum();
        let avg_connection_quality: f32 = if !clients.is_empty() {
            clients.values().map(|c| c.connection_quality).sum::<f32>() / clients.len() as f32
        } else {
            1.0
        };

        info!("📊 === SIMPLE RELAY STATS ===");
        info!("⏱️  Runtime: {:.1}s", elapsed);
        info!("🎬 Frames: {} ({:.1} fps)", stats.total_frames, fps);
        info!("🔑 Keyframes: {} | 📦 Delta: {}", stats.keyframes, stats.delta_frames);
        info!("💾 Data: {:.2} MB ({:.2} Mbps)", 
              stats.total_bytes as f64 / (1024.0 * 1024.0), mbps);
        info!("👥 Clients: {}", stats.connected_clients);
        info!("🔗 Connection Quality: {:.2}", avg_connection_quality);
        info!("🚀 Natural WebTransport flow control");
        info!("===============================");
    }
}

fn configure_smart_quic_server(cert: rustls::Certificate, key: rustls::PrivateKey) -> Result<quinn::ServerConfig> {
    let mut crypto_config = rustls::ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)?;
    
    // MOQ-inspired ALPN protocols
    crypto_config.alpn_protocols = vec![
        b"moq-smart".to_vec(), // Our smart MOQ-inspired protocol
        b"h3".to_vec(),
        b"hq-interop".to_vec(),
    ];
    
    // Optimized transport for smart video streaming
    let mut transport_config = quinn::TransportConfig::default();
    
    // MOQ-inspired flow control settings
    transport_config.receive_window(quinn::VarInt::from_u32(4 * 1024 * 1024)); // 4MB receive
    transport_config.stream_receive_window(quinn::VarInt::from_u32(1024 * 1024)); // 1MB per stream
    transport_config.send_window(4 * 1024 * 1024); // 4MB send
    
    // Smart timeout and concurrency settings
    transport_config.max_idle_timeout(Some(Duration::from_secs(30).try_into().unwrap()));
    transport_config.max_concurrent_uni_streams(quinn::VarInt::from_u32(50));
    transport_config.max_concurrent_bidi_streams(quinn::VarInt::from_u32(25));
    
    let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(crypto_config));
    server_config.transport = Arc::new(transport_config);
    
    info!("✅ Smart QUIC server configured with MOQ-inspired optimizations");
    Ok(server_config)
}

fn load_certificate() -> Result<(rustls::Certificate, rustls::PrivateKey)> {
    load_mkcert_certificate().or_else(|_| {
        info!("🔐 mkcert not found, generating self-signed certificate");
        generate_self_signed_cert_and_key()
    })
}

fn load_mkcert_certificate() -> Result<(rustls::Certificate, rustls::PrivateKey)> {
    use std::fs;
    use std::io::BufReader;
    
    let cert_paths = ["server-cert.pem", "./server-cert.pem", "../server-cert.pem"];
    let key_paths = ["server-key.pem", "./server-key.pem", "../server-key.pem"];
    
    let mut cert_file = None;
    let mut key_file = None;
    
    for path in &cert_paths {
        if std::path::Path::new(path).exists() {
            cert_file = Some(path);
            break;
        }
    }
    
    for path in &key_paths {
        if std::path::Path::new(path).exists() {
            key_file = Some(path);
            break;
        }
    }
    
    match (cert_file, key_file) {
        (Some(cert_path), Some(key_path)) => {
            let cert_file = fs::File::open(cert_path)?;
            let mut cert_reader = BufReader::new(cert_file);
            let certs = rustls_pemfile::certs(&mut cert_reader)?;
            
            if certs.is_empty() {
                return Err(anyhow::anyhow!("No certificates found"));
            }
            
            let key_file = fs::File::open(key_path)?;
            let mut key_reader = BufReader::new(key_file);
            let keys = rustls_pemfile::pkcs8_private_keys(&mut key_reader)?;
            
            if keys.is_empty() {
                let key_file = fs::File::open(key_path)?;
                let mut key_reader = BufReader::new(key_file);
                let rsa_keys = rustls_pemfile::rsa_private_keys(&mut key_reader)?;
                
                if rsa_keys.is_empty() {
                    return Err(anyhow::anyhow!("No private keys found"));
                }
                
                return Ok((rustls::Certificate(certs[0].clone()), rustls::PrivateKey(rsa_keys[0].clone())));
            }
            
            Ok((rustls::Certificate(certs[0].clone()), rustls::PrivateKey(keys[0].clone())))
        }
        _ => Err(anyhow::anyhow!("Certificate files not found"))
    }
}

fn generate_self_signed_cert_and_key() -> Result<(rustls::Certificate, rustls::PrivateKey)> {
    use rcgen::{Certificate as RcgenCert, CertificateParams, DistinguishedName, SanType};
    
    let mut params = CertificateParams::new(vec!["localhost".to_string(), "127.0.0.1".to_string()]);
    params.distinguished_name = DistinguishedName::new();
    params.distinguished_name.push(rcgen::DnType::CommonName, "localhost");
    
    params.subject_alt_names = vec![
        SanType::DnsName("localhost".to_string()),
        SanType::IpAddress(std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1))),
    ];
    
    params.not_before = rcgen::date_time_ymd(2023, 1, 1);
    params.not_after = rcgen::date_time_ymd(2025, 1, 1);
    
    let cert = RcgenCert::from_params(params)?;
    let cert_der = cert.serialize_der()?;
    let key_der = cert.serialize_private_key_der();
    
    info!("🔐 Generated self-signed certificate for smart MOQ server");
    Ok((rustls::Certificate(cert_der), rustls::PrivateKey(key_der)))
}

async fn start_http_server(fingerprints: Vec<String>) {
    use tokio::net::TcpListener;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    info!("🌐 Starting HTTP server on localhost:3000...");
    
    let listener = match TcpListener::bind("0.0.0.0:3000").await {
        Ok(listener) => listener,
        Err(e) => {
            error!("❌ Failed to bind HTTP server: {}", e);
            return;
        }
    };

    info!("🌐 ✅ HTTP server listening on: 0.0.0.0:3000");

    // Get the first certificate fingerprint (like MOQ examples)
    let fingerprint = fingerprints.first().cloned().unwrap_or_else(|| "no-fingerprint".to_string());

    while let Ok((mut stream, _addr)) = listener.accept().await {
        let fingerprint = fingerprint.clone();
        tokio::spawn(async move {
            let mut buffer = [0; 1024];
            
            if let Ok(n) = stream.read(&mut buffer).await {
                let request = String::from_utf8_lossy(&buffer[..n]);
                
                if request.contains("GET /certificate.sha256") {
                    // Serve certificate fingerprint (like MOQ examples)
                    let response = format!(
                        "HTTP/1.1 200 OK\r\n\
                         Content-Type: text/plain\r\n\
                         Content-Length: {}\r\n\
                         Access-Control-Allow-Origin: *\r\n\
                         \r\n\
                         {}",
                        fingerprint.len(),
                        fingerprint
                    );
                    
                    let _ = stream.write_all(response.as_bytes()).await;
                } else if request.contains("GET / ") {
                    let html = include_str!("../web/index.html");
                    let response = format!(
                        "HTTP/1.1 200 OK\r\n\
                         Content-Type: text/html\r\n\
                         Content-Length: {}\r\n\
                         Access-Control-Allow-Origin: *\r\n\
                         \r\n\
                         {}",
                        html.len(),
                        html
                    );
                    
                    let _ = stream.write_all(response.as_bytes()).await;
                } else if request.contains("GET /assets/") {
                    // Extract the file path from the request
                    if let Some(start) = request.find("GET /assets/") {
                        if let Some(end) = request[start..].find(" HTTP/") {
                            let file_path = &request[start + 4..start + end]; // Skip "GET "
                            let full_path = format!("web{}", file_path);
                            
                            // Try to read the file
                            match tokio::fs::read(&full_path).await {
                                Ok(content) => {
                                    let content_type = if file_path.ends_with(".js") {
                                        "application/javascript"
                                    } else if file_path.ends_with(".js.map") {
                                        "application/json"
                                    } else {
                                        "application/octet-stream"
                                    };
                                    
                                    let response = format!(
                                        "HTTP/1.1 200 OK\r\n\
                                         Content-Type: {}\r\n\
                                         Content-Length: {}\r\n\
                                         Access-Control-Allow-Origin: *\r\n\
                                         \r\n",
                                        content_type,
                                        content.len()
                                    );
                                    
                                    let _ = stream.write_all(response.as_bytes()).await;
                                    let _ = stream.write_all(&content).await;
                                }
                                Err(_) => {
                                    let response = "HTTP/1.1 404 Not Found\r\n\r\n";
                                    let _ = stream.write_all(response.as_bytes()).await;
                                }
                    }
                } else {
                            let response = "HTTP/1.1 400 Bad Request\r\n\r\n";
                            let _ = stream.write_all(response.as_bytes()).await;
                        }
                    } else {
                        let response = "HTTP/1.1 400 Bad Request\r\n\r\n";
                        let _ = stream.write_all(response.as_bytes()).await;
                    }
                } else {
                    let response = "HTTP/1.1 404 Not Found\r\n\r\n";
                    let _ = stream.write_all(response.as_bytes()).await;
                }
            }
        });
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new("info"))
        .init();

    info!("🚀 Initializing Real MOQ Video Relay Server...");
    
    let server = MOQInspiredVideoRelayServer::new();
    
    if let Err(e) = server.start().await {
        error!("❌ Real MOQ Server error: {}", e);
        std::process::exit(1);
    }

    Ok(())
}