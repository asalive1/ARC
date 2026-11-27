use arc_application::{PortError, PortResult};
use arc_domain::{select_worker, PipelineRequirements, PlacementDecision};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{types::Json, FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{
    capacity_reservations::reservation_capacity_totals,
    worker_registry::{error_with, WorkerRow},
};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedAssignment {
    pub id: Uuid,
    pub pipeline_id: Uuid,
    pub worker_id: Uuid,
    pub generation: i64,
    pub reservation_id: Uuid,
    pub requirements: PipelineRequirements,
    pub route: Option<PlannedRouteConfig>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignmentSnapshot {
    pub assignment: PlannedAssignment,
    pub desired_state: String,
    pub observed_state: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedRouteConfig {
    pub source_id: Uuid,
    pub source_revision: i64,
    pub destination_id: Uuid,
    pub destination_revision: i64,
    pub media_path: String,
    pub configuration: Value,
}

#[derive(FromRow)]
struct AssignmentRow {
    id: Uuid,
    pipeline_id: Uuid,
    worker_id: Uuid,
    generation: i64,
    reservation_id: Uuid,
    requirements: Json<PipelineRequirements>,
    source_id: Option<Uuid>,
    source_revision: Option<i64>,
    destination_id: Option<Uuid>,
    destination_revision: Option<i64>,
    media_path: Option<String>,
    route_configuration: Option<Json<Value>>,
}

#[derive(FromRow)]
struct AssignmentSnapshotRow {
    id: Uuid,
    pipeline_id: Uuid,
    worker_id: Uuid,
    generation: i64,
    reservation_id: Uuid,
    requirements: Json<PipelineRequirements>,
    source_id: Option<Uuid>,
    source_revision: Option<i64>,
    destination_id: Option<Uuid>,
    destination_revision: Option<i64>,
    media_path: Option<String>,
    route_configuration: Option<Json<Value>>,
    desired_state: String,
    observed_state: String,
}

impl TryFrom<AssignmentSnapshotRow> for AssignmentSnapshot {
    type Error = PortError;

    fn try_from(row: AssignmentSnapshotRow) -> Result<Self, Self::Error> {
        let assignment = PlannedAssignment::try_from(AssignmentRow {
            id: row.id,
            pipeline_id: row.pipeline_id,
            worker_id: row.worker_id,
            generation: row.generation,
            reservation_id: row.reservation_id,
            requirements: row.requirements,
            source_id: row.source_id,
            source_revision: row.source_revision,
            destination_id: row.destination_id,
            destination_revision: row.destination_revision,
            media_path: row.media_path,
            route_configuration: row.route_configuration,
        })?;
        Ok(Self {
            assignment,
            desired_state: row.desired_state,
            observed_state: row.observed_state,
        })
    }
}

impl TryFrom<AssignmentRow> for PlannedAssignment {
    type Error = PortError;

    fn try_from(row: AssignmentRow) -> Result<Self, Self::Error> {
        let route = match (
            row.source_id,
            row.source_revision,
            row.destination_id,
            row.destination_revision,
            row.media_path,
            row.route_configuration,
        ) {
            (None, None, None, None, None, None) => None,
            (
                Some(source_id),
                Some(source_revision),
                Some(destination_id),
                Some(destination_revision),
                Some(media_path),
                Some(configuration),
            ) => Some(PlannedRouteConfig {
                source_id,
                source_revision,
                destination_id,
                destination_revision,
                media_path,
                configuration: configuration.0,
            }),
            _ => {
                return Err(PortError {
                    code: "invalid_assignment_record",
                    message: "stored route revision snapshot is incomplete".to_owned(),
                })
            }
        };
        Ok(Self {
            id: row.id,
            pipeline_id: row.pipeline_id,
            worker_id: row.worker_id,
            generation: row.generation,
            reservation_id: row.reservation_id,
            requirements: row.requirements.0,
            route,
        })
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
            None,
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
        route: Option<&PlannedRouteConfig>,
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
        let is_preview: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM preview_capacity_reservations WHERE reservation_id = $1)",
        )
        .bind(reservation_id)
        .fetch_one(&mut **transaction)
        .await
        .map_err(|error| error_with("database_error", error))?;
        if is_preview {
            return Err(PortError {
                code: "reservation_in_use",
                message: "capacity reservation belongs to a preview".to_owned(),
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

        let pending_route =
            route.map(|route| (reservation_id, route.source_id, route.source_revision));
        let totals =
            reservation_capacity_totals(&mut **transaction, Some(worker_id), pending_route).await?;
        let reserved = totals.get(&worker_id).copied().unwrap_or_default();
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
            route: route.cloned(),
        };
        sqlx::query(
            "INSERT INTO pipeline_assignments \
             (id, resource_type, resource_id, worker_id, generation, desired_state, \
              observed_state, reservation_id, requirements, source_id, source_revision, \
              destination_id, destination_revision, media_path, route_configuration) \
             VALUES ($1, 'pipeline', $2, $3, $4, 'planned', 'not_started', $5, $6, \
                     $7, $8, $9, $10, $11, $12)",
        )
        .bind(assignment.id)
        .bind(assignment.pipeline_id)
        .bind(worker_id)
        .bind(generation)
        .bind(reservation_id)
        .bind(Json(&assignment.requirements))
        .bind(route.map(|route| route.source_id))
        .bind(route.map(|route| route.source_revision))
        .bind(route.map(|route| route.destination_id))
        .bind(route.map(|route| route.destination_revision))
        .bind(route.map(|route| route.media_path.as_str()))
        .bind(route.map(|route| Json(&route.configuration)))
        .execute(&mut **transaction)
        .await
        .map_err(|error| error_with("database_error", error))?;
        Ok(assignment)
    }

    pub async fn list_planned(&self) -> PortResult<Vec<PlannedAssignment>> {
        let rows = sqlx::query_as::<_, AssignmentRow>(
            "SELECT id, resource_id AS pipeline_id, worker_id, generation, \
             reservation_id, requirements, source_id, source_revision, destination_id, \
             destination_revision, media_path, route_configuration FROM pipeline_assignments \
             WHERE resource_type = 'pipeline' AND desired_state = 'planned' ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|error| error_with("database_error", error))?;
        rows.into_iter().map(PlannedAssignment::try_from).collect()
    }

    pub async fn list_runtime_for_worker(
        &self,
        worker_id: Uuid,
    ) -> PortResult<Vec<AssignmentSnapshot>> {
        if worker_id.is_nil() {
            return Err(PortError {
                code: "invalid_worker_id",
                message: "worker ID must not be nil".to_owned(),
            });
        }
        let rows = sqlx::query_as::<_, AssignmentSnapshotRow>(
            "SELECT id, resource_id AS pipeline_id, worker_id, generation, \
             reservation_id, requirements, source_id, source_revision, destination_id, \
             destination_revision, media_path, route_configuration, desired_state, \
             observed_state FROM pipeline_assignments \
             WHERE resource_type = 'pipeline' AND worker_id = $1 \
             AND ((desired_state = 'running' AND observed_state IN \
                   ('starting', 'running', 'failed', 'degraded')) \
               OR (desired_state = 'stopped' AND observed_state = 'stopping')) \
             ORDER BY id",
        )
        .bind(worker_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|error| error_with("database_error", error))?;
        rows.into_iter().map(AssignmentSnapshot::try_from).collect()
    }

    pub async fn get_by_pipeline(
        &self,
        pipeline_id: Uuid,
    ) -> PortResult<Option<AssignmentSnapshot>> {
        if pipeline_id.is_nil() {
            return Err(PortError {
                code: "invalid_pipeline_id",
                message: "pipeline ID must not be nil".to_owned(),
            });
        }
        let row = sqlx::query_as::<_, AssignmentSnapshotRow>(
            "SELECT id, resource_id AS pipeline_id, worker_id, generation, \
         reservation_id, requirements, source_id, source_revision, destination_id, \
         destination_revision, media_path, route_configuration, desired_state, \
         observed_state FROM pipeline_assignments \
         WHERE resource_type = 'pipeline' AND resource_id = $1 \
         ORDER BY generation DESC LIMIT 1",
        )
        .bind(pipeline_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| error_with("database_error", error))?;
        row.map(AssignmentSnapshot::try_from).transpose()
    }

    pub async fn get_by_id(&self, assignment_id: Uuid) -> PortResult<Option<AssignmentSnapshot>> {
        if assignment_id.is_nil() {
            return Err(PortError {
                code: "invalid_assignment_id",
                message: "assignment ID must not be nil".to_owned(),
            });
        }
        let row = sqlx::query_as::<_, AssignmentSnapshotRow>(
            "SELECT id, resource_id AS pipeline_id, worker_id, generation, \
             reservation_id, requirements, source_id, source_revision, destination_id, \
             destination_revision, media_path, route_configuration, desired_state, \
             observed_state FROM pipeline_assignments \
             WHERE id = $1 AND resource_type = 'pipeline'",
        )
        .bind(assignment_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| error_with("database_error", error))?;
        row.map(AssignmentSnapshot::try_from).transpose()
    }

    pub async fn begin_start(
        &self,
        assignment_id: Uuid,
        generation: i64,
    ) -> PortResult<AssignmentSnapshot> {
        if assignment_id.is_nil() || generation <= 0 {
            return Err(PortError {
                code: "invalid_assignment",
                message: "assignment ID and positive generation are required".to_owned(),
            });
        }
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| error_with("database_error", error))?;
        let row = sqlx::query_as::<_, AssignmentSnapshotRow>(
            "SELECT id, resource_id AS pipeline_id, worker_id, generation, \
             reservation_id, requirements, source_id, source_revision, destination_id, \
             destination_revision, media_path, route_configuration, desired_state, \
             observed_state FROM pipeline_assignments \
             WHERE id = $1 AND resource_type = 'pipeline'",
        )
        .bind(assignment_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?
        .ok_or_else(|| PortError {
            code: "assignment_not_found",
            message: "pipeline assignment does not exist".to_owned(),
        })?;
        let initial_snapshot = AssignmentSnapshot::try_from(row)?;
        lock_route_resources(&mut transaction, initial_snapshot.assignment.route.as_ref()).await?;
        let row = sqlx::query_as::<_, AssignmentSnapshotRow>(
            "SELECT id, resource_id AS pipeline_id, worker_id, generation, \
             reservation_id, requirements, source_id, source_revision, destination_id, \
             destination_revision, media_path, route_configuration, desired_state, \
             observed_state FROM pipeline_assignments \
             WHERE id = $1 AND resource_type = 'pipeline' FOR UPDATE",
        )
        .bind(assignment_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?
        .ok_or_else(|| PortError {
            code: "assignment_not_found",
            message: "pipeline assignment does not exist".to_owned(),
        })?;
        let mut snapshot = AssignmentSnapshot::try_from(row)?;
        if snapshot.assignment.generation != generation {
            return Err(PortError {
                code: "assignment_conflict",
                message: "assignment generation does not match the requested generation".to_owned(),
            });
        }
        if snapshot.assignment.route.is_none() {
            return Err(PortError {
                code: "assignment_conflict",
                message: "assignment has no pinned route configuration to start".to_owned(),
            });
        }
        match (
            snapshot.desired_state.as_str(),
            snapshot.observed_state.as_str(),
        ) {
            ("planned", "not_started") => {
                sqlx::query(
                    "UPDATE pipeline_assignments SET desired_state = 'running', \
                     observed_state = 'starting', updated_at = clock_timestamp() \
                     WHERE id = $1 AND generation = $2",
                )
                .bind(assignment_id)
                .bind(generation)
                .execute(&mut *transaction)
                .await
                .map_err(|error| error_with("database_error", error))?;
                snapshot.desired_state = "running".to_owned();
                snapshot.observed_state = "starting".to_owned();
            }
            ("running", "starting" | "running") => {}
            _ => {
                return Err(PortError {
                    code: "assignment_conflict",
                    message: "assignment state does not permit a start command".to_owned(),
                })
            }
        }
        set_route_resources_desired_state(
            &mut transaction,
            assignment_id,
            snapshot.assignment.route.as_ref(),
            true,
        )
        .await?;
        transaction
            .commit()
            .await
            .map_err(|error| error_with("database_error", error))?;
        Ok(snapshot)
    }

    pub async fn mark_started(
        &self,
        assignment_id: Uuid,
        generation: i64,
    ) -> PortResult<AssignmentSnapshot> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| error_with("database_error", error))?;
        let row = sqlx::query_as::<_, AssignmentSnapshotRow>(
            "UPDATE pipeline_assignments SET observed_state = 'running', \
             updated_at = clock_timestamp() \
             WHERE id = $1 AND resource_type = 'pipeline' AND generation = $2 \
             AND desired_state = 'running' AND observed_state IN ('starting', 'running') \
             RETURNING id, resource_id AS pipeline_id, worker_id, generation, \
             reservation_id, requirements, source_id, source_revision, destination_id, \
             destination_revision, media_path, route_configuration, desired_state, observed_state",
        )
        .bind(assignment_id)
        .bind(generation)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?;
        let row = row.ok_or_else(|| PortError {
            code: "assignment_conflict",
            message: "assignment state changed before Start was acknowledged".to_owned(),
        })?;
        let snapshot = AssignmentSnapshot::try_from(row)?;
        record_route_observation(
            &mut transaction,
            snapshot.assignment.route.as_ref(),
            "running",
        )
        .await?;
        transaction
            .commit()
            .await
            .map_err(|error| error_with("database_error", error))?;
        Ok(snapshot)
    }

    pub async fn reset_start(&self, assignment_id: Uuid, generation: i64) -> PortResult<()> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| error_with("database_error", error))?;
        let initial_row = sqlx::query_as::<_, AssignmentSnapshotRow>(
            "SELECT id, resource_id AS pipeline_id, worker_id, generation, \
             reservation_id, requirements, source_id, source_revision, destination_id, \
             destination_revision, media_path, route_configuration, desired_state, \
             observed_state FROM pipeline_assignments \
             WHERE id = $1 AND resource_type = 'pipeline'",
        )
        .bind(assignment_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?
        .ok_or_else(|| PortError {
            code: "assignment_not_found",
            message: "pipeline assignment does not exist".to_owned(),
        })?;
        let initial_snapshot = AssignmentSnapshot::try_from(initial_row)?;
        if initial_snapshot.assignment.generation != generation {
            return Err(PortError {
                code: "assignment_conflict",
                message: "assignment generation does not match the requested generation".to_owned(),
            });
        }
        lock_route_resources(&mut transaction, initial_snapshot.assignment.route.as_ref()).await?;
        let row = sqlx::query_as::<_, AssignmentSnapshotRow>(
            "SELECT id, resource_id AS pipeline_id, worker_id, generation, \
             reservation_id, requirements, source_id, source_revision, destination_id, \
             destination_revision, media_path, route_configuration, desired_state, \
             observed_state FROM pipeline_assignments \
             WHERE id = $1 AND resource_type = 'pipeline' FOR UPDATE",
        )
        .bind(assignment_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?
        .ok_or_else(|| PortError {
            code: "assignment_not_found",
            message: "pipeline assignment does not exist".to_owned(),
        })?;
        let snapshot = AssignmentSnapshot::try_from(row)?;
        if snapshot.assignment.generation != generation {
            return Err(PortError {
                code: "assignment_conflict",
                message: "assignment generation does not match the requested generation".to_owned(),
            });
        }
        match (
            snapshot.desired_state.as_str(),
            snapshot.observed_state.as_str(),
        ) {
            ("running", "starting") => {
                sqlx::query(
                    "UPDATE pipeline_assignments SET desired_state = 'planned', \
                     observed_state = 'not_started', updated_at = clock_timestamp() \
                     WHERE id = $1 AND generation = $2",
                )
                .bind(assignment_id)
                .bind(generation)
                .execute(&mut *transaction)
                .await
                .map_err(|error| error_with("database_error", error))?;
                set_route_resources_desired_state(
                    &mut transaction,
                    assignment_id,
                    snapshot.assignment.route.as_ref(),
                    false,
                )
                .await?;
            }
            ("planned", "not_started") => {
                set_route_resources_desired_state(
                    &mut transaction,
                    assignment_id,
                    snapshot.assignment.route.as_ref(),
                    false,
                )
                .await?;
            }
            ("running", "running") => {}
            _ => {
                return Err(PortError {
                    code: "assignment_conflict",
                    message: "assignment state changed before the start could be reset".to_owned(),
                })
            }
        }
        transaction
            .commit()
            .await
            .map_err(|error| error_with("database_error", error))
    }

    pub async fn begin_stop(
        &self,
        assignment_id: Uuid,
        generation: i64,
    ) -> PortResult<AssignmentSnapshot> {
        if assignment_id.is_nil() || generation <= 0 {
            return Err(PortError {
                code: "invalid_assignment",
                message: "assignment ID and positive generation are required".to_owned(),
            });
        }
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| error_with("database_error", error))?;
        let row = sqlx::query_as::<_, AssignmentSnapshotRow>(
            "SELECT id, resource_id AS pipeline_id, worker_id, generation, \
             reservation_id, requirements, source_id, source_revision, destination_id, \
             destination_revision, media_path, route_configuration, desired_state, \
             observed_state FROM pipeline_assignments \
             WHERE id = $1 AND resource_type = 'pipeline' FOR UPDATE",
        )
        .bind(assignment_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?
        .ok_or_else(|| PortError {
            code: "assignment_not_found",
            message: "pipeline assignment does not exist".to_owned(),
        })?;
        let mut snapshot = AssignmentSnapshot::try_from(row)?;
        if snapshot.assignment.generation != generation {
            return Err(PortError {
                code: "assignment_conflict",
                message: "assignment generation does not match the requested generation".to_owned(),
            });
        }
        match (
            snapshot.desired_state.as_str(),
            snapshot.observed_state.as_str(),
        ) {
            ("running", "starting" | "running" | "failed" | "degraded" | "unknown") => {
                sqlx::query(
                    "UPDATE pipeline_assignments SET desired_state = 'stopped', \
                     observed_state = 'stopping', updated_at = clock_timestamp() \
                     WHERE id = $1 AND generation = $2",
                )
                .bind(assignment_id)
                .bind(generation)
                .execute(&mut *transaction)
                .await
                .map_err(|error| error_with("database_error", error))?;
                snapshot.desired_state = "stopped".to_owned();
                snapshot.observed_state = "stopping".to_owned();
            }
            ("stopped", "stopping" | "stopped") => {}
            _ => {
                return Err(PortError {
                    code: "assignment_conflict",
                    message: "assignment state does not permit a stop command".to_owned(),
                })
            }
        }
        transaction
            .commit()
            .await
            .map_err(|error| error_with("database_error", error))?;
        Ok(snapshot)
    }

    pub async fn reset_failed_stop(
        &self,
        assignment_id: Uuid,
        generation: i64,
    ) -> PortResult<AssignmentSnapshot> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| error_with("database_error", error))?;
        let row = sqlx::query_as::<_, AssignmentSnapshotRow>(
            "SELECT id, resource_id AS pipeline_id, worker_id, generation, \
             reservation_id, requirements, source_id, source_revision, destination_id, \
             destination_revision, media_path, route_configuration, desired_state, \
             observed_state FROM pipeline_assignments \
             WHERE id = $1 AND resource_type = 'pipeline'",
        )
        .bind(assignment_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?
        .ok_or_else(|| PortError {
            code: "assignment_not_found",
            message: "pipeline assignment does not exist".to_owned(),
        })?;
        let initial_snapshot = AssignmentSnapshot::try_from(row)?;
        if initial_snapshot.assignment.generation != generation {
            return Err(PortError {
                code: "assignment_conflict",
                message: "assignment generation does not match the requested generation".to_owned(),
            });
        }
        lock_route_resources(&mut transaction, initial_snapshot.assignment.route.as_ref()).await?;
        let row = sqlx::query_as::<_, AssignmentSnapshotRow>(
            "UPDATE pipeline_assignments SET desired_state = 'running', \
             observed_state = 'degraded', updated_at = clock_timestamp() \
             WHERE id = $1 AND generation = $2 AND desired_state = 'stopped' \
               AND observed_state = 'stopping' \
             RETURNING id, resource_id AS pipeline_id, worker_id, generation, \
             reservation_id, requirements, source_id, source_revision, destination_id, \
             destination_revision, media_path, route_configuration, desired_state, \
             observed_state",
        )
        .bind(assignment_id)
        .bind(generation)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?
        .ok_or_else(|| PortError {
            code: "assignment_conflict",
            message: "assignment state changed before failed Stop could be recovered".to_owned(),
        })?;
        let snapshot = AssignmentSnapshot::try_from(row)?;
        record_route_observation(
            &mut transaction,
            snapshot.assignment.route.as_ref(),
            "degraded",
        )
        .await?;
        set_route_resources_desired_state(
            &mut transaction,
            assignment_id,
            snapshot.assignment.route.as_ref(),
            true,
        )
        .await?;
        transaction
            .commit()
            .await
            .map_err(|error| error_with("database_error", error))?;
        Ok(snapshot)
    }

    pub async fn mark_stopped(
        &self,
        assignment_id: Uuid,
        generation: i64,
    ) -> PortResult<AssignmentSnapshot> {
        if assignment_id.is_nil() || generation <= 0 {
            return Err(PortError {
                code: "invalid_assignment",
                message: "assignment ID and positive generation are required".to_owned(),
            });
        }
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| error_with("database_error", error))?;
        let initial_row = sqlx::query_as::<_, AssignmentSnapshotRow>(
            "SELECT id, resource_id AS pipeline_id, worker_id, generation, \
             reservation_id, requirements, source_id, source_revision, destination_id, \
             destination_revision, media_path, route_configuration, desired_state, \
             observed_state FROM pipeline_assignments \
             WHERE id = $1 AND resource_type = 'pipeline'",
        )
        .bind(assignment_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?
        .ok_or_else(|| PortError {
            code: "assignment_not_found",
            message: "pipeline assignment does not exist".to_owned(),
        })?;
        let initial_snapshot = AssignmentSnapshot::try_from(initial_row)?;
        lock_route_resources(&mut transaction, initial_snapshot.assignment.route.as_ref()).await?;
        sqlx::query("SELECT id FROM workers WHERE id = $1 FOR UPDATE")
            .bind(initial_snapshot.assignment.worker_id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(|error| error_with("database_error", error))?;

        let row = sqlx::query_as::<_, AssignmentSnapshotRow>(
            "SELECT id, resource_id AS pipeline_id, worker_id, generation, \
             reservation_id, requirements, source_id, source_revision, destination_id, \
             destination_revision, media_path, route_configuration, desired_state, \
             observed_state FROM pipeline_assignments \
             WHERE id = $1 AND resource_type = 'pipeline' FOR UPDATE",
        )
        .bind(assignment_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?
        .ok_or_else(|| PortError {
            code: "assignment_not_found",
            message: "pipeline assignment does not exist".to_owned(),
        })?;
        let mut snapshot = AssignmentSnapshot::try_from(row)?;
        if snapshot.assignment.generation != generation {
            return Err(PortError {
                code: "assignment_conflict",
                message: "assignment generation does not match the requested generation".to_owned(),
            });
        }
        if snapshot.desired_state != "stopped"
            || !matches!(snapshot.observed_state.as_str(), "stopping" | "stopped")
        {
            return Err(PortError {
                code: "assignment_conflict",
                message: "assignment state changed before Stop was acknowledged".to_owned(),
            });
        }
        let newly_stopped = snapshot.observed_state == "stopping";
        if newly_stopped {
            let updated = sqlx::query(
                "UPDATE pipeline_assignments SET observed_state = 'stopped', \
                 updated_at = clock_timestamp() \
                 WHERE id = $1 AND generation = $2 AND desired_state = 'stopped' \
                 AND observed_state = 'stopping'",
            )
            .bind(assignment_id)
            .bind(generation)
            .execute(&mut *transaction)
            .await
            .map_err(|error| error_with("database_error", error))?;
            if updated.rows_affected() != 1 {
                return Err(PortError {
                    code: "assignment_conflict",
                    message: "assignment state changed before Stop was acknowledged".to_owned(),
                });
            }
            record_route_observation(
                &mut transaction,
                snapshot.assignment.route.as_ref(),
                "stopped",
            )
            .await?;
        }
        release_reservation_in_transaction(&mut transaction, snapshot.assignment.reservation_id)
            .await?;
        set_route_resources_desired_state(
            &mut transaction,
            assignment_id,
            snapshot.assignment.route.as_ref(),
            false,
        )
        .await?;
        transaction
            .commit()
            .await
            .map_err(|error| error_with("database_error", error))?;
        snapshot.desired_state = "stopped".to_owned();
        snapshot.observed_state = "stopped".to_owned();
        Ok(snapshot)
    }

    pub async fn cancel_unstarted(&self, assignment_id: Uuid, generation: i64) -> PortResult<()> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| error_with("database_error", error))?;
        let initial_row = sqlx::query_as::<_, AssignmentSnapshotRow>(
            "SELECT id, resource_id AS pipeline_id, worker_id, generation, \
             reservation_id, requirements, source_id, source_revision, destination_id, \
             destination_revision, media_path, route_configuration, desired_state, \
             observed_state FROM pipeline_assignments \
             WHERE id = $1 AND resource_type = 'pipeline' AND reservation_id IS NOT NULL",
        )
        .bind(assignment_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?
        .ok_or_else(|| PortError {
            code: "assignment_not_found",
            message: "planned assignment does not exist".to_owned(),
        })?;
        let initial_snapshot = AssignmentSnapshot::try_from(initial_row)?;
        if initial_snapshot.assignment.generation != generation {
            return Err(PortError {
                code: "assignment_conflict",
                message: "assignment generation does not match the requested generation".to_owned(),
            });
        }
        if initial_snapshot.desired_state == "cancelled"
            && initial_snapshot.observed_state == "cancelled"
        {
            transaction
                .commit()
                .await
                .map_err(|error| error_with("database_error", error))?;
            return Ok(());
        }
        lock_route_resources(&mut transaction, initial_snapshot.assignment.route.as_ref()).await?;
        sqlx::query("SELECT id FROM workers WHERE id = $1 FOR UPDATE")
            .bind(initial_snapshot.assignment.worker_id)
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
        release_reservation_in_transaction(&mut transaction, reservation_id).await?;
        set_route_resources_desired_state(
            &mut transaction,
            assignment_id,
            initial_snapshot.assignment.route.as_ref(),
            false,
        )
        .await?;
        transaction
            .commit()
            .await
            .map_err(|error| error_with("database_error", error))?;
        Ok(())
    }
}

async fn record_route_observation(
    transaction: &mut Transaction<'_, Postgres>,
    route: Option<&PlannedRouteConfig>,
    observed_state: &str,
) -> PortResult<()> {
    let Some(route) = route else {
        return Ok(());
    };

    sqlx::query(
        "INSERT INTO route_events (id, route_instance_id, observed_state) \
         SELECT $1, id, $4 FROM route_instances \
         WHERE source_id = $2 AND destination_id = $3 \
         ORDER BY route_revision DESC LIMIT 1",
    )
    .bind(Uuid::new_v4())
    .bind(route.source_id)
    .bind(route.destination_id)
    .bind(observed_state)
    .execute(&mut **transaction)
    .await
    .map_err(|error| error_with("database_error", error))?;
    Ok(())
}

async fn lock_route_resources(
    transaction: &mut Transaction<'_, Postgres>,
    route: Option<&PlannedRouteConfig>,
) -> PortResult<()> {
    let Some(route) = route else {
        return Ok(());
    };

    let source_id: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM sources WHERE id = $1 FOR UPDATE")
            .bind(route.source_id)
            .fetch_optional(&mut **transaction)
            .await
            .map_err(|error| error_with("database_error", error))?;
    if source_id.is_none() {
        return Err(error_with(
            "assignment_resource_not_found",
            "assignment source no longer exists",
        ));
    }

    let destination_id: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM destinations WHERE id = $1 FOR UPDATE")
            .bind(route.destination_id)
            .fetch_optional(&mut **transaction)
            .await
            .map_err(|error| error_with("database_error", error))?;
    if destination_id.is_none() {
        return Err(error_with(
            "assignment_resource_not_found",
            "assignment destination no longer exists",
        ));
    }
    Ok(())
}

async fn set_route_resources_desired_state(
    transaction: &mut Transaction<'_, Postgres>,
    assignment_id: Uuid,
    route: Option<&PlannedRouteConfig>,
    enabled: bool,
) -> PortResult<()> {
    let Some(route) = route else {
        return Ok(());
    };

    if enabled {
        sqlx::query(
            "UPDATE sources SET desired_state = 'enabled', updated_at = clock_timestamp() \
             WHERE id = $1 AND desired_state <> 'enabled'",
        )
        .bind(route.source_id)
        .execute(&mut **transaction)
        .await
        .map_err(|error| error_with("database_error", error))?;
        sqlx::query(
            "UPDATE destinations SET desired_state = 'enabled', updated_at = clock_timestamp() \
             WHERE id = $1 AND desired_state <> 'enabled'",
        )
        .bind(route.destination_id)
        .execute(&mut **transaction)
        .await
        .map_err(|error| error_with("database_error", error))?;
        return Ok(());
    }

    let (source_in_use, destination_in_use): (bool, bool) = sqlx::query_as(
        "SELECT \
           EXISTS (SELECT 1 FROM pipeline_assignments \
                   WHERE resource_type = 'pipeline' AND id <> $1 AND source_id = $2 \
                     AND observed_state NOT IN ('not_started', 'stopped', 'cancelled')), \
           EXISTS (SELECT 1 FROM pipeline_assignments \
                   WHERE resource_type = 'pipeline' AND id <> $1 AND destination_id = $3 \
                     AND observed_state NOT IN ('not_started', 'stopped', 'cancelled'))",
    )
    .bind(assignment_id)
    .bind(route.source_id)
    .bind(route.destination_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| error_with("database_error", error))?;

    if !source_in_use {
        sqlx::query(
            "UPDATE sources SET desired_state = 'disabled', updated_at = clock_timestamp() \
             WHERE id = $1 AND desired_state <> 'disabled'",
        )
        .bind(route.source_id)
        .execute(&mut **transaction)
        .await
        .map_err(|error| error_with("database_error", error))?;
    }
    if !destination_in_use {
        sqlx::query(
            "UPDATE destinations SET desired_state = 'disabled', updated_at = clock_timestamp() \
             WHERE id = $1 AND desired_state <> 'disabled'",
        )
        .bind(route.destination_id)
        .execute(&mut **transaction)
        .await
        .map_err(|error| error_with("database_error", error))?;
    }
    Ok(())
}

async fn release_reservation_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    reservation_id: Uuid,
) -> PortResult<()> {
    let updated = sqlx::query(
        "UPDATE worker_capacity_reservations SET state = 'released', \
         released_at = clock_timestamp() WHERE id = $1 AND state = 'reserved'",
    )
    .bind(reservation_id)
    .execute(&mut **transaction)
    .await
    .map_err(|error| error_with("database_error", error))?;
    if updated.rows_affected() == 1 {
        return Ok(());
    }
    let state: Option<String> =
        sqlx::query_scalar("SELECT state FROM worker_capacity_reservations WHERE id = $1")
            .bind(reservation_id)
            .fetch_optional(&mut **transaction)
            .await
            .map_err(|error| error_with("database_error", error))?;
    if state.as_deref() == Some("released") {
        return Ok(());
    }
    Err(PortError {
        code: "invalid_reservation_record",
        message: "assignment reservation is missing or not releasable".to_owned(),
    })
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
        sqlx::query(
            "UPDATE pipeline_assignments SET desired_state = 'running', \
             observed_state = 'starting' WHERE id = $1",
        )
        .bind(first.id)
        .execute(&pool)
        .await
        .expect("set restart-recovery starting state");
        let runtime = reconnected
            .list_runtime_for_worker(id)
            .await
            .expect("list runtime assignments");
        assert_eq!(runtime.len(), 1);
        assert_eq!(runtime[0].assignment.id, first.id);
        assert_eq!(runtime[0].desired_state, "running");
        assert_eq!(runtime[0].observed_state, "starting");
        sqlx::query(
            "UPDATE pipeline_assignments SET desired_state = 'planned', \
             observed_state = 'not_started' WHERE id = $1",
        )
        .bind(first.id)
        .execute(&pool)
        .await
        .expect("restore planned state");
        reconnected
            .cancel_unstarted(first.id, first.generation)
            .await
            .expect("cancel");
        reconnected
            .cancel_unstarted(first.id, first.generation)
            .await
            .expect("repeat cancellation is idempotent");
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
        assignments
            .cancel_unstarted(first.id, first.generation)
            .await
            .expect("cancel retry remains idempotent");
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
