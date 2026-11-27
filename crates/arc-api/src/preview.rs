use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use arc_application::{PipelineSourceIdentity, PortError, PortResult};
use arc_domain::SourceMediaProfile;
use arc_infrastructure::capacity_reservations::{
    PgCapacityReservations, PreviewReservationDecision,
};
use arc_worker::{WorkerPreviewControl, WorkerPreviewFinished, WORKER_PROTOCOL_VERSION};
use axum::{
    extract::{
        ws::{close_code, CloseFrame, Message, WebSocket, WebSocketUpgrade},
        Extension, Path,
    },
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use futures_util::SinkExt;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::time::Duration;
use tokio::sync::{oneshot, OwnedSemaphorePermit, Semaphore};
use uuid::Uuid;

use crate::{
    admin_auth::{self, AdminAuth, AdminSession},
    worker_control_runtime::WorkerPreviewSession,
    WorkerCommandDispatcher, WorkerControlState,
};

const MAX_PREVIEWS: usize = 8;
const MAX_PCM_BYTES: usize = 4096;
const MAX_BROWSER_FRAME_BYTES: usize = 1024;
const PREVIEW_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const PREVIEW_WRITE_TIMEOUT: Duration = Duration::from_secs(3);
const PREVIEW_CLEANUP_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) async fn worker_preview(
    Path(preview_id): Path<Uuid>,
    headers: HeaderMap,
    Extension(state): Extension<WorkerControlState>,
    websocket: WebSocketUpgrade,
) -> Response {
    let (worker_id, _) = match crate::authenticate_worker(&headers, &state).await {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let (Some(broker), Some(dispatcher)) = (&state.preview_broker, &state.dispatcher) else {
        return api_error(
            StatusCode::NOT_FOUND,
            "preview_unavailable",
            "preview is unavailable",
        );
    };
    let session = match dispatcher.preview_session(worker_id).await {
        Ok(session) => session,
        Err(error) => {
            tracing::warn!(%worker_id, code = error.code, "preview worker session is unavailable");
            return api_error(
                StatusCode::CONFLICT,
                "worker_session_unavailable",
                "worker session is unavailable",
            );
        }
    };
    let sender = match broker.claim(preview_id, worker_id, session.session_id) {
        Ok(Some(sender)) => sender,
        Ok(None) => {
            return api_error(
                StatusCode::NOT_FOUND,
                "preview_not_found",
                "preview is unavailable",
            )
        }
        Err(error) => {
            tracing::error!(code = error.code, message = %error.message, "preview registry failed");
            return api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "preview_unavailable",
                "preview is unavailable",
            );
        }
    };
    websocket.max_message_size(MAX_PCM_BYTES).max_frame_size(MAX_PCM_BYTES)
        .on_upgrade(move |socket| async move {
            if sender.send(socket).is_err() {
                tracing::debug!(%worker_id, %preview_id, "preview consumer disconnected before worker joined");
            }
        }).into_response()
}

pub(crate) async fn browser_preview(
    Path(source_id): Path<Uuid>,
    headers: HeaderMap,
    Extension(pool): Extension<PgPool>,
    Extension(auth): Extension<AdminAuth>,
    Extension(session): Extension<AdminSession>,
    Extension(broker): Extension<Arc<PreviewBroker>>,
    Extension(dispatcher): Extension<Arc<WorkerCommandDispatcher>>,
    websocket: WebSocketUpgrade,
) -> Response {
    if !same_origin(&headers) {
        return api_error(
            StatusCode::FORBIDDEN,
            "preview_origin_rejected",
            "preview requires the same origin",
        );
    }
    websocket
        .max_message_size(MAX_BROWSER_FRAME_BYTES)
        .max_frame_size(MAX_BROWSER_FRAME_BYTES)
        .on_upgrade(move |socket| async move {
            serve_browser(
                socket,
                headers,
                auth,
                session.user_id,
                broker,
                dispatcher,
                pool,
                source_id,
            )
            .await;
        })
        .into_response()
}

