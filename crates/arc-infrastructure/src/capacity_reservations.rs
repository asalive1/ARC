use std::collections::HashMap;

use arc_application::{PortError, PortResult, ProcessFenceIdentity};
use arc_domain::{
    select_worker, validate_source_profile, PipelineRequirements, PlacementDecision, Platform,
    SourceMediaProfile, WorkerRejection,
};
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

#[derive(Clone, Debug)]
pub struct PreviewCapacityReservation {
    pub reservation: CapacityReservation,
    pub source_revision: i64,
    pub source: SourceMediaProfile,
    pub worker_session_id: Uuid,
}

#[derive(Clone, Debug)]
pub enum PreviewReservationDecision {
    Reserved(PreviewCapacityReservation),
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
                    COALESCE(assignments.source_id, previews.source_id, \
                        CASE WHEN reservations.id = $3 THEN $4::uuid END) AS source_id, \
                    COALESCE(assignments.source_revision, previews.source_revision, \
                        CASE WHEN reservations.id = $3 THEN $5::bigint END) AS source_revision \
             FROM worker_capacity_reservations AS reservations \
             LEFT JOIN pipeline_assignments AS assignments \
               ON assignments.reservation_id = reservations.id \
             LEFT JOIN preview_capacity_reservations AS previews \
               ON previews.reservation_id = reservations.id \
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

