package main

import (
	"context"
	"crypto/tls"
	"encoding/binary"
	"encoding/json"
	"fmt"
	"log"
	"net"
	"os"
	"sync"
	"time"
	"unsafe"

	"github.com/quic-go/quic-go"
)

// Video frame header structure (matches Swift client)
type VideoFrameHeader struct {
	FrameNumber uint64
	Timestamp   uint64
	IsKeyframe  uint8
	DataSize    uint32
}

// Control message structure
type ControlMessage struct {
	Type      string                 `json:"type"`
	Timestamp float64               `json:"timestamp"`
	Data      map[string]interface{} `json:"data"`
}

// Video stream state
type VideoStream struct {
	mutex              sync.RWMutex
	lastFrameNumber    uint64
	lastKeyframeTime   time.Time
	droppedFrames      uint64
	receivedFrames     uint64
	totalBytes         uint64
	keyframeRequested  bool
}

// Frame synchronizer
type FrameSynchronizer struct {
	mutex          sync.RWMutex
	frameBuffer    map[uint64]*ReceivedFrame
	lastProcessed  uint64
	keyframeCache  map[uint64]*ReceivedFrame
	stats          SyncStats
}

type ReceivedFrame struct {
	FrameNumber uint64
	Timestamp   uint64
	IsKeyframe  bool
	Data        []byte
	Transport   TransportType
	ReceivedAt  time.Time
}

type TransportType int

const (
	TransportDatagram TransportType = iota
	TransportStream
)

type SyncStats struct {
	TotalFrames    uint64
	DroppedFrames  uint64
	OutOfOrder     uint64
	KeyframesMissed uint64
	LastKeyframe   time.Time
}

// Magic numbers
const (
	MagicKeyframe = 0x4B455946 // "KEYF"
	MagicDatagram = 0x44415441 // "DATA"
	MagicChunk    = 0x4348554E // "CHUN"
)

// 🧩 Chunk header structure (matches Swift client)
type ChunkHeader struct {
	Magic       uint32
	FrameId     uint32
	ChunkIndex  uint16
	ChunkCount  uint16
	Flags       uint8
	TimestampMs uint32
}

// 🧩 Frame reassembly state
type FragmentedFrame struct {
	FrameId     uint32
	ChunkCount  uint16
	IsKeyframe  bool
	TimestampMs uint32
	Chunks      map[uint16][]byte // chunkIndex -> chunk data
	ReceivedAt  time.Time
}

// 🧩 Frame reassembler
type FrameReassembler struct {
	mutex           sync.RWMutex
	pendingFrames   map[uint32]*FragmentedFrame // frameId -> frame
	completedFrames chan *ReceivedFrame
}

func NewFrameSynchronizer() *FrameSynchronizer {
	return &FrameSynchronizer{
		frameBuffer:   make(map[uint64]*ReceivedFrame),
		keyframeCache: make(map[uint64]*ReceivedFrame),
	}
}

func NewFrameReassembler() *FrameReassembler {
	return &FrameReassembler{
		pendingFrames:   make(map[uint32]*FragmentedFrame),
		completedFrames: make(chan *ReceivedFrame, 100),
	}
}