fn same_origin(headers: &HeaderMap) -> bool {
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok());
    match (origin, host, scheme) {
        (Some(origin), Some(host), Some(scheme)) if matches!(scheme, "http" | "https") => {
            origin == format!("{scheme}://{host}")
        }
        _ => false,
    }
}

async fn serve_browser(
    mut browser: WebSocket,
    headers: HeaderMap,
    auth: AdminAuth,
    user_id: Uuid,
    broker: Arc<PreviewBroker>,
    dispatcher: Arc<WorkerCommandDispatcher>,
    pool: PgPool,
    source_id: Uuid,
) {
    let result = match std::env::var("ARC_API_ENABLE_FENCED_PREVIEW") {
        Ok(value) if value == "true" => {
            serve_reserved_preview(
                &mut browser,
                &headers,
                &auth,
                user_id,
                &broker,
                &dispatcher,
                pool,
                source_id,
            )
            .await
        }
        Ok(value) if value == "false" => {
            serve_active_preview(
                &mut browser,
                &headers,
                &auth,
                user_id,
                &broker,
                &dispatcher,
                &pool,
                source_id,
            )
            .await
        }
        Err(std::env::VarError::NotPresent) => {
            serve_active_preview(
                &mut browser,
                &headers,
                &auth,
                user_id,
                &broker,
                &dispatcher,
                &pool,
                source_id,
            )
            .await
        }
        _ => Err(preview_error(
            "preview_configuration_invalid",
            "fenced preview setting must be true or false",
        )),
    };
    let reason = match &result {
        Ok(()) => None,
        Err(error) => {
            tracing::warn!(code = error.code, message = %error.message, "preview ended with an error");
            Some(CloseFrame {
                code: close_code::ERROR,
                reason: error.code.into(),
            })
        }
    };
    if let Err(error) = send(&mut browser, Message::Close(reason)).await {
        tracing::debug!(code = error.code, message = %error.message, "preview browser socket did not close cleanly");
    }
}

async fn serve_active_preview(
    browser: &mut WebSocket,
    headers: &HeaderMap,
    auth: &AdminAuth,
    user_id: Uuid,
    broker: &Arc<PreviewBroker>,
    dispatcher: &WorkerCommandDispatcher,
    pool: &PgPool,
    source_id: Uuid,
) -> PortResult<()> {
    let (mut worker, revision, source) = find_active_worker(pool, dispatcher, source_id).await?;
    validate_session(auth, headers, user_id).await?;
    let (reservation, mut incoming) = broker.reserve(worker.worker_id, worker.session_id)?;
    let preview_id = reservation.preview_id;
    let mut finished = worker.start(
        WorkerPreviewControl::PreviewStart {
            protocol_version: WORKER_PROTOCOL_VERSION,
            worker_id: worker.worker_id,
            preview_id,
            identity: PipelineSourceIdentity {
                source_id,
                source_revision: revision,
            },
            source,
            reserved_source: false,
        },
        preview_id,
    )?;
    let mut completion = None;
    let result = relay(
        browser,
        headers,
        auth,
        user_id,
        dispatcher,
        worker.worker_id,
        worker.session_id,
        &mut worker.cancelled,
        &mut incoming,
        &mut finished,
        &mut completion,
    )
    .await;
    if let Err(error) = worker.stop(preview_id) {
        tracing::warn!(%preview_id, code = error.code, message = %error.message, "could not send preview stop");
    }
    result
}

