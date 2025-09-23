use anyhow::Result;
use bytes::{Buf, Bytes, BytesMut};
use quinn::{Connection, Endpoint, ServerConfig};
use rustls::{Certificate, PrivateKey};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, RwLock};
use tokio::time::interval;
use tokio::io::AsyncWriteExt;
use tracing::{error, info, warn};
use wtransport::{Endpoint as WtEndpoint, ServerConfig as WtServerConfig};
use base64::Engine;
use futures_util::SinkExt;

/**
 * 🦀 Simplified Rust QUIC Video Relay Server
 * 
 * Clean implementation following Quinn documentation patterns
 * - Receives H.264 frames from macOS app via QUIC streams
 * - Simple stream processing without TLS workarounds
 * - Relays frames to web clients via WebSocket
 */

const MAGIC_STREAM_FRAME: u32 = 0x53545246; // "STRF"
const FRAME_HEADER_SIZE: usize = 25;

#[derive(Debug, Clone)]
struct VideoFrame {
    frame_number: u64,
    timestamp: u64,
    frame_type: u8, // 0x01 = keyframe, 0x00 = delta
    data: Bytes,
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
        }
    }
}

struct VideoRelayServer {
    stats: Arc<RwLock<ServerStats>>,
    frame_sender: broadcast::Sender<VideoFrame>,
    web_clients: Arc<RwLock<Vec<quinn::Connection>>>,
}

impl VideoRelayServer {
    fn new() -> Self {
        let (frame_sender, _) = broadcast::channel(1000);
        
        Self {
            stats: Arc::new(RwLock::new(ServerStats::default())),
            frame_sender,
            web_clients: Arc::new(RwLock::new(Vec::new())),
        }
    }

    async fn start(&self) -> Result<()> {
        info!("🦀 Starting Simplified Rust QUIC Video Relay Server...");

        // Load certificate
        info!("🔐 Loading certificate...");
        let (cert, key) = load_certificate()?;
        
        // Configure QUIC server
        info!("⚙️ Configuring QUIC server...");
        let server_config = configure_server(cert, key)?;
        
        // Create QUIC endpoint
        info!("🚀 Creating QUIC endpoint on 0.0.0.0:8443...");
        let endpoint = Endpoint::server(server_config, "0.0.0.0:8443".parse()?)?;
        info!("🔗 ✅ QUIC server listening on: 0.0.0.0:8443");

        // Start statistics reporting
        let stats_clone = self.stats.clone();
        tokio::spawn(async move {
            report_statistics(stats_clone).await;
        });

        // Start WebSocket server for video streaming
        let server_clone = self.clone();
        tokio::spawn(async move {
            start_websocket_server(server_clone).await;
        });

        // Start simple HTTP server for serving the web client
        tokio::spawn(async {
            start_http_server().await;
        });

        // Start WebTransport server for video streaming
        let server_wt = self.clone();
        tokio::spawn(async move {
            start_webtransport_server(server_wt).await;
        });

        info!("🌟 ================================");
        info!("🎥 Simplified QUIC Video Relay Started");
        info!("🌟 ================================");
        info!("🔗 QUIC listening on: localhost:8443");
        info!("🌐 HTTP server on: localhost:3000");
        info!("🔌 WebSocket server on: localhost:8080");
        info!("🚀 WebTransport server on: localhost:4433");
        info!("🦀 Clean Quinn implementation");
        info!("🌟 ================================");

        // Accept QUIC connections
        info!("🔗 ⏳ Waiting for QUIC connections...");
        let mut connection_count = 0;
        
        while let Some(conn) = endpoint.accept().await {
            connection_count += 1;
            info!("🔗 📞 Incoming connection #{} from: {:?}", connection_count, conn.remote_address());
            
            match conn.await {
                Ok(connection) => {
                    info!("🔗 ✅ Connection #{} established from: {}", connection_count, connection.remote_address());
                    
                    let server = self.clone();
                    let conn_id = connection_count;
                    tokio::spawn(async move {
                        if let Err(e) = server.handle_connection(connection).await {
                            error!("❌ Connection #{} error: {}", conn_id, e);
                        }
                    });
                }
                Err(e) => {
                    error!("🔗 ❌ Connection #{} failed: {}", connection_count, e);
                    continue;
                }
            }
        }

        Ok(())
    }

