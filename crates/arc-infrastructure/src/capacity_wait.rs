use arc_application::PortResult;
use arc_domain::PipelineEstimate;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{postgres::PgListener, types::Json, FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{
    pipeline_planning::{
        validate_route, PgPipelinePlanner, PipelinePlanningDecision, RouteRevision,
    },
    worker_registry::error_with,
};

const QUEUE_LOCK: i64 = 0x415243434150;

pub(crate) async fn lock_queue(tx: &mut Transaction<'_, Postgres>) -> PortResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(QUEUE_LOCK)
        .execute(&mut **tx)
        .await
        .map_err(|error| error_with("database_error", error))?;
    Ok(())
}

async fn bound_locks(tx: &mut Transaction<'_, Postgres>) -> PortResult<()> {
    sqlx::query("SET LOCAL lock_timeout = '2s'")
        .execute(&mut **tx)
        .await
        .map_err(|error| error_with("database_error", error))?;
    Ok(())
}

pub(crate) async fn has_waiters(tx: &mut Transaction<'_, Postgres>) -> PortResult<bool> {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM capacity_wait_requests \
         WHERE state = 'waiting' AND expires_at > clock_timestamp())",
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| error_with("database_error", error))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapacityWaitRequest {
    pub request_id: Uuid,
    pub pipeline_id: Uuid,
    pub state: String,
    pub estimate: PipelineEstimate,
    pub result: Option<PipelinePlanningDecision>,
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(FromRow)]
struct WaitRow {
    request_id: Uuid,
    pipeline_id: Uuid,
    source_id: Uuid,
    source_revision: i64,
    destination_id: Uuid,
    destination_revision: i64,
    state: String,
    estimate: Json<PipelineEstimate>,
    result: Option<Json<PipelinePlanningDecision>>,
    reason: Option<String>,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

impl WaitRow {
    fn snapshot(self) -> CapacityWaitRequest {
        CapacityWaitRequest {
            request_id: self.request_id,
            pipeline_id: self.pipeline_id,
            state: self.state,
            estimate: self.estimate.0,
            result: self.result.map(|result| result.0),
            reason: self.reason,
            created_at: self.created_at,
            expires_at: self.expires_at,
        }
    }

    fn route(&self) -> RouteRevision {
        RouteRevision {
            source_id: self.source_id,
            source_revision: self.source_revision,
            destination_id: self.destination_id,
            destination_revision: self.destination_revision,
        }
    }
}

const COLUMNS: &str = "request_id, pipeline_id, source_id, source_revision, destination_id, \
    destination_revision, state, estimate, result, reason, created_at, expires_at";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueProgress {
    Empty,
    Blocked,
    Advanced,
}

#[derive(Clone)]
pub struct PgCapacityWait {
    pool: PgPool,
}

impl PgCapacityWait {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Subscribe before processing the queue, then use a bounded timer as a fallback.
    pub async fn listen(&self) -> PortResult<PgListener> {
        let mut listener = PgListener::connect_with(&self.pool)
            .await
            .map_err(|error| error_with("database_error", error))?;
        listener
            .listen("arc_capacity_change")
            .await
            .map_err(|error| error_with("database_error", error))?;
        Ok(listener)
    }

    /// Enqueue once per idempotency key. A request never reserves capacity here.
    pub async fn enqueue(
        &self,
        key: Uuid,
        pipeline_id: Uuid,
        route: RouteRevision,
    ) -> PortResult<CapacityWaitRequest> {
        if key.is_nil() || pipeline_id.is_nil() {
            return Err(error_with(
                "invalid_reservation",
                "non-nil key and pipeline ID are required",
            ));
        }
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| error_with("database_error", e))?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(key.to_string())
            .execute(&mut *tx)
            .await
            .map_err(|e| error_with("database_error", e))?;
        lock_queue(&mut tx).await?;
        bound_locks(&mut tx).await?;
        expire(&mut tx).await?;
        sqlx::query("DELETE FROM route_plan_idempotency WHERE expires_at <= clock_timestamp()")
            .execute(&mut *tx)
            .await
            .map_err(|e| error_with("database_error", e))?;
        sqlx::query("DELETE FROM capacity_wait_requests w WHERE w.state <> 'waiting' AND NOT EXISTS (SELECT 1 FROM route_plan_idempotency i WHERE i.idempotency_key = w.idempotency_key)")
            .execute(&mut *tx).await.map_err(|e| error_with("database_error", e))?;
        let request = json!({
            "pipelineId": pipeline_id, "sourceId": route.source_id,
            "sourceRevision": route.source_revision, "destinationId": route.destination_id,
            "destinationRevision": route.destination_revision, "allowWaiting": true,
        });
        let previous: Option<Json<Value>> = sqlx::query_scalar(
            "SELECT request FROM route_plan_idempotency WHERE idempotency_key = $1 FOR UPDATE",
        )
        .bind(key)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| error_with("database_error", e))?;
        if let Some(previous) = previous {
            if previous.0 != request {
                return Err(error_with(
                    "idempotency_key_reused",
                    "idempotency key was already used for a different planning request",
                ));
            }
            let row = sqlx::query_as::<_, WaitRow>(&format!(
                "SELECT {COLUMNS} FROM capacity_wait_requests WHERE idempotency_key = $1"
            ))
            .bind(key)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| error_with("database_error", e))?;
            tx.commit()
                .await
                .map_err(|e| error_with("database_error", e))?;
            return self
                .get(row.request_id)
                .await?
                .ok_or_else(|| error_with("database_error", "wait request disappeared"));
        }
        let (estimate, _) = validate_route(&mut tx, route).await?;
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM capacity_wait_requests WHERE pipeline_id = $1 AND state = 'waiting' AND expires_at > clock_timestamp())"
        ).bind(pipeline_id).fetch_one(&mut *tx).await.map_err(|e| error_with("database_error", e))?;
        if exists {
            return Err(error_with(
                "reservation_conflict",
                "pipeline already has a capacity wait",
            ));
        }
        let placeholder = PipelinePlanningDecision::Unavailable {
            estimate: estimate.clone(),
            rejections: Vec::new(),
        };
        sqlx::query("INSERT INTO route_plan_idempotency (idempotency_key, request, response, expires_at) VALUES ($1, $2, $3, clock_timestamp() + INTERVAL '24 hours')")
            .bind(key).bind(Json(request)).bind(Json(placeholder)).execute(&mut *tx).await
            .map_err(|e| error_with("database_error", e))?;
        let request_id = Uuid::new_v4();
        let row = sqlx::query_as::<_, WaitRow>(&format!(
            "INSERT INTO capacity_wait_requests (request_id, idempotency_key, pipeline_id, source_id, source_revision, destination_id, destination_revision, state, estimate) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, 'waiting', $8) RETURNING {COLUMNS}"
        )).bind(request_id).bind(key).bind(pipeline_id).bind(route.source_id).bind(route.source_revision)
            .bind(route.destination_id).bind(route.destination_revision).bind(Json(estimate))
            .fetch_one(&mut *tx).await.map_err(|e| error_with("database_error", e))?;
        tx.commit()
            .await
            .map_err(|e| error_with("database_error", e))?;
        Ok(row.snapshot())
    }

    pub async fn get(&self, request_id: Uuid) -> PortResult<Option<CapacityWaitRequest>> {
        sqlx::query("UPDATE capacity_wait_requests SET state = 'unavailable', reason = 'expired' WHERE request_id = $1 AND state = 'waiting' AND expires_at <= clock_timestamp()")
            .bind(request_id).execute(&self.pool).await.map_err(|e| error_with("database_error", e))?;
        let row = sqlx::query_as::<_, WaitRow>(&format!(
            "SELECT {COLUMNS} FROM capacity_wait_requests WHERE request_id = $1"
        ))
        .bind(request_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| error_with("database_error", e))?;
        Ok(row.map(WaitRow::snapshot))
    }

    /// Cancellation races atomically with successful planning; terminal outcomes are returned unchanged.
    pub async fn cancel(&self, request_id: Uuid) -> PortResult<Option<CapacityWaitRequest>> {
        sqlx::query("UPDATE capacity_wait_requests SET state = 'cancelled', reason = 'cancelled' WHERE request_id = $1 AND state = 'waiting' AND expires_at > clock_timestamp()")
            .bind(request_id).execute(&self.pool).await.map_err(|e| error_with("database_error", e))?;
        self.get(request_id).await
    }

    /// Attempt only the oldest live request. Call repeatedly until Empty or Blocked.
    pub async fn process_one(&self, max_heartbeat_age: Duration) -> PortResult<QueueProgress> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| error_with("database_error", e))?;
        lock_queue(&mut tx).await?;
        bound_locks(&mut tx).await?;
        let expired = expire(&mut tx).await?;
        let row = sqlx::query_as::<_, WaitRow>(&format!(
            "SELECT {COLUMNS} FROM capacity_wait_requests WHERE state = 'waiting' ORDER BY sequence LIMIT 1"
        )).fetch_optional(&mut *tx).await.map_err(|e| error_with("database_error", e))?;
        let Some(row) = row else {
            tx.commit()
                .await
                .map_err(|e| error_with("database_error", e))?;
            return Ok(if expired > 0 {
                QueueProgress::Advanced
            } else {
                QueueProgress::Empty
            });
        };
        sqlx::query("SAVEPOINT capacity_attempt")
            .execute(&mut *tx)
            .await
            .map_err(|e| error_with("database_error", e))?;
        let planner = PgPipelinePlanner::new(self.pool.clone());
        let decision = planner
            .plan_in_transaction(&mut tx, row.pipeline_id, row.route(), max_heartbeat_age)
            .await;
        match decision {
            Ok(PipelinePlanningDecision::Planned(plan)) => {
                let updated = sqlx::query("UPDATE capacity_wait_requests SET state = 'planned', result = $2 WHERE request_id = $1 AND state = 'waiting' AND expires_at > clock_timestamp()")
                    .bind(row.request_id).bind(Json(PipelinePlanningDecision::Planned(plan)))
                    .execute(&mut *tx).await.map_err(|e| error_with("database_error", e))?.rows_affected();
                if updated == 0 {
                    sqlx::query("ROLLBACK TO SAVEPOINT capacity_attempt")
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| error_with("database_error", e))?;
                    expire(&mut tx).await?;
                }
            }
            other => {
                sqlx::query("ROLLBACK TO SAVEPOINT capacity_attempt")
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| error_with("database_error", e))?;
                match other {
                    Err(error) if error.code == "database_error" => return Err(error),
                    Err(error) => {
                        sqlx::query("UPDATE capacity_wait_requests SET state = 'unavailable', reason = $2 WHERE request_id = $1 AND state = 'waiting' AND expires_at > clock_timestamp()")
                            .bind(row.request_id).bind(error.code).execute(&mut *tx).await.map_err(|e| error_with("database_error", e))?;
                        expire(&mut tx).await?;
                    }
                    Ok(PipelinePlanningDecision::Unavailable { .. }) => {
                        expire(&mut tx).await?;
                        let still_waiting: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM capacity_wait_requests WHERE request_id = $1 AND state = 'waiting')")
                            .bind(row.request_id).fetch_one(&mut *tx).await.map_err(|e| error_with("database_error", e))?;
                        tx.commit()
                            .await
                            .map_err(|e| error_with("database_error", e))?;
                        return Ok(if still_waiting {
                            QueueProgress::Blocked
                        } else {
                            QueueProgress::Advanced
                        });
                    }
                    _ => unreachable!(),
                }
            }
        }
        tx.commit()
            .await
            .map_err(|e| error_with("database_error", e))?;
        Ok(QueueProgress::Advanced)
    }
}