async fn find_active_worker(
    pool: &PgPool,
    dispatcher: &WorkerCommandDispatcher,
    source_id: Uuid,
) -> PortResult<(WorkerPreviewSession, i64, SourceMediaProfile)> {
    let rows = sqlx::query_as::<_, (Uuid, i64, Value)>(
        "SELECT a.worker_id, a.source_revision, r.configuration FROM pipeline_assignments a \
         JOIN sources s ON s.id = a.source_id AND s.current_revision = a.source_revision \
         JOIN source_revisions r ON r.source_id = a.source_id AND r.revision = a.source_revision \
         WHERE a.source_id = $1 AND a.resource_type = 'pipeline' \
         AND a.desired_state = 'running' AND a.observed_state = 'running' \
         ORDER BY a.worker_id, a.id",
    )
    .bind(source_id)
    .fetch_all(pool)
    .await
    .map_err(|error| {
        tracing::error!(%error, "preview source placement could not be read");
        preview_error("preview_unavailable", "preview placement is unavailable")
    })?;
    if rows.is_empty() {
        return Err(preview_error(
            "preview_source_inactive",
            "preview currently requires a running route for this input",
        ));
    }
    for (worker_id, revision, configuration) in rows {
        match dispatcher.preview_session(worker_id).await {
            Ok(worker) => {
                let source = serde_json::from_value(configuration).map_err(|error| {
                    tracing::error!(%error, %source_id, "preview source profile is invalid");
                    preview_error(
                        "preview_source_invalid",
                        "saved input configuration is invalid",
                    )
                })?;
                return Ok((worker, revision, source));
            }
            Err(error) => {
                tracing::debug!(%worker_id, code = error.code, "preview worker session unavailable")
            }
        }
    }
    Err(preview_error(
        "worker_session_unavailable",
        "no worker running this input is connected to this API instance",
    ))
}

async fn serve_reserved_preview(
    browser: &mut WebSocket,
    headers: &HeaderMap,
    auth: &AdminAuth,
    user_id: Uuid,
    broker: &Arc<PreviewBroker>,
    dispatcher: &WorkerCommandDispatcher,
    pool: PgPool,
    source_id: Uuid,
) -> PortResult<()> {
    validate_session(auth, headers, user_id).await?;
    let slot = broker.allocate()?;
    let preview_id = slot.preview_id;
    let reservations = PgCapacityReservations::new(pool);
    let placement = match reservations
        .reserve_fenced_preview(preview_id, source_id, chrono::Duration::seconds(30))
        .await?
    {
        PreviewReservationDecision::Reserved(placement) => placement,
        PreviewReservationDecision::Unavailable { rejections } => {
            tracing::warn!(%source_id, ?rejections, "no fenced worker has capacity for preview");
            return Err(preview_error(
                "preview_unavailable",
                "no worker has capacity for preview",
            ));
        }
    };
    let worker_id = placement.reservation.worker_id;
    let session_id = placement.worker_session_id;
    let prepared = async {
        validate_session(auth, headers, user_id).await?;
        let worker = dispatcher.preview_session(worker_id).await?;
        if worker.session_id != session_id {
            return Err(preview_error(
                "worker_session_ended",
                "worker control session was replaced",
            ));
        }
        let incoming = broker.attach(&slot, worker_id, session_id)?;
        let finished = worker.start(
            WorkerPreviewControl::PreviewStart {
                protocol_version: WORKER_PROTOCOL_VERSION,
                worker_id,
                preview_id,
                identity: PipelineSourceIdentity {
                    source_id,
                    source_revision: placement.source_revision,
                },
                source: placement.source,
                reserved_source: true,
            },
            preview_id,
        )?;
        Ok((worker, incoming, finished))
    }
    .await;
    let (mut worker, mut incoming, mut finished) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            reservations
                .release_preview(preview_id, worker_id, session_id)
                .await?;
            return Err(error);
        }
    };
    let mut completion = None;
    let result = relay(
        browser,
        headers,
        auth,
        user_id,
        dispatcher,
        worker_id,
        session_id,
        &mut worker.cancelled,
        &mut incoming,
        &mut finished,
        &mut completion,
    )
    .await;
    if completion.is_none() {
        if let Err(error) = worker.stop(preview_id) {
            tracing::warn!(%preview_id, code = error.code, message = %error.message, "could not send preview stop");
            dispatcher
                .cancel_preview_session(worker_id, session_id)
                .await?;
        }
        match tokio::time::timeout(PREVIEW_CLEANUP_TIMEOUT, &mut finished).await {
            Ok(finished) => {
                completion = Some(
                    finished
                        .map_err(|_| {
                            preview_error(
                                "worker_session_ended",
                                "worker preview completion was lost",
                            )
                        })
                        .and_then(|event| event),
                );
            }
            Err(_) => {
                dispatcher
                    .cancel_preview_session(worker_id, session_id)
                    .await?;
                tracing::error!(%preview_id, %worker_id, "preview cleanup timed out; retaining its reservation");
            }
        }
    }
    match completion {
        Some(Ok(event)) if event.cleanup_confirmed => {
            reservations
                .release_preview(preview_id, worker_id, session_id)
                .await?;
        }
        _ => {
            dispatcher
                .cancel_preview_session(worker_id, session_id)
                .await?;
            tracing::error!(%preview_id, %worker_id, "preview cleanup was not confirmed; retaining its reservation");
            return Err(preview_error(
                "preview_cleanup_unconfirmed",
                "worker preview cleanup was not confirmed",
            ));
        }
    }
    result
}