    async fn handle_connection(&self, connection: Connection) -> Result<()> {
        let remote_addr = connection.remote_address();
        info!("📺 🔗 Client connected from: {}", remote_addr);

        // Check if this is a WebTransport client (H3 ALPN) or macOS client
        // For now, assume all connections are macOS clients since WebTransport runs on separate port
        let is_webtransport = false;

        if is_webtransport {
            info!("🌐 WebTransport client detected from: {}", remote_addr);
            return self.handle_webtransport_client(connection).await;
        } else {
            info!("📱 macOS QUIC client detected from: {}", remote_addr);
        }

        // Update client count
        {
            let mut stats = self.stats.write().await;
            stats.connected_clients += 1;
            info!("📊 Total connected clients: {}", stats.connected_clients);
        }

        // Handle bidirectional streams (clean Quinn pattern)
        let server_bi = self.clone();
        let connection_bi = connection.clone();
        tokio::spawn(async move {
            let mut stream_count = 0;
           while let Ok((mut send, recv)) = connection_bi.accept_bi().await {
                stream_count += 1;
                info!("📺 ✅ Bidirectional stream #{} from {}", stream_count, remote_addr);
                
                let server = server_bi.clone();
                tokio::spawn(async move {
                    // CRITICAL: Write to SendStream before reading from RecvStream (Quinn requirement)
                    let ack_msg = b"READY";
                    if let Err(e) = send.write(ack_msg).await {
                        error!("❌ Failed to write to SendStream: {}", e);
                        return;
                    }
                    info!("📺 ✅ Initial write to SendStream completed (sent READY)");
                    
                    if let Err(e) = server.handle_video_stream_bi(recv, send).await {
                        error!("❌ Stream #{} error: {}", stream_count, e);
                    }
                });
            }
        });
        
        // Handle unidirectional streams (clean Quinn pattern)
        let server_uni = self.clone();
        let connection_uni = connection.clone();
        tokio::spawn(async move {
            let mut stream_count = 0;
            while let Ok(recv) = connection_uni.accept_uni().await {
                stream_count += 1;
                info!("📺 ✅ Unidirectional stream #{} from {}", stream_count, remote_addr);
                
                let server = server_uni.clone();
                tokio::spawn(async move {
                    if let Err(e) = server.handle_video_stream(recv).await {
                        error!("❌ Stream #{} error: {}", stream_count, e);
                    }
                });
            }
        });
        
        // Keep connection alive
        connection.closed().await;
        
        // Update client count on disconnect
        {
            let mut stats = self.stats.write().await;
            stats.connected_clients = stats.connected_clients.saturating_sub(1);
            info!("📺 ❌ Client {} disconnected. Remaining: {}", remote_addr, stats.connected_clients);
        }

        Ok(())
    }

    async fn handle_webtransport_client(&self, connection: Connection) -> Result<()> {
        let remote_addr = connection.remote_address();
        info!("🌐 WebTransport client connected from: {}", remote_addr);

        // Subscribe to video frames
        let mut frame_receiver = self.frame_sender.subscribe();
        
        // Create a unidirectional stream to send H.264 frames to the web client
        let mut send_stream = connection.open_uni().await?;
        info!("🌐 📤 Opened unidirectional stream to WebTransport client");

        // Send frames to WebTransport client
        tokio::spawn(async move {
            while let Ok(video_frame) = frame_receiver.recv().await {
                // Send raw H.264 data directly
                if let Err(e) = send_stream.write_all(&video_frame.data).await {
                    error!("🌐 ❌ Failed to send frame to WebTransport client: {}", e);
                    break;
                }
                
                if let Err(e) = send_stream.flush().await {
                    error!("🌐 ❌ Failed to flush WebTransport stream: {}", e);
                    break;
                }

                info!("🌐 📤 Sent frame #{} ({} bytes) to WebTransport client", 
                      video_frame.frame_number, video_frame.data.len());
            }
            
            info!("🌐 🏁 WebTransport stream closed");
        });

        // Keep connection alive
        connection.closed().await;
        info!("🌐 ❌ WebTransport client {} disconnected", remote_addr);

        Ok(())
    }

