use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader},
    net::IpAddr,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use arc_application::{
    AssignmentGenerationStore, AudioPreview, MediaPipeline, PipelineSourceIdentity,
    PipelineSpecification, PortError, PortResult,
};
use arc_domain::SourceMediaProfile;
use arc_shared::{
    WorkerCommandEnvelope, WorkerSessionHello, WorkerSessionMessageKind, WorkerSessionWelcome,
    WORKER_PROTOCOL_VERSION,
};
use futures_util::{SinkExt, StreamExt};
use rustls::{ClientConfig, RootCertStore};
use serde::{Deserialize, Serialize};
use tokio::{sync::watch, task::JoinSet};
use tokio_tungstenite::{
    connect_async_tls_with_config,
    tungstenite::{
        client::IntoClientRequest,
        http::{uri::PathAndQuery, HeaderValue, Request},
        protocol::{frame::coding::CloseCode, CloseFrame, WebSocketConfig},
        Message,
    },
    Connector,
};
use uuid::Uuid;

use crate::{
    preview::MAX_PREVIEW_INBOUND_BYTES, receive_worker_command, send_preview_over_websocket,
    PreviewEnd, PreviewRelayReport,
};

const MAX_WORKER_CONTROL_MESSAGE_SIZE: usize = 64 * 1024;
const WORKER_CONTROL_HELLO_TIMEOUT: Duration = Duration::from_secs(5);
const WORKER_PREVIEW_DRAIN_TIMEOUT: Duration = Duration::from_secs(10);
const WORKER_CONTROL_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
const MAX_SESSION_PREVIEWS: usize = 8;
pub const MAX_PREVIEW_ERROR_CODE_BYTES: usize = 128;
pub const MAX_PREVIEW_ERROR_MESSAGE_BYTES: usize = 4096;