async fn relay(
    browser: &mut WebSocket,
    headers: &HeaderMap,
    auth: &AdminAuth,
    user_id: Uuid,
    dispatcher: &WorkerCommandDispatcher,
    worker_id: Uuid,
    worker_session_id: Uuid,
    cancelled: &mut tokio::sync::watch::Receiver<bool>,
    incoming: &mut oneshot::Receiver<WebSocket>,
    finished: &mut oneshot::Receiver<PortResult<WorkerPreviewFinished>>,
    completion: &mut Option<PortResult<WorkerPreviewFinished>>,
) -> PortResult<()> {
    let mut auth_check = tokio::time::interval(Duration::from_secs(1));
    auth_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let deadline = tokio::time::sleep(PREVIEW_CONNECT_TIMEOUT);
    tokio::pin!(deadline);
    let mut data = loop {
        tokio::select! {
            result = &mut *incoming => break result.map_err(|_| preview_error("preview_connection_failed", "worker preview connection ended"))?,
            result = &mut *finished => return record_completion(result, completion),
            _ = &mut deadline => return Err(preview_error("preview_connection_timeout", "worker preview did not connect in time")),
            _ = auth_check.tick() => {
                validate_session(auth, headers, user_id).await?;
                validate_worker_session(dispatcher, worker_id, worker_session_id).await?;
            }
            changed = cancelled.changed() => {
                if changed.is_err() || *cancelled.borrow() {
                    return Err(preview_error("worker_session_ended", "worker control session ended"));
                }
            }
            message = browser.recv() => {
                match message {
                    Some(Ok(Message::Ping(payload))) => send(browser, Message::Pong(payload)).await?,
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) => {
                        flush_close(browser).await?;
                        return Ok(());
                    }
                    None => return Ok(()),
                    Some(Ok(_)) => return Err(preview_error("preview_protocol_error", "browser sent an unexpected message")),
                    Some(Err(error)) => return Err(PortError { code: "preview_transport_failed", message: error.to_string() }),
                }
            }
        }
    };
    let result = loop {
        tokio::select! {
            result = &mut *finished => break record_completion(result, completion),
            _ = auth_check.tick() => {
                if let Err(error) = validate_session(auth, headers, user_id).await { break Err(error); }
                if let Err(error) = validate_worker_session(dispatcher, worker_id, worker_session_id).await { break Err(error); }
            }
            changed = cancelled.changed() => {
                if changed.is_err() || *cancelled.borrow() {
                    break Err(preview_error("worker_session_ended", "worker control session ended"));
                }
            }
            message = browser.recv() => {
                match message {
                    Some(Ok(Message::Ping(payload))) => if let Err(error) = send(browser, Message::Pong(payload)).await { break Err(error); },
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) => break flush_close(browser).await,
                    None => break Ok(()),
                    Some(Ok(_)) => break Err(preview_error("preview_protocol_error", "browser sent an unexpected message")),
                    Some(Err(error)) => break Err(PortError { code: "preview_transport_failed", message: error.to_string() }),
                }
            }
            message = data.recv() => {
                match message {
                    Some(Ok(Message::Binary(packet))) if !packet.is_empty() && packet.len() <= MAX_PCM_BYTES && packet.len() % 4 == 0 => {
                        if let Err(error) = send(browser, Message::Binary(packet)).await { break Err(error); }
                    }
                    Some(Ok(Message::Ping(payload))) => if let Err(error) = send(&mut data, Message::Pong(payload)).await { break Err(error); },
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(Some(frame)))) if frame.code != close_code::NORMAL => {
                        break Err(worker_error(frame.reason.as_str(), "worker preview failed"));
                    }
                    Some(Ok(Message::Close(_))) | None => break Ok(()),
                    Some(Ok(_)) => break Err(preview_error("preview_protocol_error", "worker sent an invalid audio packet")),
                    Some(Err(error)) => break Err(PortError { code: "preview_transport_failed", message: error.to_string() }),
                }
            }
        }
    };
    if let Err(error) = send(&mut data, Message::Close(None)).await {
        tracing::debug!(code = error.code, message = %error.message, "preview worker socket did not close cleanly");
    }
    result
}

