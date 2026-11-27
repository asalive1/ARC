use std::{
    collections::HashMap,
    sync::{Arc, Weak},
    time::Duration,
};

use arc_application::{AssignmentGenerationStore, PortError, PortResult, WorkerAuthenticator};
use arc_shared::{WorkerCommandAcknowledgement, WorkerCommandEnvelope, WorkerCommandOutcome};
use arc_worker::{
    dispatch_if_fresh, CommandDispatchResult, WorkerPreviewControl, WorkerPreviewEventKind,
    WorkerPreviewFinished, MAX_PREVIEW_ERROR_CODE_BYTES, MAX_PREVIEW_ERROR_MESSAGE_BYTES,
    WORKER_PROTOCOL_VERSION,
};
use axum::extract::ws::{close_code, CloseFrame, Message, WebSocket};
use serde::Serialize;
use sqlx::PgPool;
use tokio::sync::{mpsc, oneshot, watch, Mutex};
use uuid::Uuid;

pub(crate) const MAX_WORKER_CONTROL_MESSAGE_SIZE: usize = 64 * 1024;
const WORKER_COMMAND_ACK_TIMEOUT: Duration = Duration::from_secs(30);
const WORKER_COMMAND_QUEUE_CAPACITY: usize = 16;

pub struct WorkerCommandDispatcher {
    generation_store: Arc<dyn AssignmentGenerationStore>,
    sessions: Mutex<HashMap<Uuid, ActiveSession>>,
    pipeline_locks: Mutex<HashMap<Uuid, Weak<Mutex<()>>>>,
    lease_pool: Option<PgPool>,
}

struct ActiveSession {
    session_id: Uuid,
    commands: mpsc::Sender<PendingCommand>,
    cancel: watch::Sender<bool>,
}

pub(crate) struct WorkerControlSession {
    worker_id: Uuid,
    pub(crate) session_id: Uuid,
    commands: mpsc::Receiver<PendingCommand>,
    cancelled: watch::Receiver<bool>,
}

enum PendingCommand {
    Route {
        command_id: Uuid,
        message: String,
        completion: oneshot::Sender<PortResult<()>>,
    },
    PreviewStart {
        preview_id: Uuid,
        message: String,
        completion: oneshot::Sender<PortResult<WorkerPreviewFinished>>,
    },
    PreviewStop {
        message: String,
    },
}

pub(crate) struct WorkerPreviewSession {
    pub(crate) worker_id: Uuid,
    pub(crate) session_id: Uuid,
    commands: mpsc::Sender<PendingCommand>,
    pub(crate) cancelled: watch::Receiver<bool>,
}

impl WorkerPreviewSession {
    pub(crate) fn start(
        &self,
        control: WorkerPreviewControl,
        preview_id: Uuid,
    ) -> PortResult<oneshot::Receiver<PortResult<WorkerPreviewFinished>>> {
        let message = serde_json::to_string(&control).map_err(|error| {
            session_error(
                "invalid_preview_command",
                format!("preview command could not be serialized: {error}"),
            )
        })?;
        if message.len() > MAX_WORKER_CONTROL_MESSAGE_SIZE {
            return Err(session_error(
                "worker_command_too_large",
                "preview command exceeds the control message size limit",
            ));
        }
        let (completion, completed) = oneshot::channel();
        self.commands
            .try_send(PendingCommand::PreviewStart {
                preview_id,
                message,
                completion,
            })
            .map_err(command_queue_error)?;
        Ok(completed)
    }

    pub(crate) fn stop(&self, preview_id: Uuid) -> PortResult<()> {
        let control = WorkerPreviewControl::PreviewStop {
            protocol_version: WORKER_PROTOCOL_VERSION,
            worker_id: self.worker_id,
            preview_id,
        };
        let message = serde_json::to_string(&control).map_err(|error| {
            session_error(
                "invalid_preview_command",
                format!("preview stop could not be serialized: {error}"),
            )
        })?;
        self.commands
            .try_send(PendingCommand::PreviewStop { message })
            .map_err(command_queue_error)
    }
}

impl WorkerCommandDispatcher {
    pub fn new(generation_store: Arc<dyn AssignmentGenerationStore>) -> Self {
        Self {
            generation_store,
            sessions: Mutex::new(HashMap::new()),
            pipeline_locks: Mutex::new(HashMap::new()),
            lease_pool: None,
        }
    }

    pub fn new_with_lease_pool(
        generation_store: Arc<dyn AssignmentGenerationStore>,
        lease_pool: PgPool,
    ) -> Self {
        Self {
            generation_store,
            sessions: Mutex::new(HashMap::new()),
            pipeline_locks: Mutex::new(HashMap::new()),
            lease_pool: Some(lease_pool),
        }
    }