    // Bidirectional stream handler - handles both send and receive streams properly
    async fn handle_video_stream_bi(&self, mut recv: quinn::RecvStream, mut _send: quinn::SendStream) -> Result<()> {
        info!("📺 📥 Starting bidirectional video stream handler...");
        let mut buffer = BytesMut::new();
        let mut total_bytes = 0;
        let mut read_buffer = [0u8; 8192];
        
        // Keep the stream alive with timeout-based reading
        loop {
            // Use a timeout to avoid blocking indefinitely
            let read_result = tokio::time::timeout(
                std::time::Duration::from_secs(30), // 30 second timeout
                recv.read(&mut read_buffer)
            ).await;
            
            match read_result {
                Ok(Ok(Some(n))) => {
                    total_bytes += n;
                    // info!("📺 📦 Received {} bytes (total: {})", n, total_bytes);
                    
                    // Show first few bytes for debugging
                    // if n > 0 {
                    //     let preview: Vec<String> = read_buffer[..std::cmp::min(16, n)].iter()
                    //         .map(|b| format!("{:02X}", b))
                    //         .collect();
                    //     info!("📺 🔍 First {} bytes: {}", preview.len(), preview.join(" "));
                    // }
                    
                    buffer.extend_from_slice(&read_buffer[..n]);
                    
                    // Process complete frames
                    self.process_frames_from_buffer(&mut buffer).await?;
                },
                Ok(Ok(None)) => {
                    info!("📺 🏁 Stream finished gracefully. Total bytes: {}", total_bytes);
                    break;
                },
                Ok(Err(e)) => {
                    error!("📺 ❌ Stream read error: {}", e);
                    break;
                },
                Err(_) => {
                    info!("📺 ⏰ Read timeout - keeping stream alive for more data");
                    // Continue the loop to keep stream alive
                }
            }
        }

        Ok(())
    }

    // SIMPLIFIED stream handler - no TLS detection nonsense (for unidirectional streams)
    async fn handle_video_stream(&self, mut recv: quinn::RecvStream) -> Result<()> {
        info!("📺 📥 Starting continuous unidirectional video stream handler...");
        let mut buffer = BytesMut::new();
        let mut total_bytes = 0;
        let mut read_buffer = [0u8; 8192];
        
        // Keep reading from the stream continuously
        loop {
            match recv.read(&mut read_buffer).await {
                Ok(Some(n)) => {
                    total_bytes += n;
                    // info!("📺 📦 Received {} bytes (total: {})", n, total_bytes);
                    
                    // Show first few bytes for debugging
                    // if n > 0 {
                    //     let preview: Vec<String> = read_buffer[..std::cmp::min(16, n)].iter()
                    //         .map(|b| format!("{:02X}", b))
                    //         .collect();
                    //     info!("📺 🔍 First {} bytes: {}", preview.len(), preview.join(" "));
                    // }
                    
                    buffer.extend_from_slice(&read_buffer[..n]);
                    
                    // Process complete frames as they arrive
                    self.process_frames_from_buffer(&mut buffer).await?;
                },
                Ok(None) => {
                    info!("📺 🏁 Stream finished gracefully. Total bytes: {}", total_bytes);
                    break;
                },
                Err(e) => {
                    error!("📺 ❌ Stream read error: {}", e);
                    break;
                }
            }
        }

        Ok(())
    }

    async fn process_frames_from_buffer(&self, buffer: &mut BytesMut) -> Result<()> {
        let mut processed_frames = 0;

        while buffer.len() >= FRAME_HEADER_SIZE {
            // Parse frame header
            let header = match parse_frame_header(&buffer[..FRAME_HEADER_SIZE]) {
                Some(h) if h.magic == MAGIC_STREAM_FRAME => {
                    info!("📺 ✅ Valid frame: #{}, {} bytes", h.frame_number, h.data_size);
                    h
                },
                Some(h) => {
                    warn!("📺 ❌ Invalid magic: expected 0x{:08X}, got 0x{:08X}", 
                          MAGIC_STREAM_FRAME, h.magic);
                    buffer.advance(1);
                    continue;
                },
                None => {
                    warn!("📺 ❌ Failed to parse header");
                    buffer.advance(1);
                    continue;
                }
            };

            // Check if we have complete frame
            let total_frame_size = FRAME_HEADER_SIZE + header.data_size as usize;
            if buffer.len() < total_frame_size {
                info!("📺 ⏳ Incomplete frame: need {} more bytes", 
                      total_frame_size - buffer.len());
                break;
            }

            // Extract frame data
            buffer.advance(FRAME_HEADER_SIZE);
            let frame_data = buffer.split_to(header.data_size as usize).freeze();

            // Process the frame
            self.process_video_frame(header, frame_data).await?;
            processed_frames += 1;
        }

        if processed_frames > 0 {
            info!("🎬 ✅ Processed {} frames", processed_frames);
        }

        Ok(())
    }