async fn validate_worker_session(
    dispatcher: &WorkerCommandDispatcher,
    worker_id: Uuid,
    session_id: Uuid,
) -> PortResult<()> {
    if dispatcher.is_current_session(worker_id, session_id).await? {
        Ok(())
    } else {
        Err(preview_error(
            "worker_session_ended",
            "worker control session was replaced",
        ))
    }
}

fn record_completion(
    result: Result<PortResult<WorkerPreviewFinished>, oneshot::error::RecvError>,
    completion: &mut Option<PortResult<WorkerPreviewFinished>>,
) -> PortResult<()> {
    let result = result
        .map_err(|_| preview_error("worker_session_ended", "worker preview completion was lost"))
        .and_then(|event| event);
    let playback_result = match &result {
        Err(error) => Err(error.clone()),
        Ok(event) => match (&event.error_code, &event.error_message) {
            (Some(code), Some(message)) => Err(worker_error(code, message)),
            (None, None) => Ok(()),
            _ => Err(preview_error(
                "preview_protocol_error",
                "worker preview completion is invalid",
            )),
        },
    };
    *completion = Some(result);
    playback_result
}

fn worker_error(code: &str, message: &str) -> PortError {
    let code = match code {
        "preview_codec_unsupported" => "preview_codec_unsupported",
        "preview_source_inactive" => "preview_source_inactive",
        "preview_capacity_exhausted" => "preview_capacity_exhausted",
        "preview_unsupported" => "preview_unsupported",
        _ => "worker_preview_failed",
    };
    PortError {
        code,
        message: message.to_owned(),
    }
}

async fn validate_session(auth: &AdminAuth, headers: &HeaderMap, user_id: Uuid) -> PortResult<()> {
    match admin_auth::load_session(auth, headers).await {
        Ok(Some(session)) if session.user_id == user_id => Ok(()),
        Ok(_) => Err(preview_error(
            "preview_session_revoked",
            "administrator session ended",
        )),
        Err(error) => {
            tracing::error!(?error, "preview session could not be verified");
            Err(preview_error(
                "preview_session_unavailable",
                "administrator session could not be verified",
            ))
        }
    }
}

async fn send(socket: &mut WebSocket, message: Message) -> PortResult<()> {
    tokio::time::timeout(PREVIEW_WRITE_TIMEOUT, socket.send(message))
        .await
        .map_err(|_| {
            preview_error(
                "preview_transport_timeout",
                "preview connection is too slow",
            )
        })?
        .map_err(|error| PortError {
            code: "preview_transport_failed",
            message: error.to_string(),
        })
}

async fn flush_close(socket: &mut WebSocket) -> PortResult<()> {
    tokio::time::timeout(PREVIEW_WRITE_TIMEOUT, socket.flush())
        .await
        .map_err(|_| {
            preview_error(
                "preview_transport_timeout",
                "preview connection is too slow",
            )
        })?
        .map_err(|error| PortError {
            code: "preview_transport_failed",
            message: error.to_string(),
        })
}

