// Simple MOQ streaming example based on actual working examples from kixelated/moq

use anyhow::Result;
use bytes::Bytes;
use moq_lite;
use moq_native;
use tokio::sync::broadcast;
use tracing::{error, info};

#[derive(Debug, Clone)]
struct VideoFrame {
    frame_number: u64,
    timestamp: u64,
    frame_type: u8,
    data: Bytes,
}

struct MoqVideoServer {
    frame_sender: broadcast::Sender<VideoFrame>,
}

impl MoqVideoServer {
    fn new() -> Self {
        let (frame_sender, _) = broadcast::channel(1000);
        Self { frame_sender }
    }
    
    async fn start(&self) -> Result<()> {
        info!("🚀 Starting MOQ Video Server using proper moq_lite pattern");
        
        // Create MOQ broadcast - this is the correct pattern!
        let broadcast = moq_lite::Broadcast::produce();
        
        // Create MOQ server
        let server_config = moq_native::ServerConfig::default();
        let server = server_config.init()?;
        
        // Start accepting connections and publishing in parallel
        tokio::select! {
            res = self.accept_connections(server, "video_stream".to_string(), broadcast.consumer.clone()) => res,
            res = self.publish_frames(broadcast.producer) => res,
        }
    }
    
    async fn accept_connections(
        &self,
        mut server: moq_native::Server,
        broadcast_name: String,
        consumer: moq_lite::BroadcastConsumer,
    ) -> Result<()> {
        info!("🌐 MOQ server listening on: {:?}", server.local_addr());
        
        let mut conn_id = 0;
        while let Some(session) = server.accept().await {
            let id = conn_id;
            conn_id += 1;
            
            let name = broadcast_name.clone();
            let consumer = consumer.clone();
            
            tokio::spawn(async move {
                if let Err(err) = Self::handle_session(id, session, name, consumer).await {
                    error!("❌ Failed to handle MOQ session {}: {}", id, err);
                }
            });
        }
        
        Ok(())
    }
    
    async fn handle_session(
        id: u64,
        session: moq_native::Request,
        broadcast_name: String,
        consumer: moq_lite::BroadcastConsumer,
    ) -> Result<()> {
        // Accept the session (WebTransport or QUIC)
        let session = session.ok().await?;
        
        // Create an origin to publish the broadcast
        let origin = moq_lite::Origin::produce();
        origin.producer.publish_broadcast(&broadcast_name, consumer);
        
        // Establish the MOQ session
        let session = moq_lite::Session::accept(session, origin.consumer, None).await?;
        
        info!("✅ MOQ session {} accepted", id);
        
        // Wait for session to close
        Err(session.closed().await.into())
    }
    
    async fn publish_frames(&self, producer: moq_lite::BroadcastProducer) -> Result<()> {
        info!("📡 Starting frame publishing");
        
        // Subscribe to video frames
        let mut frame_receiver = self.frame_sender.subscribe();
        
        while let Ok(video_frame) = frame_receiver.recv().await {
            // TODO: Convert video frame to proper MOQ format
            // For now, just log that we received a frame
            info!("📦 Publishing frame #{}: {} bytes", 
                  video_frame.frame_number, video_frame.data.len());
            
            // The actual frame publishing would happen here using the producer
            // This requires understanding the specific media format (CMAF, etc.)
        }
        
        Ok(())
    }
    
    // This would be called from QUIC handler to inject frames
    pub fn send_frame(&self, frame: VideoFrame) {
        let _ = self.frame_sender.send(frame);
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    
    let server = MoqVideoServer::new();
    server.start().await?;
    
    Ok(())
}