// 🧩 Process incoming chunk
func (fr *FrameReassembler) ProcessChunk(data []byte) {
	if len(data) < 15 { // Size of ChunkHeader
		log.Printf("❌ Invalid chunk size: %d", len(data))
		return
	}

	// Parse chunk header
	magic := binary.LittleEndian.Uint32(data[0:4])
	if magic != MagicChunk {
		log.Printf("❌ Invalid chunk magic: 0x%X", magic)
		return
	}

	frameId := binary.LittleEndian.Uint32(data[4:8])
	chunkIndex := binary.LittleEndian.Uint16(data[8:10])
	chunkCount := binary.LittleEndian.Uint16(data[10:12])
	flags := data[12]
	timestampMs := binary.LittleEndian.Uint32(data[13:17])

	isKeyframe := (flags & 0x01) != 0
	chunkData := data[17:]

	fr.mutex.Lock()
	defer fr.mutex.Unlock()

	// Get or create fragmented frame
	frame, exists := fr.pendingFrames[frameId]
	if !exists {
		frame = &FragmentedFrame{
			FrameId:     frameId,
			ChunkCount:  chunkCount,
			IsKeyframe:  isKeyframe,
			TimestampMs: timestampMs,
			Chunks:      make(map[uint16][]byte),
			ReceivedAt:  time.Now(),
		}
		fr.pendingFrames[frameId] = frame
		log.Printf("🧩 New fragmented frame #%d: %d chunks expected", frameId, chunkCount)
	}

	// Store chunk data
	frame.Chunks[chunkIndex] = chunkData
	
	// Show current reassembly progress
	receivedChunks := make([]int, 0, len(frame.Chunks))
	for idx := range frame.Chunks {
		receivedChunks = append(receivedChunks, int(idx)+1)
	}
	
	// Sort for display
	for i := 0; i < len(receivedChunks)-1; i++ {
		for j := i + 1; j < len(receivedChunks); j++ {
			if receivedChunks[i] > receivedChunks[j] {
				receivedChunks[i], receivedChunks[j] = receivedChunks[j], receivedChunks[i]
			}
		}
	}
	
	chunksStr := ""
	for i, chunk := range receivedChunks {
		if i > 0 {
			chunksStr += ","
		}
		chunksStr += fmt.Sprintf("%d", chunk)
	}
	
	log.Printf("📦 Chunk received: Frame #%d chunk %d/%d (%d bytes) - Have: [%s]", 
		frameId, chunkIndex+1, chunkCount, len(chunkData), chunksStr)

	// Check if frame is complete
	if len(frame.Chunks) == int(chunkCount) {
		log.Printf("✅ Frame #%d COMPLETE - All %d chunks received, assembling...", frameId, chunkCount)
		fr.assembleFrame(frame)
		delete(fr.pendingFrames, frameId)
	} else {
		missing := int(chunkCount) - len(frame.Chunks)
		log.Printf("⏳ Frame #%d WAITING - Need %d more chunks", frameId, missing)
	}
}

// 🧩 Assemble complete frame from chunks
func (fr *FrameReassembler) assembleFrame(frame *FragmentedFrame) {
	// Calculate total size
	totalSize := 0
	for i := uint16(0); i < frame.ChunkCount; i++ {
		if chunk, exists := frame.Chunks[i]; exists {
			totalSize += len(chunk)
		} else {
			log.Printf("❌ Missing chunk %d for frame #%d", i, frame.FrameId)
			return
		}
	}

	// Assemble frame data
	assembledData := make([]byte, 0, totalSize)
	for i := uint16(0); i < frame.ChunkCount; i++ {
		assembledData = append(assembledData, frame.Chunks[i]...)
	}

	// Create received frame
	receivedFrame := &ReceivedFrame{
		FrameNumber: uint64(frame.FrameId),
		Timestamp:   uint64(frame.TimestampMs),
		IsKeyframe:  frame.IsKeyframe,
		Data:        assembledData,
		Transport:   TransportDatagram,
		ReceivedAt:  frame.ReceivedAt,
	}

	frameType := "delta"
	if frame.IsKeyframe {
		frameType = "KEYFRAME"
	}

	log.Printf("🧩 ASSEMBLED frame #%d: %d bytes (%s) from %d chunks", 
		frame.FrameId, len(assembledData), frameType, frame.ChunkCount)

	// Send to completed frames channel
	select {
	case fr.completedFrames <- receivedFrame:
	default:
		log.Printf("⚠️  Completed frames channel full, dropping frame #%d", frame.FrameId)
	}
}

// 🧩 Get completed frame (non-blocking)
func (fr *FrameReassembler) GetCompletedFrame() *ReceivedFrame {
	select {
	case frame := <-fr.completedFrames:
		return frame
	default:
		return nil
	}
}

// 🧩 Cleanup old incomplete frames
func (fr *FrameReassembler) Cleanup() {
	fr.mutex.Lock()
	defer fr.mutex.Unlock()

	cutoff := time.Now().Add(-5 * time.Second)
	for frameId, frame := range fr.pendingFrames {
		if frame.ReceivedAt.Before(cutoff) {
			log.Printf("🧹 Cleaning up incomplete frame #%d (%d/%d chunks)", 
				frameId, len(frame.Chunks), frame.ChunkCount)
			delete(fr.pendingFrames, frameId)
		}
	}
}