fn api_error(status: StatusCode, code: &'static str, message: &'static str) -> Response {
    (
        status,
        Json(json!({ "error": { "code": code, "message": message } })),
    )
        .into_response()
}

pub(crate) struct PreviewBroker {
    pending: Mutex<HashMap<Uuid, Option<PendingWorker>>>,
    slots: Arc<Semaphore>,
}

struct PendingWorker {
    worker_id: Uuid,
    session_id: Uuid,
    socket: Option<oneshot::Sender<WebSocket>>,
}

pub(crate) struct PreviewReservation {
    broker: Arc<PreviewBroker>,
    pub(crate) preview_id: Uuid,
    _permit: OwnedSemaphorePermit,
}

impl PreviewBroker {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            pending: Mutex::new(HashMap::new()),
            slots: Arc::new(Semaphore::new(MAX_PREVIEWS)),
        })
    }

    fn allocate(self: &Arc<Self>) -> PortResult<PreviewReservation> {
        let permit = Arc::clone(&self.slots).try_acquire_owned().map_err(|_| {
            preview_error(
                "preview_capacity_exhausted",
                "the preview limit has been reached",
            )
        })?;
        let mut pending = self.pending.lock().map_err(|_| {
            preview_error(
                "preview_registry_unavailable",
                "preview registry is unavailable",
            )
        })?;
        let preview_id = loop {
            let id = Uuid::new_v4();
            if !pending.contains_key(&id) {
                break id;
            }
        };
        pending.insert(preview_id, None);
        Ok(PreviewReservation {
            broker: Arc::clone(self),
            preview_id,
            _permit: permit,
        })
    }

    fn attach(
        &self,
        reservation: &PreviewReservation,
        worker_id: Uuid,
        session_id: Uuid,
    ) -> PortResult<oneshot::Receiver<WebSocket>> {
        if !std::ptr::eq(self, reservation.broker.as_ref()) {
            return Err(preview_error(
                "invalid_preview_identity",
                "preview broker does not match",
            ));
        }
        if worker_id.is_nil() || session_id.is_nil() {
            return Err(preview_error(
                "invalid_preview_identity",
                "worker session identity is invalid",
            ));
        }
        let mut pending = self.pending.lock().map_err(|_| {
            preview_error(
                "preview_registry_unavailable",
                "preview registry is unavailable",
            )
        })?;
        let entry = pending.get_mut(&reservation.preview_id).ok_or_else(|| {
            preview_error(
                "preview_registry_unavailable",
                "preview slot is unavailable",
            )
        })?;
        if entry.is_some() {
            return Err(preview_error(
                "invalid_preview_identity",
                "preview slot is already attached",
            ));
        }
        let (socket, received) = oneshot::channel();
        *entry = Some(PendingWorker {
            worker_id,
            session_id,
            socket: Some(socket),
        });
        Ok(received)
    }

    pub(crate) fn reserve(
        self: &Arc<Self>,
        worker_id: Uuid,
        session_id: Uuid,
    ) -> PortResult<(PreviewReservation, oneshot::Receiver<WebSocket>)> {
        let reservation = self.allocate()?;
        let received = self.attach(&reservation, worker_id, session_id)?;
        Ok((reservation, received))
    }

    pub(crate) fn claim(
        &self,
        preview_id: Uuid,
        worker_id: Uuid,
        session_id: Uuid,
    ) -> PortResult<Option<oneshot::Sender<WebSocket>>> {
        let mut pending = self.pending.lock().map_err(|_| {
            preview_error(
                "preview_registry_unavailable",
                "preview registry is unavailable",
            )
        })?;
        if !pending
            .get(&preview_id)
            .and_then(Option::as_ref)
            .is_some_and(|entry| entry.worker_id == worker_id && entry.session_id == session_id)
        {
            return Ok(None);
        }
        Ok(pending
            .get_mut(&preview_id)
            .and_then(Option::as_mut)
            .and_then(|entry| entry.socket.take()))
    }
}