fn preview_error_fields(error: PortError) -> (String, String) {
    let code = if error.code.is_empty() || error.code.len() > MAX_PREVIEW_ERROR_CODE_BYTES {
        "worker_preview_failed"
    } else {
        error.code
    };
    let message = if error.message.is_empty() {
        "worker preview failed".to_owned()
    } else if error.message.len() > MAX_PREVIEW_ERROR_MESSAGE_BYTES {
        #[cfg(feature = "gstreamer-runtime")]
        tracing::warn!(
            code = error.code,
            "preview error message exceeded the protocol limit"
        );
        let mut end = MAX_PREVIEW_ERROR_MESSAGE_BYTES;
        while !error.message.is_char_boundary(end) {
            end -= 1;
        }
        error.message[..end].to_owned()
    } else {
        error.message
    };
    (code.to_owned(), message)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkerCommand {
    Prepare {
        specification: PipelineSpecification,
    },
    Start {
        specification: PipelineSpecification,
    },
    Stop,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum WorkerPreviewControl {
    PreviewStart {
        protocol_version: u16,
        worker_id: Uuid,
        preview_id: Uuid,
        identity: PipelineSourceIdentity,
        source: SourceMediaProfile,
        #[serde(default)]
        reserved_source: bool,
    },
    PreviewStop {
        protocol_version: u16,
        worker_id: Uuid,
        preview_id: Uuid,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerPreviewFinished {
    pub kind: WorkerPreviewEventKind,
    pub protocol_version: u16,
    pub worker_id: Uuid,
    pub preview_id: Uuid,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    #[serde(default)]
    pub cleanup_confirmed: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerPreviewEventKind {
    PreviewFinished,
}

#[async_trait::async_trait]
pub trait PreviewSessionFinalizer: Send + Sync {
    async fn finish(&self, worker_id: Uuid, session_id: Uuid) -> PortResult<()>;
}

struct CleanupTrackedPreview {
    inner: Box<dyn AudioPreview>,
    cleanup_confirmed: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl AudioPreview for CleanupTrackedPreview {
    async fn recv(&mut self) -> PortResult<Vec<u8>> {
        self.inner.recv().await
    }

    async fn close(&mut self) -> PortResult<()> {
        self.inner.close().await?;
        self.cleanup_confirmed.store(true, Ordering::Release);
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct WorkerControlClientConfig {
    pub endpoint: String,
    pub ca_certificates_file: Option<PathBuf>,
    pub worker_id: Uuid,
    pub bearer_token: String,
    pub bearer_token_file: Option<PathBuf>,
    pub allow_insecure_loopback: bool,
}

#[derive(Clone)]
pub struct WorkerControlClient {
    config: WorkerControlClientConfig,
    tls_config: Option<Arc<ClientConfig>>,
    generation_store: Arc<dyn AssignmentGenerationStore>,
    media_pipeline: Arc<dyn MediaPipeline>,
    preview_finalizer: Option<Arc<dyn PreviewSessionFinalizer>>,
    process_incarnation: Option<Uuid>,
    connection_status: watch::Sender<bool>,
    session_was_established: Arc<AtomicBool>,
}

impl WorkerControlClient {
    pub fn new(
        config: WorkerControlClientConfig,
        generation_store: Arc<dyn AssignmentGenerationStore>,
        media_pipeline: Arc<dyn MediaPipeline>,
    ) -> PortResult<Self> {
        if config.worker_id.is_nil()
            || config.bearer_token.is_empty()
            || config
                .bearer_token
                .bytes()
                .any(|byte| byte.is_ascii_whitespace())
        {
            return Err(worker_client_error(
                "invalid_worker_control_config",
                "worker identity and bearer credential must be valid",
            ));
        }
        let tls_config = config
            .ca_certificates_file
            .as_deref()
            .map(custom_tls_config)
            .transpose()?;
        let (connection_status, _) = watch::channel(false);
        let client = Self {
            config,
            tls_config,
            generation_store,
            media_pipeline,
            preview_finalizer: None,
            process_incarnation: None,
            connection_status,
            session_was_established: Arc::new(AtomicBool::new(false)),
        };
        client.websocket_request()?;
        Ok(client)
    }

    pub fn with_preview_finalizer(mut self, finalizer: Arc<dyn PreviewSessionFinalizer>) -> Self {
        self.preview_finalizer = Some(finalizer);
        self
    }

    pub fn with_process_incarnation(mut self, incarnation: Uuid) -> PortResult<Self> {
        if incarnation.is_nil() {
            return Err(worker_client_error(
                "invalid_worker_control_config",
                "worker process incarnation must be valid",
            ));
        }
        self.process_incarnation = Some(incarnation);
        Ok(self)
    }

    pub async fn run_session(&self) -> PortResult<()> {
        let _status_guard = ConnectionStatusGuard(self.connection_status.clone());
        let request = self.websocket_request()?;
        let mut websocket_config = WebSocketConfig::default();
        websocket_config.max_message_size = Some(MAX_WORKER_CONTROL_MESSAGE_SIZE);
        websocket_config.max_frame_size = Some(MAX_WORKER_CONTROL_MESSAGE_SIZE);
        let connector = self
            .tls_config
            .as_ref()
            .map(|config| Connector::Rustls(config.clone()));
        let (mut socket, _) =
            connect_async_tls_with_config(request, Some(websocket_config), false, connector)
                .await
                .map_err(|_| {
                    worker_client_error(
                        "worker_control_connection_failed",
                        "worker control connection or handshake failed",
                    )
                })?;

        let hello = WorkerSessionHello {
            kind: WorkerSessionMessageKind::Hello,
            protocol_version: WORKER_PROTOCOL_VERSION,
            worker_id: self.config.worker_id,
            process_incarnation: self.process_incarnation,
        };
        let hello = serde_json::to_string(&hello).map_err(|error| {
            worker_client_error(
                "worker_control_protocol_error",
                format!("worker hello could not be serialized: {error}"),
            )
        })?;
        socket
            .send(Message::Text(hello.into()))
            .await
            .map_err(|_| {
                worker_client_error(
                    "worker_control_connection_failed",
                    "worker hello could not be sent",
                )
            })?;

        let welcome = tokio::time::timeout(WORKER_CONTROL_HELLO_TIMEOUT, socket.next())
            .await
            .map_err(|_| {
                worker_client_error(
                    "worker_control_hello_timeout",
                    "worker control welcome was not received in time",
                )
            })?
            .ok_or_else(|| {
                worker_client_error(
                    "worker_control_connection_ended",
                    "worker control connection ended before welcome",
                )
            })?
            .map_err(|_| {
                worker_client_error(
                    "worker_control_connection_failed",
                    "worker control welcome could not be read",
                )
            })?;
        let Message::Text(welcome) = welcome else {
            return Err(worker_client_error(
                "worker_control_protocol_error",
                "worker control server did not send a text welcome",
            ));
        };
        let welcome: WorkerSessionWelcome =
            serde_json::from_str(welcome.as_str()).map_err(|error| {
                worker_client_error(
                    "worker_control_protocol_error",
                    format!("worker welcome is invalid: {error}"),
                )
            })?;
        if welcome.kind != WorkerSessionMessageKind::Welcome
            || welcome.protocol_version != WORKER_PROTOCOL_VERSION
            || welcome.worker_id != self.config.worker_id
            || welcome.session_id.is_some_and(|id| id.is_nil())
        {
            return Err(worker_client_error(
                "worker_control_protocol_error",
                "worker welcome does not match the authenticated session",
            ));
        }
        self.connection_status.send_replace(true);
        self.session_was_established.store(true, Ordering::Release);

        let mut keepalive = tokio::time::interval_at(
            tokio::time::Instant::now() + WORKER_CONTROL_KEEPALIVE_INTERVAL,
            WORKER_CONTROL_KEEPALIVE_INTERVAL,
        );
        keepalive.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut preview_stops = HashMap::<Uuid, (watch::Sender<bool>, bool)>::new();
        let mut previews: JoinSet<(Uuid, PortResult<PreviewRelayReport>, bool, bool)> =
            JoinSet::new();
        let mut fence_required = false;
        let session_result = async {
        loop {
            let message = tokio::select! {
                completed = previews.join_next(), if !previews.is_empty() => {
                    let completed = completed.expect("preview task exists");
                    if completed.is_err() {
                        fence_required = preview_stops.values().any(|(_, reserved)| *reserved);
                    }
                    let (preview_id, result, cleanup_confirmed, reserved_source) = completed
                        .map_err(|error| worker_client_error(
                            "preview_task_failed",
                            format!("preview task failed: {error}"),
                        ))?;
                    if reserved_source && !cleanup_confirmed {
                        fence_required = true;
                    }
                    preview_stops.remove(&preview_id);
                    let (error_code, error_message) = match result {
                        Ok(_) => (None, None),
                        Err(error) => {
                            let (code, message) = preview_error_fields(error);
                            (Some(code), Some(message))
                        },
                    };
                    let event = WorkerPreviewFinished {
                        kind: WorkerPreviewEventKind::PreviewFinished,
                        protocol_version: WORKER_PROTOCOL_VERSION,
                        worker_id: self.config.worker_id,
                        preview_id,
                        error_code,
                        error_message,
                        cleanup_confirmed,
                    };
                    let message = serde_json::to_string(&event).map_err(|error| {
                        worker_client_error("worker_control_protocol_error",
                            format!("preview event could not be serialized: {error}"))
                    })?;
                    socket.send(Message::Text(message.into())).await.map_err(|_| {
                        worker_client_error("worker_control_connection_failed",
                            "preview event could not be sent")
                    })?;
                    if fence_required {
                        return Err(worker_client_error(
                            "preview_fence_required", "reserved preview cleanup was not confirmed",
                        ));
                    }
                    continue;
                }
                _ = keepalive.tick() => {
                    socket.send(Message::Ping(Vec::new().into())).await.map_err(|_| {
                        worker_client_error(
                            "worker_control_connection_failed",
                            "worker keepalive could not be sent",
                        )
                    })?;
                    continue;
                }
                message = socket.next() => {
                    let Some(message) = message else { break };
                    message
                }
            };
            match message.map_err(|_| {
                worker_client_error(
                    "worker_control_connection_failed",
                    "worker control message could not be read",
                )
            })? {
                Message::Text(text) => {
                    let value: serde_json::Value = serde_json::from_str(text.as_str()).map_err(|error| {
                        worker_client_error("invalid_worker_command",
                            format!("worker command is invalid: {error}"))
                    })?;
                    if value.get("kind").is_some() {
                        let control: WorkerPreviewControl = serde_json::from_value(value).map_err(|error| {
                            worker_client_error("invalid_preview_command",
                                format!("preview control message is invalid: {error}"))
                        })?;
                        match control {
                            WorkerPreviewControl::PreviewStart {
                                protocol_version, worker_id, preview_id, identity, source, reserved_source,
                            } => {
                                if protocol_version != WORKER_PROTOCOL_VERSION
                                    || worker_id != self.config.worker_id || preview_id.is_nil()
                                    || identity.source_id.is_nil() || identity.source_revision <= 0
                                {
                                    return Err(worker_client_error(
                                        "invalid_preview_command",
                                        "preview start identity or version is invalid",
                                    ));
                                }
                                if reserved_source && !preview_stops.contains_key(&preview_id)
                                    && (self.preview_finalizer.is_none() || welcome.session_id.is_none())
                                {
                                    let event = WorkerPreviewFinished {
                                        kind: WorkerPreviewEventKind::PreviewFinished,
                                        protocol_version: WORKER_PROTOCOL_VERSION,
                                        worker_id,
                                        preview_id,
                                        error_code: Some("preview_reserved_unavailable".to_owned()),
                                        error_message: Some("reserved preview recovery is unavailable".to_owned()),
                                        cleanup_confirmed: true,
                                    };
                                    let message = serde_json::to_string(&event).map_err(|error| {
                                        worker_client_error("worker_control_protocol_error",
                                            format!("preview rejection could not be serialized: {error}"))
                                    })?;
                                    socket.send(Message::Text(message.into())).await.map_err(|_| {
                                        worker_client_error("worker_control_connection_failed",
                                            "preview rejection could not be sent")
                                    })?;
                                    continue;
                                }
                                let already_active = preview_stops.contains_key(&preview_id);
                                if already_active || preview_stops.len() >= MAX_SESSION_PREVIEWS {
                                    let event = WorkerPreviewFinished {
                                        kind: WorkerPreviewEventKind::PreviewFinished,
                                        protocol_version: WORKER_PROTOCOL_VERSION,
                                        worker_id,
                                        preview_id,
                                        error_code: Some("preview_capacity_exhausted".to_owned()),
                                        error_message: Some("worker preview capacity is exhausted or the preview is already active".to_owned()),
                                        cleanup_confirmed: !already_active,
                                    };
                                    let message = serde_json::to_string(&event).map_err(|error| {
                                        worker_client_error("worker_control_protocol_error",
                                            format!("preview event could not be serialized: {error}"))
                                    })?;
                                    socket.send(Message::Text(message.into())).await.map_err(|_| {
                                        worker_client_error("worker_control_connection_failed",
                                            "preview rejection could not be sent")
                                    })?;
                                    continue;
                                }
                                let (stop, cancelled) = watch::channel(false);
                                preview_stops.insert(preview_id, (stop, reserved_source));
                                let client = self.clone();
                                previews.spawn(async move {
                                    let cleanup = Arc::new(AtomicBool::new(true));
                                    let result = client.run_preview_with_mode(
                                        preview_id, identity, &source, cancelled, reserved_source, Arc::clone(&cleanup)
                                    ).await;
                                    (preview_id, result, cleanup.load(Ordering::Acquire), reserved_source)
                                });
                            }
                            WorkerPreviewControl::PreviewStop { protocol_version, worker_id, preview_id } => {
                                if protocol_version != WORKER_PROTOCOL_VERSION
                                    || worker_id != self.config.worker_id || preview_id.is_nil()
                                {
                                    return Err(worker_client_error(
                                        "invalid_preview_command", "preview stop identity or version is invalid",
                                    ));
                                }
                                if let Some((stop, _)) = preview_stops.get(&preview_id) {
                                    stop.send_replace(true);
                                }
                            }
                        }
                        continue;
                    }
                    let command: WorkerCommandEnvelope<WorkerCommand> =
                        serde_json::from_value(value).map_err(|error| {
                            worker_client_error(
                                "invalid_worker_command",
                                format!("worker command is invalid: {error}"),
                            )
                        })?;
                    validate_pipeline_command(&command)?;
                    let acknowledgement = receive_worker_command(
                        self.generation_store.as_ref(),
                        self.config.worker_id,
                        command,
                        |command| {
                            let media_pipeline = Arc::clone(&self.media_pipeline);
                            async move {
                                execute_pipeline_command(media_pipeline.as_ref(), command).await
                            }
                        },
                    )
                    .await?;
                    let acknowledgement =
                        serde_json::to_string(&acknowledgement).map_err(|error| {
                            worker_client_error(
                                "worker_control_protocol_error",
                                format!("command acknowledgement could not be serialized: {error}"),
                            )
                        })?;
                    socket
                        .send(Message::Text(acknowledgement.into()))
                        .await
                        .map_err(|_| {
                            worker_client_error(
                                "worker_control_connection_failed",
                                "command acknowledgement could not be sent",
                            )
                        })?;
                }
                Message::Ping(payload) => {
                    socket.send(Message::Pong(payload)).await.map_err(|_| {
                        worker_client_error(
                            "worker_control_connection_failed",
                            "worker ping response could not be sent",
                        )
                    })?;
                }
                Message::Pong(_) => {}
                Message::Close(_) => return Ok(()),
                Message::Binary(_) => {
                    return Err(worker_client_error(
                        "worker_control_protocol_error",
                        "binary worker control messages are unsupported",
                    ));
                }
                Message::Frame(_) => {
                    return Err(worker_client_error(
                        "worker_control_protocol_error",
                        "raw WebSocket frames are unsupported",
                    ));
                }
            }
        }
        Ok(())
        }.await;
        for (stop, _) in preview_stops.values() {
            stop.send_replace(true);
        }
        let mut cleanup_errors = Vec::new();
        let drain = async {
            while let Some(completed) = previews.join_next().await {
                match completed {
                    Ok((preview_id, result, confirmed, reserved)) => {
                        preview_stops.remove(&preview_id);
                        if reserved && !confirmed {
                            fence_required = true;
                        }
                        if let Err(error) = result {
                            cleanup_errors.push(format!("{}: {}", error.code, error.message))
                        }
                    }
                    Err(error) => {
                        fence_required |= preview_stops.values().any(|(_, reserved)| *reserved);
                        cleanup_errors.push(format!("preview task failed: {error}"));
                    }
                }
            }
        };
        if tokio::time::timeout(WORKER_PREVIEW_DRAIN_TIMEOUT, drain)
            .await
            .is_err()
        {
            fence_required |= preview_stops.values().any(|(_, reserved)| *reserved);
            cleanup_errors.push("preview task shutdown timed out".to_owned());
        }
        if fence_required {
            let details = session_result.as_ref().err().map_or_else(
                || cleanup_errors.join("; "),
                |error| {
                    format!(
                        "{}: {}; {}",
                        error.code,
                        error.message,
                        cleanup_errors.join("; ")
                    )
                },
            );
            return Err(worker_client_error(
                "preview_fence_required",
                format!("reserved preview cleanup was not confirmed: {details}"),
            ));
        }
        if let (Some(finalizer), Some(session_id)) = (&self.preview_finalizer, welcome.session_id) {
            if let Err(error) = finalizer.finish(self.config.worker_id, session_id).await {
                return Err(worker_client_error(
                    "preview_fence_required",
                    format!(
                        "preview session finalization failed ({}: {})",
                        error.code, error.message
                    ),
                ));
            }
        }
        match (session_result, cleanup_errors.is_empty()) {
            (result, true) => result,
            (Ok(()), false) => Err(worker_client_error(
                "preview_shutdown_failed",
                cleanup_errors.join("; "),
            )),
            (Err(error), false) => Err(worker_client_error(
                "preview_shutdown_failed",
                format!(
                    "{}: {}; {}",
                    error.code,
                    error.message,
                    cleanup_errors.join("; ")
                ),
            )),
        }
    }

    pub fn connection_status(&self) -> watch::Receiver<bool> {
        self.connection_status.subscribe()
    }

    pub fn consume_session_was_established(&self) -> bool {
        self.session_was_established.swap(false, Ordering::AcqRel)
    }

    /// Open a separate authenticated data connection to the configured control origin.
    /// The caller must await this operation through shutdown.
    pub async fn run_preview(
        &self,
        preview_id: Uuid,
        identity: PipelineSourceIdentity,
        source: &SourceMediaProfile,
        stop: watch::Receiver<bool>,
    ) -> PortResult<PreviewRelayReport> {
        self.run_preview_with_mode(
            preview_id,
            identity,
            source,
            stop,
            false,
            Arc::new(AtomicBool::new(true)),
        )
        .await
    }

    async fn run_preview_with_mode(
        &self,
        preview_id: Uuid,
        identity: PipelineSourceIdentity,
        source: &SourceMediaProfile,
        mut stop: watch::Receiver<bool>,
        reserved_source: bool,
        cleanup_confirmed: Arc<AtomicBool>,
    ) -> PortResult<PreviewRelayReport> {
        if preview_id.is_nil() || identity.source_id.is_nil() || identity.source_revision <= 0 {
            return Err(worker_client_error(
                "invalid_preview_identity",
                "preview and source identities must be valid",
            ));
        }
        let request = self.preview_websocket_request(preview_id)?;
        let mut config = WebSocketConfig::default();
        config.max_message_size = Some(MAX_PREVIEW_INBOUND_BYTES);
        config.max_frame_size = Some(MAX_PREVIEW_INBOUND_BYTES);
        let connector = self
            .tls_config
            .as_ref()
            .map(|config| Connector::Rustls(config.clone()));
        let connection = tokio::time::timeout(
            WORKER_CONTROL_HELLO_TIMEOUT,
            connect_async_tls_with_config(request, Some(config), false, connector),
        );
        tokio::pin!(connection);
        let (mut socket, _) = loop {
            if *stop.borrow() {
                return Ok(PreviewRelayReport {
                    end: PreviewEnd::Stopped,
                    packets_sent: 0,
                    packets_dropped: 0,
                });
            }
            if stop.has_changed().is_err() {
                return Err(worker_client_error(
                    "preview_control_ended",
                    "preview control ended without a stop request",
                ));
            }
            tokio::select! {
                changed = stop.changed() => {
                    if changed.is_err() {
                        return Err(worker_client_error(
                            "preview_control_ended",
                            "preview control ended without a stop request",
                        ));
                    }
                }
                result = &mut connection => {
                    break result
                        .map_err(|_| worker_client_error(
                            "preview_connection_timeout",
                            "preview data connection timed out",
                        ))?
                        .map_err(|_| worker_client_error(
                            "preview_connection_failed",
                            "preview data connection or authentication failed",
                        ))?;
                }
            }
        };
        if *stop.borrow() || stop.has_changed().is_err() {
            let stopped = *stop.borrow();
            tokio::time::timeout(WORKER_CONTROL_HELLO_TIMEOUT, socket.close(None))
                .await
                .map_err(|_| {
                    worker_client_error("preview_close_timeout", "preview connection did not close")
                })?
                .map_err(|_| {
                    worker_client_error(
                        "preview_close_failed",
                        "preview connection could not close",
                    )
                })?;
            return if stopped {
                Ok(PreviewRelayReport {
                    end: PreviewEnd::Stopped,
                    packets_sent: 0,
                    packets_dropped: 0,
                })
            } else {
                Err(worker_client_error(
                    "preview_control_ended",
                    "preview control ended without a stop request",
                ))
            };
        }
        cleanup_confirmed.store(false, Ordering::Release);
        let started = if reserved_source {
            Arc::clone(&self.media_pipeline)
                .start_reserved_preview(identity, source)
                .await
        } else {
            Arc::clone(&self.media_pipeline)
                .start_active_preview(identity, source)
                .await
        };
        let preview = match started {
            Ok(preview) => preview,
            Err(error) => {
                let close = tokio::time::timeout(
                    WORKER_CONTROL_HELLO_TIMEOUT,
                    socket.close(Some(CloseFrame {
                        code: CloseCode::Error,
                        reason: error.code.into(),
                    })),
                )
                .await;
                return match close {
                    Ok(Ok(())) => Err(error),
                    Ok(Err(_)) | Err(_) => Err(worker_client_error(
                        "preview_shutdown_failed",
                        format!(
                            "{}: {}; preview connection could not close",
                            error.code, error.message
                        ),
                    )),
                };
            }
        };
        let preview = CleanupTrackedPreview {
            inner: preview,
            cleanup_confirmed,
        };
        send_preview_over_websocket(Box::new(preview), socket, stop).await
    }

    fn preview_websocket_request(&self, preview_id: Uuid) -> PortResult<Request<()>> {
        let mut request = self.websocket_request()?;
        if request.uri().query().is_some() {
            return Err(worker_client_error(
                "invalid_worker_control_config",
                "worker control endpoint must not contain a query for preview",
            ));
        }
        let mut uri = request.uri().clone().into_parts();
        let path = format!(
            "{}/preview/{preview_id}",
            request.uri().path().trim_end_matches('/')
        );
        uri.path_and_query = Some(path.parse::<PathAndQuery>().map_err(|_| {
            worker_client_error("invalid_worker_control_config", "preview path is invalid")
        })?);
        *request.uri_mut() =
            tokio_tungstenite::tungstenite::http::Uri::from_parts(uri).map_err(|_| {
                worker_client_error("invalid_worker_control_config", "preview URI is invalid")
            })?;
        Ok(request)
    }

    fn websocket_request(&self) -> PortResult<tokio_tungstenite::tungstenite::http::Request<()>> {
        let mut request = self
            .config
            .endpoint
            .as_str()
            .into_client_request()
            .map_err(|_| {
                worker_client_error(
                    "invalid_worker_control_config",
                    "worker control endpoint is not a valid WebSocket URL",
                )
            })?;
        let scheme = request.uri().scheme_str();
        let is_secure = scheme == Some("wss");
        let is_local_insecure = scheme == Some("ws")
            && self.config.allow_insecure_loopback
            && request
                .uri()
                .host()
                .and_then(|host| host.parse::<IpAddr>().ok())
                .is_some_and(|address| address.is_loopback());
        if !is_secure && !is_local_insecure {
            return Err(worker_client_error(
                "insecure_worker_control_endpoint",
                "worker control requires WSS except for explicitly allowed loopback tests",
            ));
        }
        let bearer_token = self.current_bearer_token()?;
        let authorization =
            HeaderValue::from_str(&format!("Bearer {bearer_token}")).map_err(|_| {
                worker_client_error(
                    "invalid_worker_control_config",
                    "worker bearer credential is not a valid HTTP header value",
                )
            })?;
        let worker_id =
            HeaderValue::from_str(&self.config.worker_id.to_string()).map_err(|_| {
                worker_client_error(
                    "invalid_worker_control_config",
                    "worker identity is not a valid HTTP header value",
                )
            })?;
        request.headers_mut().insert("authorization", authorization);
        request.headers_mut().insert("x-worker-id", worker_id);
        request
            .headers_mut()
            .insert("x-forwarded-proto", HeaderValue::from_static("https"));
        Ok(request)
    }

    fn current_bearer_token(&self) -> PortResult<String> {
        let token = match &self.config.bearer_token_file {
            Some(path) => std::fs::read_to_string(path)
                .map_err(|_| {
                    worker_client_error(
                        "worker_control_secret_unavailable",
                        "worker bearer credential file could not be read",
                    )
                })?
                .trim()
                .to_owned(),
            None => self.config.bearer_token.clone(),
        };
        if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(worker_client_error(
                "invalid_worker_control_config",
                "worker bearer credential must be a 64-character hexadecimal token",
            ));
        }
        Ok(token)
    }
}

fn custom_tls_config(ca_file: &Path) -> PortResult<Arc<ClientConfig>> {
    let file = File::open(ca_file).map_err(|error| {
        worker_client_error(
            "invalid_worker_control_ca",
            format!("worker-control CA bundle could not be opened: {error}"),
        )
    })?;
    let roots = custom_ca_roots(BufReader::new(file))?;
    Ok(Arc::new(
        ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    ))
}

fn custom_ca_roots<R: BufRead>(mut reader: R) -> PortResult<RootCertStore> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let mut certificate_count = 0;
    for certificate in rustls_pemfile::certs(&mut reader) {
        let certificate = certificate.map_err(|error| {
            worker_client_error(
                "invalid_worker_control_ca",
                format!("worker-control CA PEM bundle could not be parsed: {error}"),
            )
        })?;
        roots.add(certificate).map_err(|error| {
            worker_client_error(
                "invalid_worker_control_ca",
                format!("worker-control CA certificate is invalid: {error}"),
            )
        })?;
        certificate_count += 1;
    }
    if certificate_count == 0 {
        return Err(worker_client_error(
            "invalid_worker_control_ca",
            "worker-control CA bundle contains no PEM certificates",
        ));
    }
    Ok(roots)
}

struct ConnectionStatusGuard(watch::Sender<bool>);

impl Drop for ConnectionStatusGuard {
    fn drop(&mut self) {
        self.0.send_replace(false);
    }
}

pub async fn execute_pipeline_command(
    media_pipeline: &dyn MediaPipeline,
    command: WorkerCommandEnvelope<WorkerCommand>,
) -> PortResult<()> {
    validate_pipeline_command(&command)?;
    match command.payload {
        WorkerCommand::Prepare { specification } => media_pipeline.prepare(&specification).await,
        WorkerCommand::Start { specification } => {
            media_pipeline.prepare(&specification).await?;
            media_pipeline
                .start(command.pipeline_id, command.generation)
                .await
        }
        WorkerCommand::Stop => {
            media_pipeline
                .stop(command.pipeline_id, command.generation)
                .await
        }
    }
}

fn validate_pipeline_command(command: &WorkerCommandEnvelope<WorkerCommand>) -> PortResult<()> {
    if let WorkerCommand::Prepare { specification } | WorkerCommand::Start { specification } =
        &command.payload
    {
        if specification.pipeline_id != command.pipeline_id
            || specification.assignment_generation != command.generation
            || specification.source_identity.is_some_and(|identity| {
                identity.source_id.is_nil() || identity.source_revision <= 0
            })
        {
            return Err(worker_client_error(
                "invalid_pipeline_command",
                "pipeline specification identity does not match its command",
            ));
        }
    }
    Ok(())
}

fn worker_client_error(code: &'static str, message: impl Into<String>) -> PortError {
    PortError {
        code,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeSet, HashMap},
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc, Mutex,
        },
    };

    use arc_application::{
        CommandFenceDecision, PipelineObservation, PipelineObservedState, PipelineSpecification,
        WorkerCommandIdentity,
    };
    use arc_domain::PipelineRequirements;
    use arc_shared::WorkerCommandAcknowledgement;
    use tokio::{net::TcpListener, task::JoinHandle};
    use tokio_tungstenite::{
        accept_hdr_async,
        tungstenite::{
            handshake::server::{Request, Response},
            Message,
        },
    };

    use super::*;

    #[test]
    fn preview_failure_fields_fit_the_control_protocol() {
        let (code, message) = preview_error_fields(PortError {
            code: "preview_failed",
            message: format!("{}\u{e9}", "a".repeat(4095)),
        });
        assert_eq!(code, "preview_failed");
        assert_eq!(message.len(), 4095);
        let (_, empty_message) = preview_error_fields(PortError {
            code: "preview_failed",
            message: String::new(),
        });
        assert!(!empty_message.is_empty());
    }

    #[test]
    fn custom_ca_bundle_adds_roots_without_removing_public_trust() {
        let bundle = std::io::Cursor::new(include_bytes!(
            "../tests/fixtures/worker-control-test-ca.pem"
        ));
        let roots =
            custom_ca_roots(BufReader::new(bundle)).expect("valid custom CA bundle is accepted");

        assert_eq!(roots.roots.len(), webpki_roots::TLS_SERVER_ROOTS.len() + 1);
    }

    #[test]
    fn custom_ca_file_builds_a_tls_client_config() {
        let ca_file = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("worker-control-test-ca.pem");

        custom_tls_config(&ca_file).expect("valid custom CA file builds TLS configuration");
    }

    #[test]
    fn custom_ca_bundle_rejects_non_certificate_content() {
        let bundle = std::io::Cursor::new(&b"not a PEM certificate"[..]);
        let error =
            custom_ca_roots(BufReader::new(bundle)).expect_err("invalid CA bundle is rejected");

        assert_eq!(error.code, "invalid_worker_control_ca");
    }

    #[derive(Default)]
    struct TestMediaPipeline {
        prepared: AtomicUsize,
        started: AtomicUsize,
        stopped: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl MediaPipeline for TestMediaPipeline {
        async fn prepare(&self, _: &PipelineSpecification) -> PortResult<()> {
            self.prepared.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn start(&self, _: Uuid, _: i64) -> PortResult<()> {
            self.started.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn stop(&self, _: Uuid, _: i64) -> PortResult<()> {
            self.stopped.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn observe(&self, pipeline_id: Uuid) -> PortResult<PipelineObservation> {
            Ok(PipelineObservation {
                pipeline_id,
                assignment_generation: 1,
                state: PipelineObservedState::Running,
                health_reasons: Vec::new(),
                observed_at: chrono::Utc::now(),
            })
        }
    }

    #[tokio::test]
    async fn media_pipeline_without_preview_reports_unsupported() {
        let media: Arc<dyn MediaPipeline> = Arc::new(TestMediaPipeline::default());
        let error = media
            .start_active_preview(
                arc_application::PipelineSourceIdentity {
                    source_id: Uuid::new_v4(),
                    source_revision: 1,
                },
                &arc_domain::SourceMediaProfile::AlsaL16 {
                    device: "test".to_owned(),
                },
            )
            .await
            .err()
            .expect("workers without preview support must return an error");
        assert_eq!(error.code, "preview_unsupported");
    }

    struct PreviewTestMedia {
        closed: Arc<AtomicBool>,
        reserved_started: Option<Arc<AtomicBool>>,
        close_fails: bool,
    }

    #[derive(Default)]
    struct TestPreviewFinalizer(Mutex<Vec<(Uuid, Uuid)>>);

    #[async_trait::async_trait]
    impl PreviewSessionFinalizer for TestPreviewFinalizer {
        async fn finish(&self, worker_id: Uuid, session_id: Uuid) -> PortResult<()> {
            self.0.lock().unwrap().push((worker_id, session_id));
            Ok(())
        }
    }

    struct FailingPreviewFinalizer;

    #[async_trait::async_trait]
    impl PreviewSessionFinalizer for FailingPreviewFinalizer {
        async fn finish(&self, _: Uuid, _: Uuid) -> PortResult<()> {
            Err(worker_client_error(
                "database_error",
                "database unavailable",
            ))
        }
    }

    struct PreviewTestAudio {
        sent: bool,
        closed: Arc<AtomicBool>,
        close_fails: bool,
    }

    #[async_trait::async_trait]
    impl arc_application::AudioPreview for PreviewTestAudio {
        async fn recv(&mut self) -> PortResult<Vec<u8>> {
            if !self.sent {
                self.sent = true;
                Ok(vec![1, 2, 3, 4])
            } else {
                std::future::pending().await
            }
        }

        async fn close(&mut self) -> PortResult<()> {
            self.closed.store(true, Ordering::SeqCst);
            if self.close_fails {
                Err(worker_client_error("test_close_failed", "close failed"))
            } else {
                Ok(())
            }
        }
    }

    struct FailingClosePreview;

    #[async_trait::async_trait]
    impl AudioPreview for FailingClosePreview {
        async fn recv(&mut self) -> PortResult<Vec<u8>> {
            std::future::pending().await
        }

        async fn close(&mut self) -> PortResult<()> {
            Err(worker_client_error("test_close_failed", "close failed"))
        }
    }

    #[tokio::test]
    async fn cleanup_acknowledgement_requires_successful_close() {
        let confirmed = Arc::new(AtomicBool::new(false));
        let mut preview = CleanupTrackedPreview {
            inner: Box::new(FailingClosePreview),
            cleanup_confirmed: Arc::clone(&confirmed),
        };
        assert_eq!(preview.close().await.unwrap_err().code, "test_close_failed");
        assert!(!confirmed.load(Ordering::Acquire));

        let mut preview = CleanupTrackedPreview {
            inner: Box::new(PreviewTestAudio {
                sent: false,
                closed: Arc::new(AtomicBool::new(false)),
                close_fails: false,
            }),
            cleanup_confirmed: Arc::clone(&confirmed),
        };
        preview.close().await.unwrap();
        assert!(confirmed.load(Ordering::Acquire));
    }

    #[test]
    fn legacy_preview_completion_does_not_confirm_cleanup() {
        let event: WorkerPreviewFinished = serde_json::from_value(serde_json::json!({
            "kind": "preview_finished",
            "protocolVersion": WORKER_PROTOCOL_VERSION,
            "workerId": Uuid::new_v4(),
            "previewId": Uuid::new_v4(),
            "errorCode": null,
            "errorMessage": null
        }))
        .unwrap();
        assert!(!event.cleanup_confirmed);
    }

    #[async_trait::async_trait]
    impl MediaPipeline for PreviewTestMedia {
        async fn prepare(&self, _: &PipelineSpecification) -> PortResult<()> {
            Ok(())
        }

        async fn start(&self, _: Uuid, _: i64) -> PortResult<()> {
            Ok(())
        }

        async fn stop(&self, _: Uuid, _: i64) -> PortResult<()> {
            Ok(())
        }

        async fn observe(&self, _: Uuid) -> PortResult<PipelineObservation> {
            Err(worker_client_error(
                "test_unsupported",
                "not needed by this test",
            ))
        }

        async fn start_active_preview(
            self: Arc<Self>,
            _: PipelineSourceIdentity,
            _: &SourceMediaProfile,
        ) -> PortResult<Box<dyn arc_application::AudioPreview>> {
            Ok(Box::new(PreviewTestAudio {
                sent: false,
                closed: Arc::clone(&self.closed),
                close_fails: self.close_fails,
            }))
        }

        async fn start_reserved_preview(
            self: Arc<Self>,
            identity: PipelineSourceIdentity,
            source: &SourceMediaProfile,
        ) -> PortResult<Box<dyn arc_application::AudioPreview>> {
            if let Some(started) = &self.reserved_started {
                started.store(true, Ordering::SeqCst);
            }
            self.start_active_preview(identity, source).await
        }
    }

    #[tokio::test]
    async fn preview_uses_authenticated_control_origin_and_cleans_up_on_disconnect() {
        let worker_id = Uuid::new_v4();
        let preview_id = Uuid::new_v4();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!(
            "ws://{}/api/v1/workers/control",
            listener.local_addr().unwrap()
        );
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_hdr_async(stream, |request: &Request, response: Response| {
                assert_eq!(
                    request.uri().path(),
                    format!("/api/v1/workers/control/preview/{preview_id}")
                );
                assert_eq!(request.headers()["x-worker-id"], worker_id.to_string());
                assert_eq!(
                    request.headers()["authorization"],
                    format!("Bearer {}", test_token())
                );
                Ok(response)
            })
            .await
            .unwrap();
            assert_eq!(
                socket.next().await.unwrap().unwrap(),
                Message::Binary(vec![1, 2, 3, 4].into())
            );
            socket.send(Message::Close(None)).await.unwrap();
            assert!(matches!(
                socket.next().await.unwrap().unwrap(),
                Message::Close(_)
            ));
        });
        let closed = Arc::new(AtomicBool::new(false));
        let client = WorkerControlClient::new(
            WorkerControlClientConfig {
                endpoint,
                ca_certificates_file: None,
                worker_id,
                bearer_token: test_token(),
                bearer_token_file: None,
                allow_insecure_loopback: true,
            },
            Arc::new(TestGenerationStore::default()),
            Arc::new(PreviewTestMedia {
                closed: Arc::clone(&closed),
                reserved_started: None,
                close_fails: false,
            }),
        )
        .unwrap();
        let (_stop, cancel) = watch::channel(false);
        let report = tokio::time::timeout(
            Duration::from_secs(3),
            client.run_preview(
                preview_id,
                PipelineSourceIdentity {
                    source_id: Uuid::new_v4(),
                    source_revision: 1,
                },
                &SourceMediaProfile::AlsaL16 {
                    device: "test".to_owned(),
                },
                cancel,
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(report.end, PreviewEnd::ConsumerDisconnected);
        assert!(closed.load(Ordering::SeqCst));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn preview_can_be_stopped_during_the_websocket_handshake() {
        let worker_id = Uuid::new_v4();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!(
            "ws://{}/api/v1/workers/control",
            listener.local_addr().unwrap()
        );
        let closed = Arc::new(AtomicBool::new(false));
        let client = WorkerControlClient::new(
            WorkerControlClientConfig {
                endpoint,
                ca_certificates_file: None,
                worker_id,
                bearer_token: test_token(),
                bearer_token_file: None,
                allow_insecure_loopback: true,
            },
            Arc::new(TestGenerationStore::default()),
            Arc::new(PreviewTestMedia {
                closed: Arc::clone(&closed),
                reserved_started: None,
                close_fails: false,
            }),
        )
        .unwrap();
        let (stop, cancel) = watch::channel(false);
        let cleanup = Arc::new(AtomicBool::new(true));
        let task_cleanup = Arc::clone(&cleanup);
        let task = tokio::spawn(async move {
            client
                .run_preview_with_mode(
                    Uuid::new_v4(),
                    PipelineSourceIdentity {
                        source_id: Uuid::new_v4(),
                        source_revision: 1,
                    },
                    &SourceMediaProfile::AlsaL16 {
                        device: "test".to_owned(),
                    },
                    cancel,
                    true,
                    task_cleanup,
                )
                .await
        });
        let (stream, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept())
            .await
            .unwrap()
            .unwrap();
        stop.send(true).unwrap();
        let report = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(report.end, PreviewEnd::Stopped);
        assert!(!closed.load(Ordering::SeqCst));
        assert!(cleanup.load(Ordering::Acquire));
        drop(stream);
    }

    #[tokio::test]
    async fn preview_reports_unavailable_media_and_closes_the_connection() {
        let worker_id = Uuid::new_v4();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!(
            "ws://{}/api/v1/workers/control",
            listener.local_addr().unwrap()
        );
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let message = socket.next().await.unwrap().unwrap();
            match message {
                Message::Close(Some(frame)) => {
                    assert_eq!(frame.code, CloseCode::Error);
                    assert_eq!(frame.reason, "preview_unsupported");
                }
                other => panic!("expected preview failure close, received {other:?}"),
            }
        });
        let client = client(
            endpoint,
            worker_id,
            Arc::new(TestGenerationStore::default()),
            Arc::new(TestMediaPipeline::default()),
            true,
        );
        let (_stop, cancel) = watch::channel(false);
        let cleanup = Arc::new(AtomicBool::new(true));
        let error = tokio::time::timeout(
            Duration::from_secs(3),
            client.run_preview_with_mode(
                Uuid::new_v4(),
                PipelineSourceIdentity {
                    source_id: Uuid::new_v4(),
                    source_revision: 1,
                },
                &SourceMediaProfile::AlsaL16 {
                    device: "test".to_owned(),
                },
                cancel,
                true,
                Arc::clone(&cleanup),
            ),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert_eq!(error.code, "preview_unsupported");
        assert!(!cleanup.load(Ordering::Acquire));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn control_session_runs_and_stops_preview_without_blocking_control() {
        for close_fails in [false, true] {
            let worker_id = Uuid::new_v4();
            let session_id = Uuid::new_v4();
            let preview_id = Uuid::new_v4();
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!(
                "ws://{}/api/v1/workers/control",
                listener.local_addr().unwrap()
            );
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut control = tokio_tungstenite::accept_async(stream).await.unwrap();
                assert!(matches!(
                    control.next().await.unwrap().unwrap(),
                    Message::Text(_)
                ));
                let welcome = WorkerSessionWelcome {
                    kind: WorkerSessionMessageKind::Welcome,
                    protocol_version: WORKER_PROTOCOL_VERSION,
                    worker_id,
                    session_id: Some(session_id),
                };
                control
                    .send(Message::Text(
                        serde_json::to_string(&welcome).unwrap().into(),
                    ))
                    .await
                    .unwrap();
                let start = WorkerPreviewControl::PreviewStart {
                    protocol_version: WORKER_PROTOCOL_VERSION,
                    worker_id,
                    preview_id,
                    identity: PipelineSourceIdentity {
                        source_id: Uuid::new_v4(),
                        source_revision: 1,
                    },
                    source: SourceMediaProfile::AlsaL16 {
                        device: "test".to_owned(),
                    },
                    reserved_source: true,
                };
                control
                    .send(Message::Text(serde_json::to_string(&start).unwrap().into()))
                    .await
                    .unwrap();
                let (stream, _) = listener.accept().await.unwrap();
                let mut data = tokio_tungstenite::accept_async(stream).await.unwrap();
                assert_eq!(
                    data.next().await.unwrap().unwrap(),
                    Message::Binary(vec![1, 2, 3, 4].into())
                );
                control.send(Message::Ping(vec![7].into())).await.unwrap();
                assert_eq!(
                    control.next().await.unwrap().unwrap(),
                    Message::Pong(vec![7].into())
                );
                let stop = WorkerPreviewControl::PreviewStop {
                    protocol_version: WORKER_PROTOCOL_VERSION,
                    worker_id,
                    preview_id,
                };
                control
                    .send(Message::Text(serde_json::to_string(&stop).unwrap().into()))
                    .await
                    .unwrap();
                assert!(matches!(
                    data.next().await.unwrap().unwrap(),
                    Message::Close(_)
                ));
                let Message::Text(text) = control.next().await.unwrap().unwrap() else {
                    panic!("expected preview completion event");
                };
                let event: WorkerPreviewFinished = serde_json::from_str(text.as_str()).unwrap();
                assert_eq!(event.kind, WorkerPreviewEventKind::PreviewFinished);
                assert_eq!(event.worker_id, worker_id);
                assert_eq!(event.preview_id, preview_id);
                assert_eq!(
                    event.error_code.as_deref(),
                    close_fails.then_some("test_close_failed")
                );
                assert_eq!(event.cleanup_confirmed, !close_fails);
                control.close(None).await.unwrap();
            });
            let closed = Arc::new(AtomicBool::new(false));
            let reserved_started = Arc::new(AtomicBool::new(false));
            let finalizer = Arc::new(TestPreviewFinalizer::default());
            let client = WorkerControlClient::new(
                WorkerControlClientConfig {
                    endpoint,
                    ca_certificates_file: None,
                    worker_id,
                    bearer_token: test_token(),
                    bearer_token_file: None,
                    allow_insecure_loopback: true,
                },
                Arc::new(TestGenerationStore::default()),
                Arc::new(PreviewTestMedia {
                    closed: Arc::clone(&closed),
                    reserved_started: Some(Arc::clone(&reserved_started)),
                    close_fails,
                }),
            )
            .unwrap()
            .with_preview_finalizer(finalizer.clone());
            let result = tokio::time::timeout(Duration::from_secs(5), client.run_session())
                .await
                .unwrap();
            if close_fails {
                assert_eq!(result.unwrap_err().code, "preview_fence_required");
            } else {
                result.unwrap();
            }
            tokio::time::timeout(Duration::from_secs(5), server)
                .await
                .unwrap()
                .unwrap();
            assert!(closed.load(Ordering::SeqCst));
            assert!(reserved_started.load(Ordering::SeqCst));
            let finalized = finalizer.0.lock().unwrap();
            if close_fails {
                assert!(finalized.is_empty());
            } else {
                assert_eq!(*finalized, vec![(worker_id, session_id)]);
            }
        }
    }

    #[tokio::test]
    async fn reserved_preview_is_rejected_without_session_recovery() {
        let worker_id = Uuid::new_v4();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!(
            "ws://{}/api/v1/workers/control",
            listener.local_addr().unwrap()
        );
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            socket.next().await.unwrap().unwrap();
            let welcome = WorkerSessionWelcome {
                kind: WorkerSessionMessageKind::Welcome,
                protocol_version: WORKER_PROTOCOL_VERSION,
                worker_id,
                session_id: Some(Uuid::new_v4()),
            };
            socket
                .send(Message::Text(
                    serde_json::to_string(&welcome).unwrap().into(),
                ))
                .await
                .unwrap();
            let start = WorkerPreviewControl::PreviewStart {
                protocol_version: WORKER_PROTOCOL_VERSION,
                worker_id,
                preview_id: Uuid::new_v4(),
                identity: PipelineSourceIdentity {
                    source_id: Uuid::new_v4(),
                    source_revision: 1,
                },
                source: SourceMediaProfile::AlsaL16 {
                    device: "test".to_owned(),
                },
                reserved_source: true,
            };
            socket
                .send(Message::Text(serde_json::to_string(&start).unwrap().into()))
                .await
                .unwrap();
            let Message::Text(text) = socket.next().await.unwrap().unwrap() else {
                panic!("expected preview rejection");
            };
            let event: WorkerPreviewFinished = serde_json::from_str(text.as_str()).unwrap();
            assert_eq!(
                event.error_code.as_deref(),
                Some("preview_reserved_unavailable")
            );
            assert!(event.cleanup_confirmed);
            socket.close(None).await.unwrap();
        });
        let client = client(
            endpoint,
            worker_id,
            Arc::new(TestGenerationStore::default()),
            Arc::new(TestMediaPipeline::default()),
            true,
        );
        tokio::time::timeout(Duration::from_secs(5), client.run_session())
            .await
            .unwrap()
            .unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn worker_hello_carries_configured_process_incarnation() {
        let worker_id = Uuid::new_v4();
        let incarnation = Uuid::new_v4();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!(
            "ws://{}/api/v1/workers/control",
            listener.local_addr().unwrap()
        );
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let Message::Text(text) = socket.next().await.unwrap().unwrap() else {
                panic!("expected worker hello");
            };
            let hello: WorkerSessionHello = serde_json::from_str(text.as_str()).unwrap();
            assert_eq!(hello.process_incarnation, Some(incarnation));
            let welcome = WorkerSessionWelcome {
                kind: WorkerSessionMessageKind::Welcome,
                protocol_version: WORKER_PROTOCOL_VERSION,
                worker_id,
                session_id: Some(Uuid::new_v4()),
            };
            socket
                .send(Message::Text(
                    serde_json::to_string(&welcome).unwrap().into(),
                ))
                .await
                .unwrap();
            socket.close(None).await.unwrap();
        });
        let client = client(
            endpoint,
            worker_id,
            Arc::new(TestGenerationStore::default()),
            Arc::new(TestMediaPipeline::default()),
            true,
        );
        assert!(client.clone().with_process_incarnation(Uuid::nil()).is_err());
        let client = client.with_process_incarnation(incarnation).unwrap();
        tokio::time::timeout(Duration::from_secs(5), client.run_session())
            .await
            .unwrap()
            .unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn capacity_rejection_confirms_cleanup_only_for_new_preview() {
        let worker_id = Uuid::new_v4();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (release_listener, hold_listener) = tokio::sync::oneshot::channel::<()>();
        let endpoint = format!(
            "ws://{}/api/v1/workers/control",
            listener.local_addr().unwrap()
        );
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            socket.next().await.unwrap().unwrap();
            let welcome = WorkerSessionWelcome {
                kind: WorkerSessionMessageKind::Welcome,
                protocol_version: WORKER_PROTOCOL_VERSION,
                worker_id,
                session_id: Some(Uuid::new_v4()),
            };
            socket
                .send(Message::Text(
                    serde_json::to_string(&welcome).unwrap().into(),
                ))
                .await
                .unwrap();
            let identity = PipelineSourceIdentity {
                source_id: Uuid::new_v4(),
                source_revision: 1,
            };
            let first_id = Uuid::new_v4();
            let rejected_id = Uuid::new_v4();
            let mut ids = vec![first_id];
            ids.extend((1..MAX_SESSION_PREVIEWS).map(|_| Uuid::new_v4()));
            ids.extend([first_id, rejected_id]);
            for preview_id in ids {
                let start = WorkerPreviewControl::PreviewStart {
                    protocol_version: WORKER_PROTOCOL_VERSION,
                    worker_id,
                    preview_id,
                    identity,
                    source: SourceMediaProfile::AlsaL16 {
                        device: "test".to_owned(),
                    },
                    reserved_source: true,
                };
                socket
                    .send(Message::Text(serde_json::to_string(&start).unwrap().into()))
                    .await
                    .unwrap();
            }
            for (preview_id, expected_cleanup) in [(first_id, false), (rejected_id, true)] {
                let Message::Text(text) = socket.next().await.unwrap().unwrap() else {
                    panic!("expected preview rejection");
                };
                let event: WorkerPreviewFinished = serde_json::from_str(text.as_str()).unwrap();
                assert_eq!(event.preview_id, preview_id);
                assert_eq!(
                    event.error_code.as_deref(),
                    Some("preview_capacity_exhausted")
                );
                assert_eq!(event.cleanup_confirmed, expected_cleanup);
            }
            socket.close(None).await.unwrap();
            let _ = hold_listener.await;
        });
        let client = client(
            endpoint,
            worker_id,
            Arc::new(TestGenerationStore::default()),
            Arc::new(TestMediaPipeline::default()),
            true,
        )
        .with_preview_finalizer(Arc::new(TestPreviewFinalizer::default()));
        tokio::time::timeout(Duration::from_secs(5), client.run_session())
            .await
            .unwrap()
            .unwrap();
        release_listener.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn failed_session_finalization_requires_fencing() {
        let worker_id = Uuid::new_v4();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!(
            "ws://{}/api/v1/workers/control",
            listener.local_addr().unwrap()
        );
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            socket.next().await.unwrap().unwrap();
            let welcome = WorkerSessionWelcome {
                kind: WorkerSessionMessageKind::Welcome,
                protocol_version: WORKER_PROTOCOL_VERSION,
                worker_id,
                session_id: Some(Uuid::new_v4()),
            };
            socket
                .send(Message::Text(
                    serde_json::to_string(&welcome).unwrap().into(),
                ))
                .await
                .unwrap();
            socket.close(None).await.unwrap();
        });
        let client = client(
            endpoint,
            worker_id,
            Arc::new(TestGenerationStore::default()),
            Arc::new(TestMediaPipeline::default()),
            true,
        )
        .with_preview_finalizer(Arc::new(FailingPreviewFinalizer));
        let error = tokio::time::timeout(Duration::from_secs(5), client.run_session())
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(error.code, "preview_fence_required");
        assert!(error.message.contains("database_error"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn stalled_reserved_preview_shutdown_requires_fencing() {
        struct StalledMedia(Arc<AtomicBool>);
        #[async_trait::async_trait]
        impl MediaPipeline for StalledMedia {
            async fn prepare(&self, _: &PipelineSpecification) -> PortResult<()> { Ok(()) }
            async fn start(&self, _: Uuid, _: i64) -> PortResult<()> { Ok(()) }
            async fn stop(&self, _: Uuid, _: i64) -> PortResult<()> { Ok(()) }
            async fn observe(&self, _: Uuid) -> PortResult<PipelineObservation> {
                Err(worker_client_error("test_unsupported", "not needed"))
            }
            async fn start_reserved_preview(
                self: Arc<Self>, _: PipelineSourceIdentity, _: &SourceMediaProfile,
            ) -> PortResult<Box<dyn AudioPreview>> {
                self.0.store(true, Ordering::SeqCst);
                std::future::pending().await
            }
        }
        let worker_id = Uuid::new_v4();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("ws://{}/api/v1/workers/control", listener.local_addr().unwrap());
        let started = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&started);
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut control = tokio_tungstenite::accept_async(stream).await.unwrap();
            control.next().await.unwrap().unwrap();
            let welcome = WorkerSessionWelcome {
                kind: WorkerSessionMessageKind::Welcome, protocol_version: WORKER_PROTOCOL_VERSION,
                worker_id, session_id: Some(Uuid::new_v4()),
            };
            control.send(Message::Text(serde_json::to_string(&welcome).unwrap().into())).await.unwrap();
            let start = WorkerPreviewControl::PreviewStart {
                protocol_version: WORKER_PROTOCOL_VERSION, worker_id, preview_id: Uuid::new_v4(),
                identity: PipelineSourceIdentity { source_id: Uuid::new_v4(), source_revision: 1 },
                source: SourceMediaProfile::AlsaL16 { device: "test".to_owned() },
                reserved_source: true,
            };
            control.send(Message::Text(serde_json::to_string(&start).unwrap().into())).await.unwrap();
            let (stream, _) = listener.accept().await.unwrap();
            let _data = tokio_tungstenite::accept_async(stream).await.unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                while !observed.load(Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }).await.unwrap();
            control.close(None).await.unwrap();
        });
        let finalizer = Arc::new(TestPreviewFinalizer::default());
        let client = WorkerControlClient::new(
            WorkerControlClientConfig {
                endpoint,
                ca_certificates_file: None,
                worker_id,
                bearer_token: test_token(),
                bearer_token_file: None,
                allow_insecure_loopback: true,
            },
            Arc::new(TestGenerationStore::default()),
            Arc::new(StalledMedia(started)),
        ).unwrap().with_preview_finalizer(finalizer.clone());
        let error = tokio::time::timeout(Duration::from_secs(15), client.run_session())
            .await.unwrap().unwrap_err();
        assert_eq!(error.code, "preview_fence_required");
        assert!(finalizer.0.lock().unwrap().is_empty());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn control_disconnect_stops_and_cleans_up_previews() {
        let worker_id = Uuid::new_v4();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!(
            "ws://{}/api/v1/workers/control",
            listener.local_addr().unwrap()
        );
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut control = tokio_tungstenite::accept_async(stream).await.unwrap();
            control.next().await.unwrap().unwrap();
            let welcome = WorkerSessionWelcome {
                kind: WorkerSessionMessageKind::Welcome,
                protocol_version: WORKER_PROTOCOL_VERSION,
                worker_id,
                session_id: None,
            };
            control
                .send(Message::Text(
                    serde_json::to_string(&welcome).unwrap().into(),
                ))
                .await
                .unwrap();
            let start = WorkerPreviewControl::PreviewStart {
                protocol_version: WORKER_PROTOCOL_VERSION,
                worker_id,
                preview_id: Uuid::new_v4(),
                identity: PipelineSourceIdentity {
                    source_id: Uuid::new_v4(),
                    source_revision: 1,
                },
                source: SourceMediaProfile::AlsaL16 {
                    device: "test".to_owned(),
                },
                reserved_source: false,
            };
            control
                .send(Message::Text(serde_json::to_string(&start).unwrap().into()))
                .await
                .unwrap();
            let (stream, _) = listener.accept().await.unwrap();
            let mut data = tokio_tungstenite::accept_async(stream).await.unwrap();
            assert!(matches!(
                data.next().await.unwrap().unwrap(),
                Message::Binary(_)
            ));
            control.close(None).await.unwrap();
            assert!(matches!(
                data.next().await.unwrap().unwrap(),
                Message::Close(_)
            ));
        });
        let closed = Arc::new(AtomicBool::new(false));
        let client = WorkerControlClient::new(
            WorkerControlClientConfig {
                endpoint,
                ca_certificates_file: None,
                worker_id,
                bearer_token: test_token(),
                bearer_token_file: None,
                allow_insecure_loopback: true,
            },
            Arc::new(TestGenerationStore::default()),
            Arc::new(PreviewTestMedia {
                closed: Arc::clone(&closed),
                reserved_started: None,
                close_fails: false,
            }),
        )
        .unwrap();
        tokio::time::timeout(Duration::from_secs(5), client.run_session())
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
        assert!(closed.load(Ordering::SeqCst));
    }

    #[test]
    fn preview_rejects_control_endpoint_query_parameters() {
        let worker_id = Uuid::new_v4();
        let client = client(
            "wss://example.test/api/v1/workers/control?token=unexpected".to_owned(),
            worker_id,
            Arc::new(TestGenerationStore::default()),
            Arc::new(TestMediaPipeline::default()),
            false,
        );
        let error = client
            .preview_websocket_request(Uuid::new_v4())
            .unwrap_err();
        assert_eq!(error.code, "invalid_worker_control_config");
    }

    #[derive(Default)]
    struct TestGenerationStore {
        pipelines: Mutex<HashMap<Uuid, StoredPipeline>>,
    }

    #[derive(Default)]
    struct StoredPipeline {
        generation: i64,
        worker_id: Uuid,
        assignment_id: Uuid,
        commands: HashMap<Uuid, (WorkerCommandIdentity, bool)>,
    }

    #[async_trait::async_trait]
    impl AssignmentGenerationStore for TestGenerationStore {
        async fn begin_command(
            &self,
            command: WorkerCommandIdentity,
        ) -> PortResult<CommandFenceDecision> {
            let mut pipelines = self.pipelines.lock().unwrap();
            let pipeline = pipelines.entry(command.pipeline_id).or_default();
            if command.generation < pipeline.generation {
                return Ok(CommandFenceDecision::Stale {
                    current_generation: pipeline.generation,
                });
            }
            if command.generation == pipeline.generation
                && pipeline.generation > 0
                && (command.worker_id != pipeline.worker_id
                    || command.assignment_id != pipeline.assignment_id)
            {
                return Err(PortError {
                    code: "assignment_generation_conflict",
                    message: "one generation cannot identify multiple assignments".to_owned(),
                });
            }
            if command.generation > pipeline.generation {
                pipeline.generation = command.generation;
                pipeline.worker_id = command.worker_id;
                pipeline.assignment_id = command.assignment_id;
            }
            match pipeline.commands.get(&command.command_id) {
                Some((recorded, _)) if *recorded != command => Err(PortError {
                    code: "command_identity_conflict",
                    message: "command ID was already used for a different identity".to_owned(),
                }),
                Some((_, true)) => Ok(CommandFenceDecision::Duplicate),
                Some((_, false)) => Ok(CommandFenceDecision::Process),
                None => {
                    pipeline
                        .commands
                        .insert(command.command_id, (command, false));
                    Ok(CommandFenceDecision::Process)
                }
            }
        }

        async fn complete_command(&self, command_id: Uuid) -> PortResult<()> {
            for pipeline in self.pipelines.lock().unwrap().values_mut() {
                if let Some((_, completed)) = pipeline.commands.get_mut(&command_id) {
                    *completed = true;
                    return Ok(());
                }
            }
            Err(PortError {
                code: "command_not_found",
                message: "worker command was not recorded".to_owned(),
            })
        }
    }

    fn specification(pipeline_id: Uuid, generation: i64) -> PipelineSpecification {
        PipelineSpecification {
            pipeline_id,
            assignment_generation: generation,
            source_identity: None,
            requirements: PipelineRequirements {
                source_adapter: None,
                destination_adapter: None,
                codecs: BTreeSet::new(),
                encoders: BTreeSet::new(),
                payload_formats: BTreeSet::new(),
                signal_parsers: BTreeSet::new(),
                signal_generators: BTreeSet::new(),
                audio_backends: BTreeSet::new(),
                hardware_acceleration: BTreeSet::new(),
                platform: None,
                architecture: None,
                capacity_units: 1,
                ingress_bits_per_second: None,
                egress_bits_per_second: None,
            },
            configuration: serde_json::json!({}),
        }
    }

    fn command(
        worker_id: Uuid,
        pipeline_id: Uuid,
        assignment_id: Uuid,
        generation: i64,
        payload: WorkerCommand,
    ) -> WorkerCommandEnvelope<WorkerCommand> {
        WorkerCommandEnvelope {
            protocol_version: WORKER_PROTOCOL_VERSION,
            command_id: Uuid::new_v4(),
            worker_id,
            pipeline_id,
            assignment_id,
            generation,
            payload,
        }
    }

    async fn receive_acknowledgement(
        socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    ) -> WorkerCommandAcknowledgement {
        let message = socket
            .next()
            .await
            .expect("worker sends acknowledgement")
            .expect("acknowledgement is readable");
        let Message::Text(message) = message else {
            panic!("expected command acknowledgement");
        };
        serde_json::from_str(message.as_str()).expect("acknowledgement is valid JSON")
    }

    async fn send_command(
        socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
        command: WorkerCommandEnvelope<WorkerCommand>,
    ) -> WorkerCommandAcknowledgement {
        socket
            .send(Message::Text(
                serde_json::to_string(&command)
                    .expect("command serializes")
                    .into(),
            ))
            .await
            .expect("server sends command");
        receive_acknowledgement(socket).await
    }

    async fn worker_test_server(
        worker_id: Uuid,
        pipeline_id: Uuid,
        assignment_id: Uuid,
        store: Arc<TestGenerationStore>,
    ) -> (String, JoinHandle<Vec<WorkerCommandAcknowledgement>>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test server binds");
        let address = listener.local_addr().expect("server address exists");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("worker connects");
            let mut socket = accept_hdr_async(stream, |request: &Request, response: Response| {
                assert_eq!(
                    request.headers()["x-worker-id"].to_str().unwrap(),
                    worker_id.to_string()
                );
                assert_eq!(
                    request.headers()["authorization"].to_str().unwrap(),
                    format!("Bearer {}", test_token())
                );
                Ok(response)
            })
            .await
            .expect("worker WebSocket handshake succeeds");
            let hello = socket.next().await.expect("worker sends hello").unwrap();
            let Message::Text(hello) = hello else {
                panic!("expected worker hello");
            };
            let hello: WorkerSessionHello =
                serde_json::from_str(hello.as_str()).expect("hello is valid JSON");
            let welcome = WorkerSessionWelcome {
                kind: WorkerSessionMessageKind::Welcome,
                protocol_version: WORKER_PROTOCOL_VERSION,
                worker_id: hello.worker_id,
                session_id: None,
            };
            socket
                .send(Message::Text(
                    serde_json::to_string(&welcome)
                        .expect("welcome serializes")
                        .into(),
                ))
                .await
                .expect("welcome sends");
            let mut acknowledgements = Vec::new();
            acknowledgements.push(
                send_command(
                    &mut socket,
                    command(
                        worker_id,
                        pipeline_id,
                        assignment_id,
                        1,
                        WorkerCommand::Start {
                            specification: specification(pipeline_id, 1),
                        },
                    ),
                )
                .await,
            );
            acknowledgements.push(
                send_command(
                    &mut socket,
                    command(
                        worker_id,
                        pipeline_id,
                        assignment_id,
                        1,
                        WorkerCommand::Stop,
                    ),
                )
                .await,
            );
            store
                .begin_command(WorkerCommandIdentity {
                    command_id: Uuid::new_v4(),
                    worker_id,
                    pipeline_id,
                    assignment_id: Uuid::new_v4(),
                    generation: 2,
                })
                .await
                .expect("new assignment generation is recorded");
            acknowledgements.push(
                send_command(
                    &mut socket,
                    command(
                        worker_id,
                        pipeline_id,
                        assignment_id,
                        1,
                        WorkerCommand::Start {
                            specification: specification(pipeline_id, 1),
                        },
                    ),
                )
                .await,
            );
            socket
                .close(None)
                .await
                .expect("test server closes session");
            acknowledgements
        });
        (format!("ws://{address}"), server)
    }

    fn client(
        endpoint: String,
        worker_id: Uuid,
        store: Arc<TestGenerationStore>,
        media_pipeline: Arc<TestMediaPipeline>,
        allow_insecure_loopback: bool,
    ) -> WorkerControlClient {
        WorkerControlClient::new(
            WorkerControlClientConfig {
                endpoint,
                ca_certificates_file: None,
                worker_id,
                bearer_token: test_token(),
                bearer_token_file: None,
                allow_insecure_loopback,
            },
            store,
            media_pipeline,
        )
        .expect("worker control client configuration is valid")
    }

    fn test_token() -> String {
        "ab".repeat(32)
    }

    #[test]
    fn reconnect_request_reloads_rotated_file_token() {
        let worker_id = Uuid::new_v4();
        let path = std::env::temp_dir().join(format!("arc-worker-token-{worker_id}.txt"));
        let first = "ab".repeat(32);
        let second = "cd".repeat(32);
        std::fs::write(&path, &first).expect("initial token is written");
        let client = WorkerControlClient::new(
            WorkerControlClientConfig {
                endpoint: "ws://127.0.0.1:8080/api/v1/workers/control".to_owned(),
                ca_certificates_file: None,
                worker_id,
                bearer_token: first,
                bearer_token_file: Some(path.clone()),
                allow_insecure_loopback: true,
            },
            Arc::new(TestGenerationStore::default()),
            Arc::new(TestMediaPipeline::default()),
        )
        .expect("file-backed worker token is valid");

        std::fs::write(&path, &second).expect("rotated token is written");
        let request = client
            .websocket_request()
            .expect("reconnect request reloads the token");
        assert_eq!(
            request.headers()["authorization"],
            format!("Bearer {second}")
        );
        let preview_request = client
            .preview_websocket_request(Uuid::new_v4())
            .expect("preview request reloads the token");
        assert_eq!(
            preview_request.headers()["authorization"],
            format!("Bearer {second}")
        );
        std::fs::remove_file(path).expect("temporary token file is removed");
    }

    #[test]
    fn worker_client_rejects_plaintext_endpoints_outside_loopback() {
        let error = WorkerControlClient::new(
            WorkerControlClientConfig {
                endpoint: "ws://192.0.2.1/api/v1/workers/control".to_owned(),
                ca_certificates_file: None,
                worker_id: Uuid::new_v4(),
                bearer_token: "local-worker-token".to_owned(),
                bearer_token_file: None,
                allow_insecure_loopback: true,
            },
            Arc::new(TestGenerationStore::default()),
            Arc::new(TestMediaPipeline::default()),
        )
        .err()
        .expect("non-loopback plaintext endpoint is rejected");
        assert_eq!(error.code, "insecure_worker_control_endpoint");
    }

    #[tokio::test]
    async fn start_command_rejects_a_specification_with_a_different_generation() {
        let media_pipeline = TestMediaPipeline::default();
        let pipeline_id = Uuid::new_v4();
        let error = execute_pipeline_command(
            &media_pipeline,
            command(
                Uuid::new_v4(),
                pipeline_id,
                Uuid::new_v4(),
                1,
                WorkerCommand::Start {
                    specification: specification(pipeline_id, 2),
                },
            ),
        )
        .await
        .expect_err("mismatched specification is rejected");

        assert_eq!(error.code, "invalid_pipeline_command");
        assert_eq!(media_pipeline.prepared.load(Ordering::SeqCst), 0);
        assert_eq!(media_pipeline.started.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn prepare_command_builds_the_path_without_starting_media() {
        let media_pipeline = TestMediaPipeline::default();
        let pipeline_id = Uuid::new_v4();
        execute_pipeline_command(
            &media_pipeline,
            command(
                Uuid::new_v4(),
                pipeline_id,
                Uuid::new_v4(),
                1,
                WorkerCommand::Prepare {
                    specification: specification(pipeline_id, 1),
                },
            ),
        )
        .await
        .expect("Prepare command succeeds");

        assert_eq!(media_pipeline.prepared.load(Ordering::SeqCst), 1);
        assert_eq!(media_pipeline.started.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn failed_command_handler_returns_a_retryable_failure_acknowledgement() {
        let store = TestGenerationStore::default();
        let worker_id = Uuid::new_v4();
        let command = command(
            worker_id,
            Uuid::new_v4(),
            Uuid::new_v4(),
            1,
            WorkerCommand::Stop,
        );
        let identity = WorkerCommandIdentity {
            command_id: command.command_id,
            worker_id: command.worker_id,
            pipeline_id: command.pipeline_id,
            assignment_id: command.assignment_id,
            generation: command.generation,
        };
        let acknowledgement = receive_worker_command(&store, worker_id, command, |_| async {
            Err(PortError {
                code: "media_pipeline_failed",
                message: "injected test failure".to_owned(),
            })
        })
        .await
        .expect("failed handler produces an acknowledgement");

        assert_eq!(
            acknowledgement.outcome,
            arc_shared::WorkerCommandOutcome::Failed
        );
        assert_eq!(
            acknowledgement.error_code.as_deref(),
            Some("media_pipeline_failed")
        );
        assert_eq!(
            store.begin_command(identity).await.unwrap(),
            CommandFenceDecision::Process
        );
    }

    #[tokio::test]
    async fn worker_client_keeps_an_idle_session_alive() {
        let worker_id = Uuid::new_v4();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!(
            "ws://{}/api/v1/workers/control",
            listener.local_addr().unwrap()
        );
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket =
                accept_hdr_async(stream, |_: &Request, response: Response| Ok(response))
                    .await
                    .unwrap();
            assert!(matches!(
                socket.next().await.unwrap().unwrap(),
                Message::Text(_)
            ));
            socket
                .send(Message::Text(
                    serde_json::to_string(&WorkerSessionWelcome {
                        kind: WorkerSessionMessageKind::Welcome,
                        protocol_version: WORKER_PROTOCOL_VERSION,
                        worker_id,
                        session_id: None,
                    })
                    .unwrap()
                    .into(),
                ))
                .await
                .unwrap();
            let message = tokio::time::timeout(Duration::from_secs(40), socket.next())
                .await
                .expect("idle worker sends a keepalive before the ingress timeout")
                .unwrap()
                .unwrap();
            let Message::Ping(payload) = message else {
                panic!("expected idle worker keepalive");
            };
            socket.send(Message::Pong(payload)).await.unwrap();
            socket.close(None).await.unwrap();
        });
        let client = client(
            endpoint,
            worker_id,
            Arc::new(TestGenerationStore::default()),
            Arc::new(TestMediaPipeline::default()),
            true,
        );
        let session = tokio::spawn(async move { client.run_session().await });
        server.await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), session)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn worker_client_runs_start_and_stop_handlers_and_acknowledges_after_completion() {
        let worker_id = Uuid::new_v4();
        let pipeline_id = Uuid::new_v4();
        let assignment_id = Uuid::new_v4();
        let store = Arc::new(TestGenerationStore::default());
        let media_pipeline = Arc::new(TestMediaPipeline::default());
        let (endpoint, server) =
            worker_test_server(worker_id, pipeline_id, assignment_id, store.clone()).await;
        let client = client(
            endpoint,
            worker_id,
            store.clone(),
            media_pipeline.clone(),
            true,
        );
        let observer = client.clone();
        let client_task = tokio::spawn(async move { client.run_session().await });
        let acknowledgements = server.await.expect("test server joins");
        client_task
            .await
            .expect("worker client task joins")
            .expect("worker session completes");
        assert!(observer.consume_session_was_established());
        assert_eq!(acknowledgements.len(), 3);
        assert_eq!(
            acknowledgements[0].outcome,
            arc_shared::WorkerCommandOutcome::Completed
        );
        assert_eq!(
            acknowledgements[1].outcome,
            arc_shared::WorkerCommandOutcome::Completed
        );
        assert_eq!(
            acknowledgements[2].outcome,
            arc_shared::WorkerCommandOutcome::Stale
        );
        assert_eq!(acknowledgements[2].current_generation, Some(2));
        assert_eq!(media_pipeline.prepared.load(Ordering::SeqCst), 1);
        assert_eq!(media_pipeline.started.load(Ordering::SeqCst), 1);
        assert_eq!(media_pipeline.stopped.load(Ordering::SeqCst), 1);
    }
}