func (fs *FrameSynchronizer) ProcessDatagramFrame(data []byte) {
	if len(data) < 25 { // Size of DatagramFrameHeader
		log.Printf("❌ Invalid datagram frame size: %d", len(data))
		return
	}

	// Parse datagram header
	magic := binary.LittleEndian.Uint32(data[0:4])
	
	if magic == MagicKeyframe {
		// This is a large keyframe sent as datagram (shouldn't happen with proper routing)
		fs.ProcessStreamFrame(data)
		return
	} else if magic == MagicChunk {
		log.Printf("🧩 Received chunk datagram - should be handled by reassembler")
		return
	} else if magic != MagicDatagram {
		log.Printf("❌ Invalid datagram magic: 0x%X", magic)
		return
	}

	frameNumber := binary.LittleEndian.Uint64(data[4:12])
	timestamp := binary.LittleEndian.Uint64(data[12:20])
	isKeyframe := data[20] != 0
	dataSize := binary.LittleEndian.Uint32(data[21:25])

	frameData := data[25:]
	if len(frameData) != int(dataSize) {
		log.Printf("❌ Data size mismatch: expected %d, got %d", dataSize, len(frameData))
		return
	}

	frame := &ReceivedFrame{
		FrameNumber: frameNumber,
		Timestamp:   timestamp,
		IsKeyframe:  isKeyframe,
		Data:        frameData,
		Transport:   TransportDatagram,
		ReceivedAt:  time.Now(),
	}

	fs.addFrame(frame)
}

func (fs *FrameSynchronizer) ProcessStreamFrame(data []byte) {
	if len(data) < 25 { // Size of StreamFrameHeader
		log.Printf("❌ Invalid stream frame size: %d", len(data))
		return
	}

	// Parse stream header
	magic := binary.LittleEndian.Uint32(data[0:4])
	if magic != MagicKeyframe {
		log.Printf("❌ Invalid stream magic: 0x%X", magic)
		return
	}

	frameNumber := binary.LittleEndian.Uint64(data[4:12])
	timestamp := binary.LittleEndian.Uint64(data[12:20])
	dataSize := binary.LittleEndian.Uint32(data[20:24])
	isKeyframe := data[24] != 0

	frameData := data[25:]
	if len(frameData) != int(dataSize) {
		log.Printf("❌ Data size mismatch: expected %d, got %d", dataSize, len(frameData))
		return
	}

	frame := &ReceivedFrame{
		FrameNumber: frameNumber,
		Timestamp:   timestamp,
		IsKeyframe:  isKeyframe,
		Data:        frameData,
		Transport:   TransportStream,
		ReceivedAt:  time.Now(),
	}

	fs.addFrame(frame)
}

func (fs *FrameSynchronizer) addFrame(frame *ReceivedFrame) {
	fs.mutex.Lock()
	defer fs.mutex.Unlock()

	fs.stats.TotalFrames++
	
	if frame.IsKeyframe {
		fs.stats.LastKeyframe = time.Now()
		fs.keyframeCache[frame.FrameNumber] = frame
		transportName := "DATAGRAM"
		if frame.Transport == TransportStream {
			transportName = "STREAM"
		}
		log.Printf("🔑 KEYFRAME #%d received via %s (%d bytes)", 
			frame.FrameNumber, transportName, len(frame.Data))
	}

	if frame.FrameNumber < fs.lastProcessed {
		fs.stats.OutOfOrder++
		log.Printf("⚠️  Out-of-order frame #%d (last processed: #%d)", 
			frame.FrameNumber, fs.lastProcessed)
	}

	fs.frameBuffer[frame.FrameNumber] = frame
	fs.processFrameBuffer()
}

