use std::collections::HashMap;

use arc_application::{PortError, PortResult};
use arc_domain::{select_worker, PipelineRequirements, PlacementDecision, WorkerRejection};
use chrono::{DateTime, Duration, Utc};
use sqlx::{FromRow, PgConnection, PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{
    capacity_wait::{has_waiters, lock_queue},
    worker_registry::{error_with, WorkerRow},
};

// Phase 1 estimates one source unit and one destination-branch unit per route.
const SHARED_SOURCE_CAPACITY_UNITS: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapacityReservation {
    pub id: Uuid,
    pub pipeline_id: Uuid,
    pub worker_id: Uuid,
    pub capacity_units: u32,
}

#[derive(FromRow)]
struct ReservationRow {
    id: Uuid,
    pipeline_id: Uuid,
    worker_id: Uuid,
    capacity_units: i64,
}

impl TryFrom<ReservationRow> for CapacityReservation {
    type Error = PortError;

    fn try_from(row: ReservationRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            pipeline_id: row.pipeline_id,
            worker_id: row.worker_id,
            capacity_units: u32::try_from(row.capacity_units)
                .map_err(|error| error_with("invalid_reservation_record", error))?,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReservationDecision {
    Reserved(CapacityReservation),
    Unavailable { rejections: Vec<WorkerRejection> },
}

pub struct PgCapacityReservations {
    pool: PgPool,
}

pub(crate) async fn select_with_reservations(
    connection: &mut PgConnection,
    rows: Vec<WorkerRow>,
    requirements: &PipelineRequirements,
    max_heartbeat_age: Duration,
) -> PortResult<PlacementDecision> {
    let totals = reservation_capacity_totals(connection, None, None).await?;
    let mut workers = Vec::with_capacity(rows.len());
    for row in rows {
        let mut worker = row.into_worker()?;
        if let Some(&reserved) = totals.get(&worker.id) {
            // Saturation conservatively prevents positive requests from fitting.
            let reserved = if reserved > i64::from(u32::MAX) {
                u32::MAX
            } else {
                u32::try_from(reserved)
                    .map_err(|error| error_with("invalid_reservation_record", error))?
            };
            worker.capacity.assigned_capacity_units = worker
                .capacity
                .assigned_capacity_units
                .saturating_add(reserved);
        }
        workers.push(worker);
    }
    let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut *connection)
        .await
        .map_err(|error| error_with("database_error", error))?;
    select_worker(&workers, requirements, now, max_heartbeat_age)
        .map_err(|error| error_with("invalid_placement", format!("{error:?}")))
}

pub(crate) async fn reservation_capacity_totals(
    connection: &mut PgConnection,
    worker_filter: Option<Uuid>,
    pending_route: Option<(Uuid, Uuid, i64)>,
) -> PortResult<HashMap<Uuid, i64>> {
    let (reservation_id, source_id, source_revision) = pending_route
        .map(|(reservation_id, source_id, revision)| {
            (Some(reservation_id), Some(source_id), Some(revision))
        })
        .unwrap_or((None, None, None));
    let totals: Vec<(Uuid, i64)> = sqlx::query_as(
        "WITH active_reservations AS ( \
             SELECT reservations.worker_id, reservations.capacity_units, \
                    COALESCE(assignments.source_id, \
                        CASE WHEN reservations.id = $3 THEN $4::uuid END) AS source_id, \
                    COALESCE(assignments.source_revision, \
                        CASE WHEN reservations.id = $3 THEN $5::bigint END) AS source_revision \
             FROM worker_capacity_reservations AS reservations \
             LEFT JOIN pipeline_assignments AS assignments \
               ON assignments.reservation_id = reservations.id \
             WHERE reservations.state = 'reserved' \
               AND ($2::uuid IS NULL OR reservations.worker_id = $2) \
         ), reservation_totals AS ( \
             SELECT worker_id, SUM(capacity_units)::bigint AS capacity_units \
             FROM active_reservations GROUP BY worker_id \
         ), shared_source_credits AS ( \
             SELECT worker_id, \
                    SUM((route_count - 1) * $1)::bigint AS capacity_units \
             FROM ( \
                 SELECT worker_id, source_id, source_revision, \
                        COUNT(*)::bigint AS route_count \
                 FROM active_reservations \
                 WHERE source_id IS NOT NULL AND source_revision IS NOT NULL \
                 GROUP BY worker_id, source_id, source_revision \
                 HAVING COUNT(*) > 1 \
             ) AS source_groups \
             GROUP BY worker_id \
         ) \
         SELECT reservation_totals.worker_id, \
                reservation_totals.capacity_units \
                  - COALESCE(shared_source_credits.capacity_units, 0) \
         FROM reservation_totals \
         LEFT JOIN shared_source_credits USING (worker_id)",
    )
    .bind(i64::from(SHARED_SOURCE_CAPACITY_UNITS))
    .bind(worker_filter)
    .bind(reservation_id)
    .bind(source_id)
    .bind(source_revision)
    .fetch_all(&mut *connection)
    .await
    .map_err(|error| error_with("database_error", error))?;
    Ok(totals.into_iter().collect())
}

impl PgCapacityReservations {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn reserve(
        &self,
        pipeline_id: Uuid,
        requirements: &PipelineRequirements,
        max_heartbeat_age: Duration,
    ) -> PortResult<ReservationDecision> {
        validate_reservation(pipeline_id, requirements, max_heartbeat_age)?;
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| error_with("database_error", error))?;
        lock_queue(&mut transaction).await?;
        if has_waiters(&mut transaction).await? {
            transaction
                .commit()
                .await
                .map_err(|error| error_with("database_error", error))?;
            return Ok(ReservationDecision::Unavailable {
                rejections: Vec::new(),
            });
        }
        let decision = reserve_in_transaction(
            &mut transaction,
            pipeline_id,
            requirements,
            max_heartbeat_age,
            None,
        )
        .await?;
        transaction
            .commit()
            .await
            .map_err(|error| error_with("database_error", error))?;
        Ok(decision)
    }

    pub async fn list_active(&self) -> PortResult<Vec<CapacityReservation>> {
        let rows = sqlx::query_as::<_, ReservationRow>(
            "SELECT id, pipeline_id, worker_id, capacity_units \
             FROM worker_capacity_reservations WHERE state = 'reserved' ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|error| error_with("database_error", error))?;
        rows.into_iter()
            .map(CapacityReservation::try_from)
            .collect()
    }

    pub async fn release(&self, reservation_id: Uuid) -> PortResult<()> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| error_with("database_error", error))?;
        let worker_id: Option<Uuid> =
            sqlx::query_scalar("SELECT worker_id FROM worker_capacity_reservations WHERE id = $1")
                .bind(reservation_id)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(|error| error_with("database_error", error))?;
        let worker_id = worker_id.ok_or_else(|| PortError {
            code: "reservation_not_found",
            message: "capacity reservation does not exist".to_owned(),
        })?;
        sqlx::query("SELECT id FROM workers WHERE id = $1 FOR UPDATE")
            .bind(worker_id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(|error| error_with("database_error", error))?;
        let result = sqlx::query(
            "UPDATE worker_capacity_reservations SET state = 'released', \
             released_at = clock_timestamp() WHERE id = $1 AND state = 'reserved' \
             AND NOT EXISTS (SELECT 1 FROM pipeline_assignments \
                 WHERE reservation_id = $1)",
        )
        .bind(reservation_id)
        .execute(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?;
        if result.rows_affected() != 1 {
            let state: String =
                sqlx::query_scalar("SELECT state FROM worker_capacity_reservations WHERE id = $1")
                    .bind(reservation_id)
                    .fetch_one(&mut *transaction)
                    .await
                    .map_err(|error| error_with("database_error", error))?;
            let in_use = state == "reserved";
            return Err(PortError {
                code: if in_use {
                    "reservation_in_use"
                } else {
                    "reservation_released"
                },
                message: if in_use {
                    "capacity reservation belongs to an active assignment"
                } else {
                    "capacity reservation has already been released"
                }
                .to_owned(),
            });
        }
        transaction
            .commit()
            .await
            .map_err(|error| error_with("database_error", error))?;
        Ok(())
    }
}

fn validate_reservation(
    pipeline_id: Uuid,
    requirements: &PipelineRequirements,
    max_heartbeat_age: Duration,
) -> PortResult<()> {
    if pipeline_id.is_nil()
        || requirements.capacity_units == 0
        || max_heartbeat_age < Duration::zero()
    {
        return Err(PortError {
            code: "invalid_reservation",
            message: "pipeline ID and positive capacity and nonnegative heartbeat age are required"
                .to_owned(),
        });
    }
    Ok(())
}

pub(crate) async fn reserve_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    pipeline_id: Uuid,
    requirements: &PipelineRequirements,
    max_heartbeat_age: Duration,
    source_identity: Option<(Uuid, i64)>,
) -> PortResult<ReservationDecision> {
    validate_reservation(pipeline_id, requirements, max_heartbeat_age)?;
    if source_identity.is_some() {
        sqlx::query("SELECT id FROM workers ORDER BY id FOR UPDATE")
            .fetch_all(&mut **transaction)
            .await
            .map_err(|error| error_with("database_error", error))?;
    }
    let required_worker_id = match source_identity {
        Some((source_id, source_revision)) => {
            source_worker_affinity(transaction, source_id, source_revision).await?
        }
        None => None,
    };
    let rows = if let Some(worker_id) = required_worker_id {
        sqlx::query_as::<_, WorkerRow>(
            "SELECT id, stable_name, platform, architecture, version, state, \
                 capabilities, capacity, last_heartbeat FROM workers WHERE id = $1 FOR UPDATE",
        )
        .bind(worker_id)
        .fetch_all(&mut **transaction)
        .await
    } else {
        sqlx::query_as::<_, WorkerRow>(
            "SELECT id, stable_name, platform, architecture, version, state, \
                 capabilities, capacity, last_heartbeat FROM workers ORDER BY id FOR UPDATE",
        )
        .fetch_all(&mut **transaction)
        .await
    }
    .map_err(|error| error_with("database_error", error))?;
    if required_worker_id.is_some() && rows.is_empty() {
        return Err(error_with(
            "source_worker_unavailable",
            "the worker hosting this source revision is no longer registered",
        ));
    }
    let already_reserved: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM worker_capacity_reservations \
             WHERE pipeline_id = $1 AND state = 'reserved')",
    )
    .bind(pipeline_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| error_with("database_error", error))?;
    if already_reserved {
        return Err(PortError {
            code: "reservation_conflict",
            message: "pipeline already has an active capacity reservation".to_owned(),
        });
    }

    let has_reserved_source = if let Some((source_id, source_revision)) = source_identity {
        sqlx::query_scalar(
            "SELECT EXISTS( \
                 SELECT 1 FROM pipeline_assignments AS assignments \
                 JOIN worker_capacity_reservations AS reservations \
                   ON reservations.id = assignments.reservation_id \
                 WHERE assignments.resource_type = 'pipeline' \
                   AND assignments.source_id = $1 \
                   AND assignments.source_revision = $2 \
                   AND reservations.state = 'reserved' \
             )",
        )
        .bind(source_id)
        .bind(source_revision)
        .fetch_one(&mut **transaction)
        .await
        .map_err(|error| error_with("database_error", error))?
    } else {
        false
    };
    let mut placement_requirements = requirements.clone();
    if has_reserved_source {
        placement_requirements.capacity_units = placement_requirements
            .capacity_units
            .saturating_sub(SHARED_SOURCE_CAPACITY_UNITS);
    }

    let decision = select_with_reservations(
        &mut **transaction,
        rows,
        &placement_requirements,
        max_heartbeat_age,
    )
    .await?;
    let worker_id = match decision {
        PlacementDecision::Selected { worker_id } => worker_id,
        PlacementDecision::Unavailable { rejections } => {
            return Ok(ReservationDecision::Unavailable { rejections });
        }
    };
    let reservation = CapacityReservation {
        id: Uuid::new_v4(),
        pipeline_id,
        worker_id,
        capacity_units: requirements.capacity_units,
    };
    sqlx::query(
        "INSERT INTO worker_capacity_reservations \
             (id, pipeline_id, worker_id, capacity_units, state) \
             VALUES ($1, $2, $3, $4, 'reserved')",
    )
    .bind(reservation.id)
    .bind(reservation.pipeline_id)
    .bind(reservation.worker_id)
    .bind(i64::from(reservation.capacity_units))
    .execute(&mut **transaction)
    .await
    .map_err(|error| error_with("database_error", error))?;
    Ok(ReservationDecision::Reserved(reservation))
}

