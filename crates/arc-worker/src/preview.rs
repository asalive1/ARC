use arc_application::{AudioPreview, PortError, PortResult};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::{tungstenite::Message, WebSocketStream};

const MAX_PACKET_BYTES: usize = 4096;
const MAX_DELIVERY_PACKETS: usize = 16;
pub(crate) const MAX_PREVIEW_INBOUND_BYTES: usize = 1024;
const PREVIEW_WRITE_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Eq, PartialEq)]
pub enum PreviewEnd {
    Stopped,
    ConsumerDisconnected,
}

#[derive(Debug, Eq, PartialEq)]
pub struct PreviewRelayReport {
    pub end: PreviewEnd,
    pub packets_sent: u64,
    pub packets_dropped: u64,
}

/// Send PCM over an already-authenticated, dedicated WebSocket with bounded input.
/// The caller must await this future through shutdown to observe cleanup failures.
pub async fn send_preview_over_websocket<S>(
    mut preview: Box<dyn AudioPreview>,
    mut socket: WebSocketStream<S>,
    stop: watch::Receiver<bool>,
) -> PortResult<PreviewRelayReport>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let config = socket.get_config();
    if !matches!(config.max_message_size, Some(size) if size <= MAX_PREVIEW_INBOUND_BYTES)
        || !matches!(config.max_frame_size, Some(size) if size <= MAX_PREVIEW_INBOUND_BYTES)
    {
        let error = preview_error(
            "preview_transport_config_invalid",
            "preview WebSocket input must be bounded",
        );
        return match preview.close().await {
            Ok(()) => Err(error),
            Err(cleanup) => Err(preview_error(
                "preview_shutdown_failed",
                format!(
                    "{}: {}; {}: {}",
                    error.code, error.message, cleanup.code, cleanup.message
                ),
            )),
        };
    }
    let (sender, mut packets) = mpsc::channel(MAX_DELIVERY_PACKETS);
    let relay = relay_preview(preview, sender, stop);
    tokio::pin!(relay);
    let mut relay_result = None;
    let mut peer_closed = false;
    let transport_result = loop {
        tokio::select! {
            result = &mut relay => {
                relay_result = Some(result);
                break Ok(());
            }
            packet = packets.recv() => {
                let Some(packet) = packet else { break Ok(()); };
                if let Err(error) = send_preview_message(&mut socket, Message::Binary(packet.into())).await {
                    break Err(error);
                }
            }
            message = socket.next() => {
                match message {
                    Some(Ok(Message::Ping(_))) => {
                        if let Err(error) = flush_preview_socket(&mut socket).await {
                            break Err(error);
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) => {
                        peer_closed = true;
                        break Ok(());
                    }
                    None => break Err(preview_error(
                        "preview_transport_failed",
                        "preview connection ended without a close message",
                    )),
                    Some(Ok(_)) => break Err(preview_error(
                        "preview_protocol_error",
                        "preview consumer sent an unexpected message",
                    )),
                    Some(Err(error)) => break Err(preview_error(
                        "preview_transport_failed",
                        format!("preview connection failed: {error}"),
                    )),
                }
            }
        }
    };
    drop(packets);
    let relay_result = match relay_result {
        Some(result) => result,
        None => relay.await,
    };
    let close_result = if peer_closed {
        flush_preview_socket(&mut socket).await
    } else if transport_result.is_err() {
        Ok(())
    } else {
        send_preview_message(&mut socket, Message::Close(None)).await
    };
    match (transport_result, relay_result, close_result) {
        (Ok(()), Ok(report), Ok(())) => Ok(report),
        (transport, relay, close) => {
            let errors: Vec<PortError> = [transport.err(), relay.err(), close.err()]
                .into_iter()
                .flatten()
                .collect();
            if errors.len() == 1 {
                return Err(errors.into_iter().next().expect("one error"));
            }
            Err(preview_error(
                "preview_shutdown_failed",
                errors
                    .into_iter()
                    .map(|error| format!("{}: {}", error.code, error.message))
                    .collect::<Vec<_>>()
                    .join("; "),
            ))
        }
    }
}

async fn flush_preview_socket<S>(socket: &mut WebSocketStream<S>) -> PortResult<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    tokio::time::timeout(PREVIEW_WRITE_TIMEOUT, socket.flush())
        .await
        .map_err(|_| {
            preview_error(
                "preview_transport_timeout",
                "preview connection is too slow",
            )
        })?
        .map_err(|error| preview_error("preview_transport_failed", error.to_string()))
}