func (fs *FrameSynchronizer) processFrameBuffer() {
	nextFrame := fs.lastProcessed + 1
	
	for {
		frame, exists := fs.frameBuffer[nextFrame]
		if !exists {
			break
		}
		
		if fs.canDecodeFrame(frame) {
			fs.decodeFrame(frame)
			delete(fs.frameBuffer, nextFrame)
			fs.lastProcessed = nextFrame
			nextFrame++
		} else {
			log.Printf("⏳ Frame #%d waiting for keyframe dependency", nextFrame)
			break
		}
	}
	
	// Cleanup old frames
	cutoff := time.Now().Add(-5 * time.Second)
	for frameNum, frame := range fs.frameBuffer {
		if frame.ReceivedAt.Before(cutoff) {
			delete(fs.frameBuffer, frameNum)
		}
	}
}

func (fs *FrameSynchronizer) canDecodeFrame(frame *ReceivedFrame) bool {
	if frame.IsKeyframe {
		return true
	}
	return fs.hasRecentKeyframe(frame.FrameNumber)
}

func (fs *FrameSynchronizer) hasRecentKeyframe(frameNumber uint64) bool {
	searchStart := uint64(1)
	if frameNumber > 60 {
		searchStart = frameNumber - 60
	}
	
	for i := frameNumber; i >= searchStart; i-- {
		if _, exists := fs.keyframeCache[i]; exists {
			return true
		}
		if i <= fs.lastProcessed {
			return true
		}
	}
	return false
}

func (fs *FrameSynchronizer) decodeFrame(frame *ReceivedFrame) {
	transportType := "DATAGRAM"
	if frame.Transport == TransportStream {
		transportType = "STREAM"
	}
	frameType := "delta"
	if frame.IsKeyframe {
		frameType = "KEYFRAME"
	}
	
	log.Printf("🎬 DECODE frame #%d: %d bytes (%s) via %s", 
		frame.FrameNumber, len(frame.Data), frameType, transportType)
}

func (fs *FrameSynchronizer) PrintStats() {
	fs.mutex.RLock()
	defer fs.mutex.RUnlock()
	
	log.Printf("📊 SYNC STATS:")
	log.Printf("   Total frames: %d", fs.stats.TotalFrames)
	log.Printf("   Last processed: #%d", fs.lastProcessed)
	log.Printf("   Out of order: %d", fs.stats.OutOfOrder)
	log.Printf("   Buffer size: %d frames", len(fs.frameBuffer))
	log.Printf("   Keyframe cache: %d frames", len(fs.keyframeCache))
	
	if !fs.stats.LastKeyframe.IsZero() {
		age := time.Since(fs.stats.LastKeyframe)
		log.Printf("   Last keyframe: %.1fs ago", age.Seconds())
	}
}