    async fn process_video_frame(&self, header: FrameHeader, data: Bytes) -> Result<()> {
        // Handle avcC configuration frames
        if header.frame_type == 0xFF {
            info!("🔧 Received avcC configuration: {} bytes", data.len());
            
            // Store avcC configuration for new clients
            {
                let mut stats = self.stats.write().await;
                stats.avcc_config = Some(data.clone());
            }
            
            // Create special avcC frame
            let avcc_frame = VideoFrame {
                frame_number: 0,
                timestamp: header.timestamp,
                frame_type: 0xFF, // Special avcC type
                data,
            };
            
            // Broadcast avcC to all clients
            match self.frame_sender.send(avcc_frame) {
                Ok(count) => {
                    info!("🔧 📤 Broadcasted avcC config to {} clients", count);
                }
                Err(_) => {
                    info!("🔧 ⚠️ No clients connected for avcC");
                }
            }
            
            return Ok(());
        }
        
        // Update statistics for regular frames
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

        let frame_type = if header.frame_type == 0x01 { "KEYFRAME" } else { "delta" };
        // info!("🎬 Frame #{}: {} bytes ({}) ✅", 
        //       header.frame_number, data.len(), frame_type);

        // Create video frame
        let video_frame = VideoFrame {
            frame_number: header.frame_number,
            timestamp: header.timestamp,
            frame_type: header.frame_type,
            data,
        };

        // Broadcast to clients
        match self.frame_sender.send(video_frame) {
            Ok(count) => {
                // info!("📺 📤 Broadcasted frame #{} to {} clients", 
                //       header.frame_number, count);
            }
            Err(_) => {
                // info!("📺 ⚠️ No clients connected");
            }
        }

        Ok(())
    }
}

impl Clone for VideoRelayServer {
    fn clone(&self) -> Self {
        Self {
            stats: self.stats.clone(),
            frame_sender: self.frame_sender.clone(),
            web_clients: self.web_clients.clone(),
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

async fn report_statistics(stats: Arc<RwLock<ServerStats>>) {
    let mut interval = interval(Duration::from_secs(5));
    
    loop {
        interval.tick().await;
        
        let stats = stats.read().await;
        if stats.total_frames == 0 {
            continue;
        }

        let elapsed = stats.start_time.elapsed().as_secs_f64();
        let fps = stats.total_frames as f64 / elapsed;
        let mbps = (stats.total_bytes as f64 * 8.0) / (1024.0 * 1024.0) / elapsed;

        info!("📊 === SIMPLIFIED QUIC RELAY STATS ===");
        info!("⏱️  Runtime: {:.1}s", elapsed);
        info!("🎬 Frames: {} ({:.1} fps)", stats.total_frames, fps);
        info!("🔑 Keyframes: {} | 📦 Delta: {}", stats.keyframes, stats.delta_frames);
        info!("💾 Data: {:.2} MB ({:.2} Mbps)", 
              stats.total_bytes as f64 / (1024.0 * 1024.0), mbps);
        info!("👥 Clients: {}", stats.connected_clients);
        info!("===============================");
    }
}

fn configure_server(cert: Certificate, key: PrivateKey) -> Result<ServerConfig> {
    let mut crypto_config = rustls::ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)?;
    
    // Simple ALPN setup
    crypto_config.alpn_protocols = vec![
        b"hq-interop".to_vec(),
        b"hq-29".to_vec(),
        b"h3".to_vec(),
    ];
    
    let server_config = ServerConfig::with_crypto(Arc::new(crypto_config));
    info!("✅ QUIC server configured with ALPN: hq-interop, hq-29, h3");
    Ok(server_config)
}

fn load_certificate() -> Result<(Certificate, PrivateKey)> {
    // Try mkcert first, fallback to self-signed
    load_mkcert_certificate().or_else(|_| {
        info!("🔐 mkcert not found, generating self-signed certificate");
        generate_self_signed_cert_and_key()
    })
}

fn load_mkcert_certificate() -> Result<(Certificate, PrivateKey)> {
    use std::fs;
    use std::io::BufReader;
    
    let cert_paths = [
        "server-cert.pem",
        "./server-cert.pem", 
        "../server-cert.pem"
    ];
    
    let key_paths = [
        "server-key.pem",
        "./server-key.pem",
        "../server-key.pem"
    ];
    
    let mut cert_file = None;
    let mut key_file = None;
    
    for path in &cert_paths {
        if std::path::Path::new(path).exists() {
            info!("🔐 Found certificate at: {}", path);
            cert_file = Some(path);
            break;
        }
    }
    
    for path in &key_paths {
        if std::path::Path::new(path).exists() {
            info!("🔐 Found private key at: {}", path);
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
                return Err(anyhow::anyhow!("No certificates found in PEM file"));
            }
            
            let key_file = fs::File::open(key_path)?;
            let mut key_reader = BufReader::new(key_file);
            let keys = rustls_pemfile::pkcs8_private_keys(&mut key_reader)?;
            
            if keys.is_empty() {
                let key_file = fs::File::open(key_path)?;
                let mut key_reader = BufReader::new(key_file);
                let rsa_keys = rustls_pemfile::rsa_private_keys(&mut key_reader)?;
                
                if rsa_keys.is_empty() {
                    return Err(anyhow::anyhow!("No private keys found in PEM file"));
                }
                
                let cert = Certificate(certs[0].clone());
                let key = PrivateKey(rsa_keys[0].clone());
                info!("🔐 Successfully loaded mkcert certificate (RSA key)");
                return Ok((cert, key));
            }
            
            let cert = Certificate(certs[0].clone());
            let key = PrivateKey(keys[0].clone());
            info!("🔐 Successfully loaded mkcert certificate (PKCS8 key)");
            Ok((cert, key))
        }
        _ => {
            Err(anyhow::anyhow!("mkcert certificate files not found"))
        }
    }
}

fn generate_self_signed_cert_and_key() -> Result<(Certificate, PrivateKey)> {
    use rcgen::{Certificate as RcgenCert, CertificateParams, DistinguishedName, SanType};
    
    let mut params = CertificateParams::new(vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
    ]);
    
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
    