async fn expire(tx: &mut Transaction<'_, Postgres>) -> PortResult<u64> {
    Ok(sqlx::query("UPDATE capacity_wait_requests SET state = 'unavailable', reason = 'expired' WHERE state = 'waiting' AND expires_at <= clock_timestamp()")
        .execute(&mut **tx).await.map_err(|e| error_with("database_error", e))?.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        catalog,
        postgres::{connect_postgres, migrate},
        worker_registry::PgWorkerRegistry,
    };
    use arc_application::WorkerRegistry;
    use arc_domain::{
        Architecture, DestinationMediaProfile, Platform, SourceMediaProfile, Worker,
        WorkerCapabilities, WorkerCapacity, WorkerState,
    };
    use std::{collections::BTreeSet, env, net::IpAddr};

    async fn route(
        pool: &PgPool,
        source: SourceMediaProfile,
        destination: DestinationMediaProfile,
    ) -> RouteRevision {
        let s = catalog::create_source(
            pool,
            "wait source",
            source.adapter_id(),
            serde_json::to_value(source).unwrap(),
        )
        .await
        .unwrap();
        let s = catalog::validate_source_draft(pool, s.id).await.unwrap();
        let d = catalog::create_destination(
            pool,
            "wait destination",
            destination.adapter_id(),
            serde_json::to_value(destination).unwrap(),
        )
        .await
        .unwrap();
        let d = catalog::validate_destination_draft(pool, d.id)
            .await
            .unwrap();
        RouteRevision {
            source_id: s.id,
            source_revision: s.current_revision,
            destination_id: d.id,
            destination_revision: d.current_revision,
        }
    }

    #[tokio::test]
    #[ignore = "requires an isolated PostgreSQL database"]
    async fn fifo_cancel_expiry_and_idempotency() {
        let pool = connect_postgres(&env::var("DATABASE_URL").unwrap())
            .await
            .unwrap();
        migrate(&pool).await.unwrap();
        let wait = PgCapacityWait::new(pool.clone());
        let planner = PgPipelinePlanner::new(pool.clone());
        let ip = IpAddr::from([127, 0, 0, 1]);
        let l16 = route(
            &pool,
            SourceMediaProfile::RtpL16 {
                bind_address: ip,
                port: 5000,
                payload_type: 96,
            },
            DestinationMediaProfile::RtpL16 {
                address: ip,
                port: 5002,
                payload_type: 96,
            },
        )
        .await;
        let l24 = route(
            &pool,
            SourceMediaProfile::RtpL24 {
                bind_address: ip,
                port: 5004,
                payload_type: 97,
            },
            DestinationMediaProfile::RtpL24 {
                address: ip,
                port: 5006,
                payload_type: 97,
            },
        )
        .await;
        let id = Uuid::new_v4();
        let registry = PgWorkerRegistry::new(pool.clone());
        let mut worker = Worker {
            id,
            stable_name: format!("wait-{id}"),
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
                total_capacity_units: 4,
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
        let mut listener = wait.listen().await.unwrap();
        let key = Uuid::new_v4();
        let first_pipeline = Uuid::new_v4();
        let first = wait.enqueue(key, first_pipeline, l24).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), listener.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            wait.enqueue(key, first_pipeline, l24)
                .await
                .unwrap()
                .request_id,
            first.request_id
        );
        assert_eq!(
            wait.enqueue(key, Uuid::new_v4(), l24)
                .await
                .unwrap_err()
                .code,
            "idempotency_key_reused"
        );
        assert_eq!(
            planner
                .plan_with_idempotency_key(key, first_pipeline, l24, Duration::seconds(30))
                .await
                .unwrap_err()
                .code,
            "idempotency_key_reused"
        );
        let second = wait
            .enqueue(Uuid::new_v4(), Uuid::new_v4(), l16)
            .await
            .unwrap();
        assert_eq!(
            wait.process_one(Duration::seconds(30)).await.unwrap(),
            QueueProgress::Blocked
        );
        assert_eq!(
            wait.get(second.request_id).await.unwrap().unwrap().state,
            "waiting"
        );
        assert!(matches!(
            planner
                .plan(Uuid::new_v4(), l16, Duration::seconds(30))
                .await
                .unwrap(),
            PipelinePlanningDecision::Unavailable { .. }
        ));
        let requirements = wait
            .get(second.request_id)
            .await
            .unwrap()
            .unwrap()
            .estimate
            .requirements;
        assert!(matches!(
            crate::capacity_reservations::PgCapacityReservations::new(pool.clone())
                .reserve(Uuid::new_v4(), &requirements, Duration::seconds(30))
                .await
                .unwrap(),
            crate::capacity_reservations::ReservationDecision::Unavailable { .. }
        ));
        assert_eq!(
            wait.cancel(first.request_id).await.unwrap().unwrap().state,
            "cancelled"
        );
        assert_eq!(
            wait.process_one(Duration::seconds(30)).await.unwrap(),
            QueueProgress::Advanced
        );
        let planned = wait.get(second.request_id).await.unwrap().unwrap();
        assert_eq!(planned.state, "planned");
        assert!(matches!(
            &planned.result,
            Some(PipelinePlanningDecision::Planned(_))
        ));

        let expired_key = Uuid::new_v4();
        let expired_pipeline = Uuid::new_v4();
        let expired = wait
            .enqueue(expired_key, expired_pipeline, l16)
            .await
            .unwrap();
        sqlx::query("UPDATE capacity_wait_requests SET expires_at = clock_timestamp() - INTERVAL '1 second' WHERE request_id = $1")
            .bind(expired.request_id).execute(&pool).await.unwrap();
        assert_eq!(
            wait.get(expired.request_id)
                .await
                .unwrap()
                .unwrap()
                .reason
                .as_deref(),
            Some("expired")
        );
        wait.process_one(Duration::seconds(30)).await.unwrap();
        assert_eq!(
            wait.get(expired.request_id).await.unwrap().unwrap().state,
            "unavailable"
        );
        assert_eq!(
            wait.cancel(expired.request_id)
                .await
                .unwrap()
                .unwrap()
                .state,
            "unavailable"
        );
        assert_eq!(
            wait.enqueue(expired_key, expired_pipeline, l16)
                .await
                .unwrap()
                .state,
            "unavailable"
        );
        sqlx::query("UPDATE route_plan_idempotency SET created_at = clock_timestamp() - INTERVAL '2 days', expires_at = clock_timestamp() - INTERVAL '1 day' WHERE idempotency_key = $1")
            .bind(expired_key).execute(&pool).await.unwrap();
        sqlx::query("UPDATE capacity_wait_requests SET state = 'waiting' WHERE request_id = $1")
            .bind(expired.request_id)
            .execute(&pool)
            .await
            .unwrap();
        let fresh = wait
            .enqueue(expired_key, expired_pipeline, l16)
            .await
            .unwrap();
        assert_ne!(fresh.request_id, expired.request_id);
        wait.cancel(fresh.request_id).await.unwrap();

        for cancel in [true, false] {
            let pipeline = Uuid::new_v4();
            let pending = wait.enqueue(Uuid::new_v4(), pipeline, l16).await.unwrap();
            let mut worker_lock = pool.begin().await.unwrap();
            sqlx::query("SELECT id FROM workers WHERE id = $1 FOR UPDATE")
                .bind(id)
                .execute(&mut *worker_lock)
                .await
                .unwrap();
            let runner = wait.clone();
            let task = tokio::spawn(async move { runner.process_one(Duration::seconds(30)).await });
            tokio::time::timeout(std::time::Duration::from_secs(3), async {
                loop {
                    let blocked: bool = sqlx::query_scalar(
                        "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE wait_event_type = 'Lock' AND query LIKE '%FROM workers ORDER BY id FOR UPDATE%')"
                    ).fetch_one(&pool).await.unwrap();
                    if blocked { break; }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            }).await.unwrap();
            if cancel {
                let result = tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    wait.cancel(pending.request_id),
                )
                .await
                .unwrap()
                .unwrap()
                .unwrap();
                assert_eq!(result.state, "cancelled");
            } else {
                sqlx::query("UPDATE capacity_wait_requests SET expires_at = clock_timestamp() - INTERVAL '1 second' WHERE request_id = $1")
                    .bind(pending.request_id).execute(&pool).await.unwrap();
            }
            worker_lock.commit().await.unwrap();
            assert_eq!(task.await.unwrap().unwrap(), QueueProgress::Advanced);
            let status = wait.get(pending.request_id).await.unwrap().unwrap();
            assert_eq!(
                status.state,
                if cancel { "cancelled" } else { "unavailable" }
            );
            let reservations: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM worker_capacity_reservations WHERE pipeline_id = $1",
            )
            .bind(pipeline)
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(reservations, 0, "losing attempt must roll back reservation");
        }

        let pending = wait
            .enqueue(Uuid::new_v4(), Uuid::new_v4(), l24)
            .await
            .unwrap();
        assert_eq!(
            wait.process_one(Duration::seconds(30)).await.unwrap(),
            QueueProgress::Blocked
        );
        let mut listener = wait.listen().await.unwrap();
        worker.capabilities.source_adapters.insert("rtp_l24".into());
        worker
            .capabilities
            .destination_adapters
            .insert("rtp_l24".into());
        registry.heartbeat(worker).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), listener.recv())
            .await
            .unwrap()
            .unwrap();
        let reconnected = PgCapacityWait::new(
            connect_postgres(&env::var("DATABASE_URL").unwrap())
                .await
                .unwrap(),
        );
        assert_eq!(
            reconnected
                .get(pending.request_id)
                .await
                .unwrap()
                .unwrap()
                .state,
            "waiting"
        );
        assert_eq!(
            reconnected
                .process_one(Duration::seconds(30))
                .await
                .unwrap(),
            QueueProgress::Advanced
        );
        assert_eq!(
            wait.get(pending.request_id).await.unwrap().unwrap().state,
            "planned"
        );

        let next = wait
            .enqueue(Uuid::new_v4(), Uuid::new_v4(), l16)
            .await
            .unwrap();
        assert_eq!(
            wait.process_one(Duration::seconds(30)).await.unwrap(),
            QueueProgress::Blocked
        );
        let mut listener = wait.listen().await.unwrap();
        let Some(PipelinePlanningDecision::Planned(first_plan)) = planned.result else {
            panic!("expected plan")
        };
        crate::assignments::PgAssignments::new(pool.clone())
            .cancel_unstarted(first_plan.assignment.id, first_plan.assignment.generation)
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), listener.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            wait.process_one(Duration::seconds(30)).await.unwrap(),
            QueueProgress::Advanced
        );
        assert_eq!(
            wait.get(next.request_id).await.unwrap().unwrap().state,
            "planned"
        );

        let stale = wait
            .enqueue(Uuid::new_v4(), Uuid::new_v4(), l16)
            .await
            .unwrap();
        let profile = SourceMediaProfile::RtpL16 {
            bind_address: ip,
            port: 5008,
            payload_type: 96,
        };
        catalog::update_source(
            &pool,
            l16.source_id,
            "edited source",
            profile.adapter_id(),
            serde_json::to_value(&profile).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(
            wait.process_one(Duration::seconds(30)).await.unwrap(),
            QueueProgress::Advanced
        );
        let stale = wait.get(stale.request_id).await.unwrap().unwrap();
        assert_eq!(stale.state, "unavailable");
        assert_eq!(stale.reason.as_deref(), Some("stale_route_revision"));
    }
}