async fn send_preview_message<S>(
    socket: &mut WebSocketStream<S>,
    message: Message,
) -> PortResult<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    tokio::time::timeout(PREVIEW_WRITE_TIMEOUT, socket.send(message))
        .await
        .map_err(|_| {
            preview_error(
                "preview_transport_timeout",
                "preview connection is too slow",
            )
        })?
        .map_err(|error| preview_error("preview_transport_failed", error.to_string()))
}

/// Relay an active preview without allowing a slow consumer to block media.
/// Await the relay through shutdown so its close result is observed.
pub async fn relay_preview(
    mut preview: Box<dyn AudioPreview>,
    output: mpsc::Sender<Vec<u8>>,
    mut stop: watch::Receiver<bool>,
) -> PortResult<PreviewRelayReport> {
    let result = if output.max_capacity() > MAX_DELIVERY_PACKETS {
        Err(preview_error(
            "preview_delivery_capacity_invalid",
            "preview delivery queue exceeds the worker limit",
        ))
    } else {
        run_relay(preview.as_mut(), &output, &mut stop).await
    };
    match (result, preview.close().await) {
        (Ok(report), Ok(())) => Ok(report),
        (Err(error), Ok(())) | (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(cleanup)) => Err(preview_error(
            "preview_cleanup_failed",
            format!(
                "{}: {}; preview cleanup also failed ({}: {})",
                error.code, error.message, cleanup.code, cleanup.message
            ),
        )),
    }
}

async fn run_relay(
    preview: &mut dyn AudioPreview,
    output: &mpsc::Sender<Vec<u8>>,
    stop: &mut watch::Receiver<bool>,
) -> PortResult<PreviewRelayReport> {
    let mut sent = 0u64;
    let mut dropped = 0u64;
    let end = loop {
        if *stop.borrow() {
            break PreviewEnd::Stopped;
        }
        tokio::select! {
            biased;
            _ = output.closed() => break PreviewEnd::ConsumerDisconnected,
            changed = stop.changed() => {
                if changed.is_err() {
                    return Err(preview_error(
                        "preview_control_ended",
                        "preview control ended without a stop request",
                    ));
                }
            }
            packet = preview.recv() => {
                let packet = packet?;
                if packet.is_empty() || packet.len() > MAX_PACKET_BYTES || packet.len() % 4 != 0 {
                    return Err(preview_error(
                        "preview_packet_invalid",
                        "preview produced an invalid stereo PCM packet",
                    ));
                }
                match output.try_send(packet) {
                    Ok(()) => sent += 1,
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        dropped += 1;
                        tokio::task::yield_now().await;
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => break PreviewEnd::ConsumerDisconnected,
                }
            }
        }
    };
    Ok(PreviewRelayReport {
        end,
        packets_sent: sent,
        packets_dropped: dropped,
    })
}

