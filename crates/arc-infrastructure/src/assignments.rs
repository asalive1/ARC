use arc_application::{PortError, PortResult};
use arc_domain::{select_worker, PipelineRequirements, PlacementDecision};
use chrono::{DateTime, Duration, Utc};
use sqlx::{types::Json, FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::worker_registry::{error_with, WorkerRow};

#[derive(Clone, Debug, PartialEq)]
pub struct PlannedAssignment {
    pub id: Uuid,
    pub pipeline_id: Uuid,
    pub worker_id: Uuid,
    pub generation: i64,
    pub reservation_id: Uuid,
    pub requirements: PipelineRequirements,
}

#[derive(FromRow)]
struct AssignmentRow {
    id: Uuid,
    pipeline_id: Uuid,
    worker_id: Uuid,
    generation: i64,
    reservation_id: Uuid,
    requirements: Json<PipelineRequirements>,
}

impl From<AssignmentRow> for PlannedAssignment {
    fn from(row: AssignmentRow) -> Self {
        Self {
            id: row.id,
            pipeline_id: row.pipeline_id,
            worker_id: row.worker_id,
            generation: row.generation,
            reservation_id: row.reservation_id,
            requirements: row.requirements.0,
        }
    }
}

#[derive(FromRow)]
struct ReservationRow {
    pipeline_id: Uuid,
    worker_id: Uuid,
    capacity_units: i64,
    state: String,
}

pub struct PgAssignments {
    pool: PgPool,
}

impl PgAssignments {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn plan(
        &self,
        reservation_id: Uuid,
        requirements: &PipelineRequirements,
        max_heartbeat_age: Duration,
    ) -> PortResult<PlannedAssignment> {
        validate_assignment(reservation_id, requirements, max_heartbeat_age)?;
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| error_with("database_error", error))?;
        let assignment = Self::plan_in_transaction(
            &mut transaction,
            reservation_id,
            requirements,
            max_heartbeat_age,
        )
        .await?;
        transaction
            .commit()
            .await
            .map_err(|error| error_with("database_error", error))?;
        Ok(assignment)
    }

    pub(crate) async fn plan_in_transaction(
        transaction: &mut Transaction<'_, Postgres>,
        reservation_id: Uuid,
        requirements: &PipelineRequirements,
        max_heartbeat_age: Duration,
    ) -> PortResult<PlannedAssignment> {
        validate_assignment(reservation_id, requirements, max_heartbeat_age)?;
        let worker_id: Option<Uuid> =
            sqlx::query_scalar("SELECT worker_id FROM worker_capacity_reservations WHERE id = $1")
                .bind(reservation_id)
                .fetch_optional(&mut **transaction)
                .await
                .map_err(|error| error_with("database_error", error))?;
        let worker_id = worker_id.ok_or_else(|| PortError {
            code: "reservation_not_found",
            message: "capacity reservation does not exist".to_owned(),
        })?;
        let row: WorkerRow = sqlx::query_as(
            "SELECT id, stable_name, platform, architecture, version, state, \
             capabilities, capacity, last_heartbeat FROM workers WHERE id = $1 FOR UPDATE",
        )
        .bind(worker_id)
        .fetch_one(&mut **transaction)
        .await
        .map_err(|error| error_with("database_error", error))?;
        let mut worker = row.into_worker()?;
        let reservation: ReservationRow = sqlx::query_as(
            "SELECT pipeline_id, worker_id, capacity_units, state \
             FROM worker_capacity_reservations WHERE id = $1 FOR UPDATE",
        )
        .bind(reservation_id)
        .fetch_one(&mut **transaction)
        .await
        .map_err(|error| error_with("database_error", error))?;
        if reservation.state != "reserved" {
            return Err(PortError {
                code: "reservation_released",
                message: "capacity reservation is no longer active".to_owned(),
            });
        }
        if reservation.worker_id != worker_id
            || reservation.capacity_units != i64::from(requirements.capacity_units)
        {
            return Err(PortError {
                code: "invalid_assignment",
                message: "assignment requirements do not match the reservation".to_owned(),
            });
        }
        let existing: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pipeline_assignments \
             WHERE resource_type = 'pipeline' AND resource_id = $1 \
             AND desired_state NOT IN ('cancelled', 'removed'))",
        )
        .bind(reservation.pipeline_id)
        .fetch_one(&mut **transaction)
        .await
        .map_err(|error| error_with("database_error", error))?;
        if existing {
            return Err(PortError {
                code: "assignment_conflict",
                message: "pipeline already has an active assignment".to_owned(),
            });
        }

        let reserved: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(capacity_units), 0)::bigint FROM worker_capacity_reservations \
             WHERE worker_id = $1 AND state = 'reserved'",
        )
        .bind(worker_id)
        .fetch_one(&mut **transaction)
        .await
        .map_err(|error| error_with("database_error", error))?;
        let assigned = i128::from(worker.capacity.assigned_capacity_units) + i128::from(reserved);
        if assigned > i128::from(worker.capacity.total_capacity_units) || reserved < 0 {
            return Err(PortError {
                code: "placement_unavailable",
                message: "worker capacity no longer covers its commitments".to_owned(),
            });
        }
        worker.capacity.assigned_capacity_units = u32::try_from(assigned)
            .map_err(|error| error_with("invalid_reservation_record", error))?;
        let mut eligibility_requirements = requirements.clone();
        eligibility_requirements.capacity_units = 0;
        let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut **transaction)
            .await
            .map_err(|error| error_with("database_error", error))?;
        match select_worker(&[worker], &eligibility_requirements, now, max_heartbeat_age)
            .map_err(|error| error_with("invalid_assignment", format!("{error:?}")))?
        {
            PlacementDecision::Selected { .. } => {}
            PlacementDecision::Unavailable { rejections } => {
                return Err(error_with(
                    "placement_unavailable",
                    format!("{rejections:?}"),
                ))
            }
        }
        let previous: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(generation), 0) FROM pipeline_assignments \
             WHERE resource_type = 'pipeline' AND resource_id = $1",
        )
        .bind(reservation.pipeline_id)
        .fetch_one(&mut **transaction)
        .await
        .map_err(|error| error_with("database_error", error))?;
        let generation = previous.checked_add(1).ok_or_else(|| PortError {
            code: "generation_exhausted",
            message: "pipeline assignment generation is exhausted".to_owned(),
        })?;
        let assignment = PlannedAssignment {
            id: Uuid::new_v4(),
            pipeline_id: reservation.pipeline_id,
            worker_id,
            generation,
            reservation_id,
            requirements: requirements.clone(),
        };
        sqlx::query(
            "INSERT INTO pipeline_assignments \
             (id, resource_type, resource_id, worker_id, generation, desired_state, \
              observed_state, reservation_id, requirements) \
             VALUES ($1, 'pipeline', $2, $3, $4, 'planned', 'not_started', $5, $6)",
        )
        .bind(assignment.id)
        .bind(assignment.pipeline_id)
        .bind(worker_id)
        .bind(generation)
        .bind(reservation_id)
        .bind(Json(&assignment.requirements))
        .execute(&mut **transaction)
        .await
        .map_err(|error| error_with("database_error", error))?;
        Ok(assignment)
    }

    pub async fn list_planned(&self) -> PortResult<Vec<PlannedAssignment>> {
        let rows = sqlx::query_as::<_, AssignmentRow>(
            "SELECT id, resource_id AS pipeline_id, worker_id, generation, \
             reservation_id, requirements FROM pipeline_assignments \
             WHERE resource_type = 'pipeline' AND desired_state = 'planned' ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|error| error_with("database_error", error))?;
        Ok(rows.into_iter().map(PlannedAssignment::from).collect())
    }

    pub async fn cancel_unstarted(&self, assignment_id: Uuid, generation: i64) -> PortResult<()> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| error_with("database_error", error))?;
        let worker_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT worker_id FROM pipeline_assignments \
             WHERE id = $1 AND resource_type = 'pipeline' AND reservation_id IS NOT NULL",
        )
        .bind(assignment_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?
        .flatten();
        let worker_id = worker_id.ok_or_else(|| PortError {
            code: "assignment_not_found",
            message: "planned assignment does not exist".to_owned(),
        })?;
        sqlx::query("SELECT id FROM workers WHERE id = $1 FOR UPDATE")
            .bind(worker_id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(|error| error_with("database_error", error))?;
        let reservation_id: Option<Uuid> = sqlx::query_scalar(
            "UPDATE pipeline_assignments SET desired_state = 'cancelled', \
             observed_state = 'cancelled', updated_at = clock_timestamp() \
             WHERE id = $1 AND generation = $2 AND desired_state = 'planned' \
             AND observed_state = 'not_started' RETURNING reservation_id",
        )
        .bind(assignment_id)
        .bind(generation)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?
        .flatten();
        let reservation_id = reservation_id.ok_or_else(|| PortError {
            code: "assignment_conflict",
            message: "assignment generation or state does not permit cancellation".to_owned(),
        })?;
        let result = sqlx::query(
            "UPDATE worker_capacity_reservations SET state = 'released', \
             released_at = clock_timestamp() WHERE id = $1 AND state = 'reserved'",
        )
        .bind(reservation_id)
        .execute(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?;
        if result.rows_affected() != 1 {
            return Err(PortError {
                code: "invalid_reservation_record",
                message: "assignment reservation is not active".to_owned(),
            });
        }
        transaction
            .commit()
            .await
            .map_err(|error| error_with("database_error", error))?;
        Ok(())
    }
}

fn validate_assignment(
    reservation_id: Uuid,
    requirements: &PipelineRequirements,
    max_heartbeat_age: Duration,
) -> PortResult<()> {
    if reservation_id.is_nil()
        || requirements.capacity_units == 0
        || max_heartbeat_age < Duration::zero()
    {
        return Err(PortError {
            code: "invalid_assignment",
            message:
                "reservation ID, positive capacity, and nonnegative heartbeat age are required"
                    .to_owned(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, env};

    use arc_application::WorkerRegistry;
    use arc_domain::{
        Architecture, Platform, Worker, WorkerCapabilities, WorkerCapacity, WorkerState,
    };

    use crate::{
        capacity_reservations::{PgCapacityReservations, ReservationDecision},
        postgres::{connect_postgres, migrate},
        worker_registry::PgWorkerRegistry,
    };

    use super::*;

    #[tokio::test]
    #[ignore = "requires an isolated PostgreSQL database; set DATABASE_URL and run ignored tests"]
    async fn planned_assignments_recheck_reservations_and_fence_generations() {
        let url = env::var("DATABASE_URL").expect("DATABASE_URL is set");
        let pool = connect_postgres(&url).await.expect("database connects");
        migrate(&pool).await.expect("migrate");
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
        let requirements = PipelineRequirements {
            source_adapter: Some("rtp_l16".to_owned()),
            capacity_units: 3,
            ..PipelineRequirements::default()
        };
        let pipeline = Uuid::new_v4();
        let reservations = PgCapacityReservations::new(pool.clone());
        let assignments = PgAssignments::new(pool.clone());
        let reservation = match reservations
            .reserve(pipeline, &requirements, Duration::seconds(60))
            .await
            .expect("reserve")
        {
            ReservationDecision::Reserved(value) => value,
            other => panic!("expected reservation, got {other:?}"),
        };
        let mismatched = PipelineRequirements {
            capacity_units: 2,
            ..requirements.clone()
        };
        assert_eq!(
            assignments
                .plan(reservation.id, &mismatched, Duration::seconds(60))
                .await
                .unwrap_err()
                .code,
            "invalid_assignment"
        );
        worker.capacity.assigned_capacity_units = 2;
        registry
            .heartbeat(worker.clone())
            .await
            .expect("capacity report");
        assert_eq!(
            assignments
                .plan(reservation.id, &requirements, Duration::seconds(60))
                .await
                .unwrap_err()
                .code,
            "placement_unavailable"
        );
        worker.capacity.assigned_capacity_units = 0;
        worker.capabilities.source_adapters.clear();
        registry
            .heartbeat(worker.clone())
            .await
            .expect("capability report");
        assert_eq!(
            assignments
                .plan(reservation.id, &requirements, Duration::seconds(60))
                .await
                .unwrap_err()
                .code,
            "placement_unavailable"
        );
        worker
            .capabilities
            .source_adapters
            .insert("rtp_l16".to_owned());
        registry
            .heartbeat(worker.clone())
            .await
            .expect("restore capability");
        sqlx::query("UPDATE workers SET last_heartbeat = clock_timestamp() - interval '1 hour' WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .expect("expire heartbeat");
        assert_eq!(
            assignments
                .plan(reservation.id, &requirements, Duration::seconds(60))
                .await
                .unwrap_err()
                .code,
            "placement_unavailable"
        );
        registry.heartbeat(worker).await.expect("fresh heartbeat");
        let first = assignments
            .plan(reservation.id, &requirements, Duration::seconds(60))
            .await
            .expect("plan");
        assert_eq!(first.generation, 1);
        assert_eq!(
            reservations.release(reservation.id).await.unwrap_err().code,
            "reservation_in_use"
        );
        assert_eq!(
            assignments
                .cancel_unstarted(first.id, first.generation + 1)
                .await
                .unwrap_err()
                .code,
            "assignment_conflict"
        );
        let reconnected = PgAssignments::new(connect_postgres(&url).await.expect("reconnect"));
        assert_eq!(
            reconnected.list_planned().await.expect("list"),
            vec![first.clone()]
        );
        reconnected
            .cancel_unstarted(first.id, first.generation)
            .await
            .expect("cancel");
        assert_eq!(
            reservations.release(reservation.id).await.unwrap_err().code,
            "reservation_released"
        );
        let next = match reservations
            .reserve(pipeline, &requirements, Duration::seconds(60))
            .await
            .expect("reserve again")
        {
            ReservationDecision::Reserved(value) => value,
            other => panic!("expected reservation, got {other:?}"),
        };
        let second = assignments
            .plan(next.id, &requirements, Duration::seconds(60))
            .await
            .expect("plan again");
        assert_eq!(second.generation, 2);
        assert_eq!(
            assignments
                .cancel_unstarted(first.id, first.generation)
                .await
                .unwrap_err()
                .code,
            "assignment_conflict"
        );
        sqlx::query("UPDATE pipeline_assignments SET observed_state = 'running' WHERE id = $1")
            .bind(second.id)
            .execute(&pool)
            .await
            .expect("simulate started assignment");
        assert_eq!(
            assignments
                .cancel_unstarted(second.id, second.generation)
                .await
                .unwrap_err()
                .code,
            "assignment_conflict"
        );
        sqlx::query("UPDATE pipeline_assignments SET observed_state = 'not_started' WHERE id = $1")
            .bind(second.id)
            .execute(&pool)
            .await
            .expect("restore test assignment");
        assignments
            .cancel_unstarted(second.id, second.generation)
            .await
            .expect("cancel second");

        let racing = match reservations
            .reserve(Uuid::new_v4(), &requirements, Duration::seconds(60))
            .await
            .expect("reserve for race")
        {
            ReservationDecision::Reserved(value) => value,
            other => panic!("expected reservation, got {other:?}"),
        };
        let (plan, release) = tokio::join!(
            assignments.plan(racing.id, &requirements, Duration::seconds(60)),
            reservations.release(racing.id),
        );
        match (plan, release) {
            (Ok(assignment), Err(error)) if error.code == "reservation_in_use" => {
                assignments
                    .cancel_unstarted(assignment.id, assignment.generation)
                    .await
                    .expect("cancel race");
            }
            (Err(error), Ok(())) if error.code == "reservation_released" => {}
            other => panic!("plan and release must not both succeed: {other:?}"),
        }
        sqlx::query("DELETE FROM pipeline_assignments WHERE worker_id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .expect("remove test assignments");
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
    }
}