func main() {
	// Load TLS certificates
	cert, err := tls.LoadX509KeyPair("cert/localhost.pem", "cert/localhost-key.pem")
	if err != nil {
		log.Fatal("❌ Failed to load certificates:", err)
	}

	// Create TLS config
	tlsConfig := &tls.Config{
		Certificates: []tls.Certificate{cert},
		NextProtos:   []string{"video-stream"}, // Match client ALPN
	}

	// 🚀 High-performance QUIC config for video streaming
	quicConfig := &quic.Config{
		EnableDatagrams:            true,
		MaxIdleTimeout:             time.Minute * 5,
		MaxIncomingStreams:         1000,        // Allow many control streams
		MaxIncomingUniStreams:      1000,        // Allow many uni-directional streams
		MaxStreamReceiveWindow:     10 << 20,    // 10MB receive window per stream
		MaxConnectionReceiveWindow: 100 << 20,   // 100MB total receive window
		KeepAlivePeriod:           30 * time.Second,
		InitialStreamReceiveWindow: 1 << 20,     // 1MB initial stream window
		InitialConnectionReceiveWindow: 10 << 20, // 10MB initial connection window
	}

	// Listen on UDP port
	addr, err := net.ResolveUDPAddr("udp", ":8443")
	if err != nil {
		log.Fatal("❌ Failed to resolve address:", err)
	}

	conn, err := net.ListenUDP("udp", addr)
	if err != nil {
		log.Fatal("❌ Failed to listen on UDP:", err)
	}

	// 🚀 Optimize UDP socket buffers for high-throughput video streaming
	// Set large receive buffer (50MB) to handle burst traffic
	if err := conn.SetReadBuffer(50 * 1024 * 1024); err != nil {
		log.Printf("⚠️  Warning: Failed to set UDP read buffer: %v", err)
	} else {
		log.Printf("🚀 UDP read buffer set to 50MB")
	}
	
	// Set large write buffer (10MB) for responses
	if err := conn.SetWriteBuffer(10 * 1024 * 1024); err != nil {
		log.Printf("⚠️  Warning: Failed to set UDP write buffer: %v", err)
	} else {
		log.Printf("🚀 UDP write buffer set to 10MB")
	}

	// Create QUIC listener
	listener, err := quic.Listen(conn, tlsConfig, quicConfig)
	if err != nil {
		log.Fatal("❌ Failed to create QUIC listener:", err)
	}
	defer listener.Close()

	log.Println("🌟 ================================")
	log.Println("🎥 Video Streaming Server Started")
	log.Println("🌟 ================================")
	log.Printf("🔗 Listening on: localhost:8443")
	log.Printf("📡 QUIC datagrams: ENABLED (video)")
	log.Printf("🔄 QUIC streams: ENABLED (control)")
	log.Println("🚀 High-Performance Configuration:")
	log.Printf("   📊 Max streams: %d", quicConfig.MaxIncomingStreams)
	log.Printf("   🌊 Connection window: %dMB", quicConfig.MaxConnectionReceiveWindow/(1<<20))
	log.Printf("   📦 Stream window: %dMB", quicConfig.MaxStreamReceiveWindow/(1<<20))
	log.Println("🌟 ================================")

	// Accept connections
	for {
		session, err := listener.Accept(context.Background())
		if err != nil {
			log.Printf("❌ Failed to accept connection: %v", err)
			continue
		}

		log.Printf("🔗 New video client connected: %s", session.RemoteAddr())

		// Handle each connection in a goroutine
		go handleVideoConnection(session)
	}
}

func handleVideoConnection(session quic.Connection) {
	defer session.CloseWithError(0, "connection closed")

	videoStream := &VideoStream{
		lastKeyframeTime: time.Now(),
	}

	// Create frame synchronizer
	synchronizer := NewFrameSynchronizer()

	log.Printf("✅ Video connection established with %s", session.RemoteAddr())

	// Start goroutines for different types of communication
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	// Create frame reassembler
	reassembler := NewFrameReassembler()

	// Handle datagrams (video frames + fragmented frames)
	go handleVideoDatagrams(ctx, session, videoStream, synchronizer, reassembler)

	// Handle reliable streams (keyframes and control)
	go handleControlStreams(ctx, session, videoStream, synchronizer)

	// Handle reassembled frames
	go handleReassembledFrames(ctx, reassembler, synchronizer)

	// Statistics reporting
	go reportStatistics(ctx, session, videoStream, synchronizer)

	// Cleanup timer for incomplete fragments
	go cleanupTimer(ctx, reassembler)

	// Keep connection alive
	select {
	case <-ctx.Done():
		log.Printf("🔌 Video connection closed with %s", session.RemoteAddr())
	}
}

func handleVideoDatagrams(ctx context.Context, session quic.Connection, stream *VideoStream, synchronizer *FrameSynchronizer, reassembler *FrameReassembler) {
	// 🚀 High-performance async processing with buffered channels
	const bufferSize = 10000 // Buffer up to 10K datagrams
	datagramChan := make(chan []byte, bufferSize)
	
	// Start multiple worker goroutines to process datagrams
	const numWorkers = 8
	for i := 0; i < numWorkers; i++ {
		go func(workerID int) {
			for {
				select {
				case <-ctx.Done():
					return
				case data := <-datagramChan:
					// Process datagram asynchronously
					processDatagram(data, session, stream, synchronizer, reassembler, workerID)
				}
			}
		}(i)
	}
	
	// Main receive loop - just receive and queue
	for {
		select {
		case <-ctx.Done():
			return
		default:
		}

		data, err := session.ReceiveDatagram(ctx)
		if err != nil {
			if err.Error() != "connection closed" {
				log.Printf("❌ Datagram receive error: %v", err)
			}
			return
		}

		// Queue datagram for async processing
		select {
		case datagramChan <- data:
			// Successfully queued
		default:
			// Buffer full - drop oldest or handle backpressure
			log.Printf("⚠️  Datagram buffer full, dropping packet")
		}
	}
}