impl Drop for PreviewReservation {
    fn drop(&mut self) {
        match self.broker.pending.lock() {
            Ok(mut pending) => {
                pending.remove(&self.preview_id);
            }
            Err(error) => tracing::error!(%error, "preview registry was poisoned during cleanup"),
        }
    }
}

fn preview_error(code: &'static str, message: &'static str) -> PortError {
    PortError {
        code,
        message: message.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_requires_an_exact_browser_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "127.0.0.1:4180".parse().unwrap());
        headers.insert("x-forwarded-proto", "http".parse().unwrap());
        headers.insert(header::ORIGIN, "http://127.0.0.1:4180".parse().unwrap());
        assert!(same_origin(&headers));
        headers.insert(header::ORIGIN, "http://127.0.0.1:9999".parse().unwrap());
        assert!(!same_origin(&headers));
        headers.insert(header::ORIGIN, "https://127.0.0.1:4180".parse().unwrap());
        assert!(!same_origin(&headers));
        headers.remove(header::ORIGIN);
        assert!(!same_origin(&headers));
    }

    #[test]
    fn allocated_slot_cannot_be_claimed_until_attached() {
        let broker = PreviewBroker::new();
        let worker_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let slot = broker.allocate().unwrap();
        assert!(broker
            .claim(slot.preview_id, worker_id, session_id)
            .unwrap()
            .is_none());
        let _incoming = broker.attach(&slot, worker_id, session_id).unwrap();
        assert!(broker
            .claim(slot.preview_id, worker_id, session_id)
            .unwrap()
            .is_some());
        assert!(broker.attach(&slot, worker_id, session_id).is_err());
    }

    #[test]
    fn completion_preserves_cleanup_even_when_preview_fails() {
        let event = WorkerPreviewFinished {
            kind: arc_worker::WorkerPreviewEventKind::PreviewFinished,
            protocol_version: WORKER_PROTOCOL_VERSION,
            worker_id: Uuid::new_v4(),
            preview_id: Uuid::new_v4(),
            error_code: Some("preview_capacity_exhausted".to_owned()),
            error_message: Some("capacity exhausted".to_owned()),
            cleanup_confirmed: true,
        };
        let mut completion = None;
        assert_eq!(
            record_completion(Ok(Ok(event)), &mut completion)
                .unwrap_err()
                .code,
            "preview_capacity_exhausted"
        );
        assert!(completion.unwrap().unwrap().cleanup_confirmed);
    }

    #[test]
    fn reservations_are_bounded_and_claimed_only_once_by_the_matching_session() {
        let broker = PreviewBroker::new();
        let worker_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let mut reservations = Vec::new();
        for _ in 0..MAX_PREVIEWS {
            reservations.push(broker.reserve(worker_id, session_id).unwrap().0);
        }
        assert_eq!(
            broker.reserve(worker_id, session_id).err().unwrap().code,
            "preview_capacity_exhausted"
        );
        let preview_id = reservations[0].preview_id;
        assert!(broker
            .claim(preview_id, Uuid::new_v4(), session_id)
            .unwrap()
            .is_none());
        assert!(broker
            .claim(preview_id, worker_id, Uuid::new_v4())
            .unwrap()
            .is_none());
        assert!(broker
            .claim(preview_id, worker_id, session_id)
            .unwrap()
            .is_some());
        assert!(broker
            .claim(preview_id, worker_id, session_id)
            .unwrap()
            .is_none());
        drop(reservations.pop());
        assert!(broker.reserve(worker_id, session_id).is_ok());
    }

    #[test]
    fn dropping_a_reservation_cancels_the_unclaimed_connection() {
        let broker = PreviewBroker::new();
        let worker_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let (reservation, _) = broker.reserve(worker_id, session_id).unwrap();
        let preview_id = reservation.preview_id;
        drop(reservation);
        assert!(broker
            .claim(preview_id, worker_id, session_id)
            .unwrap()
            .is_none());
    }
}
