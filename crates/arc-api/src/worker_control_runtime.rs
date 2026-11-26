use std::{
    collections::HashMap,
    sync::{Arc, Weak},
    time::Duration,
};

use arc_application::{AssignmentGenerationStore, PortError, PortResult, WorkerAuthenticator};
use arc_shared::{WorkerCommandAcknowledgement, WorkerCommandEnvelope, WorkerCommandOutcome};
use arc_worker::{dispatch_if_fresh, CommandDispatchResult};
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
    session_id: Uuid,
    commands: mpsc::Receiver<PendingCommand>,
    cancelled: watch::Receiver<bool>,
}

struct PendingCommand {
    command_id: Uuid,
    message: String,
    completion: oneshot::Sender<PortResult<()>>,
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

    pub(crate) async fn start_session(&self, worker_id: Uuid) -> PortResult<WorkerControlSession> {
        let session_id = Uuid::new_v4();
        if let Some(pool) = &self.lease_pool {
            sqlx::query(
                "INSERT INTO worker_control_sessions (worker_id, session_id) \
                 VALUES ($1, $2) \
                 ON CONFLICT (worker_id) DO UPDATE SET \
                 session_id = EXCLUDED.session_id, \
                 created_at = clock_timestamp(), \
                 replaced_at = clock_timestamp()",
            )
            .bind(worker_id)
            .bind(session_id)
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
                    let PendingCommand { command_id, message, completion } = command;
                    pending.insert(command_id, completion);
                    if let Err(error) = socket.send(Message::Text(message.into())).await {
                        tracing::debug!(%worker_id, %error, "failed to send worker command");
                        if let Some(completion) = pending.remove(&command_id) {
                            let _ = completion.send(Err(session_error(
                                "worker_session_unavailable",
                                "worker command could not be sent",
                            )));
                        }
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
            .try_send(PendingCommand {
                command_id,
                message,
                completion,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => {
                    session_error("worker_session_busy", "worker command queue is full")
                }
                mpsc::error::TrySendError::Closed(_) => session_error(
                    "worker_session_unavailable",
                    "worker control session is no longer available",
                ),
            })?;

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