    info!("🔐 Generated self-signed certificate");
    Ok((Certificate(cert_der), PrivateKey(key_der)))
}

async fn start_websocket_server(relay_server: VideoRelayServer) {
    use tokio::net::TcpListener;
    use tokio_tungstenite::accept_async;

    info!("🔌 Starting WebSocket server on localhost:8080...");
    
    let listener = match TcpListener::bind("127.0.0.1:8080").await {
        Ok(listener) => listener,
        Err(e) => {
            error!("❌ Failed to bind WebSocket server: {}", e);
            return;
        }
    };

    info!("🔌 ✅ WebSocket server listening on: 127.0.0.1:8080");

    while let Ok((stream, addr)) = listener.accept().await {
        let relay_server = relay_server.clone();
        tokio::spawn(async move {
            // info!("🔌 📞 WebSocket connection from: {}", addr);
            
            match accept_async(stream).await {
                Ok(ws_stream) => {
                    info!("🔌 ✅ WebSocket client connected (USING TCP/HTTP)");
                    handle_websocket_client(ws_stream, relay_server).await;
                }
                Err(e) => {
                    error!("❌ WebSocket handshake failed: {}", e);
                }
            }
        });
    }
}

async fn handle_websocket_client(
    mut ws_stream: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    relay_server: VideoRelayServer,
) {
    info!("🔌 📡 WEBSOCKET SESSION STARTED (TCP/HTTP PROTOCOL)");
    use tokio_tungstenite::tungstenite::Message;
    use base64::Engine;
    
    // Send stored avcC configuration to new client if available
    {
        let stats = relay_server.stats.read().await;
        if let Some(avcc_data) = &stats.avcc_config {
            info!("🔧 📤 Sending stored avcC config to new WebSocket client: {} bytes", avcc_data.len());
            
            let encoded_data = base64::engine::general_purpose::STANDARD.encode(avcc_data);
            let message = serde_json::json!({
                "type": "video_frame",
                "frame_number": 0,
                "timestamp": 0,
                "frame_type": 255, // 0xFF for avcC config
                "data": encoded_data
            });
            
            let json_str = message.to_string();
            if let Err(e) = ws_stream.send(Message::Text(json_str)).await {
                error!("🔧 ❌ Failed to send avcC config via WebSocket: {}", e);
                return;
            }
            
            info!("🔧 ✅ avcC configuration sent to new WebSocket client");
        } else {
            info!("🔧 ⚠️ No avcC configuration available for new WebSocket client");
        }
    }
    
    // Subscribe to video frames
    let mut frame_receiver = relay_server.frame_sender.subscribe();
    
    // Send frames to WebSocket client
    while let Ok(video_frame) = frame_receiver.recv().await {
        // Encode H.264 data as base64 for JSON transport
        let encoded_data = base64::engine::general_purpose::STANDARD.encode(&video_frame.data);
        
        let message = serde_json::json!({
            "type": "video_frame",
            "frame_number": video_frame.frame_number,
            "timestamp": video_frame.timestamp,
            "frame_type": video_frame.frame_type,
            "data": encoded_data
        });
        
        let json_str = message.to_string();
        
        if let Err(e) = ws_stream.send(Message::Text(json_str)).await {
            error!("🔌 ❌ Failed to send frame via WebSocket: {}", e);
            break;
        }
        
        // info!("🔌 📤 Sent frame #{} ({} bytes) via WebSocket", 
        //       video_frame.frame_number, video_frame.data.len());
    }
    
    info!("🔌 🏁 WebSocket client disconnected");
}