async fn source_worker_affinity(
    transaction: &mut Transaction<'_, Postgres>,
    source_id: Uuid,
    source_revision: i64,
) -> PortResult<Option<Uuid>> {
    let worker_ids: Vec<Option<Uuid>> = sqlx::query_scalar(
        "SELECT DISTINCT worker_id FROM pipeline_assignments \
         WHERE resource_type = 'pipeline' AND source_id = $1 AND source_revision = $2 \
           AND ((desired_state = 'planned' AND observed_state <> 'cancelled') \
             OR desired_state = 'running' \
             OR (desired_state = 'stopped' AND observed_state = 'stopping')) \
         ORDER BY worker_id",
    )
    .bind(source_id)
    .bind(source_revision)
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| error_with("database_error", error))?;

    match worker_ids.as_slice() {
        [] => Ok(None),
        [Some(worker_id)] => Ok(Some(*worker_id)),
        _ => Err(error_with(
            "source_worker_affinity_conflict",
            "active routes for this source revision do not have one registered worker placement",
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, env};

    use arc_application::WorkerRegistry;
    use arc_domain::{
        Architecture, PlacementRejection, Platform, Worker, WorkerCapabilities, WorkerCapacity,
        WorkerState,
    };

    use crate::{
        postgres::{connect_postgres, migrate},
        worker_registry::PgWorkerRegistry,
    };

    use super::*;

    #[tokio::test]
    #[ignore = "requires an isolated PostgreSQL database; set DATABASE_URL and run ignored tests"]
    async fn reservations_serialize_admission_and_survive_reconnection() {
        let url = env::var("DATABASE_URL").expect("DATABASE_URL is set");
        let pool = connect_postgres(&url).await.expect("database connects");
        migrate(&pool).await.expect("migration succeeds");
        let registry = PgWorkerRegistry::new(pool.clone());
        let id = Uuid::new_v4();
        let mut worker = Worker {
            id,
            stable_name: format!("worker-{id}"),
            version: "0.1.0".to_owned(),
            state: WorkerState::Registering,
            capabilities: WorkerCapabilities {
                platform: Some(Platform::Linux),
                architecture: Some(Architecture::X86_64),
                source_adapters: BTreeSet::from(["rtp_l16".to_owned()]),
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
        registry.register(worker.clone()).await.expect("register");
        worker.state = WorkerState::Accepting;
        registry.heartbeat(worker.clone()).await.expect("heartbeat");
        let reservations = PgCapacityReservations::new(pool.clone());
        let requirements = PipelineRequirements {
            source_adapter: Some("rtp_l16".to_owned()),
            capacity_units: 3,
            ..PipelineRequirements::default()
        };
        let first_pipeline = Uuid::new_v4();
        let second_pipeline = Uuid::new_v4();
        let (first, second) = tokio::join!(
            reservations.reserve(first_pipeline, &requirements, Duration::seconds(60)),
            reservations.reserve(second_pipeline, &requirements, Duration::seconds(60)),
        );
        let (reservation, rejected) = match (first.expect("first"), second.expect("second")) {
            (
                ReservationDecision::Reserved(value),
                ReservationDecision::Unavailable { rejections },
            )
            | (
                ReservationDecision::Unavailable { rejections },
                ReservationDecision::Reserved(value),
            ) => (value, rejections),
            other => {
                panic!("expected one reservation and one rejection, got {other:?}")
            }
        };
        assert_eq!(reservation.worker_id, id);
        assert_eq!(
            rejected[0].reason,
            PlacementRejection::InsufficientCapacity {
                required: 3,
                available: 1
            }
        );
        assert_eq!(
            reservations
                .reserve(
                    reservation.pipeline_id,
                    &requirements,
                    Duration::seconds(60)
                )
                .await
                .unwrap_err()
                .code,
            "reservation_conflict"
        );

        worker.capacity.assigned_capacity_units = 1;
        registry.heartbeat(worker.clone()).await.expect("heartbeat");
        let reconnected_pool = connect_postgres(&url).await.expect("database reconnects");
        let reconnected = PgCapacityReservations::new(reconnected_pool);
        assert_eq!(
            reconnected.list_active().await.expect("load reservations"),
            vec![reservation.clone()]
        );
        assert!(matches!(
            reconnected
                .reserve(Uuid::new_v4(), &requirements, Duration::seconds(60))
                .await
                .expect("reservation check"),
            ReservationDecision::Unavailable { rejections }
                if rejections[0].reason == PlacementRejection::InsufficientCapacity {
                    required: 3, available: 0
                }
        ));
        reconnected.release(reservation.id).await.expect("release");
        assert_eq!(
            reconnected.release(reservation.id).await.unwrap_err().code,
            "reservation_released"
        );
        let replacement = match reconnected
            .reserve(
                reservation.pipeline_id,
                &requirements,
                Duration::seconds(60),
            )
            .await
            .expect("reserve after release")
        {
            ReservationDecision::Reserved(value) => value,
            other => panic!("expected reservation, got {other:?}"),
        };
        reconnected.release(replacement.id).await.expect("release");
        assert!(reconnected
            .list_active()
            .await
            .expect("list active")
            .is_empty());

        let pipeline = Uuid::new_v4();
        let (first, second) = tokio::join!(
            reservations.reserve(pipeline, &requirements, Duration::seconds(60)),
            reservations.reserve(pipeline, &requirements, Duration::seconds(60)),
        );
        let active = match (first, second) {
            (Ok(ReservationDecision::Reserved(value)), Err(error))
            | (Err(error), Ok(ReservationDecision::Reserved(value)))
                if error.code == "reservation_conflict" =>
            {
                value
            }
            other => panic!("expected a reservation and a conflict, got {other:?}"),
        };
        reservations.release(active.id).await.expect("release");

        sqlx::query("UPDATE workers SET last_heartbeat = clock_timestamp() - interval '1 hour' WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .expect("expire heartbeat");
        assert!(matches!(
            reservations
                .reserve(Uuid::new_v4(), &requirements, Duration::seconds(60))
                .await
                .expect("stale worker check"),
            ReservationDecision::Unavailable { rejections }
                if rejections[0].reason == PlacementRejection::HeartbeatExpired
        ));
        registry.heartbeat(worker).await.expect("refresh heartbeat");
        let incompatible = PipelineRequirements {
            source_adapter: Some("missing".to_owned()),
            ..requirements.clone()
        };
        assert!(matches!(
            reservations
                .reserve(Uuid::new_v4(), &incompatible, Duration::seconds(60))
                .await
                .expect("capability check"),
            ReservationDecision::Unavailable { rejections }
                if matches!(&rejections[0].reason, PlacementRejection::MissingCapabilities(_))
        ));
        sqlx::query("UPDATE workers SET state = 'draining' WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .expect("drain worker");
        assert!(matches!(
            reservations
                .reserve(Uuid::new_v4(), &requirements, Duration::seconds(60))
                .await
                .expect("drain check"),
            ReservationDecision::Unavailable { rejections }
                if rejections[0].reason == PlacementRejection::WorkerNotAccepting
        ));
        assert_eq!(
            reservations
                .reserve(Uuid::nil(), &requirements, Duration::seconds(60))
                .await
                .unwrap_err()
                .code,
            "invalid_reservation"
        );
        let zero_capacity = PipelineRequirements {
            capacity_units: 0,
            ..requirements.clone()
        };
        assert_eq!(
            reservations
                .reserve(Uuid::new_v4(), &zero_capacity, Duration::seconds(60))
                .await
                .unwrap_err()
                .code,
            "invalid_reservation"
        );
        assert_eq!(
            reservations
                .reserve(Uuid::new_v4(), &requirements, Duration::seconds(-1))
                .await
                .unwrap_err()
                .code,
            "invalid_reservation"
        );
        assert_eq!(
            reservations.release(Uuid::new_v4()).await.unwrap_err().code,
            "reservation_not_found"
        );
        sqlx::query("DELETE FROM worker_capacity_reservations WHERE worker_id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .expect("remove test reservations");
        sqlx::query("DELETE FROM workers WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .expect("remove test worker");
        assert_eq!(
            reservations
                .reserve(Uuid::new_v4(), &requirements, Duration::seconds(60))
                .await
                .expect("empty registry"),
            ReservationDecision::Unavailable {
                rejections: Vec::new()
            }
        );
    }
}