    pub async fn dispatch<T>(
        &self,
        command: WorkerCommandEnvelope<T>,
    ) -> PortResult<CommandDispatchResult>
    where
        T: Serialize + Send,
    {
        let pipeline_id = command.pipeline_id;
        let worker_id = command.worker_id;
        let command_id = command.command_id;
        let message = serde_json::to_string(&command).map_err(|error| PortError {
            code: "invalid_worker_command",
            message: format!("worker command could not be serialized: {error}"),
        })?;
        if message.len() > MAX_WORKER_CONTROL_MESSAGE_SIZE {
            return Err(session_error(
                "worker_command_too_large",
                "worker command exceeds the control message size limit",
            ));
        }
        let pipeline_lock = self.pipeline_lock(pipeline_id).await;
        let guard = Arc::clone(&pipeline_lock).lock_owned().await;
        let result = dispatch_if_fresh(
            self.generation_store.as_ref(),
            worker_id,
            command,
            |_| async move { self.send_and_wait(worker_id, command_id, message).await },
        )
        .await;
        drop(guard);
        self.remove_pipeline_lock_if_idle(pipeline_id, &pipeline_lock)
            .await;
        result
    }

    pub(crate) async fn start_session(
        &self,
        worker_id: Uuid,
        process_incarnation: Option<Uuid>,
    ) -> PortResult<WorkerControlSession> {
        let session_id = Uuid::new_v4();
        if let Some(pool) = &self.lease_pool {
            sqlx::query(
                "INSERT INTO worker_control_sessions (worker_id, session_id, process_incarnation) \
                 VALUES ($1, $2, $3) \
                 ON CONFLICT (worker_id) DO UPDATE SET \
                 session_id = EXCLUDED.session_id, \
                 process_incarnation = EXCLUDED.process_incarnation, \
                 created_at = clock_timestamp(), \
                 replaced_at = clock_timestamp()",
            )
            .bind(worker_id)
            .bind(session_id)
            .bind(process_incarnation)
            .execute(pool)
            .await
            .map_err(|error| PortError {
                code: "worker_session_registration_failed",
                message: format!("worker control session lease could not be registered: {error}"),
            })?;
        }
        let (command_sender, commands) = mpsc::channel(WORKER_COMMAND_QUEUE_CAPACITY);
        let (cancel, cancelled) = watch::channel(false);
        let replaced = self.sessions.lock().await.insert(
            worker_id,
            ActiveSession {
                session_id,
                commands: command_sender,
                cancel,
            },
        );
        if let Some(replaced) = replaced {
            let _ = replaced.cancel.send(true);
        }
        Ok(WorkerControlSession {
            worker_id,
            session_id,
            commands,
            cancelled,
        })
    }

