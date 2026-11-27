use arc_application::{PipelineSpecification, PortError, PortResult, WorkerAuthenticator};
use arc_domain::{
    estimate_pipeline_requirements, validate_destination_profile, validate_source_profile,
    DestinationMediaProfile, MediaProfileError, SourceMediaProfile,
};
use arc_infrastructure::catalog::{
    self, DestinationRecord, DraftValidationError, ResourceDeleteError, ResourceUpdateError,
    SourceRecord,
};
use arc_infrastructure::{
    assignments::{AssignmentSnapshot, PgAssignments, PlannedAssignment},
    capacity_wait::{CapacityWaitRequest, PgCapacityWait, QueueProgress},
    pipeline_planning::{PgPipelinePlanner, PipelinePlanningDecision, RouteRevision},
    postgres::PgPool,
    routes::{
        lock_destination, DesiredRouteChange, PgRoutes, Preset, PresetRoute, RouteAction,
        RouteBatchItemCompletion, RouteBatchOperation, RouteBatchWorkItem, RouteRecord,
        RouteWriteError,
    },
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

mod admin_auth;
mod preview;
mod worker_control_runtime;
pub use admin_auth::AdminAuth;
pub use worker_control_runtime::WorkerCommandDispatcher;
use worker_control_runtime::MAX_WORKER_CONTROL_MESSAGE_SIZE;

const MAX_PLANNING_HEARTBEAT_AGE_SECONDS: i64 = 30;
const MAX_ROUTE_BATCH_ATTEMPTS: i32 = 60;
static ROUTE_EXECUTION_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
const WORKER_CONTROL_HELLO_TIMEOUT: Duration = Duration::from_secs(5);
const WORKER_CONTROL_ROUTE: &str = "/api/v1/workers/control";

async fn route_execution_slot() -> Result<tokio::sync::SemaphorePermit<'static>, ApiError> {
    tokio::time::timeout(Duration::from_secs(10), ROUTE_EXECUTION_SLOTS.acquire())
        .await
        .map_err(|_| ApiError::route_write(RouteWriteError::DestinationBusy))?
        .map_err(|_| ApiError::route_write(RouteWriteError::DestinationBusy))
}

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
        .route("/api/v1/health/ready", get(readiness))
        .route("/api/v1/sources", get(list_sources).post(create_source))
        .route(
            "/api/v1/sources/{source_id}",
            put(update_source).delete(delete_source),
        )
        .route(
            "/api/v1/destinations",
            get(list_destinations).post(create_destination),
        )
        .route(
            "/api/v1/destinations/{destination_id}",
            put(update_destination).delete(delete_destination),
        )
        .route("/api/v1/routes", get(list_routes).post(set_desired_route))
        .route("/api/v1/routes/batch", post(set_desired_routes_batch))
        .route("/api/v1/routes/unroute", post(unroute))
        .route("/api/v1/presets", get(list_presets).post(create_preset))
        .route(
            "/api/v1/presets/{preset_id}",
            put(update_preset).delete(delete_preset),
        )
        .route("/api/v1/presets/{preset_id}/execute", post(execute_preset))
        .route(
            "/api/v1/operations/routes/{operation_id}",
            get(get_route_batch_operation),
        )
        .route(
            "/api/v1/routes/plan-assignment",
            post(plan_route_assignment),
        )
        .route(
            "/api/v1/capacity-requests/{request_id}",
            get(get_capacity_request),
        )
        .route(
            "/api/v1/capacity-requests/{request_id}/cancel",
            post(cancel_capacity_request),
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
    with_capacity_processor(database_routes().layer(Extension(pool.clone())), pool)
}

pub fn router_with_database_and_admin_auth(pool: PgPool, admin_auth: AdminAuth) -> Router {
    let protected = database_routes()
        .route("/api/v1/auth/password", put(admin_auth::change_password))
        .layer(Extension(pool.clone()))
        .layer(Extension(admin_auth.clone()));
    with_capacity_processor(
        admin_auth::protect(protected, admin_auth.clone())
            .merge(admin_auth::public_routes(admin_auth)),
        pool,
    )
}

pub fn router_with_worker_authenticator(authenticator: Arc<dyn WorkerAuthenticator>) -> Router {
    router().merge(worker_control_router(authenticator, None, None))
}

pub fn router_with_worker_dispatcher(
    authenticator: Arc<dyn WorkerAuthenticator>,
    dispatcher: Arc<WorkerCommandDispatcher>,
) -> Router {
    router()
        .merge(worker_control_router(
            authenticator,
            Some(dispatcher.clone()),
            None,
        ))
        .layer(Extension(dispatcher))
}

pub fn router_with_database_and_worker_authenticator(
    pool: PgPool,
    authenticator: Arc<dyn WorkerAuthenticator>,
) -> Router {
    router_with_database(pool).merge(worker_control_router(authenticator, None, None))
}

pub fn router_with_database_and_worker_dispatcher(
    pool: PgPool,
    authenticator: Arc<dyn WorkerAuthenticator>,
    dispatcher: Arc<WorkerCommandDispatcher>,
) -> Router {
    let routes = database_routes()
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
            None,
        ))
        .layer(Extension(pool.clone()))
        .layer(Extension(dispatcher.clone()));
    let routes = with_capacity_processor(routes, pool.clone());
    with_route_batch_processor(routes, pool, dispatcher)
}

pub fn router_with_database_worker_dispatcher_and_admin_auth(
    pool: PgPool,
    authenticator: Arc<dyn WorkerAuthenticator>,
    dispatcher: Arc<WorkerCommandDispatcher>,
    admin_auth: AdminAuth,
) -> Router {
    let preview_broker = preview::PreviewBroker::new();
    let protected = database_routes()
        .route(
            "/api/v1/assignments/{assignment_id}/start",
            post(start_assignment),
        )
        .route(
            "/api/v1/assignments/{assignment_id}/stop",
            post(stop_assignment),
        )
        .route("/api/v1/auth/password", put(admin_auth::change_password))
        .route("/api/v1/sources/{source_id}/preview", get(preview::browser_preview))
        .layer(Extension(pool.clone()))
        .layer(Extension(dispatcher.clone()))
        .layer(Extension(admin_auth.clone()))
        .layer(Extension(preview_broker.clone()));
    let routes = admin_auth::protect(protected, admin_auth.clone())
        .merge(admin_auth::public_routes(admin_auth))
        .merge(worker_control_router(
            authenticator,
            Some(dispatcher.clone()),
            Some(preview_broker),
        ));
    let routes = with_capacity_processor(routes, pool.clone());
    with_route_batch_processor(routes, pool, dispatcher)
}

struct CapacityProcessor(tokio::task::JoinHandle<()>);

impl Drop for CapacityProcessor {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn with_capacity_processor(router: Router, pool: PgPool) -> Router {
    let task = tokio::spawn(async move {
        let queue = PgCapacityWait::new(pool);
        let mut listener = None;
        loop {
            if listener.is_none() {
                match tokio::time::timeout(Duration::from_secs(1), queue.listen()).await {
                    Ok(Ok(connection)) => listener = Some(connection),
                    Ok(Err(error)) => {
                        tracing::warn!(code = error.code, "capacity listener unavailable")
                    }
                    Err(_) => tracing::warn!("capacity listener timed out"),
                }
            }
            let failed = loop {
                match queue
                    .process_one(chrono::Duration::seconds(
                        MAX_PLANNING_HEARTBEAT_AGE_SECONDS,
                    ))
                    .await
                {
                    Ok(QueueProgress::Advanced) => tokio::task::yield_now().await,
                    Ok(QueueProgress::Blocked | QueueProgress::Empty) => break false,
                    Err(error) => {
                        tracing::warn!(code = error.code, "capacity queue processing failed");
                        break true;
                    }
                }
            };
            if failed {
                tokio::time::sleep(Duration::from_secs(1)).await;
            } else if let Some(connection) = &mut listener {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {},
                    result = connection.recv() => {
                        if let Err(error) = result {
                            tracing::warn!(%error, "capacity listener disconnected");
                            listener = None;
                        }
                    }
                }
            } else {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    });
    router.layer(Extension(Arc::new(CapacityProcessor(task))))
}

struct RouteBatchProcessor(tokio::task::JoinHandle<()>);

impl Drop for RouteBatchProcessor {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn with_route_batch_processor(
    router: Router,
    pool: PgPool,
    dispatcher: Arc<WorkerCommandDispatcher>,
) -> Router {
    let task = tokio::spawn(async move {
        let routes = PgRoutes::new(pool.clone());
        loop {
            match process_next_route_batch_item(&routes, &pool, &dispatcher).await {
                Ok(true) => tokio::task::yield_now().await,
                Ok(false) => tokio::time::sleep(Duration::from_secs(1)).await,
                Err(error) => {
                    tracing::warn!(code = error.code, %error.message, "route batch processing failed");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    });
    router.layer(Extension(Arc::new(RouteBatchProcessor(task))))
}

async fn process_next_route_batch_item(
    routes: &PgRoutes,
    pool: &PgPool,
    dispatcher: &Arc<WorkerCommandDispatcher>,
) -> PortResult<bool> {
    let _slot = ROUTE_EXECUTION_SLOTS
        .acquire()
        .await
        .map_err(|error| PortError {
            code: "route_batch_storage_error",
            message: error.to_string(),
        })?;
    let item = routes
        .claim_next_batch_item()
        .await
        .map_err(route_batch_storage_error)?;
    let Some(item) = item else {
        return Ok(false);
    };
    let mut guard = pool.begin().await.map_err(|error| PortError {
        code: "database_error",
        message: error.to_string(),
    })?;
    lock_destination(&mut guard, item.destination_id)
        .await
        .map_err(|error| PortError {
            code: "database_error",
            message: error.to_string(),
        })?;
    let result = process_claimed_route_batch_item(routes, pool, dispatcher, item).await;
    guard.commit().await.map_err(|error| PortError {
        code: "database_error",
        message: error.to_string(),
    })?;
    result
}

async fn process_claimed_route_batch_item(
    routes: &PgRoutes,
    pool: &PgPool,
    dispatcher: &Arc<WorkerCommandDispatcher>,
    item: RouteBatchWorkItem,
) -> PortResult<bool> {
    match routes.is_batch_item_current(&item).await {
        Ok(true) => {}
        Ok(false) => {
            routes
                .finish_batch_item(
                    &item,
                    RouteBatchItemCompletion::Failed {
                        assignment: None,
                        error_code: "route_changed",
                        error_message: "The desired route changed after this Take was saved.",
                    },
                )
                .await
                .map_err(route_batch_storage_error)?;
            return Ok(true);
        }
        Err(error) => {
            tracing::warn!(error = ?error, "could not verify the desired route before execution");
            routes
                .finish_batch_item(
                    &item,
                    RouteBatchItemCompletion::Retry {
                        assignment: None,
                        error_code: "route_batch_storage_error",
                        error_message: "The operation could not verify route state and will retry.",
                        delay_seconds: 5,
                    },
                )
                .await
                .map_err(route_batch_storage_error)?;
            return Ok(true);
        }
    }

    if item.action == "stop" {
        process_unroute_item(routes, pool, dispatcher, &item).await?;
        return Ok(true);
    }

    let assignments = PgAssignments::new(pool.clone());
    let assignment = match assignments.get_by_pipeline(item.pipeline_id).await {
        Ok(Some(snapshot)) => snapshot.assignment,
        Ok(None) => {
            let planner = PgPipelinePlanner::new(pool.clone());
            let route = RouteRevision {
                source_id: item.source_id,
                source_revision: item.source_revision,
                destination_id: item.destination_id,
                destination_revision: item.destination_revision,
            };
            let max_heartbeat_age = chrono::Duration::seconds(MAX_PLANNING_HEARTBEAT_AGE_SECONDS);
            let decision = match item.replacement_assignment_id {
                Some(replacement_assignment_id) => {
                    planner
                        .plan_replacement_with_idempotency_key(
                            item.planning_key,
                            item.pipeline_id,
                            route,
                            max_heartbeat_age,
                            replacement_assignment_id,
                        )
                        .await
                }
                None => {
                    planner
                        .plan_with_idempotency_key(
                            item.planning_key,
                            item.pipeline_id,
                            route,
                            max_heartbeat_age,
                        )
                        .await
                }
            };
            match decision {
                Ok(PipelinePlanningDecision::Planned(planned)) => planned.assignment,
                Ok(PipelinePlanningDecision::Unavailable { rejections, .. }) => {
                    let message = format!("No eligible worker was available: {rejections:?}");
                    routes
                        .finish_batch_item(
                            &item,
                            RouteBatchItemCompletion::Failed {
                                assignment: None,
                                error_code: "placement_unavailable",
                                error_message: &message,
                            },
                        )
                        .await
                        .map_err(route_batch_storage_error)?;
                    return Ok(true);
                }
                Err(error) => {
                    tracing::warn!(code = error.code, %error.message, "route batch planning failed");
                    finish_route_batch_error(routes, &item, None, error.code, &error.message)
                        .await?;
                    return Ok(true);
                }
            }
        }
        Err(error) => {
            tracing::warn!(code = error.code, %error.message, "route batch assignment lookup failed");
            finish_route_batch_error(routes, &item, None, error.code, &error.message).await?;
            return Ok(true);
        }
    };

    if item.replacement_assignment_id.is_some() {
        process_replacement_batch_item(routes, pool, dispatcher, &item, assignment).await?;
        return Ok(true);
    }

    if item.error_code.as_deref() == Some("start_cleanup_pending") {
        let initial_error = item
            .error_message
            .as_deref()
            .unwrap_or("Take failed before its cleanup was confirmed.");
        match cleanup_replacement_target(
            pool.clone(),
            Arc::clone(dispatcher),
            assignment.id,
            assignment.generation,
        )
        .await
        {
            Ok(()) => routes
                .finish_batch_item(
                    &item,
                    RouteBatchItemCompletion::Failed {
                        assignment: Some((assignment.id, assignment.generation)),
                        error_code: "worker_start_failed",
                        error_message: initial_error,
                    },
                )
                .await
                .map_err(route_batch_storage_error)?,
            Err(error) if item.attempts < MAX_ROUTE_BATCH_ATTEMPTS => {
                tracing::warn!(code = error.code, message = %error.message, "failed Take cleanup will retry");
                finish_route_batch_retry(
                    routes,
                    &item,
                    Some((assignment.id, assignment.generation)),
                    "start_cleanup_pending",
                    initial_error,
                    5,
                )
                .await?;
            }
            Err(error) => {
                tracing::warn!(code = error.code, message = %error.message, "failed Take cleanup requires manual recovery");
                routes
                    .finish_batch_item(
                        &item,
                        RouteBatchItemCompletion::Failed {
                            assignment: Some((assignment.id, assignment.generation)),
                            error_code: "start_cleanup_failed",
                            error_message: "Take failed and cleanup could not be confirmed. The output remains reserved; manual recovery is required.",
                        },
                    )
                    .await
                    .map_err(route_batch_storage_error)?;
            }
        }
        return Ok(true);
    }

    match start_assignment_for(
        pool.clone(),
        Arc::clone(dispatcher),
        assignment.id,
        assignment.generation,
    )
    .await
    {
        Ok(snapshot) => routes
            .finish_batch_item(
                &item,
                RouteBatchItemCompletion::Succeeded {
                    assignment_id: snapshot.assignment.id,
                    generation: snapshot.assignment.generation,
                },
            )
            .await
            .map_err(route_batch_storage_error)?,
        Err(error) => {
            tracing::warn!(code = error.code, message = %error.message, "route batch worker start failed");
            if is_retryable_route_error(error.code) && item.attempts < MAX_ROUTE_BATCH_ATTEMPTS {
                finish_route_batch_error(
                    routes,
                    &item,
                    Some((assignment.id, assignment.generation)),
                    error.code,
                    &error.message,
                )
                .await?;
            } else {
                finish_route_batch_retry(
                    routes,
                    &item,
                    Some((assignment.id, assignment.generation)),
                    "start_cleanup_pending",
                    &format!(
                        "Take failed: {}. Review the input and output settings before retrying.",
                        error.message
                    ),
                    1,
                )
                .await?;
            }
        }
    }
    Ok(true)
}

async fn process_unroute_item(
    routes: &PgRoutes,
    pool: &PgPool,
    dispatcher: &Arc<WorkerCommandDispatcher>,
    item: &RouteBatchWorkItem,
) -> PortResult<()> {
    let assignments = sqlx::query_as::<_, (Uuid, i64)>(
        "SELECT target.assignment_id, target.generation FROM route_stop_targets target \
         JOIN pipeline_assignments assignment ON assignment.id = target.assignment_id \
         WHERE target.operation_id = $1 AND assignment.destination_id = $2 ORDER BY target.assignment_id",
    ).bind(item.operation_id).bind(item.destination_id).fetch_all(pool).await.map_err(|error| PortError {
        code: "database_error", message: error.to_string(),
    })?;
    let mut last = None;
    for (id, generation) in assignments {
        let snapshot = PgAssignments::new(pool.clone())
            .get_by_id(id)
            .await?
            .ok_or_else(|| PortError {
                code: "assignment_not_found",
                message: "Captured Stop assignment no longer exists".to_owned(),
            })?;
        if snapshot.assignment.generation != generation {
            return finish_route_batch_error(
                routes,
                item,
                Some((id, generation)),
                "assignment_conflict",
                "Captured assignment generation changed",
            )
            .await;
        }
        if snapshot.desired_state == "cancelled" && snapshot.observed_state == "cancelled" {
            let stop_id = Uuid::new_v5(&id, format!("stop:{generation}").as_bytes());
            let acknowledged: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM worker_command_ledger WHERE command_id = $1 AND status = 'completed')",
            ).bind(stop_id).fetch_one(pool).await.map_err(|error| PortError {
                code: "database_error", message: error.to_string(),
            })?;
            if !acknowledged {
                return finish_route_batch_error(
                    routes,
                    item,
                    Some((id, generation)),
                    "stop_unconfirmed",
                    "The assignment was cancelled without a confirmed Stop",
                )
                .await;
            }
        }
        if let Err(error) =
            cleanup_replacement_target(pool.clone(), Arc::clone(dispatcher), id, generation).await
        {
            return finish_route_batch_error(
                routes,
                item,
                Some((id, generation)),
                error.code,
                &error.message,
            )
            .await;
        }
        last = Some((id, generation));
    }
    let completion = match last {
        Some((assignment_id, generation)) => RouteBatchItemCompletion::Succeeded {
            assignment_id,
            generation,
        },
        None => RouteBatchItemCompletion::Failed {
            assignment: None,
            error_code: "stop_unconfirmed",
            error_message:
                "Desired route was removed, but no assignment was available to acknowledge a Stop.",
        },
    };
    routes
        .finish_batch_item(item, completion)
        .await
        .map_err(route_batch_storage_error)
}

async fn process_replacement_batch_item(
    routes: &PgRoutes,
    pool: &PgPool,
    dispatcher: &Arc<WorkerCommandDispatcher>,
    item: &RouteBatchWorkItem,
    target: PlannedAssignment,
) -> PortResult<()> {
    let assignments = PgAssignments::new(pool.clone());
    let replacement_id = item
        .replacement_assignment_id
        .expect("replacement item has a previous assignment");
    let previous = match assignments.get_by_id(replacement_id).await {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => {
            return finish_replacement_failure(
                routes,
                item,
                &target,
                None,
                "replacement_assignment_missing",
                "The previous assignment no longer exists; no replacement was started.",
            )
            .await;
        }
        Err(error) => {
            return finish_route_batch_error(
                routes,
                item,
                Some((target.id, target.generation)),
                error.code,
                &error.message,
            )
            .await;
        }
    };
    if previous
        .assignment
        .route
        .as_ref()
        .is_none_or(|route| route.destination_id != item.destination_id)
    {
        return finish_replacement_failure(
            routes,
            item,
            &target,
            Some(&previous.assignment),
            "replacement_assignment_conflict",
            "The previous assignment does not match this destination; no replacement was started.",
        )
        .await;
    }

    let target_snapshot = match assignments.get_by_id(target.id).await {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => {
            return finish_replacement_failure(
                routes,
                item,
                &target,
                Some(&previous.assignment),
                "replacement_assignment_missing",
                "The planned replacement assignment no longer exists.",
            )
            .await;
        }
        Err(error) => {
            return finish_route_batch_error(
                routes,
                item,
                Some((target.id, target.generation)),
                error.code,
                &error.message,
            )
            .await;
        }
    };

    if item.error_code.as_deref() == Some("replacement_rollback_pending")
        || target_snapshot.desired_state == "cancelled"
    {
        return restore_previous_route(routes, pool, dispatcher, item, &target, previous).await;
    }
    if item.error_code.as_deref() == Some("replacement_cleanup_pending") {
        let cleanup = cleanup_replacement_target(
            pool.clone(),
            Arc::clone(dispatcher),
            target.id,
            target.generation,
        )
        .await;
        if let Err(error) = cleanup {
            return finish_route_batch_retry(
                routes,
                item,
                Some((target.id, target.generation)),
                "replacement_cleanup_pending",
                &format!("Replacement cleanup is still pending: {}", error.message),
                5,
            )
            .await;
        }
        let refreshed_previous = assignments
            .get_by_id(replacement_id)
            .await
            .map_err(|error| PortError {
                code: error.code,
                message: error.message,
            })?
            .ok_or_else(|| PortError {
                code: "replacement_assignment_missing",
                message: "the previous assignment disappeared during cleanup".to_owned(),
            })?;
        if assignment_is_stopped(&refreshed_previous) {
            return defer_replacement_rollback(
                routes,
                item,
                &target,
                "replacement_cleanup_failed",
                item.error_message
                    .as_deref()
                    .unwrap_or("Replacement failed and the previous route must be restored."),
            )
            .await;
        }
        let previous = if refreshed_previous.desired_state == "stopped"
            && refreshed_previous.observed_state == "stopping"
        {
            assignments
                .reset_failed_stop(replacement_id, previous.assignment.generation)
                .await
                .map_err(|error| PortError {
                    code: error.code,
                    message: error.message,
                })?
        } else {
            refreshed_previous
        };
        return finish_replacement_failure(
            routes,
            item,
            &target,
            Some(&previous.assignment),
            "replacement_prepare_failed",
            item.error_message.as_deref().unwrap_or(
                "Replacement failed before the previous route was stopped; the previous route remains active.",
            ),
        )
        .await;
    }

    if !matches!(
        (
            target_snapshot.desired_state.as_str(),
            target_snapshot.observed_state.as_str()
        ),
        ("running", "running")
    ) {
        if let Err(error) = prepare_assignment_for(Arc::clone(dispatcher), &target).await {
            tracing::warn!(code = error.code, message = %error.message, "route replacement preparation failed");
            let latest_previous = assignments
                .get_by_id(replacement_id)
                .await
                .map_err(|storage| PortError {
                    code: storage.code,
                    message: storage.message,
                })?
                .ok_or_else(|| PortError {
                    code: "replacement_assignment_missing",
                    message: "the previous assignment disappeared during preparation".to_owned(),
                })?;
            if assignment_is_stopped(&latest_previous)
                || latest_previous.observed_state == "stopping"
            {
                let cleanup = cleanup_replacement_target(
                    pool.clone(),
                    Arc::clone(dispatcher),
                    target.id,
                    target.generation,
                )
                .await;
                let message = format!(
                    "Replacement preparation failed ({}: {}); restoring the previous route.",
                    error.code, error.message
                );
                if let Err(cleanup_error) = cleanup {
                    return finish_route_batch_retry(
                        routes,
                        item,
                        Some((target.id, target.generation)),
                        "replacement_rollback_pending",
                        &format!("{message} Cleanup is pending: {}", cleanup_error.message),
                        1,
                    )
                    .await;
                }
                return defer_replacement_rollback(routes, item, &target, error.code, &message)
                    .await;
            }
            if is_retryable_route_error(error.code) && item.attempts < MAX_ROUTE_BATCH_ATTEMPTS {
                return finish_route_batch_error(
                    routes,
                    item,
                    Some((target.id, target.generation)),
                    error.code,
                    &error.message,
                )
                .await;
            }
            if latest_previous.desired_state == "stopped"
                && latest_previous.observed_state == "stopping"
            {
                assignments
                    .reset_failed_stop(replacement_id, latest_previous.assignment.generation)
                    .await
                    .map_err(|reset| PortError {
                        code: reset.code,
                        message: reset.message,
                    })?;
            }
            if let Err(cleanup_error) = cleanup_replacement_target(
                pool.clone(),
                Arc::clone(dispatcher),
                target.id,
                target.generation,
            )
            .await
            {
                return finish_route_batch_retry(
                    routes,
                    item,
                    Some((target.id, target.generation)),
                    "replacement_cleanup_pending",
                    &format!(
                        "Replacement preparation failed ({}: {}). Cleanup is pending: {}",
                        error.code, error.message, cleanup_error.message
                    ),
                    5,
                )
                .await;
            }
            return finish_replacement_failure(
                routes,
                item,
                &target,
                Some(&latest_previous.assignment),
                "replacement_prepare_failed",
                &format!(
                    "Replacement preparation failed ({}: {}); the previous route was not stopped.",
                    error.code, error.message
                ),
            )
            .await;
        }
    }

    let previous = previous;
    if !assignment_is_stopped(&previous) {
        match stop_assignment_for(
            pool.clone(),
            Arc::clone(dispatcher),
            replacement_id,
            previous.assignment.generation,
        )
        .await
        {
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(code = error.code, message = %error.message, "route replacement could not stop the previous assignment");
                let latest_previous = assignments
                    .get_by_id(replacement_id)
                    .await
                    .map_err(|storage| PortError {
                        code: storage.code,
                        message: storage.message,
                    })?
                    .ok_or_else(|| PortError {
                        code: "replacement_assignment_missing",
                        message: "the previous assignment disappeared while stopping".to_owned(),
                    })?;
                if is_retryable_route_error(error.code) && item.attempts < MAX_ROUTE_BATCH_ATTEMPTS
                {
                    return finish_route_batch_error(
                        routes,
                        item,
                        Some((target.id, target.generation)),
                        error.code,
                        &error.message,
                    )
                    .await;
                }
                if latest_previous.desired_state == "stopped"
                    && latest_previous.observed_state == "stopping"
                {
                    assignments
                        .reset_failed_stop(replacement_id, previous.assignment.generation)
                        .await
                        .map_err(|reset| PortError {
                            code: reset.code,
                            message: reset.message,
                        })?;
                }
                if let Err(cleanup_error) = cleanup_replacement_target(
                    pool.clone(),
                    Arc::clone(dispatcher),
                    target.id,
                    target.generation,
                )
                .await
                {
                    return finish_route_batch_retry(
                        routes,
                        item,
                        Some((target.id, target.generation)),
                        "replacement_cleanup_pending",
                        &format!(
                            "The previous route could not be stopped ({}: {}). Cleanup is pending: {}",
                            error.code, error.message, cleanup_error.message
                        ),
                        5,
                    )
                    .await;
                }
                return finish_replacement_failure(
                    routes,
                    item,
                    &target,
                    Some(&latest_previous.assignment),
                    "replacement_stop_failed",
                    &format!(
                        "The previous route could not be stopped ({}: {}); the replacement was not started.",
                        error.code, error.message
                    ),
                )
                .await;
            }
        }
    }

    match start_assignment_for(
        pool.clone(),
        Arc::clone(dispatcher),
        target.id,
        target.generation,
    )
    .await
    {
        Ok(snapshot) => routes
            .finish_batch_item(
                item,
                RouteBatchItemCompletion::Succeeded {
                    assignment_id: snapshot.assignment.id,
                    generation: snapshot.assignment.generation,
                },
            )
            .await
            .map_err(route_batch_storage_error),
        Err(error) => {
            tracing::warn!(code = error.code, message = %error.message, "replacement Start failed after the previous route was stopped");
            if is_retryable_route_error(error.code) && item.attempts < MAX_ROUTE_BATCH_ATTEMPTS {
                return finish_route_batch_error(
                    routes,
                    item,
                    Some((target.id, target.generation)),
                    error.code,
                    &error.message,
                )
                .await;
            }
            if let Err(cleanup_error) = cleanup_replacement_target(
                pool.clone(),
                Arc::clone(dispatcher),
                target.id,
                target.generation,
            )
            .await
            {
                let message = format!(
                    "Replacement Start failed ({}: {}); restoring the previous route. Cleanup is pending: {}",
                    error.code, error.message, cleanup_error.message
                );
                return finish_route_batch_retry(
                    routes,
                    item,
                    Some((target.id, target.generation)),
                    "replacement_rollback_pending",
                    &message,
                    1,
                )
                .await;
            }
            defer_replacement_rollback(
                routes,
                item,
                &target,
                error.code,
                &format!(
                    "Replacement Start failed ({}: {}); restoring the previous route.",
                    error.code, error.message
                ),
            )
            .await
        }
    }
}

async fn restore_previous_route(
    routes: &PgRoutes,
    pool: &PgPool,
    dispatcher: &Arc<WorkerCommandDispatcher>,
    item: &RouteBatchWorkItem,
    target: &PlannedAssignment,
    mut previous: AssignmentSnapshot,
) -> PortResult<()> {
    if let Err(error) = cleanup_replacement_target(
        pool.clone(),
        Arc::clone(dispatcher),
        target.id,
        target.generation,
    )
    .await
    {
        return retry_or_fail_replacement_rollback(
            routes,
            item,
            target,
            &format!(
                "{} Cleanup is still pending: {}",
                item.error_message
                    .as_deref()
                    .unwrap_or("Replacement failed; restoring the previous route."),
                error.message
            ),
        )
        .await;
    }

    let assignments = PgAssignments::new(pool.clone());
    previous = assignments
        .get_by_id(previous.assignment.id)
        .await
        .map_err(|error| PortError {
            code: error.code,
            message: error.message,
        })?
        .ok_or_else(|| PortError {
            code: "replacement_assignment_missing",
            message: "the previous assignment disappeared during recovery".to_owned(),
        })?;
    if previous.desired_state == "running"
        && matches!(
            previous.observed_state.as_str(),
            "starting" | "running" | "failed" | "degraded"
        )
    {
        return finish_replacement_failure(
            routes,
            item,
            target,
            Some(&previous.assignment),
            "replacement_failed_previous_preserved",
            "Replacement failed; the previous route remains assigned and was not stopped.",
        )
        .await;
    }
    if !assignment_is_stopped(&previous) {
        match stop_assignment_for(
            pool.clone(),
            Arc::clone(dispatcher),
            previous.assignment.id,
            previous.assignment.generation,
        )
        .await
        {
            Ok(snapshot) => previous = snapshot,
            Err(error) => {
                return retry_or_fail_replacement_rollback(
                    routes,
                    item,
                    target,
                    &format!(
                        "{} Previous route is waiting for Stop: {}",
                        item.error_message
                            .as_deref()
                            .unwrap_or("Replacement failed; restoring the previous route."),
                        error.message
                    ),
                )
                .await;
            }
        }
    }
    let Some(route_config) = previous.assignment.route.clone() else {
        return finish_replacement_failure(
            routes,
            item,
            target,
            Some(&previous.assignment),
            "replacement_rollback_failed",
            "The previous assignment has no pinned route snapshot; manual recovery is required.",
        )
        .await;
    };

    let rollback_pipeline_id = Uuid::new_v5(
        &item.operation_id,
        format!("route-batch-restore:{}", item.destination_id).as_bytes(),
    );
    let rollback_assignment = match assignments.get_by_pipeline(rollback_pipeline_id).await {
        Ok(Some(snapshot))
            if snapshot.desired_state != "cancelled" && snapshot.desired_state != "stopped" =>
        {
            snapshot.assignment
        }
        Ok(Some(_)) | Ok(None) => {
            let planner = PgPipelinePlanner::new(pool.clone());
            match planner
                .plan_pinned_route(
                    rollback_pipeline_id,
                    route_config,
                    previous.assignment.requirements.clone(),
                    chrono::Duration::seconds(MAX_PLANNING_HEARTBEAT_AGE_SECONDS),
                )
                .await
            {
                Ok(PipelinePlanningDecision::Planned(planned)) => planned.assignment,
                Ok(PipelinePlanningDecision::Unavailable { rejections, .. }) => {
                    return retry_or_fail_replacement_rollback(
                        routes,
                        item,
                        target,
                        &format!(
                            "{} No worker can restore the previous route yet: {rejections:?}",
                            item.error_message
                                .as_deref()
                                .unwrap_or("Replacement failed; restoring the previous route.")
                        ),
                    )
                    .await;
                }
                Err(error) => {
                    return retry_or_fail_replacement_rollback(
                        routes,
                        item,
                        target,
                        &format!(
                            "{} Recovery planning failed: {}",
                            item.error_message
                                .as_deref()
                                .unwrap_or("Replacement failed; restoring the previous route."),
                            error.message
                        ),
                    )
                    .await;
                }
            }
        }
        Err(error) => {
            return retry_or_fail_replacement_rollback(
                routes,
                item,
                target,
                &format!(
                    "{} Recovery lookup failed: {}",
                    item.error_message
                        .as_deref()
                        .unwrap_or("Replacement failed; restoring the previous route."),
                    error.message
                ),
            )
            .await;
        }
    };

    match start_assignment_for(
        pool.clone(),
        Arc::clone(dispatcher),
        rollback_assignment.id,
        rollback_assignment.generation,
    )
    .await
    {
        Ok(snapshot) => {
            let message = format!(
                "{} The previous route was restored.",
                item.error_message
                    .as_deref()
                    .unwrap_or("Replacement failed.")
            );
            routes
                .finish_batch_item(
                    item,
                    RouteBatchItemCompletion::Failed {
                        assignment: Some((snapshot.assignment.id, snapshot.assignment.generation)),
                        error_code: "replacement_failed_restored",
                        error_message: &message,
                    },
                )
                .await
                .map_err(route_batch_storage_error)
        }
        Err(error) => {
            retry_or_fail_replacement_rollback(
                routes,
                item,
                target,
                &format!(
                    "{} Restoring the previous route is still pending: {}",
                    item.error_message
                        .as_deref()
                        .unwrap_or("Replacement failed."),
                    error.message
                ),
            )
            .await
        }
    }
}

async fn cleanup_replacement_target(
    pool: PgPool,
    dispatcher: Arc<WorkerCommandDispatcher>,
    assignment_id: Uuid,
    generation: i64,
) -> Result<(), ApiError> {
    let assignments = PgAssignments::new(pool.clone());
    let snapshot = assignments
        .get_by_id(assignment_id)
        .await
        .map_err(ApiError::assignment)?
        .ok_or_else(|| ApiError {
            status: StatusCode::NOT_FOUND,
            code: "assignment_not_found",
            message: "replacement assignment disappeared during cleanup".to_owned(),
        })?;
    match (
        snapshot.desired_state.as_str(),
        snapshot.observed_state.as_str(),
    ) {
        ("cancelled", "cancelled") | ("stopped", "stopped") => Ok(()),
        ("planned", "not_started") => {
            match dispatcher
                .dispatch(stop_command(&snapshot.assignment))
                .await
            {
                Ok(CommandDispatchResult::Applied | CommandDispatchResult::Duplicate) => {
                    assignments
                        .cancel_unstarted(assignment_id, generation)
                        .await
                        .map_err(ApiError::assignment)
                }
                Ok(CommandDispatchResult::Stale { current_generation }) => {
                    Err(ApiError::worker_dispatch(PortError {
                        code: "stale_worker_command",
                        message: format!(
                            "worker has already accepted generation {current_generation}"
                        ),
                    }))
                }
                Err(error) => Err(ApiError::worker_dispatch(error)),
            }
        }
        ("running", "starting" | "running" | "failed" | "degraded" | "unknown")
        | ("stopped", "stopping") => {
            let stopped = stop_assignment_for(pool, dispatcher, assignment_id, generation).await?;
            if stopped.observed_state == "stopped" {
                Ok(())
            } else {
                Err(ApiError::assignment(PortError {
                    code: "assignment_conflict",
                    message: "replacement cleanup did not reach the stopped state".to_owned(),
                }))
            }
        }
        _ => Err(ApiError::assignment(PortError {
            code: "assignment_conflict",
            message: "replacement assignment is not in a recoverable cleanup state".to_owned(),
        })),
    }
}

fn assignment_is_stopped(snapshot: &AssignmentSnapshot) -> bool {
    snapshot.desired_state == "stopped" && snapshot.observed_state == "stopped"
}

fn is_retryable_route_error(code: &'static str) -> bool {
    matches!(
        code,
        "database_error"
            | "worker_session_unavailable"
            | "worker_session_replaced"
            | "worker_session_busy"
            | "worker_session_ended"
            | "worker_command_ack_timeout"
            | "worker_session_registration_failed"
    )
}

async fn finish_replacement_failure(
    routes: &PgRoutes,
    item: &RouteBatchWorkItem,
    target: &PlannedAssignment,
    recovery_assignment: Option<&PlannedAssignment>,
    error_code: &'static str,
    error_message: &str,
) -> PortResult<()> {
    routes
        .finish_batch_item(
            item,
            RouteBatchItemCompletion::Failed {
                assignment: recovery_assignment
                    .map(|assignment| (assignment.id, assignment.generation))
                    .or(Some((target.id, target.generation))),
                error_code,
                error_message,
            },
        )
        .await
        .map_err(route_batch_storage_error)
}

async fn finish_route_batch_retry(
    routes: &PgRoutes,
    item: &RouteBatchWorkItem,
    assignment: Option<(Uuid, i64)>,
    error_code: &'static str,
    error_message: &str,
    delay_seconds: i64,
) -> PortResult<()> {
    routes
        .finish_batch_item(
            item,
            RouteBatchItemCompletion::Retry {
                assignment,
                error_code,
                error_message,
                delay_seconds,
            },
        )
        .await
        .map_err(route_batch_storage_error)
}

async fn defer_replacement_rollback(
    routes: &PgRoutes,
    item: &RouteBatchWorkItem,
    target: &PlannedAssignment,
    _cause_code: &'static str,
    error_message: &str,
) -> PortResult<()> {
    finish_route_batch_retry(
        routes,
        item,
        Some((target.id, target.generation)),
        "replacement_rollback_pending",
        error_message,
        1,
    )
    .await
}

async fn retry_or_fail_replacement_rollback(
    routes: &PgRoutes,
    item: &RouteBatchWorkItem,
    target: &PlannedAssignment,
    error_message: &str,
) -> PortResult<()> {
    if item.attempts >= MAX_ROUTE_BATCH_ATTEMPTS {
        return finish_replacement_failure(
            routes,
            item,
            target,
            None,
            "replacement_rollback_failed",
            &format!(
                "{error_message} Automatic recovery reached its retry limit; manual recovery is required."
            ),
        )
        .await;
    }
    finish_route_batch_retry(
        routes,
        item,
        Some((target.id, target.generation)),
        "replacement_rollback_pending",
        error_message,
        5,
    )
    .await
}

async fn finish_route_batch_error(
    routes: &PgRoutes,
    item: &RouteBatchWorkItem,
    assignment: Option<(Uuid, i64)>,
    error_code: &'static str,
    error_message: &str,
) -> PortResult<()> {
    let retryable = matches!(
        error_code,
        "database_error"
            | "worker_session_unavailable"
            | "worker_session_replaced"
            | "worker_session_busy"
            | "worker_session_ended"
            | "worker_command_ack_timeout"
            | "worker_session_registration_failed"
    );
    let completion = if retryable && item.attempts < MAX_ROUTE_BATCH_ATTEMPTS {
        RouteBatchItemCompletion::Retry {
            assignment,
            error_code,
            error_message: "Worker execution is temporarily unavailable; this route will retry.",
            delay_seconds: 5,
        }
    } else if retryable {
        RouteBatchItemCompletion::Failed {
            assignment,
            error_code: "worker_unavailable",
            error_message: "The worker did not become available before the route retry limit.",
        }
    } else {
        RouteBatchItemCompletion::Failed {
            assignment,
            error_code,
            error_message,
        }
    };
    routes
        .finish_batch_item(item, completion)
        .await
        .map_err(route_batch_storage_error)
}

fn route_batch_storage_error(error: RouteWriteError) -> PortError {
    PortError {
        code: "route_batch_storage_error",
        message: format!("{error:?}"),
    }
}

#[derive(Clone)]
struct WorkerControlState {
    authenticator: Arc<dyn WorkerAuthenticator>,
    dispatcher: Option<Arc<WorkerCommandDispatcher>>,
    preview_broker: Option<Arc<preview::PreviewBroker>>,
}

fn worker_control_router(
    authenticator: Arc<dyn WorkerAuthenticator>,
    dispatcher: Option<Arc<WorkerCommandDispatcher>>,
    preview_broker: Option<Arc<preview::PreviewBroker>>,
) -> Router {
    Router::new()
        .route(WORKER_CONTROL_ROUTE, get(worker_control))
        .route("/api/v1/workers/control/preview/{preview_id}", get(preview::worker_preview))
        .layer(Extension(WorkerControlState {
            authenticator,
            dispatcher,
            preview_broker,
        }))
}

async fn worker_control(
    headers: HeaderMap,
    Extension(state): Extension<WorkerControlState>,
    websocket: WebSocketUpgrade,
) -> Response {
    let (worker_id, bearer_token) = match authenticate_worker(&headers, &state).await {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let authenticator = state.authenticator.clone();
    let dispatcher = state.dispatcher.clone();
    websocket
        .max_message_size(MAX_WORKER_CONTROL_MESSAGE_SIZE)
        .max_frame_size(MAX_WORKER_CONTROL_MESSAGE_SIZE)
        .on_upgrade(move |socket| {
            handle_worker_control(socket, worker_id, dispatcher, authenticator, bearer_token)
        })
        .into_response()
}

async fn authenticate_worker(headers: &HeaderMap, state: &WorkerControlState) -> Result<(Uuid, String), Response> {
    if !headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.eq_ignore_ascii_case("https"))
        .unwrap_or(false)
    {
        return Err(worker_control_error(
            StatusCode::UPGRADE_REQUIRED,
            "secure_worker_connection_required",
            "worker control requires TLS at the trusted ingress",
        ));
    }

    let Some(worker_id) = headers
        .get("x-worker-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Uuid::parse_str(value).ok())
        .filter(|worker_id| !worker_id.is_nil())
    else {
        return Err(worker_control_error(
            StatusCode::UNAUTHORIZED,
            "worker_authentication_failed",
            "worker identity or credential was rejected",
        ));
    };
    let Some(bearer_token) = bearer_token(&headers) else {
        return Err(worker_control_error(
            StatusCode::UNAUTHORIZED,
            "worker_authentication_failed",
            "worker identity or credential was rejected",
        ));
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
        return Err(worker_control_error(status, code, message));
    }
    Ok((worker_id, bearer_token.to_owned()))
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
    authenticator: Arc<dyn WorkerAuthenticator>,
    bearer_token: String,
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
        Some(dispatcher) => match dispatcher.start_session(worker_id, hello.process_incarnation).await {
            Ok(session) => Some(session),
            Err(error) => {
                tracing::error!(%worker_id, code = error.code, message = %error.message, "failed to register worker control session lease");
                close_worker_control(&mut socket, "worker session unavailable").await;
                return;
            }
        },
        None => None,
    };
    let welcome = arc_shared::WorkerSessionWelcome {
        kind: arc_shared::WorkerSessionMessageKind::Welcome,
        protocol_version: arc_shared::WORKER_PROTOCOL_VERSION,
        worker_id,
        session_id: session.as_ref().map(|session| session.session_id),
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
        dispatcher
            .serve_session(&mut socket, session, authenticator, bearer_token)
            .await;
        return;
    }

    let mut credential_check = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_secs(1),
        Duration::from_secs(1),
    );
    credential_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let message = tokio::select! {
            _ = credential_check.tick() => {
                if authenticator.authenticate(worker_id, &bearer_token).await.is_err() {
                    close_worker_control(&mut socket, "worker credential revoked").await;
                    return;
                }
                continue;
            }
            message = socket.recv() => message,
        };
        let Some(message) = message else { return };
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

async fn readiness(
    Extension(pool): Extension<PgPool>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let result: Result<i32, sqlx::Error> = sqlx::query_scalar("SELECT 1").fetch_one(&pool).await;
    match result {
        Ok(_) => Ok(Json(json!({ "status": "ready", "database": "ready" }))),
        Err(error) => {
            tracing::error!(%error, "API readiness database check failed");
            Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({
                    "status": "not_ready",
                    "database": "unavailable"
                })),
            ))
        }
    }
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
    #[serde(default)]
    allow_waiting: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanRouteAssignmentResponse {
    state: &'static str,
    media_path: &'static str,
    estimate: RouteResourceEstimate,
    assignment: Option<PlannedAssignment>,
    rejections: Vec<arc_domain::WorkerRejection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteResourceRequest {
    expected_revision: i64,
}

async fn delete_source(
    Path(id): Path<Uuid>,
    Extension(pool): Extension<PgPool>,
    Json(request): Json<DeleteResourceRequest>,
) -> Result<Json<Value>, ApiError> {
    validate_delete_revision(&request)?;
    catalog::delete_source(&pool, id, request.expected_revision)
        .await
        .map_err(ApiError::resource_delete)?;
    Ok(Json(json!({ "deleted": true })))
}

async fn delete_destination(
    Path(id): Path<Uuid>,
    Extension(pool): Extension<PgPool>,
    Json(request): Json<DeleteResourceRequest>,
) -> Result<Json<Value>, ApiError> {
    validate_delete_revision(&request)?;
    catalog::delete_destination(&pool, id, request.expected_revision)
        .await
        .map_err(ApiError::resource_delete)?;
    Ok(Json(json!({ "deleted": true })))
}

fn validate_delete_revision(request: &DeleteResourceRequest) -> Result<(), ApiError> {
    if request.expected_revision <= 0 {
        return Err(ApiError::invalid_request(
            "A positive expectedRevision is required",
        ));
    }
    Ok(())
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

#[derive(Serialize)]
struct RouteCollection {
    items: Vec<RouteRecord>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetDesiredRouteRequest {
    source_id: Uuid,
    destination_id: Uuid,
    expected_route_revision: i64,
    #[serde(default)]
    confirm_replace: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetDesiredRoutesBatchRequest {
    routes: Vec<SetDesiredRouteRequest>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavePresetRequest {
    name: String,
    routes: Vec<PresetRoute>,
    expected_revision: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PresetRevisionRequest {
    expected_revision: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecutePresetRequest {
    expected_revision: i64,
    routes: Vec<ExecutePresetRouteRequest>,
}

#[derive(Deserialize)]
struct ExecutePresetRouteRequest {
    #[serde(flatten)]
    route: SetDesiredRouteRequest,
    #[serde(default)]
    action: RouteAction,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnrouteRequest {
    destination_id: Uuid,
    expected_route_revision: i64,
}

async fn unroute(
    headers: HeaderMap,
    Extension(pool): Extension<PgPool>,
    Json(request): Json<UnrouteRequest>,
) -> Result<Json<RouteBatchOperation>, ApiError> {
    let _slot = route_execution_slot().await?;
    PgRoutes::new(pool)
        .unroute(
            route_idempotency_key(&headers)?,
            request.destination_id,
            request.expected_route_revision,
        )
        .await
        .map(Json)
        .map_err(ApiError::route_write)
}

async fn list_presets(Extension(pool): Extension<PgPool>) -> Result<Json<Vec<Preset>>, ApiError> {
    PgRoutes::new(pool)
        .list_presets()
        .await
        .map(Json)
        .map_err(ApiError::route_write)
}

async fn create_preset(
    Extension(pool): Extension<PgPool>,
    Json(request): Json<SavePresetRequest>,
) -> Result<Json<Preset>, ApiError> {
    PgRoutes::new(pool)
        .save_preset(None, None, request.name, request.routes)
        .await
        .map(Json)
        .map_err(ApiError::route_write)
}

async fn update_preset(
    Path(id): Path<Uuid>,
    Extension(pool): Extension<PgPool>,
    Json(request): Json<SavePresetRequest>,
) -> Result<Json<Preset>, ApiError> {
    let revision = request
        .expected_revision
        .ok_or_else(|| ApiError::invalid_request("expectedRevision is required"))?;
    PgRoutes::new(pool)
        .save_preset(Some(id), Some(revision), request.name, request.routes)
        .await
        .map(Json)
        .map_err(ApiError::route_write)
}

async fn delete_preset(
    Path(id): Path<Uuid>,
    Extension(pool): Extension<PgPool>,
    Json(request): Json<PresetRevisionRequest>,
) -> Result<Json<Value>, ApiError> {
    PgRoutes::new(pool)
        .delete_preset(id, request.expected_revision)
        .await
        .map_err(ApiError::route_write)?;
    Ok(Json(json!({ "deleted": true })))
}

fn route_idempotency_key(headers: &HeaderMap) -> Result<Uuid, ApiError> {
    headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Uuid::parse_str(value).ok())
        .filter(|key| !key.is_nil())
        .ok_or_else(|| ApiError::invalid_request("Idempotency-Key header must be a non-nil UUID"))
}

fn desired_changes(routes: Vec<SetDesiredRouteRequest>) -> Vec<DesiredRouteChange> {
    routes
        .into_iter()
        .map(|route| DesiredRouteChange {
            source_id: route.source_id,
            destination_id: route.destination_id,
            expected_route_revision: route.expected_route_revision,
            confirm_replace: route.confirm_replace,
            action: RouteAction::Start,
        })
        .collect()
}

async fn execute_preset(
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Extension(pool): Extension<PgPool>,
    Json(request): Json<ExecutePresetRequest>,
) -> Result<Json<RouteBatchOperation>, ApiError> {
    let _slot = route_execution_slot().await?;
    let key = route_idempotency_key(&headers)?;
    PgRoutes::new(pool)
        .execute_preset(
            key,
            request
                .routes
                .into_iter()
                .map(|change| DesiredRouteChange {
                    source_id: change.route.source_id,
                    destination_id: change.route.destination_id,
                    expected_route_revision: change.route.expected_route_revision,
                    confirm_replace: change.route.confirm_replace,
                    action: change.action,
                })
                .collect(),
            id,
            request.expected_revision,
        )
        .await
        .map(Json)
        .map_err(ApiError::route_write)
}

async fn list_routes(
    Extension(pool): Extension<PgPool>,
) -> Result<Json<RouteCollection>, ApiError> {
    PgRoutes::new(pool)
        .list_current()
        .await
        .map(|items| Json(RouteCollection { items }))
        .map_err(|error| ApiError::database("list routes", error))
}

async fn set_desired_route(
    Extension(pool): Extension<PgPool>,
    Json(request): Json<SetDesiredRouteRequest>,
) -> Result<Json<RouteRecord>, ApiError> {
    let _slot = route_execution_slot().await?;
    if request.expected_route_revision < 0 {
        return Err(ApiError::invalid_request(
            "expectedRouteRevision must be zero or greater",
        ));
    }

    PgRoutes::new(pool)
        .set_desired_route(
            request.source_id,
            request.destination_id,
            request.expected_route_revision,
            request.confirm_replace,
        )
        .await
        .map(Json)
        .map_err(ApiError::route_write)
}

async fn set_desired_routes_batch(
    headers: HeaderMap,
    Extension(pool): Extension<PgPool>,
    Json(request): Json<SetDesiredRoutesBatchRequest>,
) -> Result<Json<RouteBatchOperation>, ApiError> {
    let _slot = route_execution_slot().await?;
    let idempotency_key = route_idempotency_key(&headers)?;
    if request
        .routes
        .iter()
        .any(|route| route.expected_route_revision < 0)
    {
        return Err(ApiError::invalid_request(
            "expectedRouteRevision must be zero or greater",
        ));
    }

    let changes = desired_changes(request.routes);
    PgRoutes::new(pool)
        .set_desired_routes_batch(idempotency_key, changes)
        .await
        .map(Json)
        .map_err(ApiError::route_write)
}

async fn get_route_batch_operation(
    Path(operation_id): Path<Uuid>,
    Extension(pool): Extension<PgPool>,
) -> Result<Json<RouteBatchOperation>, ApiError> {
    PgRoutes::new(pool)
        .get_route_batch_operation(operation_id)
        .await
        .map_err(ApiError::route_write)?
        .map(Json)
        .ok_or_else(|| ApiError {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message: "The requested route operation does not exist".to_owned(),
        })
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
    let route = RouteRevision {
        source_id: request.source_id,
        source_revision: request.source_revision,
        destination_id: request.destination_id,
        destination_revision: request.destination_revision,
    };
    if request.allow_waiting {
        let snapshot = PgCapacityWait::new(pool)
            .enqueue(idempotency_key, request.pipeline_id, route)
            .await
            .map_err(ApiError::pipeline_planning)?;
        return Ok(Json(capacity_response(snapshot)));
    }
    let planner = PgPipelinePlanner::new(pool);
    let decision = planner
        .plan_with_idempotency_key(
            idempotency_key,
            request.pipeline_id,
            route,
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
        request_id: None,
        expires_at: None,
        reason: None,
    }))
}

fn capacity_response(snapshot: CapacityWaitRequest) -> PlanRouteAssignmentResponse {
    let assignment = match snapshot.result {
        Some(PipelinePlanningDecision::Planned(planned)) => Some(planned.assignment),
        _ => None,
    };
    let state = match snapshot.state.as_str() {
        "waiting" => "waiting",
        "planned" => "planned",
        "cancelled" => "cancelled",
        _ => "unavailable",
    };
    PlanRouteAssignmentResponse {
        state,
        media_path: snapshot.estimate.media_path.as_str(),
        estimate: RouteResourceEstimate {
            capacity_units: snapshot.estimate.requirements.capacity_units,
            ingress_payload_bits_per_second: snapshot.estimate.ingress_payload_bits_per_second,
            egress_payload_bits_per_second: snapshot.estimate.egress_payload_bits_per_second,
        },
        assignment,
        rejections: Vec::new(),
        request_id: Some(snapshot.request_id),
        expires_at: Some(snapshot.expires_at),
        reason: snapshot.reason,
    }
}

async fn get_capacity_request(
    Path(request_id): Path<Uuid>,
    Extension(pool): Extension<PgPool>,
) -> Result<Json<PlanRouteAssignmentResponse>, ApiError> {
    capacity_status(PgCapacityWait::new(pool).get(request_id).await)
}

async fn cancel_capacity_request(
    Path(request_id): Path<Uuid>,
    Extension(pool): Extension<PgPool>,
) -> Result<Json<PlanRouteAssignmentResponse>, ApiError> {
    capacity_status(PgCapacityWait::new(pool).cancel(request_id).await)
}

fn capacity_status(
    result: arc_application::PortResult<Option<CapacityWaitRequest>>,
) -> Result<Json<PlanRouteAssignmentResponse>, ApiError> {
    result
        .map_err(ApiError::pipeline_planning)?
        .map(capacity_response)
        .map(Json)
        .ok_or_else(|| ApiError {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message: "Capacity request does not exist".to_owned(),
        })
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
    let assignments = PgAssignments::new(pool.clone());
    let snapshot = assignments
        .get_by_id(assignment_id)
        .await
        .map_err(ApiError::assignment)?
        .ok_or_else(|| ApiError {
            status: StatusCode::NOT_FOUND,
            code: "assignment_not_found",
            message: "The requested assignment does not exist".to_owned(),
        })?;
    let mut guard = pool
        .begin()
        .await
        .map_err(|error| ApiError::database("lock assignment", error))?;
    if let Some(route) = &snapshot.assignment.route {
        lock_destination(&mut guard, route.destination_id)
            .await
            .map_err(|error| ApiError::database("lock assignment destination", error))?;
        let cleanup_required: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM route_batch_items \
             WHERE pipeline_id = $1 AND \
             ((state IN ('pending', 'processing') AND attempts > 0) \
              OR error_code = 'start_cleanup_failed'))",
        )
        .bind(snapshot.assignment.pipeline_id)
        .fetch_one(&mut *guard)
        .await
        .map_err(|error| ApiError::database("check assignment cleanup", error))?;
        if cleanup_required {
            return Err(ApiError {
                status: StatusCode::CONFLICT,
                code: "assignment_cleanup_required",
                message: "The route operation must confirm cleanup before this assignment can be cancelled".to_owned(),
            });
        }
    }
    assignments
        .cancel_unstarted(assignment_id, request.generation)
        .await
        .map_err(ApiError::assignment)?;
    guard
        .commit()
        .await
        .map_err(|error| ApiError::database("unlock assignment destination", error))?;
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
    let _slot = route_execution_slot().await?;
    let snapshot = PgAssignments::new(pool.clone())
        .get_by_id(assignment_id)
        .await
        .map_err(ApiError::assignment)?
        .ok_or_else(|| ApiError {
            status: StatusCode::NOT_FOUND,
            code: "assignment_not_found",
            message: "The requested assignment does not exist".to_owned(),
        })?;
    let mut guard = pool
        .begin()
        .await
        .map_err(|error| ApiError::database("lock assignment", error))?;
    if let Some(route) = &snapshot.assignment.route {
        lock_destination(&mut guard, route.destination_id)
            .await
            .map_err(|error| ApiError::database("lock assignment destination", error))?;
        let removed: bool = sqlx::query_scalar(
            "SELECT COALESCE((SELECT desired_state = 'removed' FROM route_instances \
             WHERE destination_id = $1 ORDER BY route_revision DESC LIMIT 1), FALSE)",
        )
        .bind(route.destination_id)
        .fetch_one(&mut *guard)
        .await
        .map_err(|error| ApiError::database("check desired route", error))?;
        if removed {
            return Err(ApiError {
                status: StatusCode::CONFLICT,
                code: "route_removed",
                message: "Set a desired route before starting this assignment".to_owned(),
            });
        }
    }
    let result = start_assignment_for(pool, dispatcher, assignment_id, request.generation).await;
    guard
        .commit()
        .await
        .map_err(|error| ApiError::database("unlock assignment destination", error))?;
    result.map(Json)
}

async fn start_assignment_for(
    pool: PgPool,
    dispatcher: Arc<WorkerCommandDispatcher>,
    assignment_id: Uuid,
    generation: i64,
) -> Result<AssignmentSnapshot, ApiError> {
    let assignments = PgAssignments::new(pool);
    let snapshot = assignments
        .begin_start(assignment_id, generation)
        .await
        .map_err(ApiError::assignment)?;
    if snapshot.observed_state == "running" {
        return Ok(snapshot);
    }

    let command = start_command(&snapshot.assignment)?;
    let result = match dispatcher.dispatch(command).await {
        Ok(result) => result,
        Err(error) => {
            if matches!(
                error.code,
                "worker_session_unavailable"
                    | "worker_session_replaced"
                    | "worker_session_busy"
                    | "worker_command_too_large"
                    | "worker_command_failed"
            ) {
                assignments
                    .reset_start(assignment_id, generation)
                    .await
                    .map_err(ApiError::assignment)?;
            }
            return Err(ApiError::worker_dispatch(error));
        }
    };

    match result {
        CommandDispatchResult::Applied | CommandDispatchResult::Duplicate => assignments
            .mark_started(assignment_id, generation)
            .await
            .map_err(ApiError::assignment),
        CommandDispatchResult::Stale { current_generation } => {
            assignments
                .reset_start(assignment_id, generation)
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
    let _slot = route_execution_slot().await?;
    let snapshot = PgAssignments::new(pool.clone())
        .get_by_id(assignment_id)
        .await
        .map_err(ApiError::assignment)?
        .ok_or_else(|| ApiError {
            status: StatusCode::NOT_FOUND,
            code: "assignment_not_found",
            message: "The requested assignment does not exist".to_owned(),
        })?;
    let mut guard = pool
        .begin()
        .await
        .map_err(|error| ApiError::database("lock assignment", error))?;
    if let Some(route) = &snapshot.assignment.route {
        lock_destination(&mut guard, route.destination_id)
            .await
            .map_err(|error| ApiError::database("lock assignment destination", error))?;
    }
    let result = stop_assignment_for(pool, dispatcher, assignment_id, request.generation).await;
    guard
        .commit()
        .await
        .map_err(|error| ApiError::database("unlock assignment destination", error))?;
    result.map(Json)
}

async fn stop_assignment_for(
    pool: PgPool,
    dispatcher: Arc<WorkerCommandDispatcher>,
    assignment_id: Uuid,
    generation: i64,
) -> Result<AssignmentSnapshot, ApiError> {
    let assignments = PgAssignments::new(pool);
    let snapshot = assignments
        .begin_stop(assignment_id, generation)
        .await
        .map_err(ApiError::assignment)?;
    if snapshot.observed_state == "stopped" {
        return Ok(snapshot);
    }

    match dispatcher
        .dispatch(stop_command(&snapshot.assignment))
        .await
    {
        Ok(CommandDispatchResult::Applied | CommandDispatchResult::Duplicate) => assignments
            .mark_stopped(assignment_id, generation)
            .await
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

async fn prepare_assignment_for(
    dispatcher: Arc<WorkerCommandDispatcher>,
    assignment: &PlannedAssignment,
) -> Result<(), ApiError> {
    match dispatcher.dispatch(prepare_command(assignment)?).await {
        Ok(CommandDispatchResult::Applied | CommandDispatchResult::Duplicate) => Ok(()),
        Ok(CommandDispatchResult::Stale { current_generation }) => {
            Err(ApiError::worker_dispatch(PortError {
                code: "stale_worker_command",
                message: format!("worker has already accepted generation {current_generation}"),
            }))
        }
        Err(error) => Err(ApiError::worker_dispatch(error)),
    }
}

fn pipeline_specification(
    assignment: &PlannedAssignment,
) -> Result<PipelineSpecification, ApiError> {
    let route = assignment.route.as_ref().ok_or_else(|| ApiError {
        status: StatusCode::CONFLICT,
        code: "assignment_conflict",
        message: "assignment has no pinned route configuration to start".to_owned(),
    })?;
    Ok(PipelineSpecification {
        pipeline_id: assignment.pipeline_id,
        assignment_generation: assignment.generation,
        source_identity: Some(arc_application::PipelineSourceIdentity {
            source_id: route.source_id,
            source_revision: route.source_revision,
        }),
        requirements: assignment.requirements.clone(),
        configuration: route.configuration.clone(),
    })
}

fn prepare_command(
    assignment: &PlannedAssignment,
) -> Result<WorkerCommandEnvelope<WorkerCommand>, ApiError> {
    Ok(WorkerCommandEnvelope {
        protocol_version: WORKER_PROTOCOL_VERSION,
        command_id: Uuid::new_v5(&assignment.id, b"prepare"),
        worker_id: assignment.worker_id,
        pipeline_id: assignment.pipeline_id,
        assignment_id: assignment.id,
        generation: assignment.generation,
        payload: WorkerCommand::Prepare {
            specification: pipeline_specification(assignment)?,
        },
    })
}

fn start_command(
    assignment: &PlannedAssignment,
) -> Result<WorkerCommandEnvelope<WorkerCommand>, ApiError> {
    Ok(WorkerCommandEnvelope {
        protocol_version: WORKER_PROTOCOL_VERSION,
        command_id: assignment.id,
        worker_id: assignment.worker_id,
        pipeline_id: assignment.pipeline_id,
        assignment_id: assignment.id,
        generation: assignment.generation,
        payload: WorkerCommand::Start {
            specification: pipeline_specification(assignment)?,
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

    fn resource_delete(error: ResourceDeleteError) -> Self {
        let (status, code, message) = match error {
            ResourceDeleteError::NotFound => (
                StatusCode::NOT_FOUND,
                "not_found",
                "The resource does not exist",
            ),
            ResourceDeleteError::Enabled => (
                StatusCode::CONFLICT,
                "resource_enabled",
                "Unroute all active assignments before deleting this resource",
            ),
            ResourceDeleteError::StaleRevision => (
                StatusCode::CONFLICT,
                "stale_resource_revision",
                "The resource changed; reload it before deleting",
            ),
            ResourceDeleteError::InUse => (
                StatusCode::CONFLICT,
                "resource_in_use",
                "Routes, history, assignments, signals or saved salvos reference this resource",
            ),
            ResourceDeleteError::Database(error) => {
                return Self::database("delete resource", error)
            }
        };
        Self {
            status,
            code,
            message: message.to_owned(),
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
            ResourceUpdateError::InUse => Self {
                status: StatusCode::CONFLICT,
                code: "resource_in_use",
                message: "Stop previewing this input before editing it".to_owned(),
            },
            ResourceUpdateError::Database(error) => Self::database("update resource", error),
        }
    }

    fn route_write(error: RouteWriteError) -> Self {
        match error {
            RouteWriteError::InvalidBatch => Self::invalid_request(
                "A route batch must contain 1 to 64 routes with no duplicate destinations",
            ),
            RouteWriteError::InvalidIdempotencyKey => {
                Self::invalid_request("Idempotency-Key header must be a non-nil UUID")
            }
            RouteWriteError::IdempotencyKeyReused => Self {
                status: StatusCode::CONFLICT,
                code: "idempotency_key_reused",
                message: "This idempotency key was already used for a different route batch"
                    .to_owned(),
            },
            RouteWriteError::SourceNotFound | RouteWriteError::DestinationNotFound => Self {
                status: StatusCode::NOT_FOUND,
                code: "not_found",
                message: "The requested source or destination does not exist".to_owned(),
            },
            RouteWriteError::SourceNotValidated => Self {
                status: StatusCode::CONFLICT,
                code: "source_not_validated",
                message: "Validate the source before assigning it to a route".to_owned(),
            },
            RouteWriteError::DestinationNotValidated => Self {
                status: StatusCode::CONFLICT,
                code: "destination_not_validated",
                message: "Validate the destination before assigning a route".to_owned(),
            },
            RouteWriteError::StaleRouteRevision { expected, current } => Self {
                status: StatusCode::CONFLICT,
                code: "stale_route_revision",
                message: format!(
                    "Expected route revision {expected}, but the current revision is {current}"
                ),
            },
            RouteWriteError::ReplacementNotConfirmed => Self {
                status: StatusCode::CONFLICT,
                code: "replacement_confirmation_required",
                message: "Confirm replacement of the currently desired route".to_owned(),
            },
            RouteWriteError::PresetNotFound => Self {
                status: StatusCode::NOT_FOUND,
                code: "not_found",
                message: "The requested preset does not exist".to_owned(),
            },
            RouteWriteError::PresetConflict => Self {
                status: StatusCode::CONFLICT,
                code: "preset_conflict",
                message: "The preset changed, its routes do not match, or its name is already in use".to_owned(),
            },
            RouteWriteError::InvalidPreset => Self::invalid_request(
                "A preset needs a name of 1 to 100 characters and 1 to 64 routes with distinct destinations",
            ),
            RouteWriteError::PresetLimit => Self {
                status: StatusCode::CONFLICT,
                code: "preset_limit",
                message: "The limit of 100 presets has been reached".to_owned(),
            },
            RouteWriteError::DestinationBusy => Self {
                status: StatusCode::CONFLICT,
                code: "destination_busy",
                message: "A route operation for this destination is still in progress".to_owned(),
            },
            RouteWriteError::RouteNotFound => Self {
                status: StatusCode::NOT_FOUND,
                code: "not_found",
                message: "No desired route or assignment exists for this destination".to_owned(),
            },
            RouteWriteError::InvalidMediaProfile(error) => Self::invalid_media_profile(error),
            RouteWriteError::InvalidStoredProfile(error) => {
                tracing::error!(%error, "stored route media profile is invalid");
                Self {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    code: "internal_error",
                    message: "A stored media profile could not be read".to_owned(),
                }
            }
            RouteWriteError::InvalidStoredOperation(error) => {
                tracing::error!(%error, "stored route operation is invalid");
                Self {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    code: "internal_error",
                    message: "A stored route operation could not be read".to_owned(),
                }
            }
            RouteWriteError::Database(error) => Self::database("desired route operation", error),
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
        tracing::error!(%error, operation, "database operation failed");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "database_error",
            message: "Database operation failed".to_owned(),
        }
    }

    fn pipeline_planning(error: arc_application::PortError) -> Self {
        let (status, code) = match error.code {
            "source_not_found" | "destination_not_found" => (StatusCode::NOT_FOUND, "not_found"),
            "stale_route_revision"
            | "source_not_validated"
            | "destination_not_validated"
            | "assignment_conflict"
            | "destination_in_use"
            | "reservation_conflict"
            | "source_worker_affinity_conflict"
            | "source_worker_unavailable"
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
            "worker_session_unavailable"
            | "worker_session_busy"
            | "worker_session_ended"
            | "worker_session_replaced"
            | "worker_session_registration_failed" => Self {
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
        net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
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
    use arc_gstreamer::GstreamerMediaPipeline;
    use arc_infrastructure::{
        assignments::PgAssignments,
        catalog,
        postgres::{connect_postgres, migrate},
        routes::{PgRoutes, PresetRoute, RouteBatchItemCompletion},
        worker_fencing::PgAssignmentGenerationStore,
        worker_registry::PgWorkerRegistry,
    };
    use arc_worker::{CommandDispatchResult, WorkerControlClient, WorkerControlClientConfig};
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
        Router,
    };
    use chrono::Utc;
    use futures_util::{SinkExt, StreamExt};
    use gstreamer_app::AppSink;
    use serde_json::{json, Value};
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

    #[tokio::test]
    #[ignore = "requires an isolated PostgreSQL database; set DATABASE_URL and run ignored tests"]
    async fn fenced_worker_sessions_reject_legacy_and_wrong_incarnations() {
        let pool = connect_postgres(&env::var("DATABASE_URL").unwrap())
            .await
            .unwrap();
        migrate(&pool).await.unwrap();
        let worker_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO workers \
             (id, stable_name, platform, architecture, version, state, last_heartbeat) \
             VALUES ($1, $2, 'linux', 'x86_64', 'test', 'ready', clock_timestamp())",
        )
        .bind(worker_id)
        .bind(format!("session-fence-{worker_id}"))
        .execute(&pool)
        .await
        .unwrap();
        let dispatcher = WorkerCommandDispatcher::new_with_lease_pool(
            Arc::new(PgAssignmentGenerationStore::new(pool.clone())),
            pool.clone(),
        );
        let legacy = dispatcher.start_session(worker_id, None).await.unwrap();
        dispatcher.discard_session(legacy).await;
        let incarnation = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO preview_worker_process_fences \
             (worker_id, incarnation_id, process_identity) VALUES ($1, $2, '{}'::jsonb)",
        )
        .bind(worker_id)
        .bind(incarnation)
        .execute(&pool)
        .await
        .unwrap();
        assert!(dispatcher.start_session(worker_id, None).await.is_err());
        assert!(dispatcher
            .start_session(worker_id, Some(Uuid::new_v4()))
            .await
            .is_err());
        let current = dispatcher
            .start_session(worker_id, Some(incarnation))
            .await
            .unwrap();
        assert!(dispatcher.start_session(worker_id, None).await.is_err());
        assert!(dispatcher
            .is_current_session(worker_id, current.session_id)
            .await
            .unwrap());
        dispatcher.discard_session(current).await;
    }

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
            process_incarnation: None,
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
    async fn worker_preview_control_correlates_completion_and_retains_route_session() {
        let worker_id = Uuid::new_v4();
        let preview_id = Uuid::new_v4();
        let (address, server, dispatcher, _) = start_worker_dispatch_server(worker_id).await;
        let (mut socket, _) = connect_async(worker_websocket_request(
            address, worker_id, "worker-secret", "https",
        )).await.unwrap();
        let hello = arc_shared::WorkerSessionHello {
            kind: arc_shared::WorkerSessionMessageKind::Hello,
            protocol_version: arc_shared::WORKER_PROTOCOL_VERSION,
            worker_id,
            process_incarnation: None,
        };
        socket.send(ClientMessage::Text(serde_json::to_string(&hello).unwrap().into())).await.unwrap();
        assert!(matches!(socket.next().await.unwrap().unwrap(), ClientMessage::Text(_)));
        let session = dispatcher.preview_session(worker_id).await.unwrap();
        let control = arc_worker::WorkerPreviewControl::PreviewStart {
            protocol_version: arc_shared::WORKER_PROTOCOL_VERSION,
            worker_id,
            preview_id,
            identity: arc_application::PipelineSourceIdentity {
                source_id: Uuid::new_v4(), source_revision: 1,
            },
            source: SourceMediaProfile::AlsaL16 { device: "test".to_owned() },
            reserved_source: false,
        };
        let completion = session.start(control, preview_id).unwrap();
        let ClientMessage::Text(start) = socket.next().await.unwrap().unwrap() else {
            panic!("expected preview start");
        };
        let start: arc_worker::WorkerPreviewControl = serde_json::from_str(start.as_str()).unwrap();
        assert!(matches!(start, arc_worker::WorkerPreviewControl::PreviewStart { preview_id: id, .. } if id == preview_id));
        let event = arc_worker::WorkerPreviewFinished {
            kind: arc_worker::WorkerPreviewEventKind::PreviewFinished,
            protocol_version: arc_shared::WORKER_PROTOCOL_VERSION,
            worker_id,
            preview_id,
            error_code: Some("preview_unsupported".to_owned()),
            error_message: Some("unsupported source".to_owned()),
            cleanup_confirmed: false,
        };
        socket.send(ClientMessage::Text(serde_json::to_string(&event).unwrap().into())).await.unwrap();
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), completion)
            .await.unwrap().unwrap().unwrap();
        assert_eq!(event.error_code.as_deref(), Some("preview_unsupported"));
        socket.send(ClientMessage::Ping(vec![1].into())).await.unwrap();
        assert_eq!(socket.next().await.unwrap().unwrap(), ClientMessage::Pong(vec![1].into()));
        socket.close(None).await.unwrap();
        server.abort();
    }

    #[tokio::test]
    #[ignore = "requires an isolated PostgreSQL database; set DATABASE_URL and run ignored tests"]
    async fn preview_relay_authenticates_streams_and_closes_after_session_revocation() {
        test_preview_relay(false, false, false).await;
    }

    #[tokio::test]
    #[ignore = "requires an isolated PostgreSQL database; set DATABASE_URL and run ignored tests"]
    async fn preview_relay_closes_when_worker_session_is_replaced() {
        test_preview_relay(true, false, false).await;
    }

    #[tokio::test]
    #[ignore = "requires isolated PostgreSQL and ARC_API_ENABLE_FENCED_PREVIEW=true"]
    async fn fenced_preview_relay_covers_idle_and_running_inputs() {
        assert_eq!(env::var("ARC_API_ENABLE_FENCED_PREVIEW").unwrap(), "true");
        test_preview_relay(false, true, false).await;
        test_preview_relay(true, true, false).await;
        test_preview_relay(false, true, true).await;
    }

    async fn test_preview_relay(replace_worker: bool, reserved: bool, browser_stop: bool) {
        use arc_application::{AudioPreview, MediaPipeline, PipelineObservation, PipelineSourceIdentity, PipelineSpecification};
        use arc_infrastructure::capacity_reservations::PgCapacityReservations;
        use arc_worker::PreviewSessionFinalizer;
        use sha2::{Digest, Sha256};
        use sqlx::PgPool;
        use std::sync::atomic::{AtomicBool, Ordering};

        struct TestAudio { sent: bool, closed: Arc<AtomicBool> }
        #[async_trait::async_trait]
        impl AudioPreview for TestAudio {
            async fn recv(&mut self) -> PortResult<Vec<u8>> {
                if self.sent { std::future::pending().await } else {
                    self.sent = true;
                    Ok(vec![1, 2, 3, 4])
                }
            }
            async fn close(&mut self) -> PortResult<()> {
                self.closed.store(true, Ordering::SeqCst);
                Ok(())
            }
        }
        struct TestMedia(Arc<AtomicBool>);
        #[async_trait::async_trait]
        impl MediaPipeline for TestMedia {
            async fn prepare(&self, _: &PipelineSpecification) -> PortResult<()> { Ok(()) }
            async fn start(&self, _: Uuid, _: i64) -> PortResult<()> { Ok(()) }
            async fn stop(&self, _: Uuid, _: i64) -> PortResult<()> { Ok(()) }
            async fn observe(&self, _: Uuid) -> PortResult<PipelineObservation> {
                Err(PortError { code: "test_unsupported", message: "not used".to_owned() })
            }
            async fn start_active_preview(
                self: Arc<Self>, _: PipelineSourceIdentity, _: &SourceMediaProfile,
            ) -> PortResult<Box<dyn AudioPreview>> {
                Ok(Box::new(TestAudio { sent: false, closed: Arc::clone(&self.0) }))
            }
            async fn start_reserved_preview(
                self: Arc<Self>, identity: PipelineSourceIdentity, source: &SourceMediaProfile,
            ) -> PortResult<Box<dyn AudioPreview>> {
                self.start_active_preview(identity, source).await
            }
        }
        struct TestFinalizer(PgPool);
        #[async_trait::async_trait]
        impl PreviewSessionFinalizer for TestFinalizer {
            async fn finish(&self, worker_id: Uuid, session_id: Uuid) -> PortResult<()> {
                PgCapacityReservations::new(self.0.clone()).finish_preview_session(worker_id, session_id).await?;
                Ok(())
            }
        }

        const WORKER_TOKEN: &str = "abababababababababababababababababababababababababababababababab";
        let session_token = Uuid::new_v4().to_string();
        let pool = connect_postgres(&env::var("DATABASE_URL").unwrap()).await.unwrap();
        migrate(&pool).await.unwrap();
        let worker_id = Uuid::new_v4();
        let source_id = Uuid::new_v4();
        let destination_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();
        let admin_session_id = Uuid::new_v4();
        let incarnation = Uuid::new_v4();
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("INSERT INTO workers (id, stable_name, platform, architecture, version, state, last_heartbeat) VALUES ($1, $2, 'linux', 'x86_64', 'test', 'ready', now())")
            .bind(worker_id).bind(worker_id.to_string()).execute(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO sources (id, friendly_name, source_type, desired_state, current_revision) VALUES ($1, 'preview input', 'alsa_l16', 'enabled', 1)")
            .bind(source_id).execute(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO source_revisions (id, source_id, revision, lifecycle, configuration) VALUES ($1, $2, 1, 'validated', $3)")
            .bind(Uuid::new_v4()).bind(source_id).bind(json!({"kind":"alsa_l16","device":"test"})).execute(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO destinations (id, friendly_name, destination_type, desired_state, current_revision) VALUES ($1, 'preview output', 'rtp_l16', 'enabled', 1)")
            .bind(destination_id).execute(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO destination_revisions (id, destination_id, revision, lifecycle, configuration) VALUES ($1, $2, 1, 'validated', $3)")
            .bind(Uuid::new_v4()).bind(destination_id).bind(json!({"kind":"rtp_l16","address":"127.0.0.1","port":5000,"payloadType":96})).execute(&mut *tx).await.unwrap();
        if reserved {
            let capabilities = WorkerCapabilities {
                platform: Some(Platform::Linux), architecture: Some(Architecture::X86_64),
                source_adapters: BTreeSet::from(["alsa_l16".to_owned()]),
                audio_backends: BTreeSet::from(["alsa".to_owned()]),
                ..WorkerCapabilities::default()
            };
            let capacity = WorkerCapacity {
                total_capacity_units: 4, assigned_capacity_units: 0,
                cpu_utilization_ratio: None, memory_utilization_ratio: None,
                ingress_bits_per_second: None, egress_bits_per_second: None,
            };
            sqlx::query("UPDATE workers SET state = 'accepting', capabilities = $2, capacity = $3 WHERE id = $1")
                .bind(worker_id).bind(serde_json::to_value(capabilities).unwrap()).bind(serde_json::to_value(capacity).unwrap())
                .execute(&mut *tx).await.unwrap();
            sqlx::query("INSERT INTO preview_worker_process_fences (worker_id, incarnation_id, process_identity) VALUES ($1, $2, '{}')")
                .bind(worker_id).bind(incarnation).execute(&mut *tx).await.unwrap();
        }
        if !reserved || replace_worker {
            sqlx::query("INSERT INTO pipeline_assignments (id, resource_type, resource_id, worker_id, generation, desired_state, observed_state, source_id, source_revision, destination_id, destination_revision, media_path, route_configuration) VALUES ($1, 'pipeline', $2, $3, 1, 'running', 'running', $4, 1, $5, 1, 'alsa_l16_to_rtp_l16', '{}')")
                .bind(Uuid::new_v4()).bind(Uuid::new_v4()).bind(worker_id).bind(source_id).bind(destination_id).execute(&mut *tx).await.unwrap();
        }
        sqlx::query("INSERT INTO arc_admin_users (id, username, password_hash) VALUES ($1, $2, 'test')")
            .bind(user_id).bind(user_id.to_string()).execute(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO arc_admin_sessions (id, user_id, session_token_hash, csrf_token, expires_at) VALUES ($1, $2, $3, 'test', now() + interval '1 hour')")
            .bind(admin_session_id).bind(user_id).bind(Sha256::digest(session_token.as_bytes()).to_vec()).execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();

        let store = Arc::new(TestAssignmentGenerationStore::default());
        let dispatcher = Arc::new(WorkerCommandDispatcher::new_with_lease_pool(store.clone(), pool.clone()));
        let app = super::router_with_database_worker_dispatcher_and_admin_auth(
            pool.clone(), Arc::new(TestWorkerAuthenticator { worker_id, token: WORKER_TOKEN, available: true }),
            dispatcher, super::AdminAuth::for_test(pool.clone()),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let closed = Arc::new(AtomicBool::new(false));
        let client = WorkerControlClient::new(
            WorkerControlClientConfig {
                endpoint: format!("ws://{address}/api/v1/workers/control"),
                ca_certificates_file: None, worker_id, bearer_token: WORKER_TOKEN.to_owned(),
                bearer_token_file: None, allow_insecure_loopback: true,
            }, store, Arc::new(TestMedia(Arc::clone(&closed))),
        ).unwrap();
        let client = if reserved {
            client.with_process_incarnation(incarnation).unwrap()
                .with_preview_finalizer(Arc::new(TestFinalizer(pool.clone())))
        } else {
            client
        };
        let mut status = client.connection_status();
        let worker = tokio::spawn(async move { client.run_session().await });
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !*status.borrow() { status.changed().await.unwrap(); }
        }).await.unwrap();
        let url = format!("ws://{address}/api/v1/sources/{source_id}/preview");
        let mut request = url.as_str().into_client_request().unwrap();
        request.headers_mut().insert("x-forwarded-proto", HeaderValue::from_static("http"));
        request.headers_mut().insert("origin", HeaderValue::from_str(&format!("http://{address}")).unwrap());
        request.headers_mut().insert("cookie", HeaderValue::from_str(&format!("arc_admin_session={session_token}")).unwrap());
        let mut forged = request.clone();
        forged.headers_mut().insert("origin", HeaderValue::from_static("http://attacker.invalid"));
        let error = connect_async(forged).await.unwrap_err();
        assert!(matches!(error, tokio_tungstenite::tungstenite::Error::Http(response) if response.status() == StatusCode::FORBIDDEN));
        let (mut browser, _) = connect_async(request).await.unwrap();
        let audio = tokio::time::timeout(std::time::Duration::from_secs(5), browser.next()).await.unwrap().unwrap().unwrap();
        assert_eq!(audio, ClientMessage::Binary(vec![1, 2, 3, 4].into()));
        if browser_stop {
            browser.send(ClientMessage::Close(None)).await.unwrap();
        } else if replace_worker {
            sqlx::query("UPDATE worker_control_sessions SET session_id = $1 WHERE worker_id = $2")
                .bind(Uuid::new_v4()).bind(worker_id).execute(&pool).await.unwrap();
        } else {
            sqlx::query("UPDATE arc_admin_sessions SET revoked_at = clock_timestamp() WHERE id = $1")
                .bind(admin_session_id).execute(&pool).await.unwrap();
        }
        let close = tokio::time::timeout(std::time::Duration::from_secs(4), browser.next()).await.unwrap().unwrap().unwrap();
        if browser_stop {
            assert!(matches!(close, ClientMessage::Close(_)));
        } else if replace_worker {
            assert!(matches!(close, ClientMessage::Close(Some(frame)) if frame.reason == "worker_session_ended"));
        } else {
            assert!(matches!(close, ClientMessage::Close(Some(frame)) if frame.reason == "preview_session_revoked"));
        }
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !closed.load(Ordering::SeqCst) { tokio::time::sleep(std::time::Duration::from_millis(10)).await; }
        }).await.unwrap();
        if reserved {
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let active: i64 = sqlx::query_scalar("SELECT count(*) FROM preview_capacity_reservations WHERE source_id = $1")
                        .bind(source_id).fetch_one(&pool).await.unwrap();
                    if active == 0 { break; }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            }).await.unwrap();
        }
        server.abort();
        worker.abort();
        let _ = worker.await;
        sqlx::query("DELETE FROM worker_control_sessions WHERE worker_id = $1")
            .bind(worker_id).execute(&pool).await.unwrap();
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
            process_incarnation: None,
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
            process_incarnation: None,
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
            process_incarnation: None,
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
            process_incarnation: None,
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
    #[ignore = "requires an isolated PostgreSQL database; set DATABASE_URL and run ignored tests"]
    async fn readiness_route_reports_database_health() {
        let pool = connect_postgres(&env::var("DATABASE_URL").expect("DATABASE_URL is set"))
            .await
            .expect("database connects");
        migrate(&pool).await.expect("database migrates");

        let response = router_with_database(pool)
            .oneshot(
                Request::builder()
                    .uri("/api/v1/health/ready")
                    .body(Body::empty())
                    .expect("readiness request builds"),
            )
            .await
            .expect("readiness request completes");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("readiness response reads");
        let value: Value = serde_json::from_slice(&body).expect("readiness response is JSON");
        assert_eq!(value["status"], "ready");
        assert_eq!(value["database"], "ready");
    }

    #[tokio::test]
    #[ignore = "requires an isolated PostgreSQL database; set DATABASE_URL and run ignored tests"]
    async fn catalog_delete_is_revision_guarded_and_only_removes_disabled_unused_resources() {
        let pool = connect_postgres(&env::var("DATABASE_URL").expect("DATABASE_URL is set"))
            .await
            .expect("database connects");
        migrate(&pool).await.expect("database migrates");
        let source = catalog::create_source(&pool, "Unused input", "test", json!({}))
            .await
            .expect("source persists");
        let destination = catalog::create_destination(&pool, "Unused output", "test", json!({}))
            .await
            .expect("destination persists");
        catalog::update_source(&pool, source.id, "Edited input", "test", json!({}))
            .await
            .expect("source has two revisions");
        catalog::update_destination(&pool, destination.id, "Edited output", "test", json!({}))
            .await
            .expect("destination has two revisions");
        let app = router_with_database(pool.clone());
        for (table, id, revision_table, foreign_key) in [
            ("sources", source.id, "source_revisions", "source_id"),
            (
                "destinations",
                destination.id,
                "destination_revisions",
                "destination_id",
            ),
        ] {
            let (status, body) = delete_catalog_request(app.clone(), table, id, 1).await;
            assert_eq!(status, StatusCode::CONFLICT);
            assert_eq!(body["code"], "stale_resource_revision");
            let (status, body) = delete_catalog_request(app.clone(), table, id, 0).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
            assert_eq!(body["code"], "invalid_request");
            sqlx::query(&format!(
                "UPDATE {table} SET desired_state = 'enabled' WHERE id = $1"
            ))
            .bind(id)
            .execute(&pool)
            .await
            .expect("resource is enabled");
            let (status, body) = delete_catalog_request(app.clone(), table, id, 2).await;
            assert_eq!(status, StatusCode::CONFLICT);
            assert_eq!(body["code"], "resource_enabled");
            sqlx::query(&format!(
                "UPDATE {table} SET desired_state = 'disabled' WHERE id = $1"
            ))
            .bind(id)
            .execute(&pool)
            .await
            .expect("resource is disabled");
            let (status, body) = delete_catalog_request(app.clone(), table, id, 2).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(body, json!({ "deleted": true }));
            let revisions: i64 = sqlx::query_scalar(&format!(
                "SELECT COUNT(*) FROM {revision_table} WHERE {foreign_key} = $1"
            ))
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("revision count reads");
            assert_eq!(
                revisions, 0,
                "unused draft revisions are deleted with their parent"
            );
            let (status, body) = delete_catalog_request(app.clone(), table, id, 2).await;
            assert_eq!(status, StatusCode::NOT_FOUND);
            assert_eq!(body["code"], "not_found");
        }
    }

    async fn delete_catalog_request(
        app: Router,
        table: &str,
        id: Uuid,
        revision: i64,
    ) -> (StatusCode, Value) {
        let response = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/v1/{table}/{id}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({ "expectedRevision": revision }).to_string(),
                    ))
                    .expect("delete request builds"),
            )
            .await
            .expect("delete request completes");
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("delete response reads");
        (
            status,
            serde_json::from_slice(&body).expect("delete response is JSON"),
        )
    }

    #[tokio::test]
    #[ignore = "requires an isolated PostgreSQL database; set DATABASE_URL and run ignored tests"]
    async fn catalog_delete_preserves_salvos_and_removed_route_history() {
        let pool = connect_postgres(&env::var("DATABASE_URL").expect("DATABASE_URL is set"))
            .await
            .expect("database connects");
        migrate(&pool).await.expect("database migrates");
        let source = catalog::create_source(
            &pool,
            "Referenced input",
            "rtp_l24",
            json!({"kind":"rtp_l24", "bindAddress":"0.0.0.0", "port":5000, "payloadType":96}),
        )
        .await
        .expect("source persists");
        let destination = catalog::create_destination(
            &pool,
            "Referenced output",
            "rtp_l24",
            json!({"kind":"rtp_l24", "address":"127.0.0.1", "port":6000, "payloadType":96}),
        )
        .await
        .expect("destination persists");
        catalog::validate_source_draft(&pool, source.id)
            .await
            .expect("source is validated");
        catalog::validate_destination_draft(&pool, destination.id)
            .await
            .expect("destination is validated");
        let routes = PgRoutes::new(pool.clone());
        let preset = routes
            .save_preset(
                None,
                None,
                format!("Delete guard {}", Uuid::new_v4()),
                vec![PresetRoute {
                    source_id: source.id,
                    destination_id: destination.id,
                    action: super::RouteAction::Start,
                }],
            )
            .await
            .expect("salvo persists");
        let app = router_with_database(pool.clone());
        for (table, id) in [("sources", source.id), ("destinations", destination.id)] {
            let (status, body) = delete_catalog_request(app.clone(), table, id, 1).await;
            assert_eq!(status, StatusCode::CONFLICT);
            assert_eq!(body["code"], "resource_in_use");
        }
        routes
            .delete_preset(preset.id, preset.revision)
            .await
            .expect("salvo is explicitly removed");
        let route_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO route_instances \
             (id, source_id, destination_id, route_revision, global_revision, desired_state) \
             VALUES ($1, $2, $3, 1, 1, 'removed')",
        )
        .bind(route_id)
        .bind(source.id)
        .bind(destination.id)
        .execute(&pool)
        .await
        .expect("stopped historical route persists");
        for (table, id, revisions, foreign_key) in [
            ("sources", source.id, "source_revisions", "source_id"),
            (
                "destinations",
                destination.id,
                "destination_revisions",
                "destination_id",
            ),
        ] {
            let (status, body) = delete_catalog_request(app.clone(), table, id, 1).await;
            assert_eq!(status, StatusCode::CONFLICT);
            assert_eq!(body["code"], "resource_in_use");
            let count: i64 = sqlx::query_scalar(&format!(
                "SELECT COUNT(*) FROM {revisions} WHERE {foreign_key} = $1"
            ))
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("revision remains readable");
            assert_eq!(count, 1, "failed deletion rolls back revision cascades");
        }
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM route_instances WHERE id = $1")
            .bind(route_id)
            .fetch_one(&pool)
            .await
            .expect("history remains readable");
        assert_eq!(count, 1);
    }

    #[tokio::test]
    #[ignore = "requires an isolated PostgreSQL database; set DATABASE_URL and run ignored tests"]
    async fn routes_api_returns_latest_desired_and_observed_state_per_destination() {
        let pool = connect_postgres(&env::var("DATABASE_URL").expect("DATABASE_URL is set"))
            .await
            .expect("database connects");
        migrate(&pool).await.expect("database migrates");

        let source_a =
            catalog::create_source(&pool, "Route list source A", "test", serde_json::json!({}))
                .await
                .expect("first source persists");
        let source_b =
            catalog::create_source(&pool, "Route list source B", "test", serde_json::json!({}))
                .await
                .expect("second source persists");
        let destination_a = catalog::create_destination(
            &pool,
            "Route list destination A",
            "test",
            serde_json::json!({}),
        )
        .await
        .expect("first destination persists");
        let destination_b = catalog::create_destination(
            &pool,
            "Route list destination B",
            "test",
            serde_json::json!({}),
        )
        .await
        .expect("second destination persists");

        let removed_route_id = Uuid::new_v4();
        let desired_route_id = Uuid::new_v4();
        let unknown_route_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO route_instances \
             (id, source_id, destination_id, route_revision, global_revision, desired_state) \
             VALUES ($1, $2, $3, 1, 1, 'removed'), \
                    ($4, $5, $3, 2, 2, 'desired'), \
                    ($6, $2, $7, 1, 3, 'desired')",
        )
        .bind(removed_route_id)
        .bind(source_a.id)
        .bind(destination_a.id)
        .bind(desired_route_id)
        .bind(source_b.id)
        .bind(unknown_route_id)
        .bind(destination_b.id)
        .execute(&pool)
        .await
        .expect("route history persists");
        sqlx::query(
            "INSERT INTO route_events \
             (id, route_instance_id, observed_state, occurred_at) \
             VALUES ($1, $2, 'stopped', now() - interval '1 second'), \
                    ($3, $4, 'running', now())",
        )
        .bind(Uuid::new_v4())
        .bind(removed_route_id)
        .bind(Uuid::new_v4())
        .bind(desired_route_id)
        .execute(&pool)
        .await
        .expect("route observations persist");

        let response = router_with_database(pool)
            .oneshot(
                Request::builder()
                    .uri("/api/v1/routes")
                    .body(Body::empty())
                    .expect("route collection request builds"),
            )
            .await
            .expect("route collection request completes");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("route collection response reads");
        let value: Value = serde_json::from_slice(&body).expect("response is JSON");
        let items = value["items"].as_array().expect("items is an array");

        let destination_a_id = destination_a.id.to_string();
        let destination_a_routes: Vec<_> = items
            .iter()
            .filter(|item| item["destinationId"].as_str() == Some(destination_a_id.as_str()))
            .collect();
        assert_eq!(destination_a_routes.len(), 1);
        assert_eq!(destination_a_routes[0]["sourceId"], source_b.id.to_string());
        assert_eq!(destination_a_routes[0]["desiredState"], "desired");
        assert_eq!(destination_a_routes[0]["observedState"], "running");
        assert_eq!(
            destination_a_routes[0]["observedSourceId"],
            source_b.id.to_string()
        );
        assert_eq!(destination_a_routes[0]["revision"], 2);

        let destination_b_id = destination_b.id.to_string();
        let destination_b_route = items
            .iter()
            .find(|item| item["destinationId"].as_str() == Some(destination_b_id.as_str()))
            .expect("second destination is listed");
        assert_eq!(destination_b_route["desiredState"], "desired");
        assert_eq!(destination_b_route["observedState"], "unknown");
        assert!(destination_b_route["observedSourceId"].is_null());
        assert_eq!(destination_b_route["revision"], 1);
    }

    #[tokio::test]
    #[ignore = "requires an isolated PostgreSQL database; set DATABASE_URL and run ignored tests"]
    async fn route_write_requires_revision_and_explicit_replacement_confirmation() {
        let pool = connect_postgres(&env::var("DATABASE_URL").expect("DATABASE_URL is set"))
            .await
            .expect("database connects");
        migrate(&pool).await.expect("database migrates");

        let source_profile = serde_json::json!({
            "kind": "rtp_l16",
            "bindAddress": "0.0.0.0",
            "port": 5000,
            "payloadType": 96
        });
        let destination_profile = serde_json::json!({
            "kind": "rtp_l16",
            "address": "127.0.0.1",
            "port": 5002,
            "payloadType": 96
        });
        let source_a = catalog::create_source(
            &pool,
            "Route write source A",
            "rtp_l16",
            source_profile.clone(),
        )
        .await
        .expect("first source persists");
        let source_b =
            catalog::create_source(&pool, "Route write source B", "rtp_l16", source_profile)
                .await
                .expect("second source persists");
        let destination = catalog::create_destination(
            &pool,
            "Route write destination",
            "rtp_l16",
            destination_profile,
        )
        .await
        .expect("destination persists");
        catalog::validate_source_draft(&pool, source_a.id)
            .await
            .expect("first source validates");
        catalog::validate_source_draft(&pool, source_b.id)
            .await
            .expect("second source validates");
        catalog::validate_destination_draft(&pool, destination.id)
            .await
            .expect("destination validates");
        let app = router_with_database(pool.clone());

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "sourceId": source_a.id,
                            "destinationId": destination.id,
                            "expectedRouteRevision": 0
                        })
                        .to_string(),
                    ))
                    .expect("initial route request builds"),
            )
            .await
            .expect("initial route request completes");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("initial route response reads");
        let route: Value = serde_json::from_slice(&body).expect("route response is JSON");
        assert_eq!(route["sourceId"], source_a.id.to_string());
        assert_eq!(route["desiredState"], "desired");
        assert_eq!(route["observedState"], "unknown");
        assert_eq!(route["revision"], 1);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "sourceId": source_a.id,
                            "destinationId": destination.id,
                            "expectedRouteRevision": 1
                        })
                        .to_string(),
                    ))
                    .expect("idempotent route request builds"),
            )
            .await
            .expect("idempotent route request completes");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("idempotent response reads");
        let repeated: Value =
            serde_json::from_slice(&body).expect("idempotent route response is JSON");
        assert_eq!(repeated["revision"], 1);

        let original_route_id = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM route_instances WHERE destination_id = $1 AND route_revision = 1",
        )
        .bind(destination.id)
        .fetch_one(&pool)
        .await
        .expect("original route id reads");
        sqlx::query(
            "INSERT INTO route_events (id, route_instance_id, observed_state) \
             VALUES ($1, $2, 'running')",
        )
        .bind(Uuid::new_v4())
        .bind(original_route_id)
        .execute(&pool)
        .await
        .expect("original route observation persists");

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "sourceId": source_b.id,
                            "destinationId": destination.id,
                            "expectedRouteRevision": 1
                        })
                        .to_string(),
                    ))
                    .expect("unconfirmed replacement request builds"),
            )
            .await
            .expect("unconfirmed replacement request completes");
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("replacement error response reads");
        let error: Value = serde_json::from_slice(&body).expect("error response is JSON");
        assert_eq!(error["code"], "replacement_confirmation_required");

        let routes = PgRoutes::new(pool.clone())
            .list_current()
            .await
            .expect("route state reads after rejected replacement");
        let route = routes
            .into_iter()
            .find(|route| route.destination_id == destination.id)
            .expect("test destination route remains unchanged");
        assert_eq!(route.source_id, source_a.id);
        assert_eq!(route.revision, 1);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "sourceId": source_b.id,
                            "destinationId": destination.id,
                            "expectedRouteRevision": 0,
                            "confirmReplace": true
                        })
                        .to_string(),
                    ))
                    .expect("stale replacement request builds"),
            )
            .await
            .expect("stale replacement request completes");
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("stale replacement response reads");
        let error: Value = serde_json::from_slice(&body).expect("stale error is JSON");
        assert_eq!(error["code"], "stale_route_revision");

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "sourceId": source_b.id,
                            "destinationId": destination.id,
                            "expectedRouteRevision": 1,
                            "confirmReplace": true
                        })
                        .to_string(),
                    ))
                    .expect("confirmed replacement request builds"),
            )
            .await
            .expect("confirmed replacement request completes");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("confirmed replacement response reads");
        let route: Value = serde_json::from_slice(&body).expect("route response is JSON");
        assert_eq!(route["sourceId"], source_b.id.to_string());
        assert_eq!(route["desiredState"], "desired");
        assert_eq!(route["observedState"], "running");
        assert_eq!(route["observedSourceId"], source_a.id.to_string());
        assert_eq!(route["revision"], 2);
    }

    #[tokio::test]
    #[ignore = "requires an isolated PostgreSQL database; set DATABASE_URL and run ignored tests"]
    async fn preset_crud_and_strict_execution_are_revision_guarded_and_idempotent() {
        let pool = connect_postgres(&env::var("DATABASE_URL").expect("DATABASE_URL is set"))
            .await
            .expect("database connects");
        migrate(&pool).await.expect("database migrates");
        let source = catalog::create_source(
            &pool,
            "Preset source",
            "rtp_l16",
            serde_json::json!({
                "kind": "rtp_l16", "bindAddress": "0.0.0.0", "port": 5000, "payloadType": 96
            }),
        )
        .await
        .expect("source persists");
        let destination = catalog::create_destination(
            &pool,
            "Preset destination",
            "rtp_l16",
            serde_json::json!({
                "kind": "rtp_l16", "address": "127.0.0.1", "port": 5002, "payloadType": 96
            }),
        )
        .await
        .expect("destination persists");
        catalog::validate_source_draft(&pool, source.id)
            .await
            .expect("source validates");
        catalog::validate_destination_draft(&pool, destination.id)
            .await
            .expect("destination validates");
        let app = router_with_database(pool.clone());
        let name = format!("preset-{}", Uuid::new_v4());
        let route = serde_json::json!({"sourceId": source.id, "destinationId": destination.id});
        let create = serde_json::json!({"name": name, "routes": [route]});
        let duplicate = serde_json::json!({"name": name, "routes": [route, route]});
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/presets")
                    .header("content-type", "application/json")
                    .body(Body::from(duplicate.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/presets")
                    .header("content-type", "application/json")
                    .body(Body::from(create.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let preset: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        let id = preset["id"].as_str().unwrap();
        assert_eq!(preset["revision"], 1);
        let list = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/presets")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let list: Value =
            serde_json::from_slice(&to_bytes(list.into_body(), usize::MAX).await.unwrap()).unwrap();
        assert!(list
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["id"] == id));
        let exec_uri = format!("/api/v1/presets/{id}/execute");
        let key = Uuid::new_v4().to_string();
        let execute = serde_json::json!({"expectedRevision": 1, "routes": [{
            "sourceId": source.id, "destinationId": destination.id, "expectedRouteRevision": 0, "confirmReplace": false
        }]});
        let send = |uri: &str, key: &str, body: &Value| {
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .header("idempotency-key", key)
                .body(Body::from(body.to_string()))
                .unwrap()
        };
        let stale = serde_json::json!({"expectedRevision": 2, "routes": execute["routes"]});
        let response = app
            .clone()
            .oneshot(send(&exec_uri, &Uuid::new_v4().to_string(), &stale))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let mismatch = serde_json::json!({"expectedRevision": 1, "routes": [{
            "sourceId": Uuid::new_v4(), "destinationId": destination.id, "expectedRouteRevision": 0, "confirmReplace": false
        }]});
        let response = app
            .clone()
            .oneshot(send(&exec_uri, &Uuid::new_v4().to_string(), &mismatch))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let stale_route = serde_json::json!({"expectedRevision": 1, "routes": [{
            "sourceId": source.id, "destinationId": destination.id, "expectedRouteRevision": 1, "confirmReplace": false
        }]});
        let response = app
            .clone()
            .oneshot(send(&exec_uri, &Uuid::new_v4().to_string(), &stale_route))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let response = app
            .clone()
            .oneshot(send(&exec_uri, &key, &execute))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let operation: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        let mut explicit_start = execute.clone();
        explicit_start["routes"][0]["action"] = serde_json::json!("start");
        let response = app
            .clone()
            .oneshot(send(&exec_uri, &key, &explicit_start))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "explicit Start must replay legacy requests"
        );
        let stored: Value = sqlx::query_scalar(
            "SELECT request FROM route_batch_operations WHERE idempotency_key = $1",
        )
        .bind(Uuid::parse_str(&key).unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(stored["routes"][0].get("action").is_none());
        let update = serde_json::json!({"name": name, "routes": [route], "expectedRevision": 1});
        let uri = format!("/api/v1/presets/{id}");
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(&uri)
                    .header("content-type", "application/json")
                    .body(Body::from(update.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(&uri)
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"expectedRevision":1}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(&uri)
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"expectedRevision":2}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let response = app
            .clone()
            .oneshot(send(&exec_uri, &key, &execute))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let retry: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(retry["operationId"], operation["operationId"]);
        let batch = serde_json::json!({"routes": execute["routes"]});
        let response = app
            .clone()
            .oneshot(send("/api/v1/routes/batch", &key, &batch))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let unroute_key = Uuid::new_v4().to_string();
        let unroute =
            serde_json::json!({"destinationId": destination.id, "expectedRouteRevision": 1});
        let response = app
            .clone()
            .oneshot(send("/api/v1/routes/unroute", &unroute_key, &unroute))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let stopped: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(stopped["routes"][0]["desiredState"], "removed");
        assert_eq!(stopped["routes"][0]["observedState"], "unknown");
        assert_eq!(stopped["state"], "pending");
        let response = app
            .clone()
            .oneshot(send("/api/v1/routes/unroute", &unroute_key, &unroute))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let retry: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(retry["operationId"], stopped["operationId"]);
        let operation_id = Uuid::parse_str(stopped["operationId"].as_str().unwrap()).unwrap();
        let lease = Uuid::new_v4();
        let item = sqlx::query_as::<_, super::RouteBatchWorkItem>(
            "UPDATE route_batch_items SET state = 'processing', attempts = 1, \
             lease_token = $2, lease_expires_at = clock_timestamp() + INTERVAL '60 seconds' \
             WHERE operation_id = $1 RETURNING operation_id, source_id, destination_id, source_revision, \
             destination_revision, route_revision, replacement_assignment_id, pipeline_id, planning_key, \
             error_code, error_message, attempts, lease_token, action",
        ).bind(operation_id).bind(lease).fetch_one(&pool).await.unwrap();
        let dispatcher = Arc::new(WorkerCommandDispatcher::new(Arc::new(
            TestAssignmentGenerationStore::default(),
        )));
        super::process_unroute_item(&PgRoutes::new(pool.clone()), &pool, &dispatcher, &item)
            .await
            .unwrap();
        let operation = PgRoutes::new(pool.clone())
            .get_route_batch_operation(operation_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(operation.state, "failed");
        assert_eq!(
            operation.results[0].error_code.as_deref(),
            Some("stop_unconfirmed")
        );
        assert_eq!(
            PgRoutes::new(pool.clone())
                .list_current()
                .await
                .unwrap()
                .into_iter()
                .find(|route| route.destination_id == destination.id)
                .unwrap()
                .observed_state,
            "unknown"
        );
        let response = app
            .oneshot(send(
                "/api/v1/routes/unroute",
                &Uuid::new_v4().to_string(),
                &unroute,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    #[ignore = "requires an isolated PostgreSQL database; set DATABASE_URL and run ignored tests"]
    async fn mixed_preset_actions_are_saved_revision_guarded_and_destination_wide() {
        let pool = connect_postgres(&env::var("DATABASE_URL").unwrap())
            .await
            .unwrap();
        migrate(&pool).await.unwrap();
        let source = catalog::create_source(&pool, "Mixed source", "rtp_l16",
            serde_json::json!({"kind": "rtp_l16", "bindAddress": "0.0.0.0", "port": 5000, "payloadType": 96}))
            .await.unwrap();
        catalog::validate_source_draft(&pool, source.id)
            .await
            .unwrap();
        let anchor = catalog::create_source(
            &pool,
            "Unvalidated Unroute anchor",
            "alsa_l16",
            serde_json::json!({"kind": "alsa_l16", "device": "hw:1,0"}),
        )
        .await
        .unwrap();
        let mut outputs = Vec::new();
        for index in 0..3 {
            let output = catalog::create_destination(&pool, &format!("Mixed output {index}"), "rtp_l16",
                serde_json::json!({"kind": "rtp_l16", "address": "127.0.0.1", "port": 5002 + index * 2, "payloadType": 96}))
                .await.unwrap();
            catalog::validate_destination_draft(&pool, output.id)
                .await
                .unwrap();
            outputs.push(output.id);
        }
        let routes = PgRoutes::new(pool.clone());
        routes
            .set_desired_route(source.id, outputs[1], 0, false)
            .await
            .unwrap();
        let app = router_with_database(pool.clone());
        let saved_routes = serde_json::json!([
            {"sourceId": source.id, "destinationId": outputs[0]},
            {"sourceId": anchor.id, "destinationId": outputs[1], "action": "stop"},
            {"sourceId": anchor.id, "destinationId": outputs[2], "action": "stop"}
        ]);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/presets")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "name": format!("mixed-{}", Uuid::new_v4()), "routes": saved_routes,
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let preset: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        let id = Uuid::parse_str(preset["id"].as_str().unwrap()).unwrap();
        let listed = routes
            .list_presets()
            .await
            .unwrap()
            .into_iter()
            .find(|saved| saved.id == id)
            .unwrap();
        assert_eq!(
            listed
                .routes
                .iter()
                .filter(|route| route.action == super::RouteAction::Stop)
                .count(),
            2
        );
        let uri = format!("/api/v1/presets/{id}/execute");
        let execute = serde_json::json!({"expectedRevision": 1, "routes": [
            {"sourceId": source.id, "destinationId": outputs[0], "expectedRouteRevision": 0},
            {"sourceId": anchor.id, "destinationId": outputs[1], "expectedRouteRevision": 1, "action": "stop"},
            {"sourceId": anchor.id, "destinationId": outputs[2], "expectedRouteRevision": 0, "action": "stop"}
        ]});
        let send = |body: &Value, key: Uuid| {
            Request::builder()
                .method("POST")
                .uri(&uri)
                .header("content-type", "application/json")
                .header("idempotency-key", key.to_string())
                .body(Body::from(body.to_string()))
                .unwrap()
        };
        let mut tampered = execute.clone();
        tampered["routes"][1]["action"] = serde_json::json!("start");
        assert_eq!(
            app.clone()
                .oneshot(send(&tampered, Uuid::new_v4()))
                .await
                .unwrap()
                .status(),
            StatusCode::CONFLICT
        );
        for index in 0..3 {
            let mut stale = execute.clone();
            stale["routes"][index]["expectedRouteRevision"] = serde_json::json!(9);
            assert_eq!(
                app.clone()
                    .oneshot(send(&stale, Uuid::new_v4()))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::CONFLICT
            );
            let unchanged = routes.list_current().await.unwrap();
            assert!(!unchanged
                .iter()
                .any(|route| route.destination_id == outputs[0]
                    || route.destination_id == outputs[2]));
            assert_eq!(
                unchanged
                    .iter()
                    .find(|route| route.destination_id == outputs[1])
                    .unwrap()
                    .revision,
                1
            );
        }
        let key = Uuid::new_v4();
        let response = app.clone().oneshot(send(&execute, key)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let accepted: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        let stopped = accepted["routes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|route| route["destinationId"] == outputs[1].to_string())
            .unwrap();
        assert_eq!(
            stopped["sourceId"],
            source.id.to_string(),
            "Unroute removes the actual input, not its saved anchor"
        );
        assert_eq!(stopped["desiredState"], "removed");
        let results = accepted["results"].as_array().unwrap();
        assert_eq!(
            results
                .iter()
                .find(|item| item["destinationId"] == outputs[1].to_string())
                .unwrap()["state"],
            "pending",
            "a desired route without confirmed Stop must not become an idle no-op"
        );
        let idle = results
            .iter()
            .find(|item| item["destinationId"] == outputs[2].to_string())
            .unwrap();
        assert_eq!(idle["state"], "succeeded");
        assert_eq!(idle["action"], "stop");
        routes.delete_preset(id, 1).await.unwrap();
        let replay = app.oneshot(send(&execute, key)).await.unwrap();
        assert_eq!(replay.status(), StatusCode::OK);
        let replay: Value =
            serde_json::from_slice(&to_bytes(replay.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(replay["operationId"], accepted["operationId"]);
    }

    #[tokio::test]
    #[ignore = "requires an isolated PostgreSQL database; set DATABASE_URL and run ignored tests"]
    async fn desired_route_batch_is_atomic_idempotent_and_retrievable() {
        let pool = connect_postgres(&env::var("DATABASE_URL").expect("DATABASE_URL is set"))
            .await
            .expect("database connects");
        migrate(&pool).await.expect("database migrates");

        let source_profile = serde_json::json!({
            "kind": "rtp_l16",
            "bindAddress": "0.0.0.0",
            "port": 5000,
            "payloadType": 96
        });
        let destination_profile = serde_json::json!({
            "kind": "rtp_l16",
            "address": "127.0.0.1",
            "port": 5002,
            "payloadType": 96
        });
        let source_a =
            catalog::create_source(&pool, "Batch source A", "rtp_l16", source_profile.clone())
                .await
                .expect("first source persists");
        let source_b = catalog::create_source(&pool, "Batch source B", "rtp_l16", source_profile)
            .await
            .expect("second source persists");
        let destination_a = catalog::create_destination(
            &pool,
            "Batch destination A",
            "rtp_l16",
            destination_profile.clone(),
        )
        .await
        .expect("first destination persists");
        let destination_b = catalog::create_destination(
            &pool,
            "Batch destination B",
            "rtp_l16",
            destination_profile,
        )
        .await
        .expect("second destination persists");
        for source_id in [source_a.id, source_b.id] {
            catalog::validate_source_draft(&pool, source_id)
                .await
                .expect("source validates");
        }
        for destination_id in [destination_a.id, destination_b.id] {
            catalog::validate_destination_draft(&pool, destination_id)
                .await
                .expect("destination validates");
        }
        let app = router_with_database(pool.clone());

        let invalid_revision_key = Uuid::new_v4();
        let invalid_revision_batch = serde_json::json!({
            "routes": [
                {
                    "sourceId": source_a.id,
                    "destinationId": destination_a.id,
                    "expectedRouteRevision": 0,
                    "confirmReplace": false
                },
                {
                    "sourceId": source_b.id,
                    "destinationId": destination_b.id,
                    "expectedRouteRevision": 1,
                    "confirmReplace": false
                }
            ]
        });
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/batch")
                    .header("content-type", "application/json")
                    .header("idempotency-key", invalid_revision_key.to_string())
                    .body(Body::from(invalid_revision_batch.to_string()))
                    .expect("invalid batch request builds"),
            )
            .await
            .expect("invalid batch request completes");
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let route_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM route_instances WHERE destination_id IN ($1, $2)",
        )
        .bind(destination_a.id)
        .bind(destination_b.id)
        .fetch_one(&pool)
        .await
        .expect("route count reads after rollback");
        assert_eq!(route_count, 0);

        let idempotency_key = Uuid::new_v4();
        let first_route = serde_json::json!({
            "sourceId": source_a.id,
            "destinationId": destination_a.id,
            "expectedRouteRevision": 0,
            "confirmReplace": false
        });
        let second_route = serde_json::json!({
            "sourceId": source_b.id,
            "destinationId": destination_b.id,
            "expectedRouteRevision": 0,
            "confirmReplace": false
        });
        let request_body = serde_json::json!({
            "routes": [first_route, second_route]
        });
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/batch")
                    .header("content-type", "application/json")
                    .header("idempotency-key", idempotency_key.to_string())
                    .body(Body::from(request_body.to_string()))
                    .expect("valid batch request builds"),
            )
            .await
            .expect("valid batch request completes");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("batch response reads");
        let operation: Value = serde_json::from_slice(&body).expect("batch response is JSON");
        assert_eq!(operation["state"], "pending");
        assert_eq!(operation["routes"].as_array().unwrap().len(), 2);
        assert_eq!(operation["results"].as_array().unwrap().len(), 2);
        assert!(operation["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|result| result["state"] == "pending"));
        let operation_id = operation["operationId"]
            .as_str()
            .expect("operation ID is present")
            .to_owned();

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/batch")
                    .header("content-type", "application/json")
                    .header("idempotency-key", idempotency_key.to_string())
                    .body(Body::from(
                        serde_json::json!({
                            "routes": [second_route, first_route]
                        })
                        .to_string(),
                    ))
                    .expect("replay request builds"),
            )
            .await
            .expect("replay request completes");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("replay response reads");
        let replay: Value = serde_json::from_slice(&body).expect("replay is JSON");
        assert_eq!(replay["operationId"], operation_id);
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM route_instances WHERE destination_id IN ($1, $2)",
            )
            .bind(destination_a.id)
            .bind(destination_b.id)
            .fetch_one(&pool)
            .await
            .expect("route count reads after replay"),
            2
        );

        let changed_batch = serde_json::json!({
            "routes": [{
                "sourceId": source_b.id,
                "destinationId": destination_a.id,
                "expectedRouteRevision": 0,
                "confirmReplace": false
            }]
        });
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/batch")
                    .header("content-type", "application/json")
                    .header("idempotency-key", idempotency_key.to_string())
                    .body(Body::from(changed_batch.to_string()))
                    .expect("changed replay request builds"),
            )
            .await
            .expect("changed replay request completes");
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("changed replay response reads");
        let error: Value = serde_json::from_slice(&body).expect("error response is JSON");
        assert_eq!(error["code"], "idempotency_key_reused");

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/operations/routes/{operation_id}"))
                    .body(Body::empty())
                    .expect("operation read request builds"),
            )
            .await
            .expect("operation read request completes");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("operation read response reads");
        let stored_operation: Value =
            serde_json::from_slice(&body).expect("stored operation is JSON");
        assert_eq!(stored_operation, operation);

        let routes = PgRoutes::new(pool.clone());
        let first = routes
            .claim_next_batch_item()
            .await
            .expect("first route batch item claims")
            .expect("route batch item exists");
        sqlx::query(
            "UPDATE route_batch_items SET lease_expires_at = clock_timestamp() - INTERVAL '1 second' \
             WHERE operation_id = $1 AND destination_id = $2",
        )
        .bind(first.operation_id)
        .bind(first.destination_id)
        .execute(&pool)
        .await
        .expect("processing lease expires for recovery test");
        let reclaimed = routes
            .claim_next_batch_item()
            .await
            .expect("expired route batch item is reclaimed")
            .expect("expired route batch item is available");
        assert_eq!(reclaimed.destination_id, first.destination_id);
        assert_ne!(reclaimed.lease_token, first.lease_token);
        assert!(routes
            .finish_batch_item(
                &first,
                RouteBatchItemCompletion::Succeeded {
                    assignment_id: Uuid::new_v4(),
                    generation: 1,
                },
            )
            .await
            .is_err());
        routes
            .finish_batch_item(
                &reclaimed,
                RouteBatchItemCompletion::Succeeded {
                    assignment_id: Uuid::new_v4(),
                    generation: 1,
                },
            )
            .await
            .expect("recovered batch item completes");
        let second = routes
            .claim_next_batch_item()
            .await
            .expect("second route batch item claims")
            .expect("second route batch item exists");
        routes
            .finish_batch_item(
                &second,
                RouteBatchItemCompletion::Failed {
                    assignment: None,
                    error_code: "test_failure",
                    error_message: "test route failure",
                },
            )
            .await
            .expect("failed route result is recorded");
        let partial = routes
            .get_route_batch_operation(operation_id.parse().expect("operation ID is UUID"))
            .await
            .expect("route operation can be reloaded")
            .expect("route operation exists");
        assert_eq!(partial.state, "partial");
        assert_eq!(
            partial
                .results
                .iter()
                .filter(|result| result.state == "succeeded")
                .count(),
            1
        );
        assert_eq!(
            partial
                .results
                .iter()
                .filter(|result| result.state == "failed")
                .count(),
            1
        );
    }

    #[tokio::test]
    #[ignore = "requires an isolated PostgreSQL database; set DATABASE_URL and run ignored tests"]
    async fn route_batch_execution_dispatches_and_records_worker_acknowledgement() {
        let pool = connect_postgres(&env::var("DATABASE_URL").expect("DATABASE_URL is set"))
            .await
            .expect("database connects");
        migrate(&pool).await.expect("database migrates");
        sqlx::query("TRUNCATE TABLE workers CASCADE")
            .execute(&pool)
            .await
            .expect("worker state is isolated");

        let worker_id = Uuid::new_v4();
        let mut worker = Worker {
            id: worker_id,
            stable_name: format!("route-batch-{worker_id}"),
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
            port: 5200,
            payload_type: 96,
        };
        let destination_profile = DestinationMediaProfile::RtpL16 {
            address,
            port: 5202,
            payload_type: 96,
        };
        let source = catalog::create_source(
            &pool,
            "Route batch source",
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
            "Route batch destination",
            destination_profile.adapter_id(),
            serde_json::to_value(&destination_profile).expect("destination profile serializes"),
        )
        .await
        .expect("destination creates");
        let destination = catalog::validate_destination_draft(&pool, destination.id)
            .await
            .expect("destination validates");
        let replacement_sources = [5204, 5206]
            .into_iter()
            .map(|port| SourceMediaProfile::RtpL16 {
                bind_address: address,
                port,
                payload_type: 96,
            })
            .collect::<Vec<_>>();
        let mut replacement_source_ids = Vec::new();
        for (index, profile) in replacement_sources.iter().enumerate() {
            let source = catalog::create_source(
                &pool,
                &format!("Replacement source {index}"),
                profile.adapter_id(),
                serde_json::to_value(profile).expect("replacement profile serializes"),
            )
            .await
            .expect("replacement source creates");
            let source = catalog::validate_source_draft(&pool, source.id)
                .await
                .expect("replacement source validates");
            replacement_source_ids.push(source.id);
        }

        let dispatcher = Arc::new(WorkerCommandDispatcher::new_with_lease_pool(
            Arc::new(PgAssignmentGenerationStore::new(pool.clone())),
            pool.clone(),
        ));
        let app = router_with_database_and_worker_dispatcher(
            pool.clone(),
            Arc::new(TestWorkerAuthenticator {
                worker_id,
                token: "worker-secret",
                available: true,
            }),
            dispatcher,
        );
        let direct_app = app.clone();
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("API listener binds");
        let api_address = listener.local_addr().expect("API listener has an address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("API server runs");
        });

        let (mut socket, _) = connect_async(worker_websocket_request(
            api_address,
            worker_id,
            "worker-secret",
            "https",
        ))
        .await
        .expect("worker control connection upgrades");
        let hello = arc_shared::WorkerSessionHello {
            kind: arc_shared::WorkerSessionMessageKind::Hello,
            protocol_version: arc_shared::WORKER_PROTOCOL_VERSION,
            worker_id,
            process_incarnation: None,
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

        let idempotency_key = Uuid::new_v4();
        let request = serde_json::json!({
            "routes": [{
                "sourceId": source.id,
                "destinationId": destination.id,
                "expectedRouteRevision": 0,
                "confirmReplace": false
            }]
        });
        let response = router_with_database(pool.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/batch")
                    .header("content-type", "application/json")
                    .header("idempotency-key", idempotency_key.to_string())
                    .body(Body::from(request.to_string()))
                    .expect("route batch request builds"),
            )
            .await
            .expect("route batch request completes");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("route batch response reads");
        let operation: Value =
            serde_json::from_slice(&body).expect("route batch response is valid JSON");
        let operation_id = Uuid::parse_str(
            operation["operationId"]
                .as_str()
                .expect("operation ID is returned"),
        )
        .expect("operation ID is valid");

        let command_message =
            tokio::time::timeout(std::time::Duration::from_secs(10), socket.next())
                .await
                .expect("route batch dispatch reaches the worker")
                .expect("worker session remains open")
                .expect("worker receives route batch command");
        let ClientMessage::Text(command_message) = command_message else {
            panic!("expected a text Start command");
        };
        let command: Value =
            serde_json::from_str(command_message.as_str()).expect("worker command is valid JSON");
        let command_id = Uuid::parse_str(
            command["commandId"]
                .as_str()
                .expect("worker command ID is present"),
        )
        .expect("worker command ID is valid");
        let acknowledgement = arc_shared::WorkerCommandAcknowledgement {
            kind: arc_shared::WorkerSessionMessageKind::CommandAck,
            protocol_version: arc_shared::WORKER_PROTOCOL_VERSION,
            worker_id,
            command_id,
            outcome: arc_shared::WorkerCommandOutcome::Completed,
            current_generation: None,
            error_code: None,
            error_message: None,
        };
        socket
            .send(ClientMessage::Text(
                serde_json::to_string(&acknowledgement)
                    .expect("worker acknowledgement serializes")
                    .into(),
            ))
            .await
            .expect("worker acknowledgement sends");

        let routes = PgRoutes::new(pool.clone());
        let operation = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let operation = routes
                    .get_route_batch_operation(operation_id)
                    .await
                    .expect("operation status reads")
                    .expect("operation remains persisted");
                if operation.state == "completed" {
                    break operation;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("route batch records the worker acknowledgement");
        assert_eq!(operation.results.len(), 1);
        assert_eq!(operation.results[0].state, "succeeded");
        assert!(operation.results[0].assignment_id.is_some());
        assert_eq!(operation.results[0].destination_id, destination.id);

        let previous_assignment_id = operation.results[0]
            .assignment_id
            .expect("initial assignment ID is returned");
        let previous_assignment = PgAssignments::new(pool.clone())
            .get_by_id(previous_assignment_id)
            .await
            .expect("initial assignment reads")
            .expect("initial assignment exists");
        assert_eq!(previous_assignment.observed_state, "running");

        let unchanged_route = serde_json::json!({
            "sourceId": source.id,
            "destinationId": destination.id,
            "expectedRouteRevision": 1,
            "confirmReplace": false
        });
        let preset = routes
            .save_preset(
                None,
                None,
                format!("running-{}", Uuid::new_v4()),
                vec![arc_infrastructure::routes::PresetRoute {
                    source_id: source.id,
                    destination_id: destination.id,
                    action: super::RouteAction::Start,
                }],
            )
            .await
            .expect("running route preset saves");
        for (uri, body) in [
            (
                "/api/v1/routes/batch".to_owned(),
                serde_json::json!({ "routes": [unchanged_route] }),
            ),
            (
                format!("/api/v1/presets/{}/execute", preset.id),
                serde_json::json!({ "expectedRevision": 1, "routes": [unchanged_route] }),
            ),
        ] {
            let response = router_with_database(pool.clone())
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(uri)
                        .header("content-type", "application/json")
                        .header("idempotency-key", Uuid::new_v4().to_string())
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let unchanged: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(unchanged["state"], "completed");
            assert_eq!(unchanged["results"][0]["state"], "succeeded");
            assert_eq!(
                unchanged["results"][0]["assignmentId"],
                previous_assignment_id.to_string()
            );
            assert_eq!(
                unchanged["results"][0]["pipelineId"],
                previous_assignment.assignment.pipeline_id.to_string()
            );
        }
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(300), socket.next())
                .await
                .is_err(),
            "an already-running route must not dispatch another worker command"
        );
        routes
            .delete_preset(preset.id, 1)
            .await
            .expect("test preset deletes");

        let make_acknowledgement =
            |command_id: Uuid,
             outcome: arc_shared::WorkerCommandOutcome,
             error_code: Option<&str>,
             error_message: Option<&str>| {
                arc_shared::WorkerCommandAcknowledgement {
                    kind: arc_shared::WorkerSessionMessageKind::CommandAck,
                    protocol_version: arc_shared::WORKER_PROTOCOL_VERSION,
                    worker_id,
                    command_id,
                    outcome,
                    current_generation: None,
                    error_code: error_code.map(str::to_owned),
                    error_message: error_message.map(str::to_owned),
                }
            };
        macro_rules! next_worker_command {
            ($description:literal) => {{
                let message =
                    tokio::time::timeout(std::time::Duration::from_secs(10), socket.next())
                        .await
                        .expect($description)
                        .expect("worker session remains open")
                        .expect("worker command is valid websocket data");
                let ClientMessage::Text(message) = message else {
                    panic!("expected a text worker command");
                };
                serde_json::from_str::<Value>(message.as_str())
                    .expect("worker command is valid JSON")
            }};
        }
        macro_rules! acknowledge_worker_command {
            ($command:expr, $outcome:expr, $code:expr, $message:expr) => {{
                let command_id = Uuid::parse_str(
                    $command["commandId"]
                        .as_str()
                        .expect("worker command ID is present"),
                )
                .expect("worker command ID is valid");
                let acknowledgement = make_acknowledgement(command_id, $outcome, $code, $message);
                socket
                    .send(ClientMessage::Text(
                        serde_json::to_string(&acknowledgement)
                            .expect("worker acknowledgement serializes")
                            .into(),
                    ))
                    .await
                    .expect("worker acknowledgement sends");
            }};
        }

        let failed_output = catalog::create_destination(
            &pool,
            "Failed Take recovery destination",
            destination_profile.adapter_id(),
            serde_json::to_value(&destination_profile).unwrap(),
        )
        .await
        .unwrap();
        let failed_output = catalog::validate_destination_draft(&pool, failed_output.id)
            .await
            .unwrap();
        let failed_request = serde_json::json!({"routes": [{
            "sourceId": source.id, "destinationId": failed_output.id,
            "expectedRouteRevision": 0, "confirmReplace": false
        }]});
        let response = router_with_database(pool.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/batch")
                    .header("content-type", "application/json")
                    .header("idempotency-key", Uuid::new_v4().to_string())
                    .body(Body::from(failed_request.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let response: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        let failed_operation_id =
            Uuid::parse_str(response["operationId"].as_str().unwrap()).unwrap();
        let failed_start = next_worker_command!("failed Take reaches worker");
        assert_eq!(failed_start["payload"]["kind"], "start");
        acknowledge_worker_command!(
            failed_start,
            arc_shared::WorkerCommandOutcome::Failed,
            Some("gstreamer_pipeline_start_failed"),
            Some("injected start failure")
        );
        let cleanup = next_worker_command!("failed Take dispatches cleanup Stop");
        assert_eq!(cleanup["payload"]["kind"], "stop");
        assert_eq!(cleanup["assignmentId"], failed_start["assignmentId"]);
        acknowledge_worker_command!(
            cleanup,
            arc_shared::WorkerCommandOutcome::Failed,
            Some("cleanup_failed"),
            Some("injected cleanup failure")
        );
        let pending = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let result = routes
                    .get_route_batch_operation(failed_operation_id)
                    .await
                    .unwrap()
                    .unwrap();
                if result.results[0].error_code.as_deref() == Some("start_cleanup_pending")
                    && result.results[0].state == "pending"
                {
                    break result;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("unconfirmed Stop leaves cleanup pending");
        let assignment_id = pending.results[0].assignment_id.unwrap();
        let snapshot = PgAssignments::new(pool.clone())
            .get_by_id(assignment_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(snapshot.desired_state, "planned");
        assert_eq!(snapshot.observed_state, "not_started");
        let reservation_state: String =
            sqlx::query_scalar("SELECT state FROM worker_capacity_reservations WHERE id = $1")
                .bind(snapshot.assignment.reservation_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(reservation_state, "reserved");
        let response = router_with_database(pool.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/assignments/{assignment_id}/cancel"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"generation": snapshot.assignment.generation})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let blocked_retry = serde_json::json!({"routes": [{
            "sourceId": source.id, "destinationId": failed_output.id,
            "expectedRouteRevision": 1, "confirmReplace": false
        }]});
        let response = router_with_database(pool.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/batch")
                    .header("content-type", "application/json")
                    .header("idempotency-key", Uuid::new_v4().to_string())
                    .body(Body::from(blocked_retry.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        sqlx::query(
            "UPDATE route_batch_items SET next_attempt_at = clock_timestamp() \
             WHERE operation_id = $1 AND destination_id = $2",
        )
        .bind(failed_operation_id)
        .bind(failed_output.id)
        .execute(&pool)
        .await
        .unwrap();
        let retried_cleanup = next_worker_command!("failed Take cleanup retries");
        assert_eq!(retried_cleanup["commandId"], cleanup["commandId"]);
        acknowledge_worker_command!(
            retried_cleanup,
            arc_shared::WorkerCommandOutcome::Completed,
            None,
            None
        );
        let failed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let result = routes
                    .get_route_batch_operation(failed_operation_id)
                    .await
                    .unwrap()
                    .unwrap();
                if result.state == "failed" {
                    break result;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("confirmed cleanup completes failed Take");
        assert_eq!(
            failed.results[0].error_code.as_deref(),
            Some("worker_start_failed")
        );
        let snapshot = PgAssignments::new(pool.clone())
            .get_by_id(assignment_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(snapshot.desired_state, "cancelled");
        assert_eq!(snapshot.observed_state, "cancelled");
        let reservation_state: String =
            sqlx::query_scalar("SELECT state FROM worker_capacity_reservations WHERE id = $1")
                .bind(snapshot.assignment.reservation_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(reservation_state, "released");
        let retry = serde_json::json!({"routes": [{
            "sourceId": source.id, "destinationId": failed_output.id,
            "expectedRouteRevision": 1, "confirmReplace": false
        }]});
        let response = router_with_database(pool.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/batch")
                    .header("content-type", "application/json")
                    .header("idempotency-key", Uuid::new_v4().to_string())
                    .body(Body::from(retry.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let retry_start = next_worker_command!("a new Take is no longer blocked");
        assert_eq!(retry_start["payload"]["kind"], "start");
        assert_ne!(retry_start["assignmentId"], failed_start["assignmentId"]);
        acknowledge_worker_command!(
            retry_start,
            arc_shared::WorkerCommandOutcome::Completed,
            None,
            None
        );
        let retry_assignment_id =
            Uuid::parse_str(retry_start["assignmentId"].as_str().unwrap()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let snapshot = PgAssignments::new(pool.clone())
                    .get_by_id(retry_assignment_id)
                    .await
                    .unwrap()
                    .unwrap();
                if snapshot.observed_state == "running" {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("retried Take starts");
        let unroute = serde_json::json!({
            "destinationId": failed_output.id, "expectedRouteRevision": 1
        });
        let response = router_with_database(pool.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/unroute")
                    .header("content-type", "application/json")
                    .header("idempotency-key", Uuid::new_v4().to_string())
                    .body(Body::from(unroute.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let stop = next_worker_command!("retried Take is stopped");
        acknowledge_worker_command!(
            stop,
            arc_shared::WorkerCommandOutcome::Completed,
            None,
            None
        );
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let snapshot = PgAssignments::new(pool.clone())
                    .get_by_id(retry_assignment_id)
                    .await
                    .unwrap()
                    .unwrap();
                if snapshot.observed_state == "stopped" {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("retried Take stops");

        let sibling = catalog::create_destination(
            &pool,
            "Route batch sibling destination",
            destination_profile.adapter_id(),
            serde_json::to_value(&destination_profile).unwrap(),
        )
        .await
        .unwrap();
        let sibling = catalog::validate_destination_draft(&pool, sibling.id)
            .await
            .unwrap();
        let sibling_request = serde_json::json!({"routes": [
            unchanged_route,
            {
                "sourceId": source.id, "destinationId": sibling.id,
                "expectedRouteRevision": 0, "confirmReplace": false
            }
        ]});
        let response = router_with_database(pool.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/batch")
                    .header("content-type", "application/json")
                    .header("idempotency-key", Uuid::new_v4().to_string())
                    .body(Body::from(sibling_request.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let sibling_operation: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(sibling_operation["state"], "running");
        let sibling_id =
            Uuid::parse_str(sibling_operation["operationId"].as_str().unwrap()).unwrap();
        let sibling_start = next_worker_command!("sibling Start reaches worker");
        assert_eq!(sibling_start["payload"]["kind"], "start");
        acknowledge_worker_command!(
            sibling_start,
            arc_shared::WorkerCommandOutcome::Completed,
            None,
            None
        );
        let sibling_assignment = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let operation = routes
                    .get_route_batch_operation(sibling_id)
                    .await
                    .unwrap()
                    .unwrap();
                if operation.state == "completed" {
                    break operation
                        .results
                        .iter()
                        .find(|result| result.destination_id == sibling.id)
                        .unwrap()
                        .assignment_id
                        .unwrap();
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("sibling starts");

        let assert_cleanup_pending = |operation_id, phase: &'static str, reserved_units: i64| {
            let pool = pool.clone();
            async move {
                let routes = PgRoutes::new(pool.clone());
                let pending = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    loop {
                        let operation = routes
                            .get_route_batch_operation(operation_id)
                            .await
                            .unwrap()
                            .unwrap();
                        if operation.state == "pending"
                            && operation.results[0].error_code.as_deref() == Some(phase)
                        {
                            break operation;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    }
                })
                .await
                .expect("cleanup failure remains durably pending");
                assert_eq!(pending.results[0].state, "pending");
                assert!(pending.results[0]
                    .error_message
                    .as_deref()
                    .unwrap()
                    .contains("Cleanup is pending:"));
                let target = PgAssignments::new(pool.clone())
                    .get_by_id(pending.results[0].assignment_id.unwrap())
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(target.desired_state, "planned");
                assert_eq!(target.observed_state, "not_started");
                let reservation_state: String = sqlx::query_scalar(
                    "SELECT state FROM worker_capacity_reservations WHERE id = $1",
                )
                .bind(target.assignment.reservation_id)
                .fetch_one(&pool)
                .await
                .unwrap();
                assert_eq!(reservation_state, "reserved");
                let scheduler = arc_infrastructure::scheduler::PgWorkerScheduler::new(
                    pool.clone(),
                    chrono::Duration::seconds(600),
                )
                .unwrap();
                let mut probe = target.assignment.requirements.clone();
                probe.capacity_units = 8;
                assert!(matches!(
                    scheduler.inspect_placement(&probe).await.unwrap(),
                    arc_domain::PlacementDecision::Unavailable { rejections }
                        if rejections.iter().any(|rejection| rejection.worker_id == worker_id
                            && matches!(&rejection.reason,
                                arc_domain::PlacementRejection::InsufficientCapacity {
                                    required: 8, available
                                } if i64::from(*available) == 8 - reserved_units))
                ));
                let sibling = PgAssignments::new(pool.clone())
                    .get_by_id(sibling_assignment)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(sibling.desired_state, "running");
                assert_eq!(sibling.observed_state, "running");
                target
            }
        };

        let prepare_failure_key = Uuid::new_v4();
        let prepare_failure_request = serde_json::json!({
            "routes": [{
                "sourceId": replacement_source_ids[0],
                "destinationId": destination.id,
                "expectedRouteRevision": 1,
                "confirmReplace": true
            }]
        });
        let response = router_with_database(pool.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/batch")
                    .header("content-type", "application/json")
                    .header("idempotency-key", prepare_failure_key.to_string())
                    .body(Body::from(prepare_failure_request.to_string()))
                    .expect("replacement request builds"),
            )
            .await
            .expect("replacement request completes");
        assert_eq!(response.status(), StatusCode::OK);
        let prepare_failure_body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("replacement response reads");
        let prepare_failure_operation: Value =
            serde_json::from_slice(&prepare_failure_body).expect("replacement response is JSON");
        let prepare_failure_operation_id = Uuid::parse_str(
            prepare_failure_operation["operationId"]
                .as_str()
                .expect("replacement operation ID is returned"),
        )
        .expect("replacement operation ID is valid");

        let prepare_command = next_worker_command!("replacement Prepare reaches the worker");
        assert_eq!(prepare_command["payload"]["kind"], "prepare");
        acknowledge_worker_command!(
            prepare_command,
            arc_shared::WorkerCommandOutcome::Failed,
            Some("gstreamer_pipeline_prepare_failed"),
            Some("injected replacement preparation failure")
        );
        let cleanup_command = next_worker_command!("failed preparation is cleaned up");
        assert_eq!(cleanup_command["payload"]["kind"], "stop");
        assert_eq!(
            cleanup_command["assignmentId"],
            prepare_command["assignmentId"]
        );
        acknowledge_worker_command!(
            cleanup_command,
            arc_shared::WorkerCommandOutcome::Failed,
            Some("injected_cleanup_failure"),
            Some("injected replacement cleanup failure")
        );
        let prepare_cleanup_target = assert_cleanup_pending(
            prepare_failure_operation_id,
            "replacement_cleanup_pending",
            5,
        )
        .await;
        let overlapping_take = serde_json::json!({
            "sourceId": replacement_source_ids[1],
            "destinationId": destination.id,
            "expectedRouteRevision": prepare_failure_operation["routes"][0]["revision"],
            "confirmReplace": true
        });
        let blocked_preset = routes
            .save_preset(
                None,
                None,
                format!("cleanup-blocked-{}", Uuid::new_v4()),
                vec![arc_infrastructure::routes::PresetRoute {
                    source_id: replacement_source_ids[1],
                    destination_id: destination.id,
                    action: super::RouteAction::Start,
                }],
            )
            .await
            .unwrap();
        for (uri, body) in [
            (
                "/api/v1/routes/unroute".to_owned(),
                serde_json::json!({
                    "destinationId": destination.id,
                    "expectedRouteRevision": prepare_failure_operation["routes"][0]["revision"]
                }),
            ),
            (
                "/api/v1/routes/batch".to_owned(),
                serde_json::json!({ "routes": [overlapping_take.clone()] }),
            ),
            (
                format!("/api/v1/presets/{}/execute", blocked_preset.id),
                serde_json::json!({
                    "expectedRevision": blocked_preset.revision,
                    "routes": [overlapping_take.clone()]
                }),
            ),
            ("/api/v1/routes".to_owned(), overlapping_take),
        ] {
            let response = router_with_database(pool.clone())
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(uri.as_str())
                        .header("content-type", "application/json")
                        .header("idempotency-key", Uuid::new_v4().to_string())
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::CONFLICT, "{uri}");
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let error: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(error["code"], "destination_busy", "{uri}");
        }
        routes
            .delete_preset(blocked_preset.id, blocked_preset.revision)
            .await
            .unwrap();
        let response = router_with_database(pool.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/batch")
                    .header("content-type", "application/json")
                    .header("idempotency-key", prepare_failure_key.to_string())
                    .body(Body::from(prepare_failure_request.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let replay: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            replay["operationId"],
            prepare_failure_operation["operationId"]
        );
        let previous_during_cleanup = PgAssignments::new(pool.clone())
            .get_by_id(previous_assignment_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(previous_during_cleanup.desired_state, "running");
        assert_eq!(previous_during_cleanup.observed_state, "running");
        let cleanup_retry = next_worker_command!("failed preparation cleanup retries");
        assert_eq!(cleanup_retry, cleanup_command);
        let reservation_state: String =
            sqlx::query_scalar("SELECT state FROM worker_capacity_reservations WHERE id = $1")
                .bind(prepare_cleanup_target.assignment.reservation_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(reservation_state, "reserved");
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(200), socket.next())
                .await
                .is_err(),
            "no route may start or stop while replacement cleanup is unconfirmed"
        );
        acknowledge_worker_command!(
            cleanup_retry,
            arc_shared::WorkerCommandOutcome::Completed,
            None,
            None
        );

        let prepare_failure_operation =
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let operation = routes
                        .get_route_batch_operation(prepare_failure_operation_id)
                        .await
                        .expect("replacement status reads")
                        .expect("replacement operation remains persisted");
                    if operation.state == "failed" {
                        break operation;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("preparation failure is recorded");
        assert_eq!(
            prepare_failure_operation.results[0].error_code.as_deref(),
            Some("replacement_prepare_failed")
        );
        let reservation_state: String =
            sqlx::query_scalar("SELECT state FROM worker_capacity_reservations WHERE id = $1")
                .bind(prepare_cleanup_target.assignment.reservation_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(reservation_state, "released");
        let previous_after_prepare_failure = PgAssignments::new(pool.clone())
            .get_by_id(previous_assignment_id)
            .await
            .expect("previous assignment status reads")
            .expect("previous assignment remains");
        assert_eq!(previous_after_prepare_failure.desired_state, "running");
        assert_eq!(previous_after_prepare_failure.observed_state, "running");

        let start_failure_key = Uuid::new_v4();
        let start_failure_request = serde_json::json!({
            "routes": [{
                "sourceId": replacement_source_ids[1],
                "destinationId": destination.id,
                "expectedRouteRevision": 2,
                "confirmReplace": true
            }]
        });
        let response = router_with_database(pool.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/batch")
                    .header("content-type", "application/json")
                    .header("idempotency-key", start_failure_key.to_string())
                    .body(Body::from(start_failure_request.to_string()))
                    .expect("replacement request builds"),
            )
            .await
            .expect("replacement request completes");
        assert_eq!(response.status(), StatusCode::OK);
        let start_failure_body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("replacement response reads");
        let start_failure_operation: Value =
            serde_json::from_slice(&start_failure_body).expect("replacement response is JSON");
        let start_failure_operation_id = Uuid::parse_str(
            start_failure_operation["operationId"]
                .as_str()
                .expect("replacement operation ID is returned"),
        )
        .expect("replacement operation ID is valid");

        let prepare_command = next_worker_command!("second replacement Prepare reaches the worker");
        assert_eq!(prepare_command["payload"]["kind"], "prepare");
        acknowledge_worker_command!(
            prepare_command,
            arc_shared::WorkerCommandOutcome::Completed,
            None,
            None
        );
        let stop_command = next_worker_command!("previous route Stop reaches the worker");
        assert_eq!(stop_command["payload"]["kind"], "stop");
        assert_eq!(
            stop_command["assignmentId"],
            previous_assignment_id.to_string()
        );
        acknowledge_worker_command!(
            stop_command,
            arc_shared::WorkerCommandOutcome::Completed,
            None,
            None
        );
        let failed_start_command = next_worker_command!("replacement Start reaches the worker");
        assert_eq!(failed_start_command["payload"]["kind"], "start");
        acknowledge_worker_command!(
            failed_start_command,
            arc_shared::WorkerCommandOutcome::Failed,
            Some("gstreamer_pipeline_start_failed"),
            Some("injected replacement start failure")
        );
        let failed_start_cleanup = next_worker_command!("failed replacement is stopped");
        assert_eq!(failed_start_cleanup["payload"]["kind"], "stop");
        assert_eq!(
            failed_start_cleanup["assignmentId"],
            failed_start_command["assignmentId"]
        );
        acknowledge_worker_command!(
            failed_start_cleanup,
            arc_shared::WorkerCommandOutcome::Failed,
            Some("injected_cleanup_failure"),
            Some("injected replacement cleanup failure")
        );
        let start_cleanup_target = assert_cleanup_pending(
            start_failure_operation_id,
            "replacement_rollback_pending",
            4,
        )
        .await;
        let previous_during_cleanup = PgAssignments::new(pool.clone())
            .get_by_id(previous_assignment_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(previous_during_cleanup.desired_state, "stopped");
        assert_eq!(previous_during_cleanup.observed_state, "stopped");
        let cleanup_retry = next_worker_command!("failed Start cleanup retries before restoration");
        assert_eq!(cleanup_retry, failed_start_cleanup);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(200), socket.next())
                .await
                .is_err(),
            "restoration must wait for replacement cleanup acknowledgement"
        );
        let reservation_state: String =
            sqlx::query_scalar("SELECT state FROM worker_capacity_reservations WHERE id = $1")
                .bind(start_cleanup_target.assignment.reservation_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(reservation_state, "reserved");
        acknowledge_worker_command!(
            cleanup_retry,
            arc_shared::WorkerCommandOutcome::Completed,
            None,
            None
        );
        let restore_command = next_worker_command!("previous route restoration reaches the worker");
        let reservation_state: String =
            sqlx::query_scalar("SELECT state FROM worker_capacity_reservations WHERE id = $1")
                .bind(start_cleanup_target.assignment.reservation_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(reservation_state, "released");
        assert_eq!(restore_command["payload"]["kind"], "start");
        assert_eq!(
            restore_command["payload"]["specification"]["configuration"]["source"],
            serde_json::to_value(&source_profile).expect("original profile serializes")
        );
        acknowledge_worker_command!(
            restore_command,
            arc_shared::WorkerCommandOutcome::Completed,
            None,
            None
        );

        let start_failure_operation =
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let operation = routes
                        .get_route_batch_operation(start_failure_operation_id)
                        .await
                        .expect("replacement status reads")
                        .expect("replacement operation remains persisted");
                    if operation.state == "failed" {
                        break operation;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("failed Start recovery is recorded");
        assert_eq!(
            start_failure_operation.results[0].error_code.as_deref(),
            Some("replacement_failed_restored")
        );
        let restored_route = routes
            .list_current()
            .await
            .expect("current route reads")
            .into_iter()
            .find(|route| route.destination_id == destination.id)
            .expect("destination route exists");
        assert_eq!(restored_route.observed_state, "running");
        assert_eq!(restored_route.observed_source_id, Some(source.id));

        let success_key = Uuid::new_v4();
        let success_request = serde_json::json!({
            "routes": [{
                "sourceId": replacement_source_ids[0],
                "destinationId": destination.id,
                "expectedRouteRevision": restored_route.revision,
                "confirmReplace": true
            }]
        });
        let response = router_with_database(pool.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/batch")
                    .header("content-type", "application/json")
                    .header("idempotency-key", success_key.to_string())
                    .body(Body::from(success_request.to_string()))
                    .expect("replacement request builds"),
            )
            .await
            .expect("replacement request completes");
        assert_eq!(response.status(), StatusCode::OK);
        let success_body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("replacement response reads");
        let success_operation: Value =
            serde_json::from_slice(&success_body).expect("replacement response is JSON");
        let success_operation_id = Uuid::parse_str(
            success_operation["operationId"]
                .as_str()
                .expect("replacement operation ID is returned"),
        )
        .expect("replacement operation ID is valid");

        let prepare_command =
            next_worker_command!("successful replacement Prepare reaches the worker");
        assert_eq!(prepare_command["payload"]["kind"], "prepare");
        acknowledge_worker_command!(
            prepare_command,
            arc_shared::WorkerCommandOutcome::Completed,
            None,
            None
        );
        let stop_command = next_worker_command!("restored route Stop reaches the worker");
        assert_eq!(stop_command["payload"]["kind"], "stop");
        acknowledge_worker_command!(
            stop_command,
            arc_shared::WorkerCommandOutcome::Completed,
            None,
            None
        );
        let start_command = next_worker_command!("successful replacement Start reaches the worker");
        assert_eq!(start_command["payload"]["kind"], "start");
        let unroute_key = Uuid::new_v4();
        let unroute_request = serde_json::json!({
            "destinationId": destination.id,
            "expectedRouteRevision": success_operation["routes"][0]["revision"]
        });
        let unroute_app = router_with_database(pool.clone());
        let mut unroute_task = tokio::spawn(async move {
            unroute_app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/v1/routes/unroute")
                        .header("content-type", "application/json")
                        .header("idempotency-key", unroute_key.to_string())
                        .body(Body::from(unroute_request.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap()
        });
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(200), &mut unroute_task)
                .await
                .is_err(),
            "unroute must wait for an in-flight replacement"
        );
        acknowledge_worker_command!(
            start_command,
            arc_shared::WorkerCommandOutcome::Completed,
            None,
            None
        );

        let success_operation = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let operation = routes
                    .get_route_batch_operation(success_operation_id)
                    .await
                    .expect("replacement status reads")
                    .expect("replacement operation remains persisted");
                if operation.state == "completed" {
                    break operation;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("successful replacement is recorded");
        assert_eq!(success_operation.results[0].state, "succeeded");
        let replaced_route = routes
            .list_current()
            .await
            .expect("current route reads")
            .into_iter()
            .find(|route| route.destination_id == destination.id)
            .expect("destination route exists");
        assert_eq!(replaced_route.observed_state, "running");
        assert_eq!(
            replaced_route.observed_source_id,
            Some(replacement_source_ids[0])
        );
        let response = tokio::time::timeout(std::time::Duration::from_secs(5), unroute_task)
            .await
            .expect("unroute returns after replacement")
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let unroute: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(unroute["routes"][0]["desiredState"], "removed");
        assert_eq!(unroute["routes"][0]["observedState"], "running");
        let unroute_id = Uuid::parse_str(unroute["operationId"].as_str().unwrap()).unwrap();
        let stop = next_worker_command!("unroute Stop reaches the worker");
        assert_eq!(stop["payload"]["kind"], "stop");
        assert_eq!(
            stop["assignmentId"],
            success_operation.results[0]
                .assignment_id
                .unwrap()
                .to_string()
        );
        let pending = routes
            .get_route_batch_operation(unroute_id)
            .await
            .unwrap()
            .unwrap();
        assert_ne!(pending.state, "completed");
        acknowledge_worker_command!(
            stop,
            arc_shared::WorkerCommandOutcome::Completed,
            None,
            None
        );
        let stopped = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let operation = routes
                    .get_route_batch_operation(unroute_id)
                    .await
                    .unwrap()
                    .unwrap();
                if operation.state == "completed" {
                    break operation;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("unroute completes after Stop acknowledgement");
        assert_eq!(stopped.results[0].state, "succeeded");
        let route = routes
            .list_current()
            .await
            .unwrap()
            .into_iter()
            .find(|route| route.destination_id == destination.id)
            .unwrap();
        assert_eq!(route.desired_state, "removed");
        assert_eq!(route.observed_state, "stopped");
        let lease = Uuid::new_v4();
        let recovered = sqlx::query_as::<_, super::RouteBatchWorkItem>(
            "UPDATE route_batch_items SET state = 'processing', lease_token = $2, \
             lease_expires_at = clock_timestamp() + INTERVAL '60 seconds' \
             WHERE operation_id = $1 RETURNING operation_id, source_id, destination_id, source_revision, \
             destination_revision, route_revision, replacement_assignment_id, pipeline_id, planning_key, \
             error_code, error_message, attempts, lease_token, action",
        ).bind(unroute_id).bind(lease).fetch_one(&pool).await.unwrap();
        let recovery_dispatcher = Arc::new(WorkerCommandDispatcher::new(Arc::new(
            TestAssignmentGenerationStore::default(),
        )));
        super::process_unroute_item(&routes, &pool, &recovery_dispatcher, &recovered)
            .await
            .unwrap();
        let recovered_operation = routes
            .get_route_batch_operation(unroute_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(recovered_operation.state, "completed");
        assert_eq!(
            recovered_operation.results[0].assignment_id,
            stopped.results[0].assignment_id
        );
        let sibling_snapshot = PgAssignments::new(pool.clone())
            .get_by_id(sibling_assignment)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(sibling_snapshot.desired_state, "running");
        assert_eq!(sibling_snapshot.observed_state, "running");
        let sibling_reservation = sibling_snapshot.assignment.reservation_id;
        let (reservation_state, reservation_units): (String, i64) = sqlx::query_as(
            "SELECT state, capacity_units FROM worker_capacity_reservations WHERE id = $1",
        )
        .bind(sibling_reservation)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(reservation_state, "reserved");
        let reserved_before: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(capacity_units), 0)::bigint FROM worker_capacity_reservations \
             WHERE worker_id = $1 AND state = 'reserved'",
        )
        .bind(worker_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(reserved_before, reservation_units);
        let scheduler = arc_infrastructure::scheduler::PgWorkerScheduler::new(
            pool.clone(),
            chrono::Duration::seconds(600),
        )
        .unwrap();
        let mut capacity_probe = sibling_snapshot.assignment.requirements.clone();
        capacity_probe.capacity_units = 8;
        let decision = scheduler.inspect_placement(&capacity_probe).await.unwrap();
        assert!(matches!(
            decision,
            arc_domain::PlacementDecision::Unavailable { rejections }
                if rejections.iter().any(|rejection| rejection.worker_id == worker_id
                    && matches!(&rejection.reason,
                        arc_domain::PlacementRejection::InsufficientCapacity { required: 8, available }
                            if i64::from(*available) == 8 - reservation_units))
        ));
        sqlx::query("UPDATE pipeline_assignments SET observed_state = 'unknown' WHERE id = $1")
            .bind(sibling_assignment)
            .execute(&pool)
            .await
            .unwrap();
        let response = router_with_database(pool.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/unroute")
                    .header("content-type", "application/json")
                    .header("idempotency-key", Uuid::new_v4().to_string())
                    .body(Body::from(
                        serde_json::json!({
                            "destinationId": sibling.id, "expectedRouteRevision": 1
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let sibling_stop: Value = serde_json::from_slice(&body).unwrap();
        let sibling_stop_id =
            Uuid::parse_str(sibling_stop["operationId"].as_str().unwrap()).unwrap();
        let command = next_worker_command!("unknown-state sibling receives Stop");
        assert_eq!(command["assignmentId"], sibling_assignment.to_string());
        assert_eq!(command["payload"]["kind"], "stop");
        let generation = sibling_snapshot.assignment.generation;
        let mut direct_start = tokio::spawn(async move {
            direct_app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(format!("/api/v1/assignments/{sibling_assignment}/start"))
                        .header("content-type", "application/json")
                        .body(Body::from(
                            serde_json::json!({ "generation": generation }).to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap()
        });
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(200), &mut direct_start)
                .await
                .is_err(),
            "direct Start must wait while the destination Stop is in flight"
        );
        acknowledge_worker_command!(
            command,
            arc_shared::WorkerCommandOutcome::Failed,
            Some("injected_stop_failure"),
            Some("Stop was not applied")
        );
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let operation = routes
                    .get_route_batch_operation(sibling_stop_id)
                    .await
                    .unwrap()
                    .unwrap();
                if operation.state == "failed" {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("failed Stop is reported");
        let response = tokio::time::timeout(std::time::Duration::from_secs(5), direct_start)
            .await
            .expect("direct Start returns after Stop")
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let error: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(error["code"], "route_removed");
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(200), socket.next())
                .await
                .is_err(),
            "a rejected direct Start must not reach the worker"
        );
        let reservation_state: String =
            sqlx::query_scalar("SELECT state FROM worker_capacity_reservations WHERE id = $1")
                .bind(sibling_reservation)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(reservation_state, "reserved");
        let reserved_after_failure: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(capacity_units), 0)::bigint FROM worker_capacity_reservations \
             WHERE worker_id = $1 AND state = 'reserved'",
        )
        .bind(worker_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(reserved_after_failure, reserved_before);
        let decision = scheduler.inspect_placement(&capacity_probe).await.unwrap();
        assert!(matches!(
            decision,
            arc_domain::PlacementDecision::Unavailable { rejections }
                if rejections.iter().any(|rejection| rejection.worker_id == worker_id
                    && matches!(&rejection.reason,
                        arc_domain::PlacementRejection::InsufficientCapacity { required: 8, available }
                            if i64::from(*available) == 8 - reservation_units))
        ));
        let route = routes
            .list_current()
            .await
            .unwrap()
            .into_iter()
            .find(|route| route.destination_id == sibling.id)
            .unwrap();
        assert_eq!(route.desired_state, "removed");
        assert_eq!(route.observed_state, "running");
        let main_route = routes
            .list_current()
            .await
            .unwrap()
            .into_iter()
            .find(|route| route.destination_id == destination.id)
            .unwrap();
        let response = router_with_database(pool.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/batch")
                    .header("content-type", "application/json")
                    .header("idempotency-key", Uuid::new_v4().to_string())
                    .body(Body::from(
                        serde_json::json!({"routes": [{
                            "sourceId": route.source_id, "destinationId": destination.id,
                            "expectedRouteRevision": main_route.revision, "confirmReplace": true,
                        }]})
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let started: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        let started_id = Uuid::parse_str(started["operationId"].as_str().unwrap()).unwrap();
        let command = next_worker_command!("second output starts for multi-output saved Unroute");
        assert_eq!(command["payload"]["kind"], "start");
        let main_assignment = Uuid::parse_str(command["assignmentId"].as_str().unwrap()).unwrap();
        acknowledge_worker_command!(
            command,
            arc_shared::WorkerCommandOutcome::Completed,
            None,
            None
        );
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if routes
                    .get_route_batch_operation(started_id)
                    .await
                    .unwrap()
                    .unwrap()
                    .state
                    == "completed"
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("second output Start is recorded");
        let main_route = routes
            .list_current()
            .await
            .unwrap()
            .into_iter()
            .find(|route| route.destination_id == destination.id)
            .unwrap();
        let anchor = if route.source_id == source.id {
            replacement_source_ids[0]
        } else {
            source.id
        };
        assert_ne!(anchor, route.source_id);
        let stop_preset = routes
            .save_preset(
                None,
                None,
                format!("multi-stop-{}", Uuid::new_v4()),
                vec![
                    arc_infrastructure::routes::PresetRoute {
                        source_id: anchor,
                        destination_id: sibling.id,
                        action: super::RouteAction::Stop,
                    },
                    arc_infrastructure::routes::PresetRoute {
                        source_id: anchor,
                        destination_id: destination.id,
                        action: super::RouteAction::Stop,
                    },
                ],
            )
            .await
            .unwrap();
        let response = router_with_database(pool.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/presets/{}/execute", stop_preset.id))
                    .header("content-type", "application/json")
                    .header("idempotency-key", Uuid::new_v4().to_string())
                    .body(Body::from(
                        serde_json::json!({
                            "expectedRevision": stop_preset.revision,
                            "routes": [
                                {"sourceId": anchor, "destinationId": sibling.id, "expectedRouteRevision": route.revision, "action": "stop"},
                                {"sourceId": anchor, "destinationId": destination.id, "expectedRouteRevision": main_route.revision, "action": "stop"}
                            ]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let retry: Value = serde_json::from_slice(&body).unwrap();
        let sibling_stop_id = Uuid::parse_str(retry["operationId"].as_str().unwrap()).unwrap();
        let command = next_worker_command!("failed Stop can be retried");
        let first_assignment = Uuid::parse_str(command["assignmentId"].as_str().unwrap()).unwrap();
        assert!([sibling_assignment, main_assignment].contains(&first_assignment));
        assert_eq!(command["payload"]["kind"], "stop");
        let reservation_state: String =
            sqlx::query_scalar("SELECT state FROM worker_capacity_reservations WHERE id = $1")
                .bind(sibling_reservation)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            reservation_state, "reserved",
            "retry is not yet acknowledged"
        );
        acknowledge_worker_command!(
            command,
            arc_shared::WorkerCommandOutcome::Completed,
            None,
            None
        );
        let command = next_worker_command!("the other saved output receives its own Stop");
        let second_assignment = Uuid::parse_str(command["assignmentId"].as_str().unwrap()).unwrap();
        assert!([sibling_assignment, main_assignment].contains(&second_assignment));
        assert_ne!(first_assignment, second_assignment);
        assert_eq!(command["payload"]["kind"], "stop");
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let operation = routes
                    .get_route_batch_operation(sibling_stop_id)
                    .await
                    .unwrap()
                    .unwrap();
                if operation.results.iter().any(|item| {
                    item.assignment_id == Some(first_assignment) && item.state == "succeeded"
                }) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect(
            "one output must complete without waiting for another output's Stop acknowledgement",
        );
        acknowledge_worker_command!(
            command,
            arc_shared::WorkerCommandOutcome::Completed,
            None,
            None
        );
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let operation = routes
                    .get_route_batch_operation(sibling_stop_id)
                    .await
                    .unwrap()
                    .unwrap();
                if operation.state == "completed" {
                    break;
                }
                assert_ne!(operation.state, "failed");
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("unknown-state Stop succeeds");
        let reservation_state: String =
            sqlx::query_scalar("SELECT state FROM worker_capacity_reservations WHERE id = $1")
                .bind(sibling_reservation)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(reservation_state, "released");
        let reserved_after_acknowledgement: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(capacity_units), 0)::bigint FROM worker_capacity_reservations \
             WHERE worker_id = $1 AND state = 'reserved'",
        )
        .bind(worker_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            reserved_after_acknowledgement,
            reserved_before - reservation_units
        );
        assert!(matches!(
            scheduler.inspect_placement(&capacity_probe).await.unwrap(),
            arc_domain::PlacementDecision::Selected { worker_id: selected } if selected == worker_id
        ));

        let exhausted_output = catalog::create_destination(
            &pool,
            "Exhausted cleanup destination",
            destination_profile.adapter_id(),
            serde_json::to_value(&destination_profile).unwrap(),
        )
        .await
        .unwrap();
        let exhausted_output = catalog::validate_destination_draft(&pool, exhausted_output.id)
            .await
            .unwrap();
        let request = serde_json::json!({"routes": [{
            "sourceId": source.id, "destinationId": exhausted_output.id,
            "expectedRouteRevision": 0, "confirmReplace": false
        }]});
        let response = router_with_database(pool.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/batch")
                    .header("content-type", "application/json")
                    .header("idempotency-key", Uuid::new_v4().to_string())
                    .body(Body::from(request.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let response: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        let exhausted_operation_id =
            Uuid::parse_str(response["operationId"].as_str().unwrap()).unwrap();
        let failed_start = next_worker_command!("exhaustion test Take reaches worker");
        acknowledge_worker_command!(
            failed_start,
            arc_shared::WorkerCommandOutcome::Failed,
            Some("gstreamer_pipeline_start_failed"),
            Some("injected start failure")
        );
        let cleanup = next_worker_command!("exhaustion test cleanup reaches worker");
        acknowledge_worker_command!(
            cleanup,
            arc_shared::WorkerCommandOutcome::Failed,
            Some("cleanup_failed"),
            Some("injected cleanup failure")
        );
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let operation = routes
                    .get_route_batch_operation(exhausted_operation_id)
                    .await
                    .unwrap()
                    .unwrap();
                if operation.results[0].error_code.as_deref() == Some("start_cleanup_pending")
                    && operation.results[0].state == "pending"
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("failed cleanup remains pending");
        sqlx::query(
            "UPDATE route_batch_items SET attempts = $1, next_attempt_at = clock_timestamp() \
             WHERE operation_id = $2 AND destination_id = $3",
        )
        .bind(super::MAX_ROUTE_BATCH_ATTEMPTS - 1)
        .bind(exhausted_operation_id)
        .bind(exhausted_output.id)
        .execute(&pool)
        .await
        .unwrap();
        let last_cleanup = next_worker_command!("last cleanup attempt reaches worker");
        assert_eq!(last_cleanup["commandId"], cleanup["commandId"]);
        acknowledge_worker_command!(
            last_cleanup,
            arc_shared::WorkerCommandOutcome::Failed,
            Some("cleanup_failed"),
            Some("injected cleanup failure")
        );
        let exhausted = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let operation = routes
                    .get_route_batch_operation(exhausted_operation_id)
                    .await
                    .unwrap()
                    .unwrap();
                if operation.state == "failed" {
                    break operation;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("exhausted cleanup fails explicitly");
        assert_eq!(
            exhausted.results[0].error_code.as_deref(),
            Some("start_cleanup_failed")
        );
        assert!(exhausted.results[0]
            .error_message
            .as_deref()
            .unwrap()
            .contains("manual recovery"));
        let assignment_id = exhausted.results[0].assignment_id.unwrap();
        let snapshot = PgAssignments::new(pool.clone())
            .get_by_id(assignment_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(snapshot.desired_state, "planned");
        assert_eq!(snapshot.observed_state, "not_started");
        let reservation_state: String =
            sqlx::query_scalar("SELECT state FROM worker_capacity_reservations WHERE id = $1")
                .bind(snapshot.assignment.reservation_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(reservation_state, "reserved");
        let response = router_with_database(pool.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/assignments/{assignment_id}/cancel"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"generation": snapshot.assignment.generation})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);

        server.abort();
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
    #[ignore = "requires an isolated PostgreSQL database"]
    async fn capacity_wait_api_recovers_and_processes_fifo() {
        async fn call(
            app: &axum::Router,
            method: &str,
            path: &str,
            key: Option<Uuid>,
            body: Option<Value>,
        ) -> (StatusCode, Value) {
            let mut request = Request::builder().method(method).uri(path);
            if let Some(key) = key {
                request = request.header("idempotency-key", key.to_string());
            }
            if body.is_some() {
                request = request.header("content-type", "application/json");
            }
            let response = app
                .clone()
                .oneshot(
                    request
                        .body(Body::from(body.map(|b| b.to_string()).unwrap_or_default()))
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            (status, serde_json::from_slice(&body).unwrap())
        }

        let pool = connect_postgres(&env::var("DATABASE_URL").unwrap())
            .await
            .unwrap();
        migrate(&pool).await.unwrap();
        // The ignored API tests share an isolated PostgreSQL database.
        sqlx::query("TRUNCATE TABLE workers CASCADE")
            .execute(&pool)
            .await
            .unwrap();
        let source_profile = SourceMediaProfile::RtpL16 {
            bind_address: IpAddr::from([0, 0, 0, 0]),
            port: 5000,
            payload_type: 96,
        };
        let destination_profile = DestinationMediaProfile::RtpL16 {
            address: IpAddr::from([127, 0, 0, 1]),
            port: 5002,
            payload_type: 96,
        };
        let source = catalog::create_source(
            &pool,
            "wait API source",
            source_profile.adapter_id(),
            serde_json::to_value(&source_profile).unwrap(),
        )
        .await
        .unwrap();
        let source = catalog::validate_source_draft(&pool, source.id)
            .await
            .unwrap();
        let destination = catalog::create_destination(
            &pool,
            "wait API destination",
            destination_profile.adapter_id(),
            serde_json::to_value(&destination_profile).unwrap(),
        )
        .await
        .unwrap();
        let destination = catalog::validate_destination_draft(&pool, destination.id)
            .await
            .unwrap();
        let second_destination = catalog::create_destination(
            &pool,
            "wait API second destination",
            destination_profile.adapter_id(),
            serde_json::to_value(&destination_profile).unwrap(),
        )
        .await
        .unwrap();
        let second_destination = catalog::validate_destination_draft(&pool, second_destination.id)
            .await
            .unwrap();
        let body = |pipeline: Uuid, wait: bool, destination_id: Uuid, destination_revision: i64| {
            serde_json::json!({
                "pipelineId": pipeline, "sourceId": source.id, "sourceRevision": source.current_revision,
                "destinationId": destination_id, "destinationRevision": destination_revision, "allowWaiting": wait
            })
        };
        let app = router_with_database(pool.clone());
        let (status, immediate) = call(
            &app,
            "POST",
            "/api/v1/routes/plan-assignment",
            Some(Uuid::new_v4()),
            Some(body(
                Uuid::new_v4(),
                false,
                destination.id,
                destination.current_revision,
            )),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(immediate["state"], "unavailable");
        assert!(immediate.get("requestId").is_none());
        let key = Uuid::new_v4();
        let request = body(
            Uuid::new_v4(),
            true,
            destination.id,
            destination.current_revision,
        );
        let (_, waiting) = call(
            &app,
            "POST",
            "/api/v1/routes/plan-assignment",
            Some(key),
            Some(request.clone()),
        )
        .await;
        assert_eq!(waiting["state"], "waiting");
        assert!(waiting["expiresAt"].is_string());
        let id = waiting["requestId"].as_str().unwrap();
        let path = format!("/api/v1/capacity-requests/{id}");
        let (_, cancelled) = call(&app, "POST", &format!("{path}/cancel"), None, None).await;
        assert_eq!(cancelled["state"], "cancelled");
        let (_, replay) = call(
            &app,
            "POST",
            "/api/v1/routes/plan-assignment",
            Some(key),
            Some(request.clone()),
        )
        .await;
        assert_eq!(replay["state"], "cancelled");
        let mut changed = request.clone();
        changed["allowWaiting"] = false.into();
        assert_eq!(
            call(
                &app,
                "POST",
                "/api/v1/routes/plan-assignment",
                Some(key),
                Some(changed)
            )
            .await
            .0,
            StatusCode::CONFLICT
        );

        let expired_key = Uuid::new_v4();
        let expired_body = body(
            Uuid::new_v4(),
            true,
            destination.id,
            destination.current_revision,
        );
        let (_, expired) = call(
            &app,
            "POST",
            "/api/v1/routes/plan-assignment",
            Some(expired_key),
            Some(expired_body.clone()),
        )
        .await;
        let expired_id = Uuid::parse_str(expired["requestId"].as_str().unwrap()).unwrap();
        sqlx::query("UPDATE capacity_wait_requests SET expires_at = clock_timestamp() - INTERVAL '1 second' WHERE request_id = $1")
            .bind(expired_id).execute(&pool).await.unwrap();
        let (_, expired) = call(
            &app,
            "GET",
            &format!("/api/v1/capacity-requests/{expired_id}"),
            None,
            None,
        )
        .await;
        assert_eq!(expired["state"], "unavailable");
        assert_eq!(expired["reason"], "expired");
        let (_, replay) = call(
            &app,
            "POST",
            "/api/v1/routes/plan-assignment",
            Some(expired_key),
            Some(expired_body),
        )
        .await;
        assert_eq!(replay["state"], "unavailable");
        assert_eq!(replay["reason"], "expired");

        let (_, first) = call(
            &app,
            "POST",
            "/api/v1/routes/plan-assignment",
            Some(Uuid::new_v4()),
            Some(body(
                Uuid::new_v4(),
                true,
                destination.id,
                destination.current_revision,
            )),
        )
        .await;
        let (_, second) = call(
            &app,
            "POST",
            "/api/v1/routes/plan-assignment",
            Some(Uuid::new_v4()),
            Some(body(
                Uuid::new_v4(),
                true,
                second_destination.id,
                second_destination.current_revision,
            )),
        )
        .await;
        let first_path = format!(
            "/api/v1/capacity-requests/{}",
            first["requestId"].as_str().unwrap()
        );
        let second_path = format!(
            "/api/v1/capacity-requests/{}",
            second["requestId"].as_str().unwrap()
        );
        drop(app);
        let worker_id = Uuid::new_v4();
        let registry = PgWorkerRegistry::new(pool.clone());
        let mut worker = Worker {
            id: worker_id,
            stable_name: format!("wait-api-{worker_id}"),
            version: "1".into(),
            state: WorkerState::Registering,
            capabilities: WorkerCapabilities {
                platform: Some(Platform::Linux),
                architecture: Some(Architecture::X86_64),
                source_adapters: BTreeSet::from(["rtp_l16".into()]),
                destination_adapters: BTreeSet::from(["rtp_l16".into()]),
                ..WorkerCapabilities::default()
            },
            capacity: WorkerCapacity {
                total_capacity_units: 2,
                assigned_capacity_units: 0,
                cpu_utilization_ratio: None,
                memory_utilization_ratio: None,
                ingress_bits_per_second: None,
                egress_bits_per_second: None,
            },
            last_heartbeat: Utc::now(),
        };
        registry.register(worker.clone()).await.unwrap();
        worker.state = WorkerState::Accepting;
        registry.heartbeat(worker.clone()).await.unwrap();
        let app = router_with_database(pool.clone());
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let (_, status) = call(&app, "GET", &first_path, None, None).await;
                if status["state"] == "planned" {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            call(&app, "GET", &second_path, None, None).await.1["state"],
            "waiting"
        );
        sqlx::query("ALTER TABLE workers DISABLE TRIGGER capacity_workers_changed")
            .execute(&pool)
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        worker.capacity.total_capacity_units = 4;
        registry.heartbeat(worker).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let (_, status) = call(&app, "GET", &second_path, None, None).await;
                if status["state"] == "planned" {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap();
        sqlx::query("ALTER TABLE workers ENABLE TRIGGER capacity_workers_changed")
            .execute(&pool)
            .await
            .unwrap();
        let (_, terminal) = call(&app, "POST", &format!("{second_path}/cancel"), None, None).await;
        assert_eq!(terminal["state"], "planned");
        assert!(terminal["assignment"]["id"].is_string());
        assert_eq!(
            call(
                &app,
                "GET",
                &format!("/api/v1/capacity-requests/{}", Uuid::new_v4()),
                None,
                None
            )
            .await
            .0,
            StatusCode::NOT_FOUND
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

        let competing_request = serde_json::json!({
            "pipelineId": Uuid::new_v4(),
            "sourceId": source.id,
            "sourceRevision": source.current_revision,
            "destinationId": destination.id,
            "destinationRevision": destination.current_revision,
        });
        let competing_response = router_with_database(pool.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/plan-assignment")
                    .header("content-type", "application/json")
                    .header("idempotency-key", Uuid::new_v4().to_string())
                    .body(Body::from(competing_request.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(competing_response.status(), StatusCode::CONFLICT);
        let competing_body = to_bytes(competing_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let competing_error: Value = serde_json::from_slice(&competing_body).unwrap();
        assert_eq!(competing_error["code"], "destination_in_use");

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
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let cancellation_retry: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(cancellation_retry["state"], "cancelled");
        assert_eq!(cancellation_retry["generation"], generation);

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
    #[ignore = "requires an isolated PostgreSQL database and GStreamer plugins; set DATABASE_URL and run ignored tests"]
    async fn api_dispatch_controls_a_live_rtp_to_srt_media_pipeline() {
        use gstreamer::prelude::*;

        let url = env::var("DATABASE_URL").expect("DATABASE_URL is set");
        let pool = connect_postgres(&url).await.expect("database connects");
        migrate(&pool).await.expect("database migrates");
        gstreamer::init().expect("GStreamer initializes");

        let loopback = Ipv4Addr::LOCALHOST;
        let rtp_reservation = UdpSocket::bind((loopback, 0)).expect("RTP source port reserves");
        let rtp_port = rtp_reservation
            .local_addr()
            .expect("RTP source port is available")
            .port();
        let srt_reservation = UdpSocket::bind((loopback, 0)).expect("SRT listener port reserves");
        let srt_port = srt_reservation
            .local_addr()
            .expect("SRT listener port is available")
            .port();
        drop((rtp_reservation, srt_reservation));

        let worker_id = Uuid::new_v4();
        let mut worker = Worker {
            id: worker_id,
            stable_name: format!("api-media-{worker_id}"),
            version: "0.1.0".to_owned(),
            state: WorkerState::Registering,
            capabilities: WorkerCapabilities {
                platform: Some(Platform::Linux),
                architecture: Some(Architecture::X86_64),
                source_adapters: BTreeSet::from(["rtp_mp2t".to_owned()]),
                destination_adapters: BTreeSet::from(["srt_mpeg_ts".to_owned()]),
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
        registry
            .register(worker.clone())
            .await
            .expect("worker registers");
        worker.state = WorkerState::Accepting;
        registry
            .heartbeat(worker)
            .await
            .expect("worker becomes eligible");

        let source_profile = SourceMediaProfile::RtpMp2t {
            bind_address: loopback.into(),
            port: rtp_port,
        };
        let destination_profile = DestinationMediaProfile::SrtMpegTs {
            host: loopback.to_string(),
            port: srt_port,
            mode: arc_domain::SrtMode::Caller,
        };
        let source = catalog::create_source(
            &pool,
            "Live RTP source",
            source_profile.adapter_id(),
            serde_json::to_value(&source_profile).expect("source serializes"),
        )
        .await
        .expect("source creates");
        let source = catalog::validate_source_draft(&pool, source.id)
            .await
            .expect("source validates");
        let destination = catalog::create_destination(
            &pool,
            "Live SRT destination",
            destination_profile.adapter_id(),
            serde_json::to_value(&destination_profile).expect("destination serializes"),
        )
        .await
        .expect("destination creates");
        let destination = catalog::validate_destination_draft(&pool, destination.id)
            .await
            .expect("destination validates");

        let receiver_source = gstreamer::ElementFactory::make("srtsrc")
            .property("uri", format!("srt://127.0.0.1:{srt_port}?mode=listener"))
            .build()
            .expect("SRT receiver builds");
        let receiver_sink = AppSink::builder().sync(false).build();
        let receiver_pipeline = gstreamer::Pipeline::new();
        receiver_pipeline
            .add_many([
                &receiver_source,
                receiver_sink.upcast_ref::<gstreamer::Element>(),
            ])
            .expect("SRT receiver elements add");
        receiver_source
            .link(receiver_sink.upcast_ref::<gstreamer::Element>())
            .expect("SRT receiver links");
        receiver_pipeline
            .set_state(gstreamer::State::Playing)
            .expect("SRT receiver starts");

        let dispatcher = Arc::new(WorkerCommandDispatcher::new_with_lease_pool(
            Arc::new(PgAssignmentGenerationStore::new(pool.clone())),
            pool.clone(),
        ));
        const WORKER_TOKEN: &str =
            "abababababababababababababababababababababababababababababababab";
        let app = router_with_database_and_worker_dispatcher(
            pool.clone(),
            Arc::new(TestWorkerAuthenticator {
                worker_id,
                token: WORKER_TOKEN,
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

        let media_pipeline =
            Arc::new(GstreamerMediaPipeline::new().expect("worker GStreamer initializes"));
        let worker_client = WorkerControlClient::new(
            WorkerControlClientConfig {
                endpoint: format!("ws://{address}/api/v1/workers/control"),
                ca_certificates_file: None,
                worker_id,
                bearer_token: WORKER_TOKEN.to_owned(),
                bearer_token_file: None,
                allow_insecure_loopback: true,
            },
            Arc::new(PgAssignmentGenerationStore::new(pool.clone())),
            media_pipeline,
        )
        .expect("worker control client builds");
        let worker_status = worker_client.connection_status();
        let worker_task = tokio::spawn(async move { worker_client.run_session().await });
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let mut status = worker_status;
            while !*status.borrow() {
                status.changed().await.expect("worker status remains open");
            }
        })
        .await
        .expect("worker establishes control session");

        let pipeline_id = Uuid::new_v4();
        let plan_request = serde_json::json!({
            "pipelineId": pipeline_id,
            "sourceId": source.id,
            "sourceRevision": source.current_revision,
            "destinationId": destination.id,
            "destinationRevision": destination.current_revision,
        });
        let plan_response = app
            .clone()
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
        let assignment_id = Uuid::parse_str(
            plan["assignment"]["id"]
                .as_str()
                .expect("assignment ID is returned"),
        )
        .expect("assignment ID is valid");
        let generation = plan["assignment"]["generation"]
            .as_i64()
            .expect("assignment generation is returned");

        let start_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/assignments/{assignment_id}/start"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "generation": generation }).to_string(),
                    ))
                    .expect("start request builds"),
            )
            .await
            .expect("start request completes");
        assert_eq!(start_response.status(), StatusCode::OK);

        let mut transport_stream = Vec::with_capacity(188 * 7);
        for packet_index in 0..7u8 {
            let mut packet = vec![packet_index; 188];
            packet[0] = 0x47;
            packet[1] = 0x01;
            packet[2] = 0x00;
            packet[3] = 0x10 | packet_index;
            transport_stream.extend_from_slice(&packet);
        }
        let sender = UdpSocket::bind((loopback, 0)).expect("RTP sender binds");
        for sequence in 1..=4u16 {
            let mut datagram = Vec::with_capacity(12 + transport_stream.len());
            datagram.extend_from_slice(&[0x80, 33]);
            datagram.extend_from_slice(&sequence.to_be_bytes());
            datagram.extend_from_slice(&(u32::from(sequence) * 3_600).to_be_bytes());
            datagram.extend_from_slice(&0x4152_4331_u32.to_be_bytes());
            datagram.extend_from_slice(&transport_stream);
            sender
                .send_to(&datagram, (loopback, rtp_port))
                .expect("RTP fixture packet sends");
        }

        let mut received = Vec::new();
        for _ in 0..5 {
            let Some(sample) = receiver_sink.try_pull_sample(gstreamer::ClockTime::from_seconds(1))
            else {
                continue;
            };
            let buffer = sample
                .buffer()
                .expect("SRT receiver sample has a buffer")
                .map_readable()
                .expect("SRT receiver buffer is readable");
            received.extend_from_slice(buffer.as_slice());
            if received
                .windows(transport_stream.len())
                .any(|candidate| candidate == transport_stream.as_slice())
            {
                break;
            }
        }
        assert!(
            received
                .windows(transport_stream.len())
                .any(|candidate| candidate == transport_stream.as_slice()),
            "API-dispatched SRT output did not contain the original MPEG-TS packets"
        );

        let stop_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/assignments/{assignment_id}/stop"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "generation": generation }).to_string(),
                    ))
                    .expect("stop request builds"),
            )
            .await
            .expect("stop request completes");
        assert_eq!(stop_response.status(), StatusCode::OK);

        worker_task.abort();
        let _ = worker_task.await;
        server.abort();
        receiver_pipeline
            .set_state(gstreamer::State::Null)
            .expect("SRT receiver stops");
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
        PgRoutes::new(pool.clone())
            .set_desired_route(source.id, destination.id, 0, false)
            .await
            .expect("desired route saves");
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

        let dispatcher = Arc::new(WorkerCommandDispatcher::new_with_lease_pool(
            Arc::new(PgAssignmentGenerationStore::new(pool.clone())),
            pool.clone(),
        ));
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
        let observations_before_start: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM route_events AS event \
             JOIN route_instances AS route ON route.id = event.route_instance_id \
             WHERE route.source_id = $1 AND route.destination_id = $2",
        )
        .bind(source.id)
        .bind(destination.id)
        .fetch_one(&pool)
        .await
        .expect("route observations read");
        assert_eq!(observations_before_start, 0);

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
            process_incarnation: None,
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
        let message =
            match tokio::time::timeout(std::time::Duration::from_secs(2), socket.next()).await {
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
        let routes_before_start_ack = PgRoutes::new(pool.clone())
            .list_current()
            .await
            .expect("route state reads before Start acknowledgement");
        let route_before_start_ack = routes_before_start_ack
            .into_iter()
            .find(|route| route.destination_id == destination.id)
            .expect("test destination route exists before Start acknowledgement");
        assert_eq!(route_before_start_ack.observed_state, "unknown");
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
            error_code: None,
            error_message: None,
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

        let routes_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/routes")
                    .body(Body::empty())
                    .expect("routes request builds"),
            )
            .await
            .expect("routes request completes");
        assert_eq!(routes_response.status(), StatusCode::OK);
        let routes_body = to_bytes(routes_response.into_body(), usize::MAX)
            .await
            .expect("routes response reads");
        let routes: Value =
            serde_json::from_slice(&routes_body).expect("routes response is valid JSON");
        let routes = routes["items"]
            .as_array()
            .expect("routes response contains an item list");
        let route = routes
            .iter()
            .find(|route| route["destinationId"] == destination.id.to_string())
            .expect("routes response contains the test destination");
        assert_eq!(route["sourceId"], source.id.to_string());
        assert_eq!(route["desiredState"], "desired");
        assert_eq!(route["observedState"], "running");
        assert_eq!(route["observedSourceId"], source.id.to_string());

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
            .uri(format!("/api/v1/assignments/{second_assignment_id}/start"))
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
            error_code: None,
            error_message: None,
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
        let routes_before_stop_ack = PgRoutes::new(pool.clone())
            .list_current()
            .await
            .expect("route state reads before Stop acknowledgement");
        let route_before_stop_ack = routes_before_stop_ack
            .into_iter()
            .find(|route| route.destination_id == destination.id)
            .expect("test destination route exists before Stop acknowledgement");
        assert_eq!(route_before_stop_ack.observed_state, "running");
        let competing_plan = serde_json::json!({
            "pipelineId": Uuid::new_v4(),
            "sourceId": source.id,
            "sourceRevision": source.current_revision,
            "destinationId": destination.id,
            "destinationRevision": destination.current_revision,
        });
        let competing_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/plan-assignment")
                    .header("content-type", "application/json")
                    .header("idempotency-key", Uuid::new_v4().to_string())
                    .body(Body::from(competing_plan.to_string()))
                    .expect("competing plan request builds"),
            )
            .await
            .expect("competing plan request completes");
        assert_eq!(competing_response.status(), StatusCode::CONFLICT);
        let competing_body = to_bytes(competing_response.into_body(), usize::MAX)
            .await
            .expect("competing plan response reads");
        let competing_error: Value =
            serde_json::from_slice(&competing_body).expect("competing plan error is valid JSON");
        assert_eq!(competing_error["code"], "destination_in_use");

        let acknowledgement = arc_shared::WorkerCommandAcknowledgement {
            kind: arc_shared::WorkerSessionMessageKind::CommandAck,
            protocol_version: arc_shared::WORKER_PROTOCOL_VERSION,
            worker_id: assigned_worker_id,
            command_id: stop_command_id,
            outcome: arc_shared::WorkerCommandOutcome::Completed,
            current_generation: None,
            error_code: None,
            error_message: None,
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
        let routes_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/routes")
                    .body(Body::empty())
                    .expect("stopped routes request builds"),
            )
            .await
            .expect("stopped routes request completes");
        assert_eq!(routes_response.status(), StatusCode::OK);
        let routes_body = to_bytes(routes_response.into_body(), usize::MAX)
            .await
            .expect("stopped routes response reads");
        let routes: Value =
            serde_json::from_slice(&routes_body).expect("stopped routes response is valid JSON");
        let route = routes["items"]
            .as_array()
            .expect("routes are an array")
            .iter()
            .find(|route| route["destinationId"] == destination.id.to_string())
            .expect("test destination route exists");
        assert_eq!(route["desiredState"], "desired");
        assert_eq!(route["observedState"], "stopped");
        assert_eq!(route["observedSourceId"], source.id.to_string());
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
            .uri(format!("/api/v1/assignments/{second_assignment_id}/stop"))
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
            error_code: None,
            error_message: None,
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