    /// The caller must hold the new process's host fence throughout this
    /// transaction and prevent unfenced or legacy workers from using this
    /// identity. The verifier must reject an unverified initial enrollment and
    /// every previous process that has not definitively terminated.
    pub async fn recover_fenced_previews<F>(
        &self,
        worker_id: Uuid,
        incarnation_id: Uuid,
        identity: &ProcessFenceIdentity,
        verify_previous: F,
    ) -> PortResult<u64>
    where
        F: FnOnce(Option<&ProcessFenceIdentity>) -> PortResult<()>,
    {
        if worker_id.is_nil()
            || incarnation_id.is_nil()
            || identity.host_id.is_nil()
            || identity.boot_id.is_nil()
            || identity.host_pid == 0
        {
            return Err(error_with(
                "invalid_worker_fence",
                "worker process fence identity is invalid",
            ));
        }
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| error_with("database_error", error))?;
        lock_queue(&mut transaction).await?;
        sqlx::query("SELECT id FROM workers WHERE id = $1 FOR UPDATE")
            .bind(worker_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|error| error_with("database_error", error))?
            .ok_or_else(|| error_with("worker_not_found", "preview worker is not registered"))?;
        let previous: Option<(Uuid, sqlx::types::Json<ProcessFenceIdentity>)> = sqlx::query_as(
            "SELECT incarnation_id, process_identity FROM preview_worker_process_fences \
             WHERE worker_id = $1 FOR UPDATE",
        )
        .bind(worker_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?;
        if previous.as_ref().is_some_and(|(id, _)| *id == incarnation_id) {
            return Err(error_with(
                "invalid_worker_fence",
                "worker incarnation must change during process recovery",
            ));
        }
        verify_previous(previous.as_ref().map(|(_, value)| &value.0))?;
        if previous.is_none() {
            let has_previews: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM preview_capacity_reservations AS previews \
                 JOIN worker_capacity_reservations AS reservations ON reservations.id = previews.reservation_id \
                 WHERE reservations.worker_id = $1 AND reservations.state = 'reserved')",
            )
            .bind(worker_id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(|error| error_with("database_error", error))?;
            if has_previews {
                return Err(error_with(
                    "worker_fence_enrollment_unsafe",
                    "unfenced preview reservations require operator recovery",
                ));
            }
        }
        sqlx::query("DELETE FROM worker_control_sessions WHERE worker_id = $1")
            .bind(worker_id)
            .execute(&mut *transaction)
            .await
            .map_err(|error| error_with("database_error", error))?;
        let released = sqlx::query(
            "WITH removed AS ( \
                 DELETE FROM preview_capacity_reservations AS previews \
                 USING worker_capacity_reservations AS reservations \
                 WHERE previews.reservation_id = reservations.id \
                   AND reservations.worker_id = $1 AND reservations.state = 'reserved' \
                 RETURNING previews.reservation_id \
             ) \
             UPDATE worker_capacity_reservations SET state = 'released', \
                    released_at = clock_timestamp() \
             WHERE id IN (SELECT reservation_id FROM removed)",
        )
        .bind(worker_id)
        .execute(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?
        .rows_affected();
        sqlx::query(
            "INSERT INTO preview_worker_process_fences \
             (worker_id, incarnation_id, process_identity) VALUES ($1, $2, $3) \
             ON CONFLICT (worker_id) DO UPDATE SET \
             incarnation_id = EXCLUDED.incarnation_id, process_identity = EXCLUDED.process_identity",
        )
        .bind(worker_id)
        .bind(incarnation_id)
        .bind(sqlx::types::Json(identity))
        .execute(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?;
        transaction
            .commit()
            .await
            .map_err(|error| error_with("database_error", error))?;
        Ok(released)
    }

    pub async fn reserve_preview(
        &self,
        preview_id: Uuid,
        source_id: Uuid,
        max_heartbeat_age: Duration,
    ) -> PortResult<PreviewReservationDecision> {
        self.reserve_preview_inner(preview_id, source_id, max_heartbeat_age, false)
            .await
    }

    pub async fn reserve_fenced_preview(
        &self,
        preview_id: Uuid,
        source_id: Uuid,
        max_heartbeat_age: Duration,
    ) -> PortResult<PreviewReservationDecision> {
        self.reserve_preview_inner(preview_id, source_id, max_heartbeat_age, true)
            .await
    }

    async fn reserve_preview_inner(
        &self,
        preview_id: Uuid,
        source_id: Uuid,
        max_heartbeat_age: Duration,
        require_fenced: bool,
    ) -> PortResult<PreviewReservationDecision> {
        if preview_id.is_nil() || source_id.is_nil() || max_heartbeat_age < Duration::zero() {
            return Err(error_with(
                "invalid_preview_identity",
                "preview placement identity is invalid",
            ));
        }
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| error_with("database_error", error))?;
        lock_queue(&mut transaction).await?;
        let row: Option<(i64, String, String, serde_json::Value)> = sqlx::query_as(
            "SELECT s.current_revision, r.lifecycle, s.source_type, r.configuration \
             FROM sources AS s JOIN source_revisions AS r \
               ON r.source_id = s.id AND r.revision = s.current_revision \
             WHERE s.id = $1 FOR SHARE OF s, r",
        )
        .bind(source_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?;
        let (source_revision, lifecycle, adapter, configuration) = row.ok_or_else(|| {
            error_with(
                "source_not_found",
                "source or its current revision does not exist",
            )
        })?;
        if !matches!(lifecycle.as_str(), "validated" | "active") {
            return Err(error_with(
                "source_not_validated",
                "source must be validated before preview",
            ));
        }
        let source: SourceMediaProfile = serde_json::from_value(configuration)
            .map_err(|error| error_with("invalid_source_record", error))?;
        if source.adapter_id() != adapter {
            return Err(error_with(
                "invalid_source_record",
                "source adapter does not match its saved profile",
            ));
        }
        validate_source_profile(&source)
            .map_err(|error| error_with("invalid_source_record", error))?;
        let old_route: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pipeline_assignments \
             WHERE resource_type = 'pipeline' AND source_id = $1 AND source_revision <> $2 \
             AND ((desired_state = 'planned' AND observed_state <> 'cancelled') \
                  OR desired_state = 'running' \
                  OR (desired_state = 'stopped' AND observed_state = 'stopping')))",
        )
        .bind(source_id)
        .bind(source_revision)
        .fetch_one(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?;
        if old_route {
            return Err(error_with(
                "preview_source_revision_in_use",
                "another revision of this source is assigned to a route",
            ));
        }
        if has_waiters(&mut transaction).await? {
            transaction
                .commit()
                .await
                .map_err(|error| error_with("database_error", error))?;
            return Ok(PreviewReservationDecision::Unavailable {
                rejections: Vec::new(),
            });
        }
        let mut requirements = PipelineRequirements {
            source_adapter: Some(source.adapter_id().to_owned()),
            capacity_units: 2,
            ..PipelineRequirements::default()
        };
        if matches!(source, SourceMediaProfile::AlsaL16 { .. }) {
            requirements.platform = Some(Platform::Linux);
            requirements.audio_backends.insert("alsa".to_owned());
        }
        let decision = reserve_in_transaction(
            &mut transaction,
            preview_id,
            &requirements,
            max_heartbeat_age,
            Some((source_id, source_revision)),
            true,
            require_fenced,
        )
        .await?;
        let reservation = match decision {
            ReservationDecision::Reserved(reservation) => reservation,
            ReservationDecision::Unavailable { rejections } => {
                transaction
                    .commit()
                    .await
                    .map_err(|error| error_with("database_error", error))?;
                return Ok(PreviewReservationDecision::Unavailable { rejections });
            }
        };
        let worker_session_id: Uuid = sqlx::query_scalar(
            "SELECT session_id FROM worker_control_sessions WHERE worker_id = $1 FOR SHARE",
        )
        .bind(reservation.worker_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?
        .ok_or_else(|| {
            error_with(
                "worker_session_unavailable",
                "selected worker has no control session",
            )
        })?;
        sqlx::query(
            "INSERT INTO preview_capacity_reservations \
             (preview_id, reservation_id, source_id, source_revision, worker_session_id) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(preview_id)
        .bind(reservation.id)
        .bind(source_id)
        .bind(source_revision)
        .bind(worker_session_id)
        .execute(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?;
        transaction
            .commit()
            .await
            .map_err(|error| error_with("database_error", error))?;
        Ok(PreviewReservationDecision::Reserved(
            PreviewCapacityReservation {
                reservation,
                source_revision,
                source,
                worker_session_id,
            },
        ))
    }

    /// Call only after the worker confirms cleanup, or if preview was never started.
    pub async fn release_preview(
        &self,
        preview_id: Uuid,
        worker_id: Uuid,
        worker_session_id: Uuid,
    ) -> PortResult<()> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| error_with("database_error", error))?;
        lock_queue(&mut transaction).await?;
        sqlx::query("SELECT id FROM workers WHERE id = $1 FOR UPDATE")
            .bind(worker_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|error| error_with("database_error", error))?
            .ok_or_else(|| error_with("worker_not_found", "preview worker is not registered"))?;
        let reservation_id: Uuid = sqlx::query_scalar(
            "DELETE FROM preview_capacity_reservations AS previews \
             USING worker_capacity_reservations AS reservations \
             WHERE previews.preview_id = $1 AND previews.worker_session_id = $2 \
               AND previews.reservation_id = reservations.id AND reservations.worker_id = $3 \
               AND reservations.state = 'reserved' RETURNING previews.reservation_id",
        )
        .bind(preview_id)
        .bind(worker_session_id)
        .bind(worker_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?
        .ok_or_else(|| {
            error_with(
                "preview_reservation_not_found",
                "matching preview reservation does not exist",
            )
        })?;
        sqlx::query(
            "UPDATE worker_capacity_reservations SET state = 'released', released_at = clock_timestamp() \
             WHERE id = $1 AND state = 'reserved'",
        )
        .bind(reservation_id)
        .execute(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?;
        transaction
            .commit()
            .await
            .map_err(|error| error_with("database_error", error))?;
        Ok(())
    }

    /// Call only after the control session has stopped accepting starts and
    /// every preview it started has either closed successfully or never opened.
    pub async fn finish_preview_session(
        &self,
        worker_id: Uuid,
        worker_session_id: Uuid,
    ) -> PortResult<u64> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| error_with("database_error", error))?;
        lock_queue(&mut transaction).await?;
        sqlx::query("SELECT id FROM workers WHERE id = $1 FOR UPDATE")
            .bind(worker_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|error| error_with("database_error", error))?
            .ok_or_else(|| error_with("worker_not_found", "preview worker is not registered"))?;
        sqlx::query("DELETE FROM worker_control_sessions WHERE worker_id = $1 AND session_id = $2")
            .bind(worker_id)
            .bind(worker_session_id)
            .execute(&mut *transaction)
            .await
            .map_err(|error| error_with("database_error", error))?;
        let released = sqlx::query(
            "WITH removed AS ( \
                 DELETE FROM preview_capacity_reservations AS previews \
                 USING worker_capacity_reservations AS reservations \
                 WHERE previews.reservation_id = reservations.id \
                   AND previews.worker_session_id = $2 \
                   AND reservations.worker_id = $1 AND reservations.state = 'reserved' \
                 RETURNING previews.reservation_id \
             ) \
             UPDATE worker_capacity_reservations SET state = 'released', \
                    released_at = clock_timestamp() \
             WHERE id IN (SELECT reservation_id FROM removed)",
        )
        .bind(worker_id)
        .bind(worker_session_id)
        .execute(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?
        .rows_affected();
        transaction
            .commit()
            .await
            .map_err(|error| error_with("database_error", error))?;
        Ok(released)
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
            false,
            false,
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
                 WHERE reservation_id = $1) \
             AND NOT EXISTS (SELECT 1 FROM preview_capacity_reservations \
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
                    "capacity reservation belongs to an active assignment or preview"
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
    require_control_session: bool,
    require_fenced: bool,
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
                 capabilities, capacity, last_heartbeat FROM workers WHERE id = $1 \
                 AND (NOT $2 OR EXISTS (SELECT 1 FROM worker_control_sessions \
                     WHERE worker_id = workers.id)) \
                 AND (NOT $3 OR EXISTS (SELECT 1 FROM worker_control_sessions AS sessions \
                     JOIN preview_worker_process_fences AS fences ON fences.worker_id = sessions.worker_id \
                     AND fences.incarnation_id = sessions.process_incarnation \
                     WHERE sessions.worker_id = workers.id)) FOR UPDATE",
        )
        .bind(worker_id)
        .bind(require_control_session)
        .bind(require_fenced)
        .fetch_all(&mut **transaction)
        .await
    } else {
        sqlx::query_as::<_, WorkerRow>(
            "SELECT id, stable_name, platform, architecture, version, state, \
                 capabilities, capacity, last_heartbeat FROM workers \
                 WHERE (NOT $1 OR EXISTS (SELECT 1 FROM worker_control_sessions \
                     WHERE worker_id = workers.id)) \
                 AND (NOT $2 OR EXISTS (SELECT 1 FROM worker_control_sessions AS sessions \
                     JOIN preview_worker_process_fences AS fences ON fences.worker_id = sessions.worker_id \
                     AND fences.incarnation_id = sessions.process_incarnation \
                     WHERE sessions.worker_id = workers.id)) ORDER BY id FOR UPDATE",
        )
        .bind(require_control_session)
        .bind(require_fenced)
        .fetch_all(&mut **transaction)
        .await
    }
    .map_err(|error| error_with("database_error", error))?;
    if required_worker_id.is_some() && rows.is_empty() {
        return Err(error_with(
            if require_control_session {
                "worker_session_unavailable"
            } else {
                "source_worker_unavailable"
            },
            "the worker hosting this source revision is unavailable",
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
             ) OR EXISTS( \
                 SELECT 1 FROM preview_capacity_reservations AS previews \
                 JOIN worker_capacity_reservations AS reservations \
                   ON reservations.id = previews.reservation_id \
                 WHERE previews.source_id = $1 AND previews.source_revision = $2 \
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
    let conflicting_preview: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM preview_capacity_reservations AS previews \
         JOIN worker_capacity_reservations AS reservations ON reservations.id = previews.reservation_id \
         WHERE previews.source_id = $1 AND previews.source_revision <> $2 \
           AND reservations.state = 'reserved')",
    )
    .bind(source_id)
    .bind(source_revision)
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| error_with("database_error", error))?;
    if conflicting_preview {
        return Err(error_with(
            "preview_source_revision_in_use",
            "another revision of this source is reserved for preview",
        ));
    }
    let worker_ids: Vec<Option<Uuid>> = sqlx::query_scalar(
        "SELECT DISTINCT worker_id FROM ( \
         SELECT worker_id FROM pipeline_assignments \
         WHERE resource_type = 'pipeline' AND source_id = $1 AND source_revision = $2 \
           AND ((desired_state = 'planned' AND observed_state <> 'cancelled') \
             OR desired_state = 'running' \
             OR (desired_state = 'stopped' AND observed_state = 'stopping')) \
         UNION ALL \
         SELECT reservations.worker_id FROM preview_capacity_reservations AS previews \
         JOIN worker_capacity_reservations AS reservations ON reservations.id = previews.reservation_id \
         WHERE previews.source_id = $1 AND previews.source_revision = $2 \
           AND reservations.state = 'reserved' \
         ) AS source_workers \
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
        catalog::{self, ResourceDeleteError, ResourceUpdateError},
        pipeline_planning::{PgPipelinePlanner, PipelinePlanningDecision, RouteRevision},
        postgres::{connect_postgres, migrate},
        worker_registry::PgWorkerRegistry,
    };

    use super::*;

    #[tokio::test]
    #[ignore = "requires an isolated PostgreSQL database; set DATABASE_URL and run ignored tests"]
    async fn fenced_preview_recovery_requires_proof_and_releases_atomically() {
        let pool = connect_postgres(&env::var("DATABASE_URL").unwrap())
            .await
            .unwrap();
        migrate(&pool).await.unwrap();
        let worker_id = Uuid::new_v4();
        let mut worker = Worker {
            id: worker_id,
            stable_name: format!("fence-worker-{worker_id}"),
            version: "test".to_owned(),
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
        let registry = PgWorkerRegistry::new(pool.clone());
        registry.register(worker.clone()).await.unwrap();
        worker.state = WorkerState::Accepting;
        registry.heartbeat(worker).await.unwrap();
        let reservations = PgCapacityReservations::new(pool.clone());
        let first = ProcessFenceIdentity {
            host_id: Uuid::new_v4(),
            boot_id: Uuid::new_v4(),
            device: 12,
            inode: 34,
            proc_device: 56,
            host_pid: 78,
            start_ticks: 90,
        };
        let incarnation = Uuid::new_v4();
        assert_eq!(
            reservations
                .recover_fenced_previews(worker_id, incarnation, &first, |_| {
                    Err(error_with("worker_fence_unverified", "not verified"))
                })
                .await
                .unwrap_err()
                .code,
            "worker_fence_unverified"
        );
        let session_id = Uuid::new_v4();
        sqlx::query("INSERT INTO worker_control_sessions (worker_id, session_id) VALUES ($1, $2)")
            .bind(worker_id)
            .bind(session_id)
            .execute(&pool)
            .await
            .unwrap();
        let source = SourceMediaProfile::RtpL16 {
            bind_address: std::net::Ipv4Addr::LOCALHOST.into(),
            port: 5000,
            payload_type: 96,
        };
        let source_row = catalog::create_source(
            &pool,
            "fence preview source",
            source.adapter_id(),
            serde_json::to_value(&source).unwrap(),
        )
        .await
        .unwrap();
        catalog::validate_source_draft(&pool, source_row.id)
            .await
            .unwrap();
        let preview_id = Uuid::new_v4();
        assert!(matches!(
            reservations
                .reserve_preview(preview_id, source_row.id, Duration::seconds(60))
                .await
                .unwrap(),
            PreviewReservationDecision::Reserved(_)
        ));
        assert_eq!(
            reservations
                .reserve_fenced_preview(Uuid::new_v4(), source_row.id, Duration::seconds(60))
                .await
                .unwrap_err()
                .code,
            "worker_session_unavailable"
        );
        assert_eq!(
            reservations
                .recover_fenced_previews(worker_id, incarnation, &first, |previous| {
                    assert!(previous.is_none());
                    Ok(())
                })
                .await
                .unwrap_err()
                .code,
            "worker_fence_enrollment_unsafe"
        );
        reservations
            .release_preview(preview_id, worker_id, session_id)
            .await
            .unwrap();
        assert_eq!(
            reservations
                .recover_fenced_previews(worker_id, incarnation, &first, |previous| {
                    assert!(previous.is_none());
                    Ok(())
                })
                .await
                .unwrap(),
            0
        );
        assert!(sqlx::query(
            "INSERT INTO worker_control_sessions (worker_id, session_id) VALUES ($1, $2)"
        )
        .bind(worker_id)
        .bind(Uuid::new_v4())
        .execute(&pool)
        .await
        .is_err());
        assert!(sqlx::query(
            "INSERT INTO worker_control_sessions (worker_id, session_id, process_incarnation) \
             VALUES ($1, $2, $3)"
        )
        .bind(worker_id)
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .execute(&pool)
        .await
        .is_err());
        let session_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO worker_control_sessions (worker_id, session_id, process_incarnation) \
             VALUES ($1, $2, $3)",
        )
            .bind(worker_id)
            .bind(session_id)
            .bind(incarnation)
            .execute(&pool)
            .await
            .unwrap();
        let preview_id = Uuid::new_v4();
        assert!(matches!(
            reservations
                .reserve_fenced_preview(preview_id, source_row.id, Duration::seconds(60))
                .await
                .unwrap(),
            PreviewReservationDecision::Reserved(_)
        ));
        let mut next = first.clone();
        next.host_pid += 1;
        let next_incarnation = Uuid::new_v4();
        let error = reservations
            .recover_fenced_previews(worker_id, next_incarnation, &next, |previous| {
                assert_eq!(previous, Some(&first));
                Err(error_with("worker_fence_unverified", "previous process is alive"))
            })
            .await
            .unwrap_err();
        assert_eq!(error.code, "worker_fence_unverified");
        let owned: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM preview_capacity_reservations WHERE preview_id = $1)",
        )
        .bind(preview_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(owned);
        let mut competing = first.clone();
        competing.host_pid += 2;
        let competing_incarnation = Uuid::new_v4();
        let verify = |previous: Option<&ProcessFenceIdentity>| {
            if previous == Some(&first) {
                Ok(())
            } else {
                Err(error_with("worker_fence_unverified", "incarnation changed"))
            }
        };
        let (one, two) = tokio::join!(
            reservations.recover_fenced_previews(worker_id, next_incarnation, &next, verify),
            reservations.recover_fenced_previews(
                worker_id,
                competing_incarnation,
                &competing,
                verify,
            ),
        );
        let (expected_incarnation, expected_identity) = match (one, two) {
            (Ok(1), Err(error)) => {
                assert_eq!(error.code, "worker_fence_unverified");
                (next_incarnation, &next)
            }
            (Err(error), Ok(1)) => {
                assert_eq!(error.code, "worker_fence_unverified");
                (competing_incarnation, &competing)
            }
            results => panic!("expected exactly one recovery: {results:?}"),
        };
        let session: Option<Uuid> =
            sqlx::query_scalar("SELECT session_id FROM worker_control_sessions WHERE worker_id = $1")
                .bind(worker_id)
                .fetch_optional(&pool)
                .await
                .unwrap();
        assert!(session.is_none());
        let state: String = sqlx::query_scalar(
            "SELECT state FROM worker_capacity_reservations WHERE pipeline_id = $1",
        )
        .bind(preview_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state, "released");
        let saved: (Uuid, sqlx::types::Json<ProcessFenceIdentity>) = sqlx::query_as(
            "SELECT incarnation_id, process_identity FROM preview_worker_process_fences WHERE worker_id = $1",
        )
        .bind(worker_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(saved.0, expected_incarnation);
        assert_eq!(&saved.1.0, expected_identity);
    }

    #[tokio::test]
    #[ignore = "requires an isolated PostgreSQL database; set DATABASE_URL and run ignored tests"]
    async fn preview_reservations_serialize_with_routes_and_remain_owned_until_release() {
        let pool = connect_postgres(&env::var("DATABASE_URL").unwrap())
            .await
            .unwrap();
        migrate(&pool).await.unwrap();
        let registry = PgWorkerRegistry::new(pool.clone());
        for _ in 0..2 {
            let id = Uuid::new_v4();
            let mut worker = Worker {
                id,
                stable_name: format!("preview-worker-{id}"),
                version: "test".to_owned(),
                state: WorkerState::Registering,
                capabilities: WorkerCapabilities {
                    platform: Some(Platform::Linux),
                    architecture: Some(Architecture::X86_64),
                    source_adapters: BTreeSet::from(["rtp_l16".to_owned()]),
                    destination_adapters: BTreeSet::from(["rtp_l16".to_owned()]),
                    ..WorkerCapabilities::default()
                },
                capacity: WorkerCapacity {
                    total_capacity_units: 3,
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
            sqlx::query(
                "INSERT INTO worker_control_sessions (worker_id, session_id) VALUES ($1, $2)",
            )
            .bind(id)
            .bind(Uuid::new_v4())
            .execute(&pool)
            .await
            .unwrap();
        }
        let source = SourceMediaProfile::RtpL16 {
            bind_address: std::net::Ipv4Addr::LOCALHOST.into(),
            port: 5000,
            payload_type: 96,
        };
        let destination = arc_domain::DestinationMediaProfile::RtpL16 {
            address: std::net::Ipv4Addr::LOCALHOST.into(),
            port: 5002,
            payload_type: 96,
        };
        let source_row = catalog::create_source(
            &pool,
            "preview source",
            source.adapter_id(),
            serde_json::to_value(&source).unwrap(),
        )
        .await
        .unwrap();
        catalog::validate_source_draft(&pool, source_row.id)
            .await
            .unwrap();
        let destination_row = catalog::create_destination(
            &pool,
            "preview destination",
            destination.adapter_id(),
            serde_json::to_value(&destination).unwrap(),
        )
        .await
        .unwrap();
        catalog::validate_destination_draft(&pool, destination_row.id)
            .await
            .unwrap();
        let planner = PgPipelinePlanner::new(pool.clone());
        let reservations = PgCapacityReservations::new(pool.clone());
        let preview_id = Uuid::new_v4();
        let route = RouteRevision {
            source_id: source_row.id,
            source_revision: 1,
            destination_id: destination_row.id,
            destination_revision: 1,
        };
        let (preview, route) = tokio::join!(
            reservations.reserve_preview(preview_id, source_row.id, Duration::seconds(60)),
            planner.plan(Uuid::new_v4(), route, Duration::seconds(60)),
        );
        let PreviewReservationDecision::Reserved(preview) = preview.unwrap() else {
            panic!("preview placement failed");
        };
        let PipelinePlanningDecision::Planned(route) = route.unwrap() else {
            panic!("route placement failed");
        };
        assert_eq!(preview.reservation.worker_id, route.assignment.worker_id);
        let totals = reservation_capacity_totals(&mut *pool.acquire().await.unwrap(), None, None)
            .await
            .unwrap();
        assert_eq!(totals.get(&preview.reservation.worker_id), Some(&3));
        assert!(matches!(
            reservations
                .reserve_preview(Uuid::new_v4(), source_row.id, Duration::seconds(60))
                .await
                .unwrap(),
            PreviewReservationDecision::Unavailable { .. }
        ));
        assert_eq!(
            reservations
                .release(preview.reservation.id)
                .await
                .unwrap_err()
                .code,
            "reservation_in_use"
        );
        assert_eq!(
            crate::assignments::PgAssignments::new(pool.clone())
                .plan(
                    preview.reservation.id,
                    &PipelineRequirements {
                        source_adapter: Some("rtp_l16".to_owned()),
                        capacity_units: 2,
                        ..PipelineRequirements::default()
                    },
                    Duration::seconds(60),
                )
                .await
                .unwrap_err()
                .code,
            "reservation_in_use"
        );
        assert_eq!(
            reservations
                .release_preview(preview_id, preview.reservation.worker_id, Uuid::new_v4())
                .await
                .unwrap_err()
                .code,
            "preview_reservation_not_found"
        );
        assert!(matches!(
            catalog::update_source(
                &pool,
                source_row.id,
                "edited",
                source.adapter_id(),
                serde_json::to_value(&source).unwrap()
            )
            .await,
            Err(ResourceUpdateError::InUse)
        ));
        assert!(matches!(
            catalog::delete_source(&pool, source_row.id, 1).await,
            Err(ResourceDeleteError::InUse)
        ));
        let reconnected = PgCapacityReservations::new(
            connect_postgres(&env::var("DATABASE_URL").unwrap())
                .await
                .unwrap(),
        );
        assert!(matches!(
            reconnected
                .reserve_preview(Uuid::new_v4(), source_row.id, Duration::seconds(60))
                .await
                .unwrap(),
            PreviewReservationDecision::Unavailable { .. }
        ));
        reconnected
            .release_preview(
                preview_id,
                preview.reservation.worker_id,
                preview.worker_session_id,
            )
            .await
            .unwrap();
        let totals = reservation_capacity_totals(&mut *pool.acquire().await.unwrap(), None, None)
            .await
            .unwrap();
        assert_eq!(totals.get(&route.assignment.worker_id), Some(&2));
        let raced_id = Uuid::new_v4();
        let (finished, raced) = tokio::join!(
            reconnected
                .finish_preview_session(route.assignment.worker_id, preview.worker_session_id),
            reconnected.reserve_preview(raced_id, source_row.id, Duration::seconds(60)),
        );
        match raced {
            Ok(PreviewReservationDecision::Reserved(raced)) => {
                assert_eq!(raced.worker_session_id, preview.worker_session_id);
                assert_eq!(finished.unwrap(), 1);
            }
            Err(error) => {
                assert_eq!(error.code, "worker_session_unavailable");
                assert_eq!(finished.unwrap(), 0);
            }
            Ok(PreviewReservationDecision::Unavailable { .. }) => {
                panic!("unexpected capacity rejection");
            }
        }
        let stale_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM preview_capacity_reservations WHERE preview_id = $1",
        )
        .bind(raced_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(stale_count, 0);
        sqlx::query("INSERT INTO worker_control_sessions (worker_id, session_id) VALUES ($1, $2)")
            .bind(route.assignment.worker_id)
            .bind(Uuid::new_v4())
            .execute(&pool)
            .await
            .unwrap();
        let next_id = Uuid::new_v4();
        let PreviewReservationDecision::Reserved(next) = reconnected
            .reserve_preview(next_id, source_row.id, Duration::seconds(60))
            .await
            .unwrap()
        else {
            panic!("preview did not share the planned route");
        };
        assert_eq!(next.reservation.worker_id, route.assignment.worker_id);
        assert_eq!(
            reconnected
                .finish_preview_session(next.reservation.worker_id, Uuid::new_v4())
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            reconnected
                .finish_preview_session(next.reservation.worker_id, next.worker_session_id)
                .await
                .unwrap(),
            1
        );
        let replacement_session = Uuid::new_v4();
        sqlx::query("INSERT INTO worker_control_sessions (worker_id, session_id) VALUES ($1, $2)")
            .bind(next.reservation.worker_id)
            .bind(replacement_session)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            reconnected
                .finish_preview_session(next.reservation.worker_id, next.worker_session_id)
                .await
                .unwrap(),
            0
        );
        let replacement_id = Uuid::new_v4();
        let PreviewReservationDecision::Reserved(replacement) = reconnected
            .reserve_preview(replacement_id, source_row.id, Duration::seconds(60))
            .await
            .unwrap()
        else {
            panic!("preview did not use the replacement session");
        };
        assert_eq!(replacement.worker_session_id, replacement_session);
        assert_eq!(
            reconnected
                .finish_preview_session(next.reservation.worker_id, next.worker_session_id)
                .await
                .unwrap(),
            0
        );
        assert!(matches!(
            catalog::update_source(
                &pool,
                source_row.id,
                "edited",
                source.adapter_id(),
                serde_json::to_value(&source).unwrap()
            )
            .await,
            Err(ResourceUpdateError::InUse)
        ));
        reconnected
            .release_preview(
                replacement_id,
                replacement.reservation.worker_id,
                replacement.worker_session_id,
            )
            .await
            .unwrap();
        sqlx::query("DELETE FROM worker_control_sessions WHERE worker_id = $1")
            .bind(route.assignment.worker_id)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            reconnected
                .reserve_preview(Uuid::new_v4(), source_row.id, Duration::seconds(60))
                .await
                .unwrap_err()
                .code,
            "worker_session_unavailable"
        );
        let other_source = catalog::create_source(
            &pool,
            "another preview source",
            source.adapter_id(),
            serde_json::to_value(&source).unwrap(),
        )
        .await
        .unwrap();
        catalog::validate_source_draft(&pool, other_source.id)
            .await
            .unwrap();
        let other_id = Uuid::new_v4();
        let PreviewReservationDecision::Reserved(other) = reconnected
            .reserve_preview(other_id, other_source.id, Duration::seconds(60))
            .await
            .unwrap()
        else {
            panic!("preview did not select the connected worker");
        };
        assert_ne!(other.reservation.worker_id, route.assignment.worker_id);
        reconnected
            .release_preview(
                other_id,
                other.reservation.worker_id,
                other.worker_session_id,
            )
            .await
            .unwrap();
        catalog::update_source(
            &pool,
            source_row.id,
            "new revision",
            source.adapter_id(),
            serde_json::to_value(&source).unwrap(),
        )
        .await
        .unwrap();
        catalog::validate_source_draft(&pool, source_row.id)
            .await
            .unwrap();
        assert_eq!(
            reconnected
                .reserve_preview(Uuid::new_v4(), source_row.id, Duration::seconds(60))
                .await
                .unwrap_err()
                .code,
            "preview_source_revision_in_use"
        );
    }

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