async fn start_http_server() {
    use tokio::net::TcpListener;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    info!("🌐 Starting HTTP server on localhost:3000...");
    
    let listener = match TcpListener::bind("127.0.0.1:3000").await {
        Ok(listener) => listener,
        Err(e) => {
            error!("❌ Failed to bind HTTP server: {}", e);
            return;
        }
    };

    info!("🌐 ✅ HTTP server listening on: 127.0.0.1:3000");

    while let Ok((mut stream, _addr)) = listener.accept().await {
        tokio::spawn(async move {
            let mut buffer = [0; 1024];
            
            if let Ok(n) = stream.read(&mut buffer).await {
                let request = String::from_utf8_lossy(&buffer[..n]);
                
                if request.contains("GET / ") {
                    // Serve the HTML file
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
                    
                    if let Err(e) = stream.write_all(response.as_bytes()).await {
                        error!("Failed to send HTTP response: {}", e);
                    }
                } else {
                    // 404 response
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
        .with_env_filter(tracing_subscriber::EnvFilter::new("quic_video_relay=info,warn"))
        .init();

    let server = VideoRelayServer::new();
    
    if let Err(e) = server.start().await {
        error!("❌ Server error: {}", e);
        std::process::exit(1);
    }

    Ok(())
}

async fn start_webtransport_server(relay_server: VideoRelayServer) {
    info!("🚀 Starting WebTransport server on localhost:4433...");
    
    // Load TLS identity for WebTransport
    let identity = match wtransport::Identity::load_pemfiles("server-cert.pem", "server-key.pem").await {
        Ok(identity) => identity,
        Err(e) => {
            error!("❌ Failed to load WebTransport TLS identity: {}", e);
            return;
        }
    };
    
    // Configure WebTransport server
    let config = WtServerConfig::builder()
        .with_bind_default(4433)
        .with_identity(&identity)
        .build();
    
    // Create WebTransport endpoint
    let server = match WtEndpoint::server(config) {
        Ok(server) => server,
        Err(e) => {
            error!("❌ Failed to create WebTransport server: {}", e);
            return;
        }
    };
    
    info!("🚀 ✅ WebTransport server listening on: 127.0.0.1:4433");
    
    // Accept WebTransport connections
    loop {
        let incoming = server.accept().await;
        let session_request = match incoming.await {
            Ok(session_request) => session_request,
            Err(e) => {
                error!("❌ WebTransport connection error: {}", e);
                continue;
            }
        };
        
        let connection = match session_request.accept().await {
            Ok(connection) => connection,
            Err(e) => {
                error!("❌ WebTransport session accept error: {}", e);
                continue;
            }
        };
        
        info!("🚀 ✅ WebTransport client connected from: {} (USING QUIC/UDP)", connection.remote_address());
        
        let relay_server = relay_server.clone();
        tokio::spawn(async move {
            handle_webtransport_session(connection, relay_server).await;
        });
    }
}

async fn handle_webtransport_session(connection: wtransport::Connection, relay_server: VideoRelayServer) {
    info!("🚀 📡 WEBTRANSPORT SESSION STARTED (QUIC/UDP PROTOCOL)");
    let remote_addr = connection.remote_address();
    
    // Subscribe to video frames (same as WebSocket)
    let mut frame_receiver = relay_server.frame_sender.subscribe();
    
    // Open a unidirectional stream to send frames
    let stream = match connection.open_uni().await {
        Ok(stream) => stream,
        Err(e) => {
            error!("🚀 ❌ Failed to open WebTransport stream: {}", e);
            return;
        }
    };
    
    let mut send_stream = match stream.await {
        Ok(send_stream) => send_stream,
        Err(e) => {
            error!("🚀 ❌ Failed to get WebTransport send stream: {}", e);
            return;
        }
    };
    
    info!("🚀 📤 Opened unidirectional stream to WebTransport client");
    
    // Send stored avcC configuration to new client (BINARY FORMAT for WebTransport)
    {
        let stats = relay_server.stats.read().await;
        if let Some(avcc_data) = &stats.avcc_config {
            info!("🚀 📤 Sending stored avcC config to new WebTransport client: {} bytes", avcc_data.len());
            
            // Binary frame format: [frame_type:1][frame_number:4][timestamp:8][data_size:4][data:N]
            let frame_type: u8 = 255; // 0xFF for avcC config
            let frame_number: u32 = 0;
            let timestamp: u64 = 0;
            let data_size: u32 = avcc_data.len() as u32;
            
            // Write binary header
            if let Err(e) = send_stream.write_all(&[frame_type]).await {
                error!("🚀 ❌ Failed to send avcC frame_type via WebTransport: {}", e);
                return;
            }
            if let Err(e) = send_stream.write_all(&frame_number.to_le_bytes()).await {
                error!("🚀 ❌ Failed to send avcC frame_number via WebTransport: {}", e);
                return;
            }
            if let Err(e) = send_stream.write_all(&timestamp.to_le_bytes()).await {
                error!("🚀 ❌ Failed to send avcC timestamp via WebTransport: {}", e);
                return;
            }
            if let Err(e) = send_stream.write_all(&data_size.to_le_bytes()).await {
                error!("🚀 ❌ Failed to send avcC data_size via WebTransport: {}", e);
                return;
            }
            
            // Write binary data (no base64 encoding!)
            if let Err(e) = send_stream.write_all(avcc_data).await {
                error!("🚀 ❌ Failed to send avcC data via WebTransport: {}", e);
                return;
            }
            
            info!("🚀 ✅ avcC configuration sent to new WebTransport client (BINARY)");
        } else {
            info!("🚀 ⚠️ No avcC configuration available for new WebTransport client");
        }
    }
    
    // Send frames to WebTransport client (BINARY FORMAT - no JSON!)
    while let Ok(video_frame) = frame_receiver.recv().await {
        // Binary frame format: [frame_type:1][frame_number:4][timestamp:8][data_size:4][data:N]
        let frame_type: u8 = video_frame.frame_type;
        let frame_number: u32 = video_frame.frame_number as u32;
        let timestamp: u64 = video_frame.timestamp;
        let data_size: u32 = video_frame.data.len() as u32;
        
        // Write binary header (17 bytes total)
        if let Err(e) = send_stream.write_all(&[frame_type]).await {
            error!("🚀 ❌ Failed to send frame_type via WebTransport: {}", e);
            break;
        }
        if let Err(e) = send_stream.write_all(&frame_number.to_le_bytes()).await {
            error!("🚀 ❌ Failed to send frame_number via WebTransport: {}", e);
            break;
        }
        if let Err(e) = send_stream.write_all(&timestamp.to_le_bytes()).await {
            error!("🚀 ❌ Failed to send timestamp via WebTransport: {}", e);
            break;
        }
        if let Err(e) = send_stream.write_all(&data_size.to_le_bytes()).await {
            error!("🚀 ❌ Failed to send data_size via WebTransport: {}", e);
            break;
        }
        
        // Write raw H.264 binary data (no base64 encoding!)
        if let Err(e) = send_stream.write_all(&video_frame.data).await {
            error!("🚀 ❌ Failed to send frame data via WebTransport: {}", e);
            break;
        }
        
        // info!("🚀 📤 Sent frame #{} ({} bytes) via WebTransport (BINARY)", 
        //       video_frame.frame_number, video_frame.data.len());
    }
    
    info!("🚀 🏁 WebTransport client {} disconnected", remote_addr);
}