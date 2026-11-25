use arc_application::PortResult;
use arc_domain::{
    estimate_pipeline_requirements, DestinationMediaProfile, PipelineEstimate, SourceMediaProfile,
    WorkerRejection,
};
use chrono::Duration;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{types::Json, FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{
    assignments::{PgAssignments, PlannedAssignment, PlannedRouteConfig},
    capacity_reservations::{reserve_in_transaction, ReservationDecision},
    capacity_wait::{has_waiters, lock_queue},
    worker_registry::error_with,
};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PlannedPipeline {
    pub estimate: PipelineEstimate,
    pub assignment: PlannedAssignment,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum PipelinePlanningDecision {
    Planned(PlannedPipeline),
    Unavailable {
        estimate: PipelineEstimate,
        rejections: Vec<WorkerRejection>,
    },
}

pub struct PgPipelinePlanner {
    pool: PgPool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RouteRevision {
    pub source_id: Uuid,
    pub source_revision: i64,
    pub destination_id: Uuid,
    pub destination_revision: i64,
}

#[derive(FromRow)]
struct StoredEndpoint {
    current_revision: i64,
    lifecycle: String,
    adapter_id: String,
    configuration: Value,
}

#[derive(FromRow)]
struct IdempotencyRecord {
    request: Json<Value>,
    response: Json<PipelinePlanningDecision>,
}

impl PgPipelinePlanner {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn plan(
        &self,
        pipeline_id: Uuid,
        route: RouteRevision,
        max_heartbeat_age: Duration,
    ) -> PortResult<PipelinePlanningDecision> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| error_with("database_error", error))?;
        lock_queue(&mut transaction).await?;
        let decision = self
            .plan_now_in_transaction(&mut transaction, pipeline_id, route, max_heartbeat_age)
            .await?;
        transaction
            .commit()
            .await
            .map_err(|error| error_with("database_error", error))?;
        Ok(decision)
    }

    pub async fn plan_with_idempotency_key(
        &self,
        idempotency_key: Uuid,
        pipeline_id: Uuid,
        route: RouteRevision,
        max_heartbeat_age: Duration,
    ) -> PortResult<PipelinePlanningDecision> {
        if idempotency_key.is_nil() {
            return Err(error_with(
                "invalid_idempotency_key",
                "idempotency key must not be nil",
            ));
        }

        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| error_with("database_error", error))?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(idempotency_key.to_string())
            .execute(&mut *transaction)
            .await
            .map_err(|error| error_with("database_error", error))?;
        lock_queue(&mut transaction).await?;
        sqlx::query("DELETE FROM route_plan_idempotency WHERE expires_at <= clock_timestamp()")
            .execute(&mut *transaction)
            .await
            .map_err(|error| error_with("database_error", error))?;

        let request = json!({
            "pipelineId": pipeline_id,
            "sourceId": route.source_id,
            "sourceRevision": route.source_revision,
            "destinationId": route.destination_id,
            "destinationRevision": route.destination_revision,
        });
        if let Some(record) = sqlx::query_as::<_, IdempotencyRecord>(
            "SELECT request, response FROM route_plan_idempotency \
             WHERE idempotency_key = $1 FOR UPDATE",
        )
        .bind(idempotency_key)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?
        {
            if record.request.0 != request {
                return Err(error_with(
                    "idempotency_key_reused",
                    "idempotency key was already used for a different planning request",
                ));
            }
            let decision = record.response.0;
            transaction
                .commit()
                .await
                .map_err(|error| error_with("database_error", error))?;
            return Ok(decision);
        }

        let decision = self
            .plan_now_in_transaction(&mut transaction, pipeline_id, route, max_heartbeat_age)
            .await?;
        sqlx::query(
            "INSERT INTO route_plan_idempotency \
             (idempotency_key, request, response, expires_at) \
             VALUES ($1, $2, $3, clock_timestamp() + INTERVAL '24 hours')",
        )
        .bind(idempotency_key)
        .bind(Json(request))
        .bind(Json(&decision))
        .execute(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?;
        transaction
            .commit()
            .await
            .map_err(|error| error_with("database_error", error))?;
        Ok(decision)
    }

    async fn plan_now_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        pipeline_id: Uuid,
        route: RouteRevision,
        max_heartbeat_age: Duration,
    ) -> PortResult<PipelinePlanningDecision> {
        if has_waiters(transaction).await? {
            if pipeline_id.is_nil() || max_heartbeat_age < Duration::zero() {
                return Err(error_with(
                    "invalid_reservation",
                    "pipeline ID and nonnegative heartbeat age are required",
                ));
            }
            let (estimate, _) = validate_route(transaction, route).await?;
            return Ok(PipelinePlanningDecision::Unavailable {
                estimate,
                rejections: Vec::new(),
            });
        }
        self.plan_in_transaction(transaction, pipeline_id, route, max_heartbeat_age)
            .await
    }

    pub(crate) async fn plan_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        pipeline_id: Uuid,
        route: RouteRevision,
        max_heartbeat_age: Duration,
    ) -> PortResult<PipelinePlanningDecision> {
        let (estimate, route_config) = validate_route(transaction, route).await?;
        let decision = match reserve_in_transaction(
            transaction,
            pipeline_id,
            &estimate.requirements,
            max_heartbeat_age,
        )
        .await?
        {
            ReservationDecision::Reserved(reservation) => {
                let assignment = PgAssignments::plan_in_transaction(
                    transaction,
                    reservation.id,
                    &estimate.requirements,
                    max_heartbeat_age,
                    Some(&route_config),
                )
                .await?;
                PipelinePlanningDecision::Planned(PlannedPipeline {
                    estimate,
                    assignment,
                })
            }
            ReservationDecision::Unavailable { rejections } => {
                PipelinePlanningDecision::Unavailable {
                    estimate,
                    rejections,
                }
            }
        };
        Ok(decision)
    }
}

pub(crate) async fn validate_route(
    transaction: &mut Transaction<'_, Postgres>,
    route: RouteRevision,
) -> PortResult<(PipelineEstimate, PlannedRouteConfig)> {
    if route.source_id.is_nil()
        || route.destination_id.is_nil()
        || route.source_revision <= 0
        || route.destination_revision <= 0
    {
        return Err(error_with(
            "invalid_route_revision",
            "source and destination IDs and positive revisions are required",
        ));
    }
    let source = load_source(transaction, route).await?;
    let destination = load_destination(transaction, route).await?;
    let estimate = estimate_pipeline_requirements(&source, &destination)
        .map_err(|error| error_with("invalid_media_profile", error))?;
    let route_config = PlannedRouteConfig {
        source_id: route.source_id,
        source_revision: route.source_revision,
        destination_id: route.destination_id,
        destination_revision: route.destination_revision,
        media_path: estimate.media_path.as_str().to_owned(),
        configuration: json!({
            "source": source,
            "destination": destination,
        }),
    };
    Ok((estimate, route_config))
}

async fn load_source(
    transaction: &mut Transaction<'_, Postgres>,
    route: RouteRevision,
) -> PortResult<SourceMediaProfile> {
    let row: StoredEndpoint = sqlx::query_as(
        "SELECT s.current_revision, r.lifecycle, s.source_type AS adapter_id, r.configuration \
         FROM sources s JOIN source_revisions r \
           ON r.source_id = s.id AND r.revision = s.current_revision \
         WHERE s.id = $1 FOR SHARE OF s, r",
    )
    .bind(route.source_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| error_with("database_error", error))?
    .ok_or_else(|| {
        error_with(
            "source_not_found",
            "source or its current revision does not exist",
        )
    })?;
    if row.current_revision != route.source_revision {
        return Err(error_with(
            "stale_route_revision",
            "source has changed since this route was selected",
        ));
    }
    if !matches!(row.lifecycle.as_str(), "validated" | "active") {
        return Err(error_with(
            "source_not_validated",
            "source revision must be validated before planning",
        ));
    }
    let profile: SourceMediaProfile = serde_json::from_value(row.configuration)
        .map_err(|error| error_with("invalid_source_record", error))?;
    if profile.adapter_id() != row.adapter_id {
        return Err(error_with(
            "invalid_source_record",
            "source adapter does not match its stored profile",
        ));
    }
    Ok(profile)
}

async fn load_destination(
    transaction: &mut Transaction<'_, Postgres>,
    route: RouteRevision,
) -> PortResult<DestinationMediaProfile> {
    let row: StoredEndpoint = sqlx::query_as(
        "SELECT d.current_revision, r.lifecycle, d.destination_type AS adapter_id, r.configuration \
         FROM destinations d JOIN destination_revisions r \
           ON r.destination_id = d.id AND r.revision = d.current_revision \
         WHERE d.id = $1 FOR SHARE OF d, r",
    )
    .bind(route.destination_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| error_with("database_error", error))?
    .ok_or_else(|| {
        error_with(
            "destination_not_found",
            "destination or its current revision does not exist",
        )
    })?;
    if row.current_revision != route.destination_revision {
        return Err(error_with(
            "stale_route_revision",
            "destination has changed since this route was selected",
        ));
    }
    if !matches!(row.lifecycle.as_str(), "validated" | "active") {
        return Err(error_with(
            "destination_not_validated",
            "destination revision must be validated before planning",
        ));
    }
    let profile: DestinationMediaProfile = serde_json::from_value(row.configuration)
        .map_err(|error| error_with("invalid_destination_record", error))?;
    if profile.adapter_id() != row.adapter_id {
        return Err(error_with(
            "invalid_destination_record",
            "destination adapter does not match its stored profile",
        ));
    }
    Ok(profile)
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, env, net::IpAddr};

    use arc_application::WorkerRegistry;
    use arc_domain::{
        Architecture, PlacementRejection, Platform, Worker, WorkerCapabilities, WorkerCapacity,
        WorkerState,
    };
    use chrono::Utc;

    use crate::{
        capacity_reservations::PgCapacityReservations,
        catalog,
        postgres::{connect_postgres, migrate},
        worker_registry::PgWorkerRegistry,
    };

    use super::*;

    #[tokio::test]
    #[ignore = "requires an isolated PostgreSQL database; set DATABASE_URL and run ignored tests"]
    async fn planning_is_atomic_and_serializes_capacity_admission() {
        let url = env::var("DATABASE_URL").expect("DATABASE_URL is set");
        let pool = connect_postgres(&url).await.unwrap();
        migrate(&pool).await.unwrap();
        let registry = PgWorkerRegistry::new(pool.clone());
        let planner = PgPipelinePlanner::new(pool.clone());
        let assignments = PgAssignments::new(pool.clone());
        let reservations = PgCapacityReservations::new(pool.clone());
        let worker_id = Uuid::new_v4();
        let mut worker = Worker {
            id: worker_id,
            stable_name: format!("worker-{worker_id}"),
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
        registry.heartbeat(worker).await.unwrap();
        let address = IpAddr::from([127, 0, 0, 1]);
        let source_profile = SourceMediaProfile::RtpL16 {
            bind_address: address,
            port: 5000,
            payload_type: 96,
        };
        let destination_profile = DestinationMediaProfile::RtpL16 {
            address,
            port: 5002,
            payload_type: 96,
        };
        let source_record = catalog::create_source(
            &pool,
            "Planning source",
            source_profile.adapter_id(),
            serde_json::to_value(&source_profile).unwrap(),
        )
        .await
        .unwrap();
        let source_record = catalog::validate_source_draft(&pool, source_record.id)
            .await
            .unwrap();
        let destination_record = catalog::create_destination(
            &pool,
            "Planning destination",
            destination_profile.adapter_id(),
            serde_json::to_value(&destination_profile).unwrap(),
        )
        .await
        .unwrap();
        let destination_record = catalog::validate_destination_draft(&pool, destination_record.id)
            .await
            .unwrap();
        let route = RouteRevision {
            source_id: source_record.id,
            source_revision: source_record.current_revision,
            destination_id: destination_record.id,
            destination_revision: destination_record.current_revision,
        };
        let age = Duration::seconds(60);
        let invalid_profile = SourceMediaProfile::RtpL16 {
            bind_address: address,
            port: 0,
            payload_type: 96,
        };
        let invalid_source = catalog::create_source(
            &pool,
            "Invalid source",
            invalid_profile.adapter_id(),
            serde_json::to_value(&invalid_profile).unwrap(),
        )
        .await
        .unwrap();
        catalog::validate_source_draft(&pool, invalid_source.id)
            .await
            .unwrap();
        let invalid_route = RouteRevision {
            source_id: invalid_source.id,
            ..route
        };
        assert_eq!(
            planner
                .plan(Uuid::new_v4(), invalid_route, age)
                .await
                .unwrap_err()
                .code,
            "invalid_media_profile"
        );
        assert_eq!(
            planner
                .plan(Uuid::nil(), route, age)
                .await
                .unwrap_err()
                .code,
            "invalid_reservation"
        );
        let stale_route = RouteRevision {
            source_revision: route.source_revision + 1,
            ..route
        };
        assert_eq!(
            planner
                .plan(Uuid::new_v4(), stale_route, age)
                .await
                .unwrap_err()
                .code,
            "stale_route_revision"
        );

        let conflicted_pipeline = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO pipeline_assignments \
             (id, resource_type, resource_id, worker_id, generation, desired_state, observed_state) \
             VALUES ($1, 'pipeline', $2, $3, 1, 'running', 'running')",
        )
        .bind(Uuid::new_v4())
        .bind(conflicted_pipeline)
        .bind(worker_id)
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(
            planner
                .plan(conflicted_pipeline, route, age)
                .await
                .unwrap_err()
                .code,
            "assignment_conflict"
        );
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM worker_capacity_reservations WHERE pipeline_id = $1",
        )
        .bind(conflicted_pipeline)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 0, "failed assignment must roll back its reservation");
        sqlx::query("DELETE FROM pipeline_assignments WHERE resource_id = $1")
            .bind(conflicted_pipeline)
            .execute(&pool)
            .await
            .unwrap();

        let first_pipeline = Uuid::new_v4();
        let second_pipeline = Uuid::new_v4();
        let (first, second) = tokio::join!(
            planner.plan(first_pipeline, route, age),
            planner.plan(second_pipeline, route, age),
        );
        let planned = match (first.unwrap(), second.unwrap()) {
            (
                PipelinePlanningDecision::Planned(plan),
                PipelinePlanningDecision::Unavailable { rejections, .. },
            )
            | (
                PipelinePlanningDecision::Unavailable { rejections, .. },
                PipelinePlanningDecision::Planned(plan),
            ) => {
                assert!(matches!(
                    rejections[0].reason,
                    PlacementRejection::InsufficientCapacity {
                        required: 2,
                        available: 0
                    }
                ));
                plan
            }
            other => panic!("expected one plan and one capacity rejection, got {other:?}"),
        };
        assert_eq!(planned.assignment.worker_id, worker_id);
        assert_eq!(
            planned.assignment.requirements,
            planned.estimate.requirements
        );
        assert_eq!(planned.assignment.generation, 1);
        assert_eq!(
            planned.assignment.route,
            Some(PlannedRouteConfig {
                source_id: route.source_id,
                source_revision: route.source_revision,
                destination_id: route.destination_id,
                destination_revision: route.destination_revision,
                media_path: "rtp_l16_to_rtp_l16".to_owned(),
                configuration: json!({
                    "source": source_profile,
                    "destination": destination_profile,
                }),
            })
        );
        let reconnected = PgAssignments::new(connect_postgres(&url).await.unwrap());
        assert_eq!(
            reconnected.list_planned().await.unwrap(),
            vec![planned.assignment.clone()]
        );
        assert_eq!(
            reservations.list_active().await.unwrap()[0].id,
            planned.assignment.reservation_id
        );
        let updated_source_profile = SourceMediaProfile::RtpL16 {
            bind_address: address,
            port: 5010,
            payload_type: 96,
        };
        let updated_source = catalog::update_source(
            &pool,
            source_record.id,
            "Planning source",
            updated_source_profile.adapter_id(),
            serde_json::to_value(&updated_source_profile).unwrap(),
        )
        .await
        .unwrap();
        let updated_source = catalog::validate_source_draft(&pool, updated_source.id)
            .await
            .unwrap();
        assert_eq!(updated_source.current_revision, route.source_revision + 1);
        assert_eq!(
            reconnected.list_planned().await.unwrap()[0].route,
            planned.assignment.route
        );
        assert_eq!(
            planner
                .plan(planned.assignment.pipeline_id, route, age)
                .await
                .unwrap_err()
                .code,
            "stale_route_revision"
        );
        assignments
            .cancel_unstarted(planned.assignment.id, planned.assignment.generation)
            .await
            .unwrap();
        assert_eq!(
            planner
                .plan(planned.assignment.pipeline_id, route, age)
                .await
                .unwrap_err()
                .code,
            "stale_route_revision"
        );
        let updated_route = RouteRevision {
            source_revision: updated_source.current_revision,
            ..route
        };
        let replacement = match planner
            .plan(planned.assignment.pipeline_id, updated_route, age)
            .await
            .unwrap()
        {
            PipelinePlanningDecision::Planned(plan) => plan,
            other => panic!("expected replacement plan, got {other:?}"),
        };
        assert_eq!(replacement.assignment.generation, 2);
        assignments
            .cancel_unstarted(replacement.assignment.id, replacement.assignment.generation)
            .await
            .unwrap();
        let duplicate_pipeline = Uuid::new_v4();
        let (one, two) = tokio::join!(
            planner.plan(duplicate_pipeline, updated_route, age),
            planner.plan(duplicate_pipeline, updated_route, age),
        );
        let duplicate = match (one, two) {
            (Ok(PipelinePlanningDecision::Planned(plan)), Err(error))
            | (Err(error), Ok(PipelinePlanningDecision::Planned(plan)))
                if error.code == "reservation_conflict" =>
            {
                plan
            }
            other => panic!("expected one plan and a conflict, got {other:?}"),
        };
        assert_eq!(reservations.list_active().await.unwrap().len(), 1);
        assignments
            .cancel_unstarted(duplicate.assignment.id, duplicate.assignment.generation)
            .await
            .unwrap();

        let idempotency_key = Uuid::new_v4();
        let idempotent_pipeline = Uuid::new_v4();
        let (one, two) = tokio::join!(
            planner.plan_with_idempotency_key(
                idempotency_key,
                idempotent_pipeline,
                updated_route,
                age,
            ),
            planner.plan_with_idempotency_key(
                idempotency_key,
                idempotent_pipeline,
                updated_route,
                age,
            ),
        );
        let replayed = match (one.unwrap(), two.unwrap()) {
            (
                PipelinePlanningDecision::Planned(first),
                PipelinePlanningDecision::Planned(second),
            ) => {
                assert_eq!(first, second);
                first
            }
            other => panic!("expected both idempotent calls to replay, got {other:?}"),
        };
        assert_eq!(reservations.list_active().await.unwrap().len(), 1);
        let changed_route = RouteRevision {
            destination_revision: updated_route.destination_revision + 1,
            ..updated_route
        };
        assert_eq!(
            planner
                .plan_with_idempotency_key(
                    idempotency_key,
                    idempotent_pipeline,
                    changed_route,
                    age,
                )
                .await
                .unwrap_err()
                .code,
            "idempotency_key_reused"
        );
        assignments
            .cancel_unstarted(replayed.assignment.id, replayed.assignment.generation)
            .await
            .unwrap();

        registry.begin_drain(worker_id).await.unwrap();
        assert!(matches!(
            planner.plan(Uuid::new_v4(), updated_route, age).await.unwrap(),
            PipelinePlanningDecision::Unavailable { rejections, .. }
                if matches!(rejections[0].reason, PlacementRejection::WorkerNotAccepting)
        ));
        assert!(reservations.list_active().await.unwrap().is_empty());
        sqlx::query("DELETE FROM pipeline_assignments WHERE worker_id = $1")
            .bind(worker_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM worker_capacity_reservations WHERE worker_id = $1")
            .bind(worker_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM workers WHERE id = $1")
            .bind(worker_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM sources WHERE id IN ($1, $2)")
            .bind(source_record.id)
            .bind(invalid_source.id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM destinations WHERE id = $1")
            .bind(destination_record.id)
            .execute(&pool)
            .await
            .unwrap();
    }
}