    pub(crate) async fn serve_session(
        &self,
        socket: &mut WebSocket,
        mut session: WorkerControlSession,
        authenticator: Arc<dyn WorkerAuthenticator>,
        bearer_token: String,
    ) {
        let worker_id = session.worker_id;
        let mut pending = HashMap::new();
        let mut previews: HashMap<Uuid, oneshot::Sender<PortResult<WorkerPreviewFinished>>> =
            HashMap::new();
        let mut credential_check = tokio::time::interval_at(
            tokio::time::Instant::now() + Duration::from_secs(1),
            Duration::from_secs(1),
        );
        credential_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let close_reason = loop {
            tokio::select! {
                _ = credential_check.tick() => {
                    if let Err(error) = authenticator.authenticate(worker_id, &bearer_token).await {
                        tracing::warn!(%worker_id, code = error.code, "closing worker session after credential revocation or provider failure");
                        break Some("worker credential revoked");
                    }
                }
                cancellation = session.cancelled.changed() => {
                    if cancellation.is_err() || *session.cancelled.borrow() {
                        break Some("worker session replaced");
                    }
                }
                command = session.commands.recv() => {
                    let Some(command) = command else {
                        break Some("worker command session unavailable");
                    };
                    let message = match command {
                        PendingCommand::Route { command_id, message, completion } => {
                            pending.insert(command_id, completion);
                            message
                        }
                        PendingCommand::PreviewStart { preview_id, message, completion } => {
                            if previews.contains_key(&preview_id) {
                                let _ = completion.send(Err(session_error("duplicate_preview", "preview identity is already active")));
                                continue;
                            }
                            previews.insert(preview_id, completion);
                            message
                        }
                        PendingCommand::PreviewStop { message } => message,
                    };
                    if let Err(error) = socket.send(Message::Text(message.into())).await {
                        tracing::debug!(%worker_id, %error, "failed to send worker command");
                        break Some("worker command delivery failed");
                    }
                }
                message = socket.recv() => {
                    match message {
                        Some(Ok(Message::Ping(payload))) => {
                            if let Err(error) = socket.send(Message::Pong(payload)).await {
                                tracing::debug!(%worker_id, %error, "failed to respond to worker ping");
                                break None;
                            }
                        }
                        Some(Ok(Message::Pong(_))) => {}
                        Some(Ok(Message::Close(_))) | None => break None,
                        Some(Ok(Message::Text(text))) => {
                            if let Ok(event) = serde_json::from_str::<WorkerPreviewFinished>(text.as_str()) {
                                if event.kind != WorkerPreviewEventKind::PreviewFinished
                                    || event.protocol_version != WORKER_PROTOCOL_VERSION
                                    || event.worker_id != worker_id || event.preview_id.is_nil()
                                    || event.error_code.as_ref().is_some_and(|s| s.is_empty() || s.len() > MAX_PREVIEW_ERROR_CODE_BYTES)
                                    || event.error_message.as_ref().is_some_and(|s| s.is_empty() || s.len() > MAX_PREVIEW_ERROR_MESSAGE_BYTES)
                                    || event.error_code.is_some() != event.error_message.is_some()
                                {
                                    break Some("invalid worker preview event");
                                }
                                if let Some(completion) = previews.remove(&event.preview_id) {
                                    let _ = completion.send(Ok(event));
                                }
                                continue;
                            }
                            let acknowledgement =
                                serde_json::from_str::<WorkerCommandAcknowledgement>(text.as_str());
                            let Ok(acknowledgement) = acknowledgement else {
                                break Some("invalid worker command acknowledgement");
                            };
                            if acknowledgement.validate_for_worker(worker_id).is_err()
                            {
                                break Some("invalid worker command acknowledgement");
                            }
                            let Some(completion) = pending.remove(&acknowledgement.command_id) else {
                                break Some("unknown worker command acknowledgement");
                            };
                            let result = match acknowledgement.outcome {
                                WorkerCommandOutcome::Completed => Ok(()),
                                WorkerCommandOutcome::Stale => Err(session_error(
                                    "stale_worker_command",
                                    "worker rejected a stale command generation",
                                )),
                                WorkerCommandOutcome::Failed => Err(session_error(
                                    "worker_command_failed",
                                    format!(
                                        "{}: {}",
                                        acknowledgement.error_code.as_deref().unwrap_or(
                                            "worker_command_failed"
                                        ),
                                        acknowledgement.error_message.as_deref().unwrap_or(
                                            "worker reported an execution failure"
                                        )
                                    ),
                                )),
                            };
                            let _ = completion.send(result);
                        }
                        Some(Ok(Message::Binary(_))) => {
                            break Some("unsupported worker control message");
                        }
                        Some(Err(error)) => {
                            tracing::debug!(%worker_id, %error, "worker control session ended with an error");
                            break None;
                        }
                    }
                }
            }
        };

        if let Some(reason) = close_reason {
            close_worker_control(socket, reason).await;
        }
        for (_, completion) in pending {
            let _ = completion.send(Err(session_error(
                "worker_session_ended",
                "worker control session ended before command completion",
            )));
        }
        for (_, completion) in previews {
            let _ = completion.send(Err(session_error(
                "worker_session_ended",
                "worker control session ended before preview completion",
            )));
        }
        self.unregister_session(worker_id, session.session_id).await;
    }

    pub(crate) async fn discard_session(&self, session: WorkerControlSession) {
        let worker_id = session.worker_id;
        let session_id = session.session_id;
        drop(session);
        self.unregister_session(worker_id, session_id).await;
    }

