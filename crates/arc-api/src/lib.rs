use arc_application::{PipelineSpecification, PortError, WorkerAuthenticator};
use arc_domain::{
    estimate_pipeline_requirements, validate_destination_profile, validate_source_profile,
    DestinationMediaProfile, MediaProfileError, SourceMediaProfile,
};
use arc_infrastructure::catalog::{
    self, DestinationRecord, DraftValidationError, ResourceUpdateError, SourceRecord,
};
use arc_infrastructure::{
    assignments::{AssignmentSnapshot, PgAssignments, PlannedAssignment},
    pipeline_planning::{PgPipelinePlanner, PipelinePlanningDecision, RouteRevision},
    postgres::PgPool,
};
use arc_shared::{WorkerCommandEnvelope, WORKER_PROTOCOL_VERSION};
use arc_worker::{CommandDispatchResult, WorkerCommand};
use axum::{
    extract::{
        ws::{close_code, CloseFrame, Message, WebSocket, WebSocketUpgrade},
        Extension, Path,
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

mod worker_control_runtime;
pub use worker_control_runtime::WorkerCommandDispatcher;
use worker_control_runtime::MAX_WORKER_CONTROL_MESSAGE_SIZE;

const MAX_PLANNING_HEARTBEAT_AGE_SECONDS: i64 = 30;
const WORKER_CONTROL_HELLO_TIMEOUT: Duration = Duration::from_secs(5);
const WORKER_CONTROL_ROUTE: &str = "/api/v1/workers/control";

pub fn router() -> Router {
    Router::new()
        .route("/api/v1/health/live", get(liveness))
        .route("/api/v1/system/capabilities", get(capabilities))
        .route("/api/v1/routes/plan", post(plan_route))
        .route(
            "/api/v1/sources/{source_id}/validate",
            post(validate_source_draft),
        )
        .route(
            "/api/v1/destinations/{destination_id}/validate",
            post(validate_destination_draft),
        )
}

fn database_routes() -> Router {
    router()
        .route("/api/v1/sources", get(list_sources).post(create_source))
        .route("/api/v1/sources/{source_id}", put(update_source))
        .route(
            "/api/v1/destinations",
            get(list_destinations).post(create_destination),
        )
        .route(
            "/api/v1/destinations/{destination_id}",
            put(update_destination),
        )
        .route(
            "/api/v1/routes/plan-assignment",
            post(plan_route_assignment),
        )
        .route(
            "/api/v1/pipelines/{pipeline_id}/assignment",
            get(get_pipeline_assignment),
        )
        .route(
            "/api/v1/assignments/{assignment_id}/cancel",
            post(cancel_assignment),
        )
}

pub fn router_with_database(pool: PgPool) -> Router {
    database_routes().layer(Extension(pool))
}

pub fn router_with_worker_authenticator(authenticator: Arc<dyn WorkerAuthenticator>) -> Router {
    router().merge(worker_control_router(authenticator, None))
}

pub fn router_with_worker_dispatcher(
    authenticator: Arc<dyn WorkerAuthenticator>,
    dispatcher: Arc<WorkerCommandDispatcher>,
) -> Router {
    router()
        .merge(worker_control_router(
            authenticator,
            Some(dispatcher.clone()),
        ))
        .layer(Extension(dispatcher))
}

pub fn router_with_database_and_worker_authenticator(
    pool: PgPool,
    authenticator: Arc<dyn WorkerAuthenticator>,
) -> Router {
    router_with_database(pool).merge(worker_control_router(authenticator, None))
}

pub fn router_with_database_and_worker_dispatcher(
    pool: PgPool,
    authenticator: Arc<dyn WorkerAuthenticator>,
    dispatcher: Arc<WorkerCommandDispatcher>,
) -> Router {
    database_routes()
        .route(
            "/api/v1/assignments/{assignment_id}/start",
            post(start_assignment),
        )
        .route(
            "/api/v1/assignments/{assignment_id}/stop",
            post(stop_assignment),
        )
        .merge(worker_control_router(
            authenticator,
            Some(dispatcher.clone()),
        ))
        .layer(Extension(pool))
        .layer(Extension(dispatcher))
}

#[derive(Clone)]
struct WorkerControlState {
    authenticator: Arc<dyn WorkerAuthenticator>,
    dispatcher: Option<Arc<WorkerCommandDispatcher>>,
}

fn worker_control_router(
    authenticator: Arc<dyn WorkerAuthenticator>,
    dispatcher: Option<Arc<WorkerCommandDispatcher>>,
) -> Router {
    Router::new()
        .route(WORKER_CONTROL_ROUTE, get(worker_control))
        .layer(Extension(WorkerControlState {
            authenticator,
            dispatcher,
        }))
}

async fn worker_control(
    headers: HeaderMap,
    Extension(state): Extension<WorkerControlState>,
    websocket: WebSocketUpgrade,
) -> Response {
    if !headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.eq_ignore_ascii_case("https"))
        .unwrap_or(false)
    {
        return worker_control_error(
            StatusCode::UPGRADE_REQUIRED,
            "secure_worker_connection_required",
            "worker control requires TLS at the trusted ingress",
        );
    }

    let Some(worker_id) = headers
        .get("x-worker-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Uuid::parse_str(value).ok())
        .filter(|worker_id| !worker_id.is_nil())
    else {
        return worker_control_error(
            StatusCode::UNAUTHORIZED,
            "worker_authentication_failed",
            "worker identity or credential was rejected",
        );
    };
    let Some(bearer_token) = bearer_token(&headers) else {
        return worker_control_error(
            StatusCode::UNAUTHORIZED,
            "worker_authentication_failed",
            "worker identity or credential was rejected",
        );
    };

    if let Err(error) = state
        .authenticator
        .authenticate(worker_id, bearer_token)
        .await
    {
        let (status, code, message) = if error.code == "worker_authentication_failed" {
            (
                StatusCode::UNAUTHORIZED,
                "worker_authentication_failed",
                "worker identity or credential was rejected",
            )
        } else {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "worker_authentication_unavailable",
                "worker credentials could not be verified",
            )
        };
        return worker_control_error(status, code, message);
    }

    websocket
        .max_message_size(MAX_WORKER_CONTROL_MESSAGE_SIZE)
        .max_frame_size(MAX_WORKER_CONTROL_MESSAGE_SIZE)
        .on_upgrade(move |socket| handle_worker_control(socket, worker_id, state.dispatcher))
        .into_response()
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let authorization = headers.get("authorization")?.to_str().ok()?;
    let (scheme, token) = authorization.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer")
        || token.is_empty()
        || token.bytes().any(|byte| byte.is_ascii_whitespace())
    {
        return None;
    }
    Some(token)
}

fn worker_control_error(status: StatusCode, code: &'static str, message: &'static str) -> Response {
    (
        status,
        Json(json!({
            "error": {
                "code": code,
                "message": message,
            }
        })),
    )
        .into_response()
}

async fn handle_worker_control(
    mut socket: WebSocket,
    worker_id: Uuid,
    dispatcher: Option<Arc<WorkerCommandDispatcher>>,
) {
    let first_message = tokio::time::timeout(WORKER_CONTROL_HELLO_TIMEOUT, socket.recv()).await;
    let Ok(Some(Ok(Message::Text(text)))) = first_message else {
        close_worker_control(&mut socket, "worker hello required").await;
        return;
    };
    let Ok(hello) = serde_json::from_str::<arc_shared::WorkerSessionHello>(text.as_str()) else {
        close_worker_control(&mut socket, "invalid worker hello").await;
        return;
    };
    if hello.validate_for_worker(worker_id).is_err() {
        close_worker_control(&mut socket, "invalid worker hello").await;
        return;
    }

    let session = match &dispatcher {
        Some(dispatcher) => Some(dispatcher.start_session(worker_id).await),
        None => None,
    };
    let welcome = arc_shared::WorkerSessionWelcome {
        kind: arc_shared::WorkerSessionMessageKind::Welcome,
        protocol_version: arc_shared::WORKER_PROTOCOL_VERSION,
        worker_id,
    };
    match serde_json::to_string(&welcome) {
        Ok(message) => {
            if let Err(error) = socket.send(Message::Text(message.into())).await {
                tracing::debug!(%worker_id, %error, "failed to send worker welcome");
                if let (Some(dispatcher), Some(session)) = (&dispatcher, session) {
                    dispatcher.discard_session(session).await;
                }
                return;
            }
        }
        Err(error) => {
            tracing::error!(%worker_id, %error, "failed to serialize worker welcome");
            close_worker_control(&mut socket, "worker session unavailable").await;
            if let (Some(dispatcher), Some(session)) = (&dispatcher, session) {
                dispatcher.discard_session(session).await;
            }
            return;
        }
    }

    if let (Some(dispatcher), Some(session)) = (dispatcher, session) {
        dispatcher.serve_session(&mut socket, session).await;
        return;
    }

    while let Some(message) = socket.recv().await {
        match message {
            Ok(Message::Ping(payload)) => {
                if let Err(error) = socket.send(Message::Pong(payload)).await {
                    tracing::debug!(%worker_id, %error, "failed to respond to worker ping");
                    return;
                }
            }
            Ok(Message::Pong(_)) => {}
            Ok(Message::Close(_)) => return,
            Ok(Message::Text(_)) | Ok(Message::Binary(_)) => {
                close_worker_control(
                    &mut socket,
                    "worker command and event handling is not enabled",
                )
                .await;
                return;
            }
            Err(error) => {
                tracing::debug!(%worker_id, %error, "worker control session ended with an error");
                return;
            }
        }
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

async fn liveness() -> Json<Value> {
    Json(json!({ "status": "alive" }))
}

async fn capabilities() -> Json<Value> {
    Json(json!({
        "apiVersion": "1.0",
        "workers": []
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateSourceRequest {
    friendly_name: String,
    profile: SourceMediaProfile,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateDestinationRequest {
    friendly_name: String,
    profile: DestinationMediaProfile,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlanRouteRequest {
    source: SourceMediaProfile,
    destination: DestinationMediaProfile,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanRouteResponse {
    media_path: &'static str,
    estimate: RouteResourceEstimate,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RouteResourceEstimate {
    capacity_units: u32,
    ingress_payload_bits_per_second: Option<u64>,
    egress_payload_bits_per_second: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlanRouteAssignmentRequest {
    pipeline_id: Uuid,
    source_id: Uuid,
    source_revision: i64,
    destination_id: Uuid,
    destination_revision: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanRouteAssignmentResponse {
    state: &'static str,
    media_path: &'static str,
    estimate: RouteResourceEstimate,
    assignment: Option<PlannedAssignment>,
    rejections: Vec<arc_domain::WorkerRejection>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CancelAssignmentRequest {
    generation: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssignmentGenerationRequest {
    generation: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CancelAssignmentResponse {
    assignment_id: Uuid,
    generation: i64,
    state: &'static str,
}

async fn list_sources(
    Extension(pool): Extension<PgPool>,
) -> Result<Json<Vec<SourceRecord>>, ApiError> {
    catalog::list_sources(&pool)
        .await
        .map(Json)
        .map_err(|error| ApiError::database("list sources", error))
}

async fn validate_source_draft(
    Path(source_id): Path<Uuid>,
    Extension(pool): Extension<PgPool>,
) -> Result<Json<SourceRecord>, ApiError> {
    catalog::validate_source_draft(&pool, source_id)
        .await
        .map(Json)
        .map_err(ApiError::draft_validation)
}

async fn create_source(
    Extension(pool): Extension<PgPool>,
    Json(request): Json<CreateSourceRequest>,
) -> Result<(StatusCode, Json<SourceRecord>), ApiError> {
    validate_friendly_name(&request.friendly_name)?;
    validate_source_profile(&request.profile).map_err(ApiError::invalid_media_profile)?;
    let configuration =
        serde_json::to_value(&request.profile).map_err(ApiError::serialize_profile)?;

    let source = catalog::create_source(
        &pool,
        request.friendly_name.trim(),
        request.profile.adapter_id(),
        configuration,
    )
    .await
    .map_err(|error| ApiError::database("create source", error))?;

    Ok((StatusCode::CREATED, Json(source)))
}

async fn update_source(
    Path(source_id): Path<Uuid>,
    Extension(pool): Extension<PgPool>,
    Json(request): Json<CreateSourceRequest>,
) -> Result<Json<SourceRecord>, ApiError> {
    validate_friendly_name(&request.friendly_name)?;
    validate_source_profile(&request.profile).map_err(ApiError::invalid_media_profile)?;
    let configuration =
        serde_json::to_value(&request.profile).map_err(ApiError::serialize_profile)?;

    catalog::update_source(
        &pool,
        source_id,
        request.friendly_name.trim(),
        request.profile.adapter_id(),
        configuration,
    )
    .await
    .map(Json)
    .map_err(ApiError::resource_update)
}

async fn list_destinations(
    Extension(pool): Extension<PgPool>,
) -> Result<Json<Vec<DestinationRecord>>, ApiError> {
    catalog::list_destinations(&pool)
        .await
        .map(Json)
        .map_err(|error| ApiError::database("list destinations", error))
}

async fn validate_destination_draft(
    Path(destination_id): Path<Uuid>,
    Extension(pool): Extension<PgPool>,
) -> Result<Json<DestinationRecord>, ApiError> {
    catalog::validate_destination_draft(&pool, destination_id)
        .await
        .map(Json)
        .map_err(ApiError::draft_validation)
}

async fn create_destination(
    Extension(pool): Extension<PgPool>,
    Json(request): Json<CreateDestinationRequest>,
) -> Result<(StatusCode, Json<DestinationRecord>), ApiError> {
    validate_friendly_name(&request.friendly_name)?;
    validate_destination_profile(&request.profile).map_err(ApiError::invalid_media_profile)?;
    let configuration =
        serde_json::to_value(&request.profile).map_err(ApiError::serialize_profile)?;

    let destination = catalog::create_destination(
        &pool,
        request.friendly_name.trim(),
        request.profile.adapter_id(),
        configuration,
    )
    .await
    .map_err(|error| ApiError::database("create destination", error))?;

    Ok((StatusCode::CREATED, Json(destination)))
}

async fn update_destination(
    Path(destination_id): Path<Uuid>,
    Extension(pool): Extension<PgPool>,
    Json(request): Json<CreateDestinationRequest>,
) -> Result<Json<DestinationRecord>, ApiError> {
    validate_friendly_name(&request.friendly_name)?;
    validate_destination_profile(&request.profile).map_err(ApiError::invalid_media_profile)?;
    let configuration =
        serde_json::to_value(&request.profile).map_err(ApiError::serialize_profile)?;

    catalog::update_destination(
        &pool,
        destination_id,
        request.friendly_name.trim(),
        request.profile.adapter_id(),
        configuration,
    )
    .await
    .map(Json)
    .map_err(ApiError::resource_update)
}

async fn plan_route(
    Json(request): Json<PlanRouteRequest>,
) -> Result<Json<PlanRouteResponse>, ApiError> {
    let estimate = estimate_pipeline_requirements(&request.source, &request.destination)
        .map_err(ApiError::invalid_media_profile)?;
    Ok(Json(PlanRouteResponse {
        media_path: estimate.media_path.as_str(),
        estimate: RouteResourceEstimate {
            capacity_units: estimate.requirements.capacity_units,
            ingress_payload_bits_per_second: estimate.ingress_payload_bits_per_second,
            egress_payload_bits_per_second: estimate.egress_payload_bits_per_second,
        },
    }))
}

async fn plan_route_assignment(
    headers: HeaderMap,
    Extension(pool): Extension<PgPool>,
    Json(request): Json<PlanRouteAssignmentRequest>,
) -> Result<Json<PlanRouteAssignmentResponse>, ApiError> {
    let idempotency_key = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Uuid::parse_str(value).ok())
        .filter(|key| !key.is_nil())
        .ok_or_else(|| {
            ApiError::invalid_request("Idempotency-Key header must be a non-nil UUID")
        })?;
    let planner = PgPipelinePlanner::new(pool);
    let decision = planner
        .plan_with_idempotency_key(
            idempotency_key,
            request.pipeline_id,
            RouteRevision {
                source_id: request.source_id,
                source_revision: request.source_revision,
                destination_id: request.destination_id,
                destination_revision: request.destination_revision,
            },
            chrono::Duration::seconds(MAX_PLANNING_HEARTBEAT_AGE_SECONDS),
        )
        .await
        .map_err(ApiError::pipeline_planning)?;
    let (state, estimate, assignment, rejections) = match decision {
        PipelinePlanningDecision::Planned(planned) => (
            "planned",
            planned.estimate,
            Some(planned.assignment),
            Vec::new(),
        ),
        PipelinePlanningDecision::Unavailable {
            estimate,
            rejections,
        } => ("unavailable", estimate, None, rejections),
    };
    Ok(Json(PlanRouteAssignmentResponse {
        state,
        media_path: estimate.media_path.as_str(),
        estimate: RouteResourceEstimate {
            capacity_units: estimate.requirements.capacity_units,
            ingress_payload_bits_per_second: estimate.ingress_payload_bits_per_second,
            egress_payload_bits_per_second: estimate.egress_payload_bits_per_second,
        },
        assignment,
        rejections,
    }))
}

async fn get_pipeline_assignment(
    Path(pipeline_id): Path<Uuid>,
    Extension(pool): Extension<PgPool>,
) -> Result<Json<AssignmentSnapshot>, ApiError> {
    PgAssignments::new(pool)
        .get_by_pipeline(pipeline_id)
        .await
        .map_err(ApiError::assignment)?
        .map(Json)
        .ok_or_else(|| ApiError {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message: "No assignment exists for the requested pipeline".to_owned(),
        })
}

async fn cancel_assignment(
    Path(assignment_id): Path<Uuid>,
    Extension(pool): Extension<PgPool>,
    Json(request): Json<CancelAssignmentRequest>,
) -> Result<Json<CancelAssignmentResponse>, ApiError> {
    if request.generation <= 0 {
        return Err(ApiError::invalid_request(
            "generation must be a positive assignment generation",
        ));
    }
    PgAssignments::new(pool)
        .cancel_unstarted(assignment_id, request.generation)
        .await
        .map_err(ApiError::assignment)?;
    Ok(Json(CancelAssignmentResponse {
        assignment_id,
        generation: request.generation,
        state: "cancelled",
    }))
}

async fn start_assignment(
    Path(assignment_id): Path<Uuid>,
    Extension(pool): Extension<PgPool>,
    Extension(dispatcher): Extension<Arc<WorkerCommandDispatcher>>,
    Json(request): Json<AssignmentGenerationRequest>,
) -> Result<Json<AssignmentSnapshot>, ApiError> {
    let assignments = PgAssignments::new(pool);
    let snapshot = assignments
        .begin_start(assignment_id, request.generation)
        .await
        .map_err(ApiError::assignment)?;
    if snapshot.observed_state == "running" {
        return Ok(Json(snapshot));
    }

    let command = start_command(&snapshot.assignment)?;
    let result = match dispatcher.dispatch(command).await {
        Ok(result) => result,
        Err(error) => {
            if matches!(
                error.code,
                "worker_session_unavailable" | "worker_session_busy" | "worker_command_too_large"
            ) {
                assignments
                    .reset_start(assignment_id, request.generation)
                    .await
                    .map_err(ApiError::assignment)?;
            }
            return Err(ApiError::worker_dispatch(error));
        }
    };

    match result {
        CommandDispatchResult::Applied | CommandDispatchResult::Duplicate => assignments
            .mark_started(assignment_id, request.generation)
            .await
            .map(Json)
            .map_err(ApiError::assignment),
        CommandDispatchResult::Stale { current_generation } => {
            assignments
                .reset_start(assignment_id, request.generation)
                .await
                .map_err(ApiError::assignment)?;
            Err(ApiError::worker_dispatch(PortError {
                code: "stale_worker_command",
                message: format!("worker has already accepted generation {current_generation}"),
            }))
        }
    }
}

async fn stop_assignment(
    Path(assignment_id): Path<Uuid>,
    Extension(pool): Extension<PgPool>,
    Extension(dispatcher): Extension<Arc<WorkerCommandDispatcher>>,
    Json(request): Json<AssignmentGenerationRequest>,
) -> Result<Json<AssignmentSnapshot>, ApiError> {
    let assignments = PgAssignments::new(pool);
    let snapshot = assignments
        .begin_stop(assignment_id, request.generation)
        .await
        .map_err(ApiError::assignment)?;
    if snapshot.observed_state == "stopped" {
        return Ok(Json(snapshot));
    }

    match dispatcher
        .dispatch(stop_command(&snapshot.assignment))
        .await
    {
        Ok(CommandDispatchResult::Applied | CommandDispatchResult::Duplicate) => assignments
            .mark_stopped(assignment_id, request.generation)
            .await
            .map(Json)
            .map_err(ApiError::assignment),
        Ok(CommandDispatchResult::Stale { current_generation }) => {
            Err(ApiError::worker_dispatch(PortError {
                code: "stale_worker_command",
                message: format!("worker has already accepted generation {current_generation}"),
            }))
        }
        Err(error) => Err(ApiError::worker_dispatch(error)),
    }
}

fn start_command(
    assignment: &PlannedAssignment,
) -> Result<WorkerCommandEnvelope<WorkerCommand>, ApiError> {
    let route = assignment.route.as_ref().ok_or_else(|| ApiError {
        status: StatusCode::CONFLICT,
        code: "assignment_conflict",
        message: "assignment has no pinned route configuration to start".to_owned(),
    })?;
    Ok(WorkerCommandEnvelope {
        protocol_version: WORKER_PROTOCOL_VERSION,
        command_id: assignment.id,
        worker_id: assignment.worker_id,
        pipeline_id: assignment.pipeline_id,
        assignment_id: assignment.id,
        generation: assignment.generation,
        payload: WorkerCommand::Start {
            specification: PipelineSpecification {
                pipeline_id: assignment.pipeline_id,
                assignment_generation: assignment.generation,
                requirements: assignment.requirements.clone(),
                configuration: route.configuration.clone(),
            },
        },
    })
}

fn stop_command(assignment: &PlannedAssignment) -> WorkerCommandEnvelope<WorkerCommand> {
    let command_name = format!("stop:{}", assignment.generation);
    WorkerCommandEnvelope {
        protocol_version: WORKER_PROTOCOL_VERSION,
        command_id: Uuid::new_v5(&assignment.id, command_name.as_bytes()),
        worker_id: assignment.worker_id,
        pipeline_id: assignment.pipeline_id,
        assignment_id: assignment.id,
        generation: assignment.generation,
        payload: WorkerCommand::Stop,
    }
}

fn validate_friendly_name(value: &str) -> Result<(), ApiError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 120 {
        return Err(ApiError::invalid_request(
            "friendlyName must contain 1 to 120 non-whitespace characters",
        ));
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorResponse {
    code: &'static str,
    message: String,
}

struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code: "invalid_request",
            message: message.into(),
        }
    }

    fn invalid_media_profile(error: MediaProfileError) -> Self {
        Self::invalid_request(error.to_string())
    }

    fn draft_validation(error: DraftValidationError) -> Self {
        match error {
            DraftValidationError::NotFound => Self {
                status: StatusCode::NOT_FOUND,
                code: "not_found",
                message: "The requested source or destination does not exist".to_owned(),
            },
            DraftValidationError::NotDraft => Self {
                status: StatusCode::CONFLICT,
                code: "invalid_lifecycle",
                message: "Only draft resources can be validated".to_owned(),
            },
            DraftValidationError::Database(error) => Self::database("validate draft", error),
        }
    }

    fn resource_update(error: ResourceUpdateError) -> Self {
        match error {
            ResourceUpdateError::NotFound => Self {
                status: StatusCode::NOT_FOUND,
                code: "not_found",
                message: "The requested source or destination does not exist".to_owned(),
            },
            ResourceUpdateError::Enabled => Self {
                status: StatusCode::CONFLICT,
                code: "resource_enabled",
                message: "Enabled sources and destinations cannot be edited".to_owned(),
            },
            ResourceUpdateError::Database(error) => Self::database("update resource", error),
        }
    }

    fn serialize_profile(error: serde_json::Error) -> Self {
        tracing::error!(%error, "media profile serialization failed");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal_error",
            message: "Media profile could not be stored".to_owned(),
        }
    }

    fn database(operation: &'static str, error: impl std::fmt::Display) -> Self {
        tracing::error!(%error, operation, "catalog database operation failed");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "database_error",
            message: "Catalog database operation failed".to_owned(),
        }
    }

    fn pipeline_planning(error: arc_application::PortError) -> Self {
        let (status, code) = match error.code {
            "source_not_found" | "destination_not_found" => (StatusCode::NOT_FOUND, "not_found"),
            "stale_route_revision"
            | "source_not_validated"
            | "destination_not_validated"
            | "assignment_conflict"
            | "reservation_conflict"
            | "idempotency_key_reused" => (StatusCode::CONFLICT, error.code),
            "invalid_route_revision" | "invalid_media_profile" | "invalid_reservation" => {
                (StatusCode::UNPROCESSABLE_ENTITY, error.code)
            }
            "database_error" => return Self::database("plan route assignment", error.message),
            _ => {
                tracing::error!(
                    code = error.code,
                    message = %error.message,
                    "route assignment planning failed"
                );
                return Self {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    code: "internal_error",
                    message: "Route assignment planning failed".to_owned(),
                };
            }
        };
        Self {
            status,
            code,
            message: error.message,
        }
    }

    fn assignment(error: arc_application::PortError) -> Self {
        match error.code {
            "assignment_not_found" => Self {
                status: StatusCode::NOT_FOUND,
                code: "not_found",
                message: error.message,
            },
            "assignment_conflict" => Self {
                status: StatusCode::CONFLICT,
                code: error.code,
                message: error.message,
            },
            "invalid_pipeline_id" | "invalid_assignment" | "invalid_assignment_id" => Self {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                code: error.code,
                message: error.message,
            },
            "database_error" => Self::database("read or update assignment", error.message),
            _ => {
                tracing::error!(
                    code = error.code,
                    message = %error.message,
                    "assignment operation failed"
                );
                Self {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    code: "internal_error",
                    message: "Assignment operation failed".to_owned(),
                }
            }
        }
    }

    fn worker_dispatch(error: PortError) -> Self {
        match error.code {
            "stale_worker_command" | "assignment_generation_conflict" => Self {
                status: StatusCode::CONFLICT,
                code: error.code,
                message: error.message,
            },
            "worker_session_unavailable" | "worker_session_busy" | "worker_session_ended" => Self {
                status: StatusCode::SERVICE_UNAVAILABLE,
                code: error.code,
                message: error.message,
            },
            "worker_command_ack_timeout" => Self {
                status: StatusCode::GATEWAY_TIMEOUT,
                code: error.code,
                message: error.message,
            },
            "database_error" => Self::database("dispatch worker command", error.message),
            _ => {
                tracing::error!(
                    code = error.code,
                    message = %error.message,
                    "worker command dispatch failed"
                );
                Self {
                    status: StatusCode::BAD_GATEWAY,
                    code: "worker_dispatch_failed",
                    message: "The worker command could not be completed".to_owned(),
                }
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorResponse {
                code: self.code,
                message: self.message,
            }),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeSet, HashMap},
        env,
        net::{IpAddr, SocketAddr},
        sync::{Arc, Mutex},
    };

    use arc_application::{
        AssignmentGenerationStore, CommandFenceDecision, PortError, PortResult,
        WorkerAuthenticator, WorkerCommandIdentity, WorkerRegistry,
    };
    use arc_domain::{
        Architecture, DestinationMediaProfile, Platform, SourceMediaProfile, Worker,
        WorkerCapabilities, WorkerCapacity, WorkerState,
    };
    use arc_infrastructure::{
        assignments::PgAssignments,
        catalog,
        postgres::{connect_postgres, migrate},
        worker_fencing::PgAssignmentGenerationStore,
        worker_registry::PgWorkerRegistry,
    };
    use arc_worker::CommandDispatchResult;
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
    };
    use chrono::Utc;
    use futures_util::{SinkExt, StreamExt};
    use serde_json::Value;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        task::JoinHandle,
    };
    use tokio_tungstenite::{
        connect_async,
        tungstenite::{client::IntoClientRequest, http::HeaderValue, Message as ClientMessage},
    };
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::{
        router, router_with_database, router_with_database_and_worker_dispatcher,
        router_with_worker_authenticator, router_with_worker_dispatcher, WorkerCommandDispatcher,
    };

    struct TestWorkerAuthenticator {
        worker_id: Uuid,
        token: &'static str,
        available: bool,
    }

    #[async_trait::async_trait]
    impl WorkerAuthenticator for TestWorkerAuthenticator {
        async fn authenticate(&self, worker_id: Uuid, bearer_token: &str) -> PortResult<()> {
            if !self.available {
                return Err(PortError {
                    code: "worker_authentication_unavailable",
                    message: "provider detail".to_owned(),
                });
            }
            if worker_id == self.worker_id && bearer_token == self.token {
                Ok(())
            } else {
                Err(PortError {
                    code: "worker_authentication_failed",
                    message: "credential detail".to_owned(),
                })
            }
        }
    }

    #[derive(Default)]
    struct TestAssignmentGenerationStore {
        pipelines: Mutex<HashMap<Uuid, TestPipelineFence>>,
    }

    #[derive(Default)]
    struct TestPipelineFence {
        generation: i64,
        worker_id: Uuid,
        assignment_id: Uuid,
        commands: HashMap<Uuid, (WorkerCommandIdentity, bool)>,
    }

    #[async_trait::async_trait]
    impl AssignmentGenerationStore for TestAssignmentGenerationStore {
        async fn begin_command(
            &self,
            command: WorkerCommandIdentity,
        ) -> PortResult<CommandFenceDecision> {
            let mut pipelines = self.pipelines.lock().unwrap();
            let fence = pipelines.entry(command.pipeline_id).or_default();
            if command.generation < fence.generation {
                return Ok(CommandFenceDecision::Stale {
                    current_generation: fence.generation,
                });
            }
            if command.generation == fence.generation
                && fence.generation > 0
                && (command.worker_id != fence.worker_id
                    || command.assignment_id != fence.assignment_id)
            {
                return Err(PortError {
                    code: "assignment_generation_conflict",
                    message: "one generation cannot identify multiple assignments".to_owned(),
                });
            }
            if command.generation > fence.generation {
                fence.generation = command.generation;
                fence.worker_id = command.worker_id;
                fence.assignment_id = command.assignment_id;
            }
            match fence.commands.get(&command.command_id) {
                Some((recorded, _)) if *recorded != command => Err(PortError {
                    code: "command_identity_conflict",
                    message: "command ID was already used for a different identity".to_owned(),
                }),
                Some((_, true)) => Ok(CommandFenceDecision::Duplicate),
                Some((_, false)) => Ok(CommandFenceDecision::Process),
                None => {
                    fence.commands.insert(command.command_id, (command, false));
                    Ok(CommandFenceDecision::Process)
                }
            }
        }

        async fn complete_command(&self, command_id: Uuid) -> PortResult<()> {
            for fence in self.pipelines.lock().unwrap().values_mut() {
                if let Some((_, completed)) = fence.commands.get_mut(&command_id) {
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

    fn worker_websocket_request(
        address: SocketAddr,
        worker_id: Uuid,
        token: &str,
        forwarded_proto: &str,
    ) -> tokio_tungstenite::tungstenite::http::Request<()> {
        let url = format!("ws://{address}/api/v1/workers/control");
        let mut request = url
            .into_client_request()
            .expect("websocket client request builds");
        request.headers_mut().insert(
            "x-forwarded-proto",
            HeaderValue::from_str(forwarded_proto).expect("forwarded protocol is a header value"),
        );
        request.headers_mut().insert(
            "x-worker-id",
            HeaderValue::from_str(&worker_id.to_string()).expect("worker ID is a header value"),
        );
        request.headers_mut().insert(
            "authorization",
            HeaderValue::from_str(&format!("Bearer {token}"))
                .expect("bearer credential is a header value"),
        );
        request
    }

    async fn start_worker_control_server(
        worker_id: Uuid,
        available: bool,
    ) -> (SocketAddr, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener binds");
        let address = listener.local_addr().expect("listener has an address");
        let app = router_with_worker_authenticator(Arc::new(TestWorkerAuthenticator {
            worker_id,
            token: "worker-secret",
            available,
        }));
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("worker control server runs");
        });
        (address, server)
    }

    async fn start_worker_dispatch_server(
        worker_id: Uuid,
    ) -> (
        SocketAddr,
        JoinHandle<()>,
        Arc<WorkerCommandDispatcher>,
        Arc<TestAssignmentGenerationStore>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener binds");
        let address = listener.local_addr().expect("listener has an address");
        let generation_store = Arc::new(TestAssignmentGenerationStore::default());
        let dispatcher = Arc::new(WorkerCommandDispatcher::new(generation_store.clone()));
        let app = router_with_worker_dispatcher(
            Arc::new(TestWorkerAuthenticator {
                worker_id,
                token: "worker-secret",
                available: true,
            }),
            dispatcher.clone(),
        );
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("worker dispatch server runs");
        });
        (address, server, dispatcher, generation_store)
    }

    async fn assert_worker_connection_rejected(
        address: SocketAddr,
        worker_id: Uuid,
        token: &str,
        forwarded_proto: &str,
        expected_status: StatusCode,
    ) {
        let error = connect_async(worker_websocket_request(
            address,
            worker_id,
            token,
            forwarded_proto,
        ))
        .await
        .expect_err("worker connection should be rejected");
        let tokio_tungstenite::tungstenite::Error::Http(response) = error else {
            panic!("expected an HTTP rejection, received {error}");
        };
        assert_eq!(response.status(), expected_status);
    }

    #[tokio::test]
    async fn worker_websocket_authenticates_tls_identity_and_protocol_hello() {
        let worker_id = Uuid::new_v4();
        let (address, server) = start_worker_control_server(worker_id, true).await;
        let (mut socket, response) = connect_async(worker_websocket_request(
            address,
            worker_id,
            "worker-secret",
            "https",
        ))
        .await
        .expect("authenticated worker upgrades");
        assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
        let hello = arc_shared::WorkerSessionHello {
            kind: arc_shared::WorkerSessionMessageKind::Hello,
            protocol_version: arc_shared::WORKER_PROTOCOL_VERSION,
            worker_id,
        };
        socket
            .send(ClientMessage::Text(
                serde_json::to_string(&hello)
                    .expect("worker hello serializes")
                    .into(),
            ))
            .await
            .expect("worker hello sends");
        let welcome = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
            .await
            .expect("server responds to hello")
            .expect("server keeps session open")
            .expect("server sends welcome");
        let ClientMessage::Text(welcome) = welcome else {
            panic!("expected a text welcome message");
        };
        let welcome: arc_shared::WorkerSessionWelcome =
            serde_json::from_str(welcome.as_str()).expect("server welcome is valid JSON");
        assert_eq!(welcome.kind, arc_shared::WorkerSessionMessageKind::Welcome);
        assert_eq!(welcome.worker_id, worker_id);
        socket.close(None).await.expect("worker session closes");
        server.abort();
    }

    #[tokio::test]
    async fn worker_websocket_dispatch_completes_only_after_correlated_acknowledgement() {
        let worker_id = Uuid::new_v4();
        let pipeline_id = Uuid::new_v4();
        let assignment_id = Uuid::new_v4();
        let command_id = Uuid::new_v4();
        let (address, server, dispatcher, generation_store) =
            start_worker_dispatch_server(worker_id).await;
        let (mut socket, _) = connect_async(worker_websocket_request(
            address,
            worker_id,
            "worker-secret",
            "https",
        ))
        .await
        .expect("authenticated worker upgrades");
        let hello = arc_shared::WorkerSessionHello {
            kind: arc_shared::WorkerSessionMessageKind::Hello,
            protocol_version: arc_shared::WORKER_PROTOCOL_VERSION,
            worker_id,
        };
        socket
            .send(ClientMessage::Text(
                serde_json::to_string(&hello)
                    .expect("worker hello serializes")
                    .into(),
            ))
            .await
            .expect("worker hello sends");
        let welcome = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
            .await
            .expect("server responds to hello")
            .expect("server keeps session open")
            .expect("server sends welcome");
        assert!(matches!(welcome, ClientMessage::Text(_)));
        socket
            .send(ClientMessage::Ping(vec![1].into()))
            .await
            .expect("worker readiness ping sends");
        assert!(matches!(
            tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
                .await
                .expect("worker session is active")
                .expect("server keeps session open")
                .expect("server responds to ping"),
            ClientMessage::Pong(_)
        ));

        let command = arc_shared::WorkerCommandEnvelope {
            protocol_version: arc_shared::WORKER_PROTOCOL_VERSION,
            command_id,
            worker_id,
            pipeline_id,
            assignment_id,
            generation: 1,
            payload: serde_json::json!({ "kind": "start" }),
        };
        let dispatch = {
            let dispatcher = dispatcher.clone();
            tokio::spawn(async move { dispatcher.dispatch(command).await })
        };
        let outgoing = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
            .await
            .expect("dispatcher sends command")
            .expect("server keeps session open")
            .expect("server sends command");
        let ClientMessage::Text(outgoing) = outgoing else {
            panic!("expected a worker command");
        };
        let outgoing: arc_shared::WorkerCommandEnvelope<Value> =
            serde_json::from_str(outgoing.as_str()).expect("worker command is valid JSON");
        assert_eq!(outgoing.command_id, command_id);
        assert_eq!(outgoing.assignment_id, assignment_id);
        assert_eq!(outgoing.generation, 1);
        assert!(!dispatch.is_finished());

        let acknowledgement = arc_worker::receive_worker_command(
            generation_store.as_ref(),
            worker_id,
            outgoing,
            |_| async { Ok(()) },
        )
        .await
        .expect("worker handler completes command");
        socket
            .send(ClientMessage::Text(
                serde_json::to_string(&acknowledgement)
                    .expect("command acknowledgement serializes")
                    .into(),
            ))
            .await
            .expect("worker acknowledgement sends");
        assert_eq!(
            dispatch
                .await
                .expect("dispatch task joins")
                .expect("command applies"),
            CommandDispatchResult::Applied
        );

        let duplicate = arc_shared::WorkerCommandEnvelope {
            protocol_version: arc_shared::WORKER_PROTOCOL_VERSION,
            command_id,
            worker_id,
            pipeline_id,
            assignment_id,
            generation: 1,
            payload: serde_json::json!({ "kind": "start" }),
        };
        assert_eq!(
            dispatcher
                .dispatch(duplicate)
                .await
                .expect("completed retry is recognized"),
            CommandDispatchResult::Duplicate
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), socket.next())
                .await
                .is_err()
        );

        socket.close(None).await.expect("worker session closes");
        server.abort();
    }

    #[tokio::test]
    async fn stale_worker_acknowledgement_rejects_dispatch_without_closing_session() {
        let worker_id = Uuid::new_v4();
        let pipeline_id = Uuid::new_v4();
        let assignment_id = Uuid::new_v4();
        let command_id = Uuid::new_v4();
        let (address, server, dispatcher, generation_store) =
            start_worker_dispatch_server(worker_id).await;
        let (mut socket, _) = connect_async(worker_websocket_request(
            address,
            worker_id,
            "worker-secret",
            "https",
        ))
        .await
        .expect("authenticated worker upgrades");
        let hello = arc_shared::WorkerSessionHello {
            kind: arc_shared::WorkerSessionMessageKind::Hello,
            protocol_version: arc_shared::WORKER_PROTOCOL_VERSION,
            worker_id,
        };
        socket
            .send(ClientMessage::Text(
                serde_json::to_string(&hello)
                    .expect("worker hello serializes")
                    .into(),
            ))
            .await
            .expect("worker hello sends");
        let _welcome = socket
            .next()
            .await
            .expect("server keeps session open")
            .expect("server sends welcome");
        socket
            .send(ClientMessage::Ping(vec![1].into()))
            .await
            .expect("worker readiness ping sends");
        let _pong = socket
            .next()
            .await
            .expect("worker session becomes active")
            .expect("server responds to ping");

        let command = arc_shared::WorkerCommandEnvelope {
            protocol_version: arc_shared::WORKER_PROTOCOL_VERSION,
            command_id,
            worker_id,
            pipeline_id,
            assignment_id,
            generation: 2,
            payload: serde_json::json!({ "kind": "start" }),
        };
        let dispatch = {
            let dispatcher = dispatcher.clone();
            tokio::spawn(async move { dispatcher.dispatch(command).await })
        };
        let outgoing = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
            .await
            .expect("dispatcher sends command")
            .expect("server keeps session open")
            .expect("server sends command");
        let ClientMessage::Text(outgoing) = outgoing else {
            panic!("expected a worker command");
        };
        let outgoing: arc_shared::WorkerCommandEnvelope<Value> =
            serde_json::from_str(outgoing.as_str()).expect("worker command is valid JSON");

        assert_eq!(
            generation_store
                .begin_command(WorkerCommandIdentity {
                    command_id: Uuid::new_v4(),
                    worker_id,
                    pipeline_id,
                    assignment_id: Uuid::new_v4(),
                    generation: 3,
                })
                .await
                .expect("new generation advances"),
            CommandFenceDecision::Process
        );
        let acknowledgement = arc_worker::receive_worker_command(
            generation_store.as_ref(),
            worker_id,
            outgoing,
            |_| async { panic!("stale command must not reach its handler") },
        )
        .await
        .expect("worker rejects stale command");
        assert_eq!(
            acknowledgement.outcome,
            arc_shared::WorkerCommandOutcome::Stale
        );
        assert_eq!(acknowledgement.current_generation, Some(3));
        socket
            .send(ClientMessage::Text(
                serde_json::to_string(&acknowledgement)
                    .expect("stale acknowledgement serializes")
                    .into(),
            ))
            .await
            .expect("worker stale acknowledgement sends");
        let error = dispatch
            .await
            .expect("dispatch task joins")
            .expect_err("stale generation must reject dispatch");
        assert_eq!(error.code, "stale_worker_command");

        socket
            .send(ClientMessage::Ping(vec![2].into()))
            .await
            .expect("session remains open after stale acknowledgement");
        assert!(matches!(
            socket.next().await.expect("session stays active").unwrap(),
            ClientMessage::Pong(_)
        ));
        socket.close(None).await.expect("worker session closes");
        server.abort();
    }

    #[tokio::test]
    async fn replacement_worker_session_closes_old_connection_and_keeps_dispatch_available() {
        let worker_id = Uuid::new_v4();
        let (address, server, dispatcher, generation_store) =
            start_worker_dispatch_server(worker_id).await;
        let (mut old_socket, _) = connect_async(worker_websocket_request(
            address,
            worker_id,
            "worker-secret",
            "https",
        ))
        .await
        .expect("first worker session upgrades");
        let hello = arc_shared::WorkerSessionHello {
            kind: arc_shared::WorkerSessionMessageKind::Hello,
            protocol_version: arc_shared::WORKER_PROTOCOL_VERSION,
            worker_id,
        };
        old_socket
            .send(ClientMessage::Text(
                serde_json::to_string(&hello)
                    .expect("worker hello serializes")
                    .into(),
            ))
            .await
            .expect("first worker hello sends");
        let _welcome = old_socket
            .next()
            .await
            .expect("server keeps first session open")
            .expect("first session receives welcome");
        old_socket
            .send(ClientMessage::Ping(vec![1].into()))
            .await
            .expect("first session readiness ping sends");
        assert!(matches!(
            old_socket
                .next()
                .await
                .expect("first session stays open")
                .unwrap(),
            ClientMessage::Pong(_)
        ));

        let (mut new_socket, _) = connect_async(worker_websocket_request(
            address,
            worker_id,
            "worker-secret",
            "https",
        ))
        .await
        .expect("replacement worker session upgrades");
        new_socket
            .send(ClientMessage::Text(
                serde_json::to_string(&hello)
                    .expect("worker hello serializes")
                    .into(),
            ))
            .await
            .expect("replacement worker hello sends");
        let _welcome = new_socket
            .next()
            .await
            .expect("server keeps replacement session open")
            .expect("replacement session receives welcome");
        new_socket
            .send(ClientMessage::Ping(vec![2].into()))
            .await
            .expect("replacement readiness ping sends");
        assert!(matches!(
            new_socket
                .next()
                .await
                .expect("replacement session stays open")
                .unwrap(),
            ClientMessage::Pong(_)
        ));

        assert!(matches!(
            tokio::time::timeout(std::time::Duration::from_secs(2), old_socket.next())
                .await
                .expect("old session is replaced")
                .expect("old session receives close")
                .expect("old session closes cleanly"),
            ClientMessage::Close(_)
        ));

        let command = arc_shared::WorkerCommandEnvelope {
            protocol_version: arc_shared::WORKER_PROTOCOL_VERSION,
            command_id: Uuid::new_v4(),
            worker_id,
            pipeline_id: Uuid::new_v4(),
            assignment_id: Uuid::new_v4(),
            generation: 1,
            payload: serde_json::json!({ "kind": "start" }),
        };
        let command_id = command.command_id;
        let dispatch = {
            let dispatcher = dispatcher.clone();
            tokio::spawn(async move { dispatcher.dispatch(command).await })
        };
        let outgoing = tokio::time::timeout(std::time::Duration::from_secs(2), new_socket.next())
            .await
            .expect("dispatcher targets replacement session")
            .expect("replacement session remains open")
            .expect("replacement session receives command");
        let ClientMessage::Text(outgoing) = outgoing else {
            panic!("expected a worker command");
        };
        let outgoing: arc_shared::WorkerCommandEnvelope<Value> =
            serde_json::from_str(outgoing.as_str()).expect("worker command is valid JSON");
        assert_eq!(outgoing.command_id, command_id);

        let acknowledgement = arc_worker::receive_worker_command(
            generation_store.as_ref(),
            worker_id,
            outgoing,
            |_| async { Ok(()) },
        )
        .await
        .expect("worker handler completes command");
        new_socket
            .send(ClientMessage::Text(
                serde_json::to_string(&acknowledgement)
                    .expect("command acknowledgement serializes")
                    .into(),
            ))
            .await
            .expect("replacement acknowledges command");
        assert_eq!(
            dispatch
                .await
                .expect("dispatch task joins")
                .expect("command applies"),
            CommandDispatchResult::Applied
        );

        new_socket
            .close(None)
            .await
            .expect("replacement session closes");
        server.abort();
    }

    #[tokio::test]
    async fn worker_websocket_rejects_bad_credentials_cleartext_and_provider_failures() {
        let worker_id = Uuid::new_v4();
        let (address, server) = start_worker_control_server(worker_id, true).await;
        assert_worker_connection_rejected(
            address,
            worker_id,
            "wrong-secret",
            "https",
            StatusCode::UNAUTHORIZED,
        )
        .await;
        assert_worker_connection_rejected(
            address,
            Uuid::nil(),
            "worker-secret",
            "https",
            StatusCode::UNAUTHORIZED,
        )
        .await;
        assert_worker_connection_rejected(
            address,
            worker_id,
            "worker-secret",
            "http",
            StatusCode::UPGRADE_REQUIRED,
        )
        .await;
        server.abort();

        let (address, server) = start_worker_control_server(worker_id, false).await;
        assert_worker_connection_rejected(
            address,
            worker_id,
            "worker-secret",
            "https",
            StatusCode::SERVICE_UNAVAILABLE,
        )
        .await;
        server.abort();
    }

    #[tokio::test]
    async fn worker_websocket_closes_sessions_with_an_invalid_hello() {
        let worker_id = Uuid::new_v4();
        let (address, server) = start_worker_control_server(worker_id, true).await;
        let (mut socket, _) = connect_async(worker_websocket_request(
            address,
            worker_id,
            "worker-secret",
            "https",
        ))
        .await
        .expect("authenticated worker upgrades");
        let hello = arc_shared::WorkerSessionHello {
            kind: arc_shared::WorkerSessionMessageKind::Hello,
            protocol_version: arc_shared::WORKER_PROTOCOL_VERSION + 1,
            worker_id,
        };
        socket
            .send(ClientMessage::Text(
                serde_json::to_string(&hello)
                    .expect("worker hello serializes")
                    .into(),
            ))
            .await
            .expect("worker hello sends");
        let close = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
            .await
            .expect("server rejects invalid hello")
            .expect("server sends a close frame")
            .expect("server close frame is valid");
        let ClientMessage::Close(Some(frame)) = close else {
            panic!("expected a policy close frame");
        };
        assert_eq!(
            frame.code,
            tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Policy
        );
        server.abort();
    }

    #[tokio::test]
    async fn liveness_route_returns_alive() {
        let response = router()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/health/live")
                    .body(Body::empty())
                    .expect("request builds"),
            )
            .await
            .expect("router handles request");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body reads");
        let value: Value = serde_json::from_slice(&body).expect("response is JSON");
        assert_eq!(value["status"], "alive");
    }

    #[tokio::test]
    async fn capability_route_returns_worker_collection() {
        let response = router()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/system/capabilities")
                    .body(Body::empty())
                    .expect("request builds"),
            )
            .await
            .expect("router handles request");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body reads");
        let value: Value = serde_json::from_slice(&body).expect("response is JSON");
        assert_eq!(value["apiVersion"], "1.0");
        assert_eq!(value["workers"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn route_planner_accepts_the_selected_rtp_to_srt_profile() {
        let response = router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/plan")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "source": {
                                "kind": "rtp_mp2t",
                                "bindAddress": "0.0.0.0",
                                "port": 5000
                            },
                            "destination": {
                                "kind": "srt_mpeg_ts",
                                "host": "127.0.0.1",
                                "port": 6000,
                                "mode": "caller"
                            }
                        })
                        .to_string(),
                    ))
                    .expect("request builds"),
            )
            .await
            .expect("router handles request");

        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body reads");
        assert_eq!(
            status,
            StatusCode::OK,
            "unexpected error response: {}",
            String::from_utf8_lossy(&body)
        );
        let value: Value = serde_json::from_slice(&body).expect("response is JSON");
        assert_eq!(value["mediaPath"], "rtp_mp2t_to_srt_mpeg_ts");
        assert_eq!(
            value["estimate"],
            serde_json::json!({
                "capacityUnits": 2,
                "ingressPayloadBitsPerSecond": null,
                "egressPayloadBitsPerSecond": null
            })
        );
    }
    #[tokio::test]
    #[ignore = "requires an isolated PostgreSQL database; set DATABASE_URL and run ignored tests"]
    async fn assignment_planning_api_pins_revisions() {
        let pool = connect_postgres(&env::var("DATABASE_URL").expect("DATABASE_URL is set"))
            .await
            .expect("database connects");
        migrate(&pool).await.expect("migrations succeed");
        let source_profile = SourceMediaProfile::RtpL16 {
            bind_address: IpAddr::from([0, 0, 0, 0]),
            port: 50_000,
            payload_type: 96,
        };
        let destination_profile = DestinationMediaProfile::RtpL16 {
            address: IpAddr::from([127, 0, 0, 1]),
            port: 50_002,
            payload_type: 96,
        };
        let source = catalog::create_source(
            &pool,
            "API planning source",
            source_profile.adapter_id(),
            serde_json::to_value(source_profile).unwrap(),
        )
        .await
        .unwrap();
        let source = catalog::validate_source_draft(&pool, source.id)
            .await
            .unwrap();
        let destination = catalog::create_destination(
            &pool,
            "API planning destination",
            destination_profile.adapter_id(),
            serde_json::to_value(destination_profile).unwrap(),
        )
        .await
        .unwrap();
        let destination = catalog::validate_destination_draft(&pool, destination.id)
            .await
            .unwrap();

        let worker_id = Uuid::new_v4();
        let mut worker = Worker {
            id: worker_id,
            stable_name: format!("api-planning-{worker_id}"),
            version: "0.1.0".to_owned(),
            state: WorkerState::Registering,
            capabilities: WorkerCapabilities {
                platform: Some(Platform::Linux),
                architecture: Some(Architecture::X86_64),
                source_adapters: BTreeSet::from(["rtp_l16".to_owned()]),
                destination_adapters: BTreeSet::from(["rtp_l16".to_owned()]),
                ..WorkerCapabilities::default()
            },
            capacity: WorkerCapacity {
                total_capacity_units: 4,
                assigned_capacity_units: 0,
                cpu_utilization_ratio: None,
                memory_utilization_ratio: None,
                ingress_bits_per_second: None,
                egress_bits_per_second: None,
            },
            last_heartbeat: Utc::now(),
        };
        let registry = PgWorkerRegistry::new(pool.clone());
        registry.register(worker.clone()).await.unwrap();
        worker.state = WorkerState::Accepting;
        registry.heartbeat(worker).await.unwrap();

        let pipeline_id = Uuid::new_v4();
        let idempotency_key = Uuid::new_v4();
        let request_body = serde_json::json!({
            "pipelineId": pipeline_id,
            "sourceId": source.id,
            "sourceRevision": source.current_revision,
            "destinationId": destination.id,
            "destinationRevision": destination.current_revision,
        });
        let response = router_with_database(pool.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/plan-assignment")
                    .header("content-type", "application/json")
                    .header("idempotency-key", idempotency_key.to_string())
                    .body(Body::from(request_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            status,
            StatusCode::OK,
            "unexpected error response: {}",
            String::from_utf8_lossy(&body)
        );
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["state"], "planned");
        assert_eq!(value["assignment"]["pipelineId"], pipeline_id.to_string());
        assert_eq!(value["assignment"]["workerId"], worker_id.to_string());
        assert_eq!(
            value["assignment"]["route"]["sourceRevision"],
            source.current_revision
        );
        assert_eq!(
            value["assignment"]["route"]["destinationRevision"],
            destination.current_revision
        );
        let replay_response = router_with_database(pool.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/plan-assignment")
                    .header("content-type", "application/json")
                    .header("idempotency-key", idempotency_key.to_string())
                    .body(Body::from(request_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(replay_response.status(), StatusCode::OK);
        let replay_body = to_bytes(replay_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let replay: Value = serde_json::from_slice(&replay_body).unwrap();
        assert_eq!(replay, value);

        let changed_request = serde_json::json!({
            "pipelineId": pipeline_id,
            "sourceId": source.id,
            "sourceRevision": source.current_revision + 1,
            "destinationId": destination.id,
            "destinationRevision": destination.current_revision,
        });
        let changed_response = router_with_database(pool.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/plan-assignment")
                    .header("content-type", "application/json")
                    .header("idempotency-key", idempotency_key.to_string())
                    .body(Body::from(changed_request.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(changed_response.status(), StatusCode::CONFLICT);
        let changed_body = to_bytes(changed_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let changed_error: Value = serde_json::from_slice(&changed_body).unwrap();
        assert_eq!(changed_error["code"], "idempotency_key_reused");

        let assignment_id = value["assignment"]["id"].as_str().unwrap();
        let generation = value["assignment"]["generation"].as_i64().unwrap();

        let response = router_with_database(pool.clone())
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/pipelines/{pipeline_id}/assignment"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let snapshot: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(snapshot["desiredState"], "planned");
        assert_eq!(snapshot["observedState"], "not_started");
        assert_eq!(
            snapshot["assignment"]["route"]["sourceRevision"],
            source.current_revision
        );

        let response = router_with_database(pool.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/assignments/{assignment_id}/cancel"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "generation": generation }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let cancelled: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(cancelled["state"], "cancelled");
        assert_eq!(cancelled["generation"], generation);
        let cancelled_source_state: String =
            sqlx::query_scalar("SELECT desired_state FROM sources WHERE id = $1")
                .bind(source.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        let cancelled_destination_state: String =
            sqlx::query_scalar("SELECT desired_state FROM destinations WHERE id = $1")
                .bind(destination.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(cancelled_source_state, "disabled");
        assert_eq!(cancelled_destination_state, "disabled");

        let response = router_with_database(pool.clone())
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/pipelines/{pipeline_id}/assignment"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let snapshot: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(snapshot["desiredState"], "cancelled");
        assert_eq!(snapshot["observedState"], "cancelled");

        let response = router_with_database(pool.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/assignments/{assignment_id}/cancel"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "generation": generation }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let error: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(error["code"], "assignment_conflict");

        let stale_request = serde_json::json!({
            "pipelineId": Uuid::new_v4(),
            "sourceId": source.id,
            "sourceRevision": source.current_revision + 1,
            "destinationId": destination.id,
            "destinationRevision": destination.current_revision,
        });
        let response = router_with_database(pool)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/plan-assignment")
                    .header("content-type", "application/json")
                    .header("idempotency-key", Uuid::new_v4().to_string())
                    .body(Body::from(stale_request.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["code"], "stale_route_revision");
    }

    #[tokio::test]
    #[ignore = "requires an isolated PostgreSQL database; set DATABASE_URL and run ignored tests"]
    async fn starting_assignment_dispatches_its_pinned_configuration() {
        let url = env::var("DATABASE_URL").expect("DATABASE_URL is set");
        let pool = connect_postgres(&url).await.expect("database connects");
        migrate(&pool).await.expect("database migrates");
        let worker_id = Uuid::new_v4();
        let mut worker = Worker {
            id: worker_id,
            stable_name: format!("api-start-{worker_id}"),
            version: "0.1.0".to_owned(),
            state: WorkerState::Registering,
            capabilities: WorkerCapabilities {
                platform: Some(Platform::Linux),
                architecture: Some(Architecture::X86_64),
                source_adapters: BTreeSet::from(["rtp_l16".to_owned()]),
                destination_adapters: BTreeSet::from(["rtp_l16".to_owned()]),
                ..WorkerCapabilities::default()
            },
            capacity: WorkerCapacity {
                total_capacity_units: 8,
                assigned_capacity_units: 0,
                cpu_utilization_ratio: None,
                memory_utilization_ratio: None,
                ingress_bits_per_second: None,
                egress_bits_per_second: None,
            },
            last_heartbeat: Utc::now(),
        };
        let registry = PgWorkerRegistry::new(pool.clone());
        registry
            .register(worker.clone())
            .await
            .expect("worker registers");
        worker.state = WorkerState::Accepting;
        registry
            .heartbeat(worker)
            .await
            .expect("worker becomes eligible");

        let address = IpAddr::from([127, 0, 0, 1]);
        let source_profile = SourceMediaProfile::RtpL16 {
            bind_address: address,
            port: 5100,
            payload_type: 96,
        };
        let destination_profile = DestinationMediaProfile::RtpL16 {
            address,
            port: 5102,
            payload_type: 96,
        };
        let source = catalog::create_source(
            &pool,
            "Start source",
            source_profile.adapter_id(),
            serde_json::to_value(&source_profile).expect("source profile serializes"),
        )
        .await
        .expect("source creates");
        let source = catalog::validate_source_draft(&pool, source.id)
            .await
            .expect("source validates");
        let destination = catalog::create_destination(
            &pool,
            "Start destination",
            destination_profile.adapter_id(),
            serde_json::to_value(&destination_profile).expect("destination profile serializes"),
        )
        .await
        .expect("destination creates");
        let destination = catalog::validate_destination_draft(&pool, destination.id)
            .await
            .expect("destination validates");
        let second_destination_profile = DestinationMediaProfile::RtpL16 {
            address,
            port: 5104,
            payload_type: 96,
        };
        let second_destination = catalog::create_destination(
            &pool,
            "Second start destination",
            second_destination_profile.adapter_id(),
            serde_json::to_value(&second_destination_profile)
                .expect("second destination profile serializes"),
        )
        .await
        .expect("second destination creates");
        let second_destination = catalog::validate_destination_draft(&pool, second_destination.id)
            .await
            .expect("second destination validates");

        let pipeline_id = Uuid::new_v4();
        let plan_request = serde_json::json!({
            "pipelineId": pipeline_id,
            "sourceId": source.id,
            "sourceRevision": source.current_revision,
            "destinationId": destination.id,
            "destinationRevision": destination.current_revision,
        });
        let plan_response = router_with_database(pool.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/plan-assignment")
                    .header("content-type", "application/json")
                    .header("idempotency-key", Uuid::new_v4().to_string())
                    .body(Body::from(plan_request.to_string()))
                    .expect("planning request builds"),
            )
            .await
            .expect("planning request completes");
        assert_eq!(plan_response.status(), StatusCode::OK);
        let plan_body = to_bytes(plan_response.into_body(), usize::MAX)
            .await
            .expect("planning response reads");
        let plan: Value =
            serde_json::from_slice(&plan_body).expect("planning response is valid JSON");
        let assigned_worker_id = Uuid::parse_str(
            plan["assignment"]["workerId"]
                .as_str()
                .expect("assignment worker ID is returned"),
        )
        .expect("assignment worker ID is valid");
        let assignment_id = Uuid::parse_str(
            plan["assignment"]["id"]
                .as_str()
                .expect("assignment ID is returned"),
        )
        .expect("assignment ID is valid");
        let generation = plan["assignment"]["generation"]
            .as_i64()
            .expect("assignment generation is returned");

        let dispatcher = Arc::new(WorkerCommandDispatcher::new(Arc::new(
            PgAssignmentGenerationStore::new(pool.clone()),
        )));
        let app = router_with_database_and_worker_dispatcher(
            pool.clone(),
            Arc::new(TestWorkerAuthenticator {
                worker_id: assigned_worker_id,
                token: "worker-secret",
                available: true,
            }),
            dispatcher,
        );
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("API listener binds");
        let address = listener.local_addr().expect("API listener has an address");
        let server_app = app.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, server_app)
                .await
                .expect("API server runs");
        });

        let unavailable_start = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/assignments/{assignment_id}/start"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "generation": generation }).to_string(),
                    ))
                    .expect("unavailable start request builds"),
            )
            .await
            .expect("unavailable start request completes");
        let unavailable_status = unavailable_start.status();
        let unavailable_body = to_bytes(unavailable_start.into_body(), usize::MAX)
            .await
            .expect("unavailable start response reads");
        assert_eq!(
            unavailable_status,
            StatusCode::SERVICE_UNAVAILABLE,
            "unexpected unavailable-start response: {}",
            String::from_utf8_lossy(&unavailable_body)
        );
        let reset = PgAssignments::new(pool.clone())
            .get_by_id(assignment_id)
            .await
            .expect("assignment state reads")
            .expect("assignment remains persisted");
        assert_eq!(reset.desired_state, "planned");
        assert_eq!(reset.observed_state, "not_started");
        let reset_source_state: String =
            sqlx::query_scalar("SELECT desired_state FROM sources WHERE id = $1")
                .bind(source.id)
                .fetch_one(&pool)
                .await
                .expect("reset source state reads");
        let reset_destination_state: String =
            sqlx::query_scalar("SELECT desired_state FROM destinations WHERE id = $1")
                .bind(destination.id)
                .fetch_one(&pool)
                .await
                .expect("reset destination state reads");
        assert_eq!(reset_source_state, "disabled");
        assert_eq!(reset_destination_state, "disabled");

        let (mut socket, _) = connect_async(worker_websocket_request(
            address,
            assigned_worker_id,
            "worker-secret",
            "https",
        ))
        .await
        .expect("worker control connection upgrades");
        let hello = arc_shared::WorkerSessionHello {
            kind: arc_shared::WorkerSessionMessageKind::Hello,
            protocol_version: arc_shared::WORKER_PROTOCOL_VERSION,
            worker_id: assigned_worker_id,
        };
        socket
            .send(ClientMessage::Text(
                serde_json::to_string(&hello)
                    .expect("worker hello serializes")
                    .into(),
            ))
            .await
            .expect("worker hello sends");
        let welcome = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
            .await
            .expect("worker welcome arrives")
            .expect("control session remains open")
            .expect("worker welcome is received");
        assert!(matches!(welcome, ClientMessage::Text(_)));

        let start_request = Request::builder()
            .method("POST")
            .uri(format!("/api/v1/assignments/{assignment_id}/start"))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "generation": generation }).to_string(),
            ))
            .expect("start request builds");
        let start_app = app.clone();
        let start_task = tokio::spawn(async move { start_app.oneshot(start_request).await });
        let message = match tokio::time::timeout(
            std::time::Duration::from_secs(2),
            socket.next(),
        )
        .await
        {
            Ok(Some(Ok(message))) => message,
            Ok(Some(Err(error))) => panic!("worker control message fails: {error}"),
            Ok(None) => panic!("worker control session closes before Start"),
            Err(_) if start_task.is_finished() => {
                let response = start_task
                    .await
                    .expect("start request task joins")
                    .expect("start request completes");
                let status = response.status();
                let body = to_bytes(response.into_body(), usize::MAX)
                    .await
                    .expect("failed Start response reads");
                panic!(
                    "Start endpoint returned {status} without dispatching a command: {}",
                    String::from_utf8_lossy(&body)
                );
            }
            Err(_) => panic!("worker does not receive Start command before timeout"),
        };
        let ClientMessage::Text(message) = message else {
            panic!("expected a text Start command");
        };
        let command: Value =
            serde_json::from_str(message.as_str()).expect("Start command is valid JSON");
        assert_eq!(command["commandId"], assignment_id.to_string());
        assert_eq!(command["workerId"], assigned_worker_id.to_string());
        assert_eq!(command["pipelineId"], pipeline_id.to_string());
        assert_eq!(command["assignmentId"], assignment_id.to_string());
        assert_eq!(command["generation"], generation);
        assert_eq!(command["payload"]["kind"], "start");
        assert_eq!(
            command["payload"]["specification"]["pipelineId"],
            pipeline_id.to_string()
        );
        assert_eq!(
            command["payload"]["specification"]["assignmentGeneration"],
            generation
        );
        assert_eq!(
            command["payload"]["specification"]["configuration"]["source"],
            serde_json::to_value(source_profile).expect("source profile serializes")
        );
        let starting_source_state: String =
            sqlx::query_scalar("SELECT desired_state FROM sources WHERE id = $1")
                .bind(source.id)
                .fetch_one(&pool)
                .await
                .expect("starting source state reads");
        let starting_destination_state: String =
            sqlx::query_scalar("SELECT desired_state FROM destinations WHERE id = $1")
                .bind(destination.id)
                .fetch_one(&pool)
                .await
                .expect("starting destination state reads");
        assert_eq!(starting_source_state, "enabled");
        assert_eq!(starting_destination_state, "enabled");

        let acknowledgement = arc_shared::WorkerCommandAcknowledgement {
            kind: arc_shared::WorkerSessionMessageKind::CommandAck,
            protocol_version: arc_shared::WORKER_PROTOCOL_VERSION,
            worker_id: assigned_worker_id,
            command_id: assignment_id,
            outcome: arc_shared::WorkerCommandOutcome::Completed,
            current_generation: None,
        };
        socket
            .send(ClientMessage::Text(
                serde_json::to_string(&acknowledgement)
                    .expect("Start acknowledgement serializes")
                    .into(),
            ))
            .await
            .expect("Start acknowledgement sends");

        let start_response = start_task
            .await
            .expect("start request task joins")
            .expect("start request completes");
        assert_eq!(start_response.status(), StatusCode::OK);
        let start_body = to_bytes(start_response.into_body(), usize::MAX)
            .await
            .expect("start response reads");
        let started: Value =
            serde_json::from_slice(&start_body).expect("start response is valid JSON");
        assert_eq!(started["desiredState"], "running");
        assert_eq!(started["observedState"], "running");

        let repeat_start = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/assignments/{assignment_id}/start"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "generation": generation }).to_string(),
                    ))
                    .expect("repeat start request builds"),
            )
            .await
            .expect("repeat start request completes");
        assert_eq!(repeat_start.status(), StatusCode::OK);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), socket.next())
                .await
                .is_err(),
            "an already-running assignment must not be dispatched again"
        );

        let cancel_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/assignments/{assignment_id}/cancel"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "generation": generation }).to_string(),
                    ))
                    .expect("cancel request builds"),
            )
            .await
            .expect("cancel request completes");
        assert_eq!(cancel_response.status(), StatusCode::CONFLICT);

        let second_pipeline_id = Uuid::new_v4();
        let second_plan_request = serde_json::json!({
            "pipelineId": second_pipeline_id,
            "sourceId": source.id,
            "sourceRevision": source.current_revision,
            "destinationId": second_destination.id,
            "destinationRevision": second_destination.current_revision,
        });
        let second_plan_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/plan-assignment")
                    .header("content-type", "application/json")
                    .header("idempotency-key", Uuid::new_v4().to_string())
                    .body(Body::from(second_plan_request.to_string()))
                    .expect("second planning request builds"),
            )
            .await
            .expect("second planning request completes");
        assert_eq!(second_plan_response.status(), StatusCode::OK);
        let second_plan_body = to_bytes(second_plan_response.into_body(), usize::MAX)
            .await
            .expect("second planning response reads");
        let second_plan: Value =
            serde_json::from_slice(&second_plan_body).expect("second plan response is valid JSON");
        let second_assignment_id = Uuid::parse_str(
            second_plan["assignment"]["id"]
                .as_str()
                .expect("second assignment ID is returned"),
        )
        .expect("second assignment ID is valid");
        let second_generation = second_plan["assignment"]["generation"]
            .as_i64()
            .expect("second assignment generation is returned");

        let second_start_request = Request::builder()
            .method("POST")
            .uri(format!(
                "/api/v1/assignments/{second_assignment_id}/start"
            ))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "generation": second_generation }).to_string(),
            ))
            .expect("second Start request builds");
        let second_start_app = app.clone();
        let second_start_task =
            tokio::spawn(async move { second_start_app.oneshot(second_start_request).await });
        let message = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
            .await
            .expect("worker receives second Start command")
            .expect("control session remains open")
            .expect("second Start command is valid websocket data");
        let ClientMessage::Text(message) = message else {
            panic!("expected a text second Start command");
        };
        let second_start_command: Value =
            serde_json::from_str(message.as_str()).expect("second Start command is valid JSON");
        assert_eq!(
            second_start_command["assignmentId"],
            second_assignment_id.to_string()
        );
        assert_eq!(
            second_start_command["payload"]["specification"]["configuration"]["destination"],
            serde_json::to_value(&second_destination_profile)
                .expect("second destination profile serializes")
        );
        let second_start_acknowledgement = arc_shared::WorkerCommandAcknowledgement {
            kind: arc_shared::WorkerSessionMessageKind::CommandAck,
            protocol_version: arc_shared::WORKER_PROTOCOL_VERSION,
            worker_id: assigned_worker_id,
            command_id: second_assignment_id,
            outcome: arc_shared::WorkerCommandOutcome::Completed,
            current_generation: None,
        };
        socket
            .send(ClientMessage::Text(
                serde_json::to_string(&second_start_acknowledgement)
                    .expect("second Start acknowledgement serializes")
                    .into(),
            ))
            .await
            .expect("second Start acknowledgement sends");
        let second_start_response = second_start_task
            .await
            .expect("second Start request task joins")
            .expect("second Start request completes");
        assert_eq!(second_start_response.status(), StatusCode::OK);

        let stop_request = Request::builder()
            .method("POST")
            .uri(format!("/api/v1/assignments/{assignment_id}/stop"))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "generation": generation }).to_string(),
            ))
            .expect("stop request builds");
        let stop_app = app.clone();
        let stop_task = tokio::spawn(async move { stop_app.oneshot(stop_request).await });
        let message = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
            .await
            .expect("worker receives Stop command")
            .expect("control session remains open")
            .expect("Stop command is valid websocket data");
        let ClientMessage::Text(message) = message else {
            panic!("expected a text Stop command");
        };
        let command: Value =
            serde_json::from_str(message.as_str()).expect("Stop command is valid JSON");
        let stop_command_id = Uuid::new_v5(&assignment_id, format!("stop:{generation}").as_bytes());
        assert_eq!(command["commandId"], stop_command_id.to_string());
        assert_eq!(command["workerId"], assigned_worker_id.to_string());
        assert_eq!(command["pipelineId"], pipeline_id.to_string());
        assert_eq!(command["assignmentId"], assignment_id.to_string());
        assert_eq!(command["generation"], generation);
        assert_eq!(command["payload"]["kind"], "stop");

        let stopping = PgAssignments::new(pool.clone())
            .get_by_id(assignment_id)
            .await
            .expect("stopping assignment state reads")
            .expect("stopping assignment remains persisted");
        assert_eq!(stopping.desired_state, "stopped");
        assert_eq!(stopping.observed_state, "stopping");
        let stopping_source_state: String =
            sqlx::query_scalar("SELECT desired_state FROM sources WHERE id = $1")
                .bind(source.id)
                .fetch_one(&pool)
                .await
                .expect("stopping source state reads");
        let stopping_destination_state: String =
            sqlx::query_scalar("SELECT desired_state FROM destinations WHERE id = $1")
                .bind(destination.id)
                .fetch_one(&pool)
                .await
                .expect("stopping destination state reads");
        assert_eq!(stopping_source_state, "enabled");
        assert_eq!(stopping_destination_state, "enabled");
        let reservation_state: String =
            sqlx::query_scalar("SELECT state FROM worker_capacity_reservations WHERE id = $1")
                .bind(stopping.assignment.reservation_id)
                .fetch_one(&pool)
                .await
                .expect("reservation state reads");
        assert_eq!(reservation_state, "reserved");

        let acknowledgement = arc_shared::WorkerCommandAcknowledgement {
            kind: arc_shared::WorkerSessionMessageKind::CommandAck,
            protocol_version: arc_shared::WORKER_PROTOCOL_VERSION,
            worker_id: assigned_worker_id,
            command_id: stop_command_id,
            outcome: arc_shared::WorkerCommandOutcome::Completed,
            current_generation: None,
        };
        socket
            .send(ClientMessage::Text(
                serde_json::to_string(&acknowledgement)
                    .expect("Stop acknowledgement serializes")
                    .into(),
            ))
            .await
            .expect("Stop acknowledgement sends");

        let stop_response = stop_task
            .await
            .expect("stop request task joins")
            .expect("stop request completes");
        assert_eq!(stop_response.status(), StatusCode::OK);
        let stop_body = to_bytes(stop_response.into_body(), usize::MAX)
            .await
            .expect("stop response reads");
        let stopped: Value =
            serde_json::from_slice(&stop_body).expect("stop response is valid JSON");
        assert_eq!(stopped["desiredState"], "stopped");
        assert_eq!(stopped["observedState"], "stopped");
        let stopped_source_state: String =
            sqlx::query_scalar("SELECT desired_state FROM sources WHERE id = $1")
                .bind(source.id)
                .fetch_one(&pool)
                .await
                .expect("stopped source state reads");
        let stopped_destination_state: String =
            sqlx::query_scalar("SELECT desired_state FROM destinations WHERE id = $1")
                .bind(destination.id)
                .fetch_one(&pool)
                .await
                .expect("stopped destination state reads");
        assert_eq!(stopped_source_state, "enabled");
        assert_eq!(stopped_destination_state, "disabled");
        let active_destination_state: String =
            sqlx::query_scalar("SELECT desired_state FROM destinations WHERE id = $1")
                .bind(second_destination.id)
                .fetch_one(&pool)
                .await
                .expect("active destination state reads");
        assert_eq!(active_destination_state, "enabled");
        let reservation_state: String =
            sqlx::query_scalar("SELECT state FROM worker_capacity_reservations WHERE id = $1")
                .bind(stopping.assignment.reservation_id)
                .fetch_one(&pool)
                .await
                .expect("released reservation state reads");
        assert_eq!(reservation_state, "released");

        let repeat_stop = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/assignments/{assignment_id}/stop"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "generation": generation }).to_string(),
                    ))
                    .expect("repeat stop request builds"),
            )
            .await
            .expect("repeat stop request completes");
        assert_eq!(repeat_stop.status(), StatusCode::OK);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), socket.next())
                .await
                .is_err(),
            "an already-stopped assignment must not be dispatched again"
        );

        let cancel_stopped = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/assignments/{assignment_id}/cancel"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "generation": generation }).to_string(),
                    ))
                    .expect("cancel-stopped request builds"),
            )
            .await
            .expect("cancel-stopped request completes");
        assert_eq!(cancel_stopped.status(), StatusCode::CONFLICT);

        let second_stop_request = Request::builder()
            .method("POST")
            .uri(format!(
                "/api/v1/assignments/{second_assignment_id}/stop"
            ))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "generation": second_generation }).to_string(),
            ))
            .expect("second Stop request builds");
        let second_stop_app = app.clone();
        let second_stop_task =
            tokio::spawn(async move { second_stop_app.oneshot(second_stop_request).await });
        let message = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
            .await
            .expect("worker receives second Stop command")
            .expect("control session remains open")
            .expect("second Stop command is valid websocket data");
        let ClientMessage::Text(message) = message else {
            panic!("expected a text second Stop command");
        };
        let second_stop_command: Value =
            serde_json::from_str(message.as_str()).expect("second Stop command is valid JSON");
        let second_stop_command_id = Uuid::new_v5(
            &second_assignment_id,
            format!("stop:{second_generation}").as_bytes(),
        );
        assert_eq!(
            second_stop_command["commandId"],
            second_stop_command_id.to_string()
        );
        let second_stop_acknowledgement = arc_shared::WorkerCommandAcknowledgement {
            kind: arc_shared::WorkerSessionMessageKind::CommandAck,
            protocol_version: arc_shared::WORKER_PROTOCOL_VERSION,
            worker_id: assigned_worker_id,
            command_id: second_stop_command_id,
            outcome: arc_shared::WorkerCommandOutcome::Completed,
            current_generation: None,
        };
        socket
            .send(ClientMessage::Text(
                serde_json::to_string(&second_stop_acknowledgement)
                    .expect("second Stop acknowledgement serializes")
                    .into(),
            ))
            .await
            .expect("second Stop acknowledgement sends");
        let second_stop_response = second_stop_task
            .await
            .expect("second Stop request task joins")
            .expect("second Stop request completes");
        assert_eq!(second_stop_response.status(), StatusCode::OK);
        let stopped_shared_source_state: String =
            sqlx::query_scalar("SELECT desired_state FROM sources WHERE id = $1")
                .bind(source.id)
                .fetch_one(&pool)
                .await
                .expect("final source state reads");
        let stopped_second_destination_state: String =
            sqlx::query_scalar("SELECT desired_state FROM destinations WHERE id = $1")
                .bind(second_destination.id)
                .fetch_one(&pool)
                .await
                .expect("final destination state reads");
        assert_eq!(stopped_shared_source_state, "disabled");
        assert_eq!(stopped_second_destination_state, "disabled");

        socket.close(None).await.expect("worker session closes");
        server.abort();
    }

    #[tokio::test]
    async fn route_planner_reports_known_alsa_payload_rate() {
        let response = router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/plan")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "source": { "kind": "alsa_l16", "device": "hw:1,0" },
                            "destination": {
                                "kind": "rtp_l16",
                                "address": "127.0.0.1",
                                "port": 5004,
                                "payloadType": 96
                            }
                        })
                        .to_string(),
                    ))
                    .expect("request builds"),
            )
            .await
            .expect("router handles request");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body reads");
        let value: Value = serde_json::from_slice(&body).expect("response is JSON");
        assert_eq!(
            value,
            serde_json::json!({
                "mediaPath": "alsa_l16_to_rtp_l16",
                "estimate": {
                    "capacityUnits": 2,
                    "ingressPayloadBitsPerSecond": null,
                    "egressPayloadBitsPerSecond": 1536000
                }
            })
        );
    }

    #[tokio::test]
    async fn route_planner_accepts_matching_rtp_l16_profiles() {
        let response = router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/plan")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "source": {
                                "kind": "rtp_l16",
                                "bindAddress": "0.0.0.0",
                                "port": 5004,
                                "payloadType": 96
                            },
                            "destination": {
                                "kind": "rtp_l16",
                                "address": "127.0.0.1",
                                "port": 5006,
                                "payloadType": 97
                            }
                        })
                        .to_string(),
                    ))
                    .expect("request builds"),
            )
            .await
            .expect("router handles request");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body reads");
        let value: Value = serde_json::from_slice(&body).expect("response is JSON");
        assert_eq!(value["mediaPath"], "rtp_l16_to_rtp_l16");
    }

    #[tokio::test]
    async fn route_planner_accepts_srt_mpeg_ts_input_to_output() {
        let response = router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/plan")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "source": {
                                "kind": "srt_mpeg_ts",
                                "host": "0.0.0.0",
                                "port": 6000,
                                "mode": "listener"
                            },
                            "destination": {
                                "kind": "srt_mpeg_ts",
                                "host": "127.0.0.1",
                                "port": 6002,
                                "mode": "caller"
                            }
                        })
                        .to_string(),
                    ))
                    .expect("request builds"),
            )
            .await
            .expect("router handles request");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body reads");
        let value: Value = serde_json::from_slice(&body).expect("response is JSON");
        assert_eq!(value["mediaPath"], "srt_mpeg_ts_to_srt_mpeg_ts");
    }

    #[tokio::test]
    async fn route_planner_accepts_matching_rtp_l24_profiles() {
        let response = router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/plan")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "source": {
                                "kind": "rtp_l24",
                                "bindAddress": "0.0.0.0",
                                "port": 5004,
                                "payloadType": 96
                            },
                            "destination": {
                                "kind": "rtp_l24",
                                "address": "127.0.0.1",
                                "port": 5006,
                                "payloadType": 97
                            }
                        })
                        .to_string(),
                    ))
                    .expect("request builds"),
            )
            .await
            .expect("router handles request");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body reads");
        let value: Value = serde_json::from_slice(&body).expect("response is JSON");
        assert_eq!(value["mediaPath"], "rtp_l24_to_rtp_l24");
    }

    #[tokio::test]
    async fn route_planner_rejects_an_unsupported_conversion() {
        let response = router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/plan")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "source": {
                                "kind": "alsa_l16",
                                "device": "hw:1,0"
                            },
                            "destination": {
                                "kind": "srt_mpeg_ts",
                                "host": "127.0.0.1",
                                "port": 6000,
                                "mode": "caller"
                            }
                        })
                        .to_string(),
                    ))
                    .expect("request builds"),
            )
            .await
            .expect("router handles request");

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn http_server_serves_liveness_over_a_socket() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("ephemeral listener binds");
        let address = listener.local_addr().expect("listener has an address");
        let server = tokio::spawn(async move {
            axum::serve(listener, router())
                .await
                .expect("test server serves requests");
        });

        let mut stream = TcpStream::connect(address)
            .await
            .expect("test server accepts a connection");
        stream
            .write_all(
                b"GET /api/v1/health/live HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            )
            .await
            .expect("request writes");
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .await
            .expect("response reads");
        server.abort();

        let response = String::from_utf8(response).expect("HTTP response is UTF-8");
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("{\"status\":\"alive\"}"));
    }
}