fn preview_error(code: &'static str, message: impl Into<String>) -> PortError {
    PortError {
        code,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc,
        },
        time::Duration,
    };

    use super::*;

    struct TestPreview {
        packets: mpsc::Receiver<Vec<u8>>,
        received: Arc<AtomicUsize>,
        closed: Arc<AtomicBool>,
        close_fails: bool,
    }

    #[async_trait::async_trait]
    impl AudioPreview for TestPreview {
        async fn recv(&mut self) -> PortResult<Vec<u8>> {
            let packet = self
                .packets
                .recv()
                .await
                .ok_or_else(|| preview_error("preview_source_ended", "test source ended"))?;
            self.received.fetch_add(1, Ordering::SeqCst);
            Ok(packet)
        }

        async fn close(&mut self) -> PortResult<()> {
            self.closed.store(true, Ordering::SeqCst);
            if self.close_fails {
                return Err(preview_error("preview_close_failed", "test close failed"));
            }
            Ok(())
        }
    }

    fn preview(
        close_fails: bool,
    ) -> (
        TestPreview,
        mpsc::Sender<Vec<u8>>,
        Arc<AtomicUsize>,
        Arc<AtomicBool>,
    ) {
        let (sender, packets) = mpsc::channel(16);
        let received = Arc::new(AtomicUsize::new(0));
        let closed = Arc::new(AtomicBool::new(false));
        (
            TestPreview {
                packets,
                received: Arc::clone(&received),
                closed: Arc::clone(&closed),
                close_fails,
            },
            sender,
            received,
            closed,
        )
    }

    #[tokio::test]
    async fn relay_drops_when_full_and_closes_on_stop() {
        let (preview, source, received, closed) = preview(false);
        let (output, mut audio) = mpsc::channel(2);
        let (stop, cancelled) = watch::channel(false);
        let relay = tokio::spawn(relay_preview(Box::new(preview), output, cancelled));
        source.send(vec![1, 2, 3, 4]).await.unwrap();
        assert_eq!(audio.recv().await.unwrap(), vec![1, 2, 3, 4]);
        for _ in 0..3 {
            source.send(vec![5; 4]).await.unwrap();
        }
        tokio::time::timeout(Duration::from_secs(2), async {
            while received.load(Ordering::SeqCst) < 4 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        stop.send(true).unwrap();
        let report = relay.await.unwrap().unwrap();
        assert_eq!(
            report,
            PreviewRelayReport {
                end: PreviewEnd::Stopped,
                packets_sent: 3,
                packets_dropped: 1,
            }
        );
        assert!(closed.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn disconnect_closes_a_preview_waiting_for_audio() {
        let (preview, _source, _received, closed) = preview(false);
        let (output, audio) = mpsc::channel(1);
        let (_stop, cancelled) = watch::channel(false);
        let relay = tokio::spawn(relay_preview(Box::new(preview), output, cancelled));
        drop(audio);
        let report = tokio::time::timeout(Duration::from_secs(2), relay)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(report.end, PreviewEnd::ConsumerDisconnected);
        assert!(closed.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn lost_control_is_reported_and_closes_the_preview() {
        let (preview, _source, _received, closed) = preview(false);
        let (output, _audio) = mpsc::channel(1);
        let (stop, cancelled) = watch::channel(false);
        drop(stop);
        let error = relay_preview(Box::new(preview), output, cancelled)
            .await
            .unwrap_err();
        assert_eq!(error.code, "preview_control_ended");
        assert!(closed.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn reports_invalid_audio_and_cleanup_failure() {
        let (preview, source, _received, closed) = preview(true);
        let (output, _audio) = mpsc::channel(1);
        let (_stop, cancelled) = watch::channel(false);
        source.send(vec![0; 3]).await.unwrap();
        let error = relay_preview(Box::new(preview), output, cancelled)
            .await
            .unwrap_err();
        assert_eq!(error.code, "preview_cleanup_failed");
        assert!(error.message.contains("preview_packet_invalid"));
        assert!(error.message.contains("preview_close_failed"));
        assert!(closed.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn rejects_an_unbounded_delivery_queue_and_closes() {
        let (preview, _source, _received, closed) = preview(false);
        let (output, _audio) = mpsc::channel(MAX_DELIVERY_PACKETS + 1);
        let (_stop, cancelled) = watch::channel(false);
        let error = relay_preview(Box::new(preview), output, cancelled)
            .await
            .unwrap_err();
        assert_eq!(error.code, "preview_delivery_capacity_invalid");
        assert!(closed.load(Ordering::SeqCst));
    }

    async fn websocket_pair(
        capacity: usize,
    ) -> (
        WebSocketStream<tokio::io::DuplexStream>,
        WebSocketStream<tokio::io::DuplexStream>,
    ) {
        use tokio_tungstenite::tungstenite::protocol::{Role, WebSocketConfig};

        let (server, client) = tokio::io::duplex(capacity);
        let mut config = WebSocketConfig::default();
        config.max_message_size = Some(MAX_PREVIEW_INBOUND_BYTES);
        config.max_frame_size = Some(MAX_PREVIEW_INBOUND_BYTES);
        (
            WebSocketStream::from_raw_socket(server, Role::Server, Some(config)).await,
            WebSocketStream::from_raw_socket(client, Role::Client, None).await,
        )
    }

    #[tokio::test]
    async fn websocket_rejects_unbounded_input_and_closes_preview() {
        use tokio_tungstenite::tungstenite::protocol::Role;

        let (preview, _source, _received, closed) = preview(false);
        let (server, _client) = tokio::io::duplex(8192);
        let server = WebSocketStream::from_raw_socket(server, Role::Server, None).await;
        let (_stop, cancel) = watch::channel(false);
        let error = send_preview_over_websocket(Box::new(preview), server, cancel)
            .await
            .unwrap_err();
        assert_eq!(error.code, "preview_transport_config_invalid");
        assert!(closed.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn websocket_delivers_audio_and_closes_preview_on_peer_disconnect() {
        let (preview, source, _received, closed) = preview(false);
        let (server, mut client) = websocket_pair(8192).await;
        let (_stop, cancel) = watch::channel(false);
        let relay = tokio::spawn(send_preview_over_websocket(
            Box::new(preview),
            server,
            cancel,
        ));
        source.send(vec![1, 2, 3, 4]).await.unwrap();
        let message = tokio::time::timeout(Duration::from_secs(2), client.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(message, Message::Binary(vec![1, 2, 3, 4].into()));
        client.send(Message::Close(None)).await.unwrap();
        let result = tokio::time::timeout(Duration::from_secs(2), relay)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(result.end, PreviewEnd::ConsumerDisconnected);
        assert!(closed.load(Ordering::SeqCst));
        assert!(matches!(
            client.next().await.unwrap().unwrap(),
            Message::Close(_)
        ));
    }

    #[tokio::test]
    async fn websocket_stop_closes_a_silent_preview() {
        let (preview, _source, _received, closed) = preview(false);
        let (server, mut client) = websocket_pair(8192).await;
        let (stop, cancel) = watch::channel(false);
        let relay = tokio::spawn(send_preview_over_websocket(
            Box::new(preview),
            server,
            cancel,
        ));
        stop.send(true).unwrap();
        let result = tokio::time::timeout(Duration::from_secs(2), relay)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(result.end, PreviewEnd::Stopped);
        assert!(closed.load(Ordering::SeqCst));
        assert!(matches!(
            client.next().await.unwrap().unwrap(),
            Message::Close(_)
        ));
    }

    #[tokio::test]
    async fn websocket_responds_to_ping_while_audio_is_silent() {
        let (preview, _source, _received, closed) = preview(false);
        let (server, mut client) = websocket_pair(8192).await;
        let (stop, cancel) = watch::channel(false);
        let relay = tokio::spawn(send_preview_over_websocket(
            Box::new(preview),
            server,
            cancel,
        ));
        client.send(Message::Ping(vec![1, 2].into())).await.unwrap();
        let message = tokio::time::timeout(Duration::from_secs(2), client.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(message, Message::Pong(vec![1, 2].into()));
        stop.send(true).unwrap();
        assert_eq!(relay.await.unwrap().unwrap().end, PreviewEnd::Stopped);
        assert!(closed.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn websocket_reports_protocol_and_cleanup_errors() {
        let (preview, _source, _received, closed) = preview(true);
        let (server, mut client) = websocket_pair(8192).await;
        let (_stop, cancel) = watch::channel(false);
        let relay = tokio::spawn(send_preview_over_websocket(
            Box::new(preview),
            server,
            cancel,
        ));
        client
            .send(Message::Text("unexpected".into()))
            .await
            .unwrap();
        let error = tokio::time::timeout(Duration::from_secs(2), relay)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.code, "preview_shutdown_failed");
        assert!(error.message.contains("preview_protocol_error"));
        assert!(error.message.contains("preview_close_failed"));
        assert!(closed.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn websocket_write_timeout_closes_preview() {
        let (preview, source, _received, closed) = preview(false);
        let (server, _client) = websocket_pair(8).await;
        let (_stop, cancel) = watch::channel(false);
        let relay = tokio::spawn(send_preview_over_websocket(
            Box::new(preview),
            server,
            cancel,
        ));
        source.send(vec![1; MAX_PACKET_BYTES]).await.unwrap();
        let error = tokio::time::timeout(Duration::from_secs(8), relay)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert!(
            error.code == "preview_transport_timeout"
                || error.message.contains("preview_transport_timeout")
        );
        assert!(closed.load(Ordering::SeqCst));
    }

    #[cfg(feature = "gstreamer-runtime")]
    #[tokio::test]
    #[ignore = "requires GStreamer RTP plugins and isolated loopback UDP"]
    async fn websocket_delivers_real_rtp_audio_and_keeps_route_running_after_disconnect() {
        use arc_application::{MediaPipeline, PipelineSourceIdentity, PipelineSpecification};
        use arc_domain::{
            estimate_pipeline_requirements, DestinationMediaProfile, SourceMediaProfile,
        };
        use arc_gstreamer::GstreamerMediaPipeline;
        use std::net::{IpAddr, Ipv4Addr};
        use tokio::net::{TcpListener, UdpSocket};
        use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
        use uuid::Uuid;

        async fn assert_route_receiving(output: &UdpSocket) {
            let mut buffer = [0u8; 2048];
            tokio::time::timeout(Duration::from_secs(5), async {
                for _ in 0..4 {
                    let length = output.recv(&mut buffer).await.unwrap();
                    assert!(length > 12);
                    assert!(buffer[12..length].iter().any(|byte| *byte != 0));
                }
            })
            .await
            .expect("route receives RTP audio");
        }

        let output = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let input_reservation = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let input_port = input_reservation.local_addr().unwrap().port();
        drop(input_reservation);
        let source = SourceMediaProfile::RtpL16 {
            bind_address: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: input_port,
            payload_type: 96,
        };
        let destination = DestinationMediaProfile::RtpL16 {
            address: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: output.local_addr().unwrap().port(),
            payload_type: 97,
        };
        let identity = PipelineSourceIdentity {
            source_id: Uuid::new_v4(),
            source_revision: 1,
        };
        let pipeline_id = Uuid::new_v4();
        let specification = PipelineSpecification {
            pipeline_id,
            assignment_generation: 1,
            source_identity: Some(identity),
            requirements: estimate_pipeline_requirements(&source, &destination)
                .unwrap()
                .requirements,
            configuration: serde_json::json!({ "source": source, "destination": destination }),
        };
        let media = Arc::new(GstreamerMediaPipeline::new().unwrap());
        media.prepare(&specification).await.unwrap();
        media.start(pipeline_id, 1).await.unwrap();

        let sender = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let (stop_sender, mut generator_stop) = watch::channel(false);
        let generator = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(5));
            let mut sequence = 0u16;
            let mut timestamp = 0u32;
            loop {
                tokio::select! {
                    changed = generator_stop.changed() => {
                        if changed.is_err() || *generator_stop.borrow() {
                            break;
                        }
                    }
                    _ = interval.tick() => {
                        let mut packet = vec![0u8; 12 + 240 * 4];
                        packet[0] = 0x80;
                        packet[1] = 96;
                        packet[2..4].copy_from_slice(&sequence.to_be_bytes());
                        packet[4..8].copy_from_slice(&timestamp.to_be_bytes());
                        packet[8..12].copy_from_slice(&1u32.to_be_bytes());
                        for sample in packet[12..].chunks_exact_mut(2) {
                            sample.copy_from_slice(&0x1234i16.to_be_bytes());
                        }
                        sender.send_to(&packet, (Ipv4Addr::LOCALHOST, input_port)).await.unwrap();
                        sequence = sequence.wrapping_add(1);
                        timestamp = timestamp.wrapping_add(240);
                    }
                }
            }
        });

        assert_route_receiving(&output).await;
        let preview = Arc::clone(&media)
            .start_active_preview(identity, &source)
            .await
            .unwrap();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let accepting = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            tokio_tungstenite::accept_async(socket).await.unwrap()
        });
        let mut config = WebSocketConfig::default();
        config.max_message_size = Some(MAX_PREVIEW_INBOUND_BYTES);
        config.max_frame_size = Some(MAX_PREVIEW_INBOUND_BYTES);
        let (socket, _) = tokio_tungstenite::connect_async_with_config(
            format!("ws://{address}"),
            Some(config),
            false,
        )
        .await
        .unwrap();
        let mut received = accepting.await.unwrap();
        let (_stop, cancel) = watch::channel(false);
        let relay = tokio::spawn(send_preview_over_websocket(preview, socket, cancel));
        let message = tokio::time::timeout(Duration::from_secs(5), received.next())
            .await
            .expect("preview audio arrives")
            .expect("relay remains open")
            .unwrap();
        let Message::Binary(packet) = message else {
            panic!("preview delivers binary PCM");
        };
        assert!(packet.iter().any(|byte| *byte != 0));
        assert_eq!(packet.len() % 4, 0);

        received.send(Message::Close(None)).await.unwrap();
        let report = tokio::time::timeout(Duration::from_secs(5), relay)
            .await
            .expect("relay closes after disconnect")
            .unwrap()
            .unwrap();
        assert_eq!(report.end, PreviewEnd::ConsumerDisconnected);
        let mut buffer = [0u8; 2048];
        while output.try_recv(&mut buffer).is_ok() {}
        assert_route_receiving(&output).await;
        media.stop(pipeline_id, 1).await.unwrap();
        stop_sender.send(true).unwrap();
        generator.await.unwrap();
    }
}