    async fn send_and_wait(
        &self,
        worker_id: Uuid,
        command_id: Uuid,
        message: String,
    ) -> PortResult<()> {
        let (session_id, sender) = self
            .sessions
            .lock()
            .await
            .get(&worker_id)
            .map(|session| (session.session_id, session.commands.clone()))
            .ok_or_else(|| {
                session_error(
                    "worker_session_unavailable",
                    "worker has no active control session",
                )
            })?;
        if let Some(pool) = &self.lease_pool {
            let current: Option<Uuid> = sqlx::query_scalar(
                "SELECT session_id FROM worker_control_sessions WHERE worker_id = $1",
            )
            .bind(worker_id)
            .fetch_optional(pool)
            .await
            .map_err(|error| PortError {
                code: "worker_session_registration_failed",
                message: format!("worker control session lease could not be read: {error}"),
            })?;
            if current != Some(session_id) {
                return Err(session_error(
                    "worker_session_replaced",
                    "worker control session was replaced by another API instance",
                ));
            }
        }
        let (completion, completed) = oneshot::channel();
        sender
            .try_send(PendingCommand::Route {
                command_id,
                message,
                completion,
            })
            .map_err(command_queue_error)?;

        match tokio::time::timeout(WORKER_COMMAND_ACK_TIMEOUT, completed).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(session_error(
                "worker_session_ended",
                "worker control session ended before command completion",
            )),
            Err(_) => Err(session_error(
                "worker_command_ack_timeout",
                "worker command completion was not acknowledged in time",
            )),
        }
    }

    pub(crate) async fn preview_session(
        &self,
        worker_id: Uuid,
    ) -> PortResult<WorkerPreviewSession> {
        let handle = self
            .sessions
            .lock()
            .await
            .get(&worker_id)
            .map(|session| WorkerPreviewSession {
                worker_id,
                session_id: session.session_id,
                commands: session.commands.clone(),
                cancelled: session.cancel.subscribe(),
            })
            .ok_or_else(|| {
                session_error(
                    "worker_session_unavailable",
                    "worker has no active control session",
                )
            })?;
        if !self
            .is_current_session(worker_id, handle.session_id)
            .await?
        {
            return Err(session_error(
                "worker_session_replaced",
                "worker control session was replaced",
            ));
        }
        Ok(handle)
    }

    pub(crate) async fn cancel_preview_session(
        &self,
        worker_id: Uuid,
        session_id: Uuid,
    ) -> PortResult<()> {
        if let Some(session) = self.sessions.lock().await.get(&worker_id) {
            if session.session_id == session_id {
                session.cancel.send(true).map_err(|_| {
                    session_error(
                        "worker_session_ended",
                        "worker control session could not be cancelled",
                    )
                })?;
            }
        }
        Ok(())
    }

    pub(crate) async fn is_current_session(
        &self,
        worker_id: Uuid,
        session_id: Uuid,
    ) -> PortResult<bool> {
        if !self
            .sessions
            .lock()
            .await
            .get(&worker_id)
            .is_some_and(|session| session.session_id == session_id)
        {
            return Ok(false);
        }
        if let Some(pool) = &self.lease_pool {
            let current: Option<Uuid> = sqlx::query_scalar(
                "SELECT session_id FROM worker_control_sessions WHERE worker_id = $1",
            )
            .bind(worker_id)
            .fetch_optional(pool)
            .await
            .map_err(|error| {
                session_error(
                    "worker_session_registration_failed",
                    format!("worker session lease could not be read: {error}"),
                )
            })?;
            return Ok(current == Some(session_id));
        }
        Ok(true)
    }

    async fn pipeline_lock(&self, pipeline_id: Uuid) -> Arc<Mutex<()>> {
        let mut locks = self.pipeline_locks.lock().await;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(&pipeline_id).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(Mutex::new(()));
        locks.insert(pipeline_id, Arc::downgrade(&lock));
        lock
    }

    async fn remove_pipeline_lock_if_idle(&self, pipeline_id: Uuid, lock: &Arc<Mutex<()>>) {
        if Arc::strong_count(lock) != 1 {
            return;
        }
        let mut locks = self.pipeline_locks.lock().await;
        let current = locks.get(&pipeline_id).and_then(Weak::upgrade);
        let is_current = current
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, lock));
        drop(current);
        if is_current && Arc::strong_count(lock) == 1 {
            locks.remove(&pipeline_id);
        }
    }

    async fn unregister_session(&self, worker_id: Uuid, session_id: Uuid) {
        let mut sessions = self.sessions.lock().await;
        if sessions
            .get(&worker_id)
            .is_some_and(|session| session.session_id == session_id)
        {
            sessions.remove(&worker_id);
        }
        drop(sessions);
        if let Some(pool) = &self.lease_pool {
            let _ = sqlx::query(
                "DELETE FROM worker_control_sessions \
                 WHERE worker_id = $1 AND session_id = $2",
            )
            .bind(worker_id)
            .bind(session_id)
            .execute(pool)
            .await;
        }
    }
}

fn command_queue_error(error: mpsc::error::TrySendError<PendingCommand>) -> PortError {
    match error {
        mpsc::error::TrySendError::Full(_) => {
            session_error("worker_session_busy", "worker command queue is full")
        }
        mpsc::error::TrySendError::Closed(_) => session_error(
            "worker_session_unavailable",
            "worker control session is no longer available",
        ),
    }
}

fn session_error(code: &'static str, message: impl Into<String>) -> PortError {
    PortError {
        code,
        message: message.into(),
    }
}

async fn close_worker_control(socket: &mut WebSocket, reason: &'static str) {
    let frame = CloseFrame {
        code: close_code::POLICY,
        reason: reason.into(),
    };
    if let Err(error) = socket.send(Message::Close(Some(frame))).await {
        tracing::debug!(%error, "failed to close worker control session cleanly");
    }
}