// 🚀 Process individual datagram (called by worker goroutines)
func processDatagram(data []byte, session quic.Connection, stream *VideoStream, synchronizer *FrameSynchronizer, reassembler *FrameReassembler, workerID int) {
	// Debug: Log all incoming datagrams (reduced logging for performance)
	if len(data) > 0 {
		// Only log every 100th packet to reduce spam
		if data[0] == 0x4E { // Chunk packets
			// No logging for chunks to improve performance
		} else {
			log.Printf("🔍 Worker %d: Received datagram %d bytes, first byte: 0x%02X", workerID, len(data), data[0])
		}
	}
	
	// Check if this is a control message (starts with 0xFF)
	if len(data) > 0 && data[0] == 0xFF {
		log.Printf("📋 Worker %d: Processing control message", workerID)
		handleControlDatagram(data[1:], session, stream)
		return
	}

	// Check magic number to route to correct handler
	if len(data) >= 4 {
		magic := binary.LittleEndian.Uint32(data[0:4])
		if magic == MagicChunk {
			// Process chunk without logging for performance
			reassembler.ProcessChunk(data)
			return
		}
	}

	// Process regular video frame with synchronizer
	log.Printf("📦 Worker %d: Processing video frame datagram", workerID)
	synchronizer.ProcessDatagramFrame(data)
}

func processVideoFrame(data []byte, session quic.Connection, stream *VideoStream) {
	if len(data) < int(unsafe.Sizeof(VideoFrameHeader{})) {
		log.Printf("❌ Invalid frame data size: %d", len(data))
		return
	}

	// Parse header
	headerSize := int(unsafe.Sizeof(VideoFrameHeader{}))
	header := (*VideoFrameHeader)(unsafe.Pointer(&data[0]))
	
	// Convert from little endian if needed
	frameNumber := header.FrameNumber
	timestamp := header.Timestamp
	isKeyframe := header.IsKeyframe != 0
	dataSize := header.DataSize

	// Extract video data
	videoData := data[headerSize:]
	if len(videoData) != int(dataSize) {
		log.Printf("❌ Data size mismatch: expected %d, got %d", dataSize, len(videoData))
		return
	}

	stream.mutex.Lock()
	defer stream.mutex.Unlock()

	// Update statistics
	stream.receivedFrames++
	stream.totalBytes += uint64(len(data))

	// Check for dropped frames
	if frameNumber > stream.lastFrameNumber+1 {
		dropped := frameNumber - stream.lastFrameNumber - 1
		stream.droppedFrames += dropped
		log.Printf("⚠️  Dropped %d frames (gap: %d -> %d)", dropped, stream.lastFrameNumber, frameNumber)
		
		// Request keyframe if too many drops
		if dropped > 5 && !stream.keyframeRequested {
			stream.keyframeRequested = true
			go requestKeyframe(session)
		}
	}

	stream.lastFrameNumber = frameNumber

	frameType := "delta"
	if isKeyframe {
		frameType = "KEYFRAME"
		stream.lastKeyframeTime = time.Now()
		stream.keyframeRequested = false // Reset keyframe request
	}

	log.Printf("📦 Video frame #%d: %d bytes (%s) [timestamp: %d]", 
		frameNumber, len(videoData), frameType, timestamp)

	// Here you would typically:
	// 1. Decode the H.264/VP8 data
	// 2. Display the frame
	// 3. Save to file for analysis
	
	// For now, just save raw frames to files for debugging
	if isKeyframe {
		saveFrameToFile(videoData, frameNumber, "keyframe")
	}
}

func handleControlDatagram(data []byte, session quic.Connection, stream *VideoStream) {
	var msg ControlMessage
	if err := json.Unmarshal(data, &msg); err != nil {
		log.Printf("❌ Failed to parse control message: %v", err)
		return
	}

	log.Printf("📋 Control message: %s (timestamp: %.3f)", msg.Type, msg.Timestamp)

	switch msg.Type {
	case "client_ready":
		log.Printf("✅ Client is ready for video streaming")
		
	case "keyframe_request":
		log.Printf("🔑 Client requested keyframe")
		// Forward keyframe request if needed
		
	case "quality_change":
		if bitrate, ok := msg.Data["bitrate"].(float64); ok {
			log.Printf("📊 Client requested bitrate change: %.0f kbps", bitrate/1000)
		}
	}
}

func handleControlStreams(ctx context.Context, session quic.Connection, stream *VideoStream, synchronizer *FrameSynchronizer) {
	// TODO: Implement proper QUIC stream handling
	// For now, we handle stream frames that come as datagrams
	log.Printf("🔄 Control streams handler ready")
	
	// This would normally accept QUIC streams and process them
	// For the current implementation, stream frames are sent as large datagrams
	// and handled by the synchronizer
}

func requestKeyframe(session quic.Connection) {
	log.Printf("🔑 Requesting keyframe from client")
	
	msg := ControlMessage{
		Type:      "keyframe_request",
		Timestamp: float64(time.Now().UnixMilli()),
		Data: map[string]interface{}{
			"reason": "packet_loss",
		},
	}
	
	jsonData, _ := json.Marshal(msg)
	
	// Send as control datagram
	var controlPacket []byte
	controlPacket = append(controlPacket, 0xFF) // Control marker
	controlPacket = append(controlPacket, jsonData...)
	
	if err := session.SendDatagram(controlPacket); err != nil {
		log.Printf("❌ Failed to send keyframe request: %v", err)
	}
}

func reportStatistics(ctx context.Context, session quic.Connection, stream *VideoStream, synchronizer *FrameSynchronizer) {
	ticker := time.NewTicker(5 * time.Second)
	defer ticker.Stop()
	
	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			// Print synchronizer statistics
			synchronizer.PrintStats()
			
			stream.mutex.RLock()
			received := stream.receivedFrames
			dropped := stream.droppedFrames
			totalBytes := stream.totalBytes
			timeSinceKeyframe := time.Since(stream.lastKeyframeTime)
			stream.mutex.RUnlock()
			
			if received > 0 {
				dropRate := float64(dropped) / float64(received+dropped) * 100
				throughput := float64(totalBytes) / 1024 / 1024 // MB
				
				log.Printf("📊 Legacy Stats: %d frames received, %d dropped (%.1f%%), %.2f MB total, keyframe age: %.1fs",
					received, dropped, dropRate, throughput, timeSinceKeyframe.Seconds())
			}
		}
	}
}

func saveFrameToFile(data []byte, frameNumber uint64, frameType string) {
	// Create frames directory if it doesn't exist
	os.MkdirAll("frames", 0755)
	
	filename := fmt.Sprintf("frames/frame_%06d_%s.h264", frameNumber, frameType)
	if err := os.WriteFile(filename, data, 0644); err != nil {
		log.Printf("❌ Failed to save frame: %v", err)
	} else {
		log.Printf("💾 Saved %s frame to %s (%d bytes)", frameType, filename, len(data))
	}
}

// 🧩 Handle reassembled frames from the reassembler
func handleReassembledFrames(ctx context.Context, reassembler *FrameReassembler, synchronizer *FrameSynchronizer) {
	ticker := time.NewTicker(1 * time.Millisecond) // Check frequently for completed frames
	defer ticker.Stop()

	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			// Check for completed frames
			for {
				frame := reassembler.GetCompletedFrame()
				if frame == nil {
					break
				}
				
				// Send reassembled frame to synchronizer
				synchronizer.addFrame(frame)
			}
		}
	}
}

// 🧩 Cleanup timer for incomplete fragmented frames
func cleanupTimer(ctx context.Context, reassembler *FrameReassembler) {
	ticker := time.NewTicker(2 * time.Second) // Cleanup every 2 seconds
	defer ticker.Stop()

	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			reassembler.Cleanup()
		}
	}
}
