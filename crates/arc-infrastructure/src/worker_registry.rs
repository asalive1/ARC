use arc_application::{PortError, PortResult, WorkerRegistry};
use arc_domain::{Architecture, Platform, Worker, WorkerCapabilities, WorkerCapacity, WorkerState};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{types::Json, FromRow, PgPool};
use uuid::Uuid;

pub struct PgWorkerRegistry {
    pool: PgPool,
}

impl PgWorkerRegistry {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn begin_drain(&self, worker_id: Uuid) -> PortResult<WorkerState> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| error_with("database_error", error))?;
        let row: Option<WorkerRow> = sqlx::query_as(
            "SELECT id, stable_name, platform, architecture, version, state, \
             capabilities, capacity, last_heartbeat FROM workers WHERE id = $1 FOR UPDATE",
        )
        .bind(worker_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?;
        let worker = row
            .ok_or_else(|| PortError {
                code: "worker_not_found",
                message: "worker is not registered".to_owned(),
            })?
            .into_worker()?;
        let state = match worker.state {
            WorkerState::Ready | WorkerState::Accepting => {
                sqlx::query(
                    "UPDATE workers SET state = 'draining', updated_at = clock_timestamp() \
                     WHERE id = $1",
                )
                .bind(worker_id)
                .execute(&mut *transaction)
                .await
                .map_err(|error| error_with("database_error", error))?;
                WorkerState::Draining
            }
            WorkerState::Draining | WorkerState::Drained => worker.state,
            _ => {
                return Err(PortError {
                    code: "worker_state_conflict",
                    message: "worker must be ready or accepting before draining".to_owned(),
                })
            }
        };
        transaction
            .commit()
            .await
            .map_err(|error| error_with("database_error", error))?;
        Ok(state)
    }
}

#[derive(FromRow)]
pub(crate) struct WorkerRow {
    id: Uuid,
    stable_name: String,
    platform: String,
    architecture: String,
    version: String,
    state: String,
    capabilities: Json<WorkerCapabilities>,
    capacity: Json<WorkerCapacity>,
    last_heartbeat: DateTime<Utc>,
}

impl WorkerRow {
    pub(crate) fn into_worker(self) -> PortResult<Worker> {
        let state = serde_json::from_value(serde_json::Value::String(self.state))
            .map_err(|error| error_with("invalid_worker_record", error))?;
        let worker = Worker {
            id: self.id,
            stable_name: self.stable_name,
            version: self.version,
            state,
            capabilities: self.capabilities.0,
            capacity: self.capacity.0,
            last_heartbeat: self.last_heartbeat,
        };
        let (platform, architecture) = validate_worker(&worker)?;
        if self.platform != platform || self.architecture != architecture {
            return Err(PortError {
                code: "invalid_worker_record",
                message: "stored worker platform and architecture do not match its capabilities"
                    .to_owned(),
            });
        }
        Ok(worker)
    }
}

pub(crate) fn error_with(code: &'static str, error: impl std::fmt::Display) -> PortError {
    PortError {
        code,
        message: error.to_string(),
    }
}

fn validate_worker(worker: &Worker) -> PortResult<(&'static str, &'static str)> {
    if worker.id.is_nil()
        || worker.stable_name.trim().is_empty()
        || worker.stable_name.trim() != worker.stable_name
        || worker.version.trim().is_empty()
        || worker.capacity.assigned_capacity_units > worker.capacity.total_capacity_units
        || [
            worker.capacity.cpu_utilization_ratio,
            worker.capacity.memory_utilization_ratio,
        ]
        .into_iter()
        .flatten()
        .any(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
    {
        return Err(PortError {
            code: "invalid_worker",
            message: "worker identity, version, or capacity is invalid".to_owned(),
        });
    }
    let platform = match worker.capabilities.platform {
        Some(Platform::Linux) => "linux",
        Some(Platform::Windows) => "windows",
        None => {
            return Err(PortError {
                code: "invalid_worker",
                message: "worker platform is required".to_owned(),
            })
        }
    };
    let architecture = match worker.capabilities.architecture {
        Some(Architecture::X86_64) => "x86_64",
        Some(Architecture::Aarch64) => "aarch64",
        None => {
            return Err(PortError {
                code: "invalid_worker",
                message: "worker architecture is required".to_owned(),
            })
        }
    };
    Ok((platform, architecture))
}

fn state_name(state: WorkerState) -> &'static str {
    match state {
        WorkerState::Registering => "registering",
        WorkerState::Warming => "warming",
        WorkerState::Ready => "ready",
        WorkerState::Accepting => "accepting",
        WorkerState::Draining => "draining",
        WorkerState::Drained => "drained",
        WorkerState::Degraded => "degraded",
        WorkerState::Failed => "failed",
        WorkerState::Offline => "offline",
    }
}

#[async_trait]
impl WorkerRegistry for PgWorkerRegistry {
    async fn register(&self, worker: Worker) -> PortResult<()> {
        let (platform, architecture) = validate_worker(&worker)?;
        if worker.state != WorkerState::Registering || worker.capacity.assigned_capacity_units != 0
        {
            return Err(PortError {
                code: "invalid_worker",
                message: "new workers must register without assignments in registering state"
                    .to_owned(),
            });
        }
        let result = sqlx::query(
            "INSERT INTO workers \
             (id, stable_name, platform, architecture, version, state, capabilities, capacity, last_heartbeat) \
             VALUES ($1, $2, $3, $4, $5, 'registering', $6, $7, clock_timestamp()) \
             ON CONFLICT DO NOTHING",
        )
        .bind(worker.id)
        .bind(&worker.stable_name)
        .bind(platform)
        .bind(architecture)
        .bind(&worker.version)
        .bind(Json(&worker.capabilities))
        .bind(Json(&worker.capacity))
        .execute(&self.pool)
        .await
        .map_err(|error| error_with("database_error", error))?;
        if result.rows_affected() != 1 {
            return Err(PortError {
                code: "worker_conflict",
                message: "worker ID or stable name is already registered".to_owned(),
            });
        }
        Ok(())
    }

    async fn heartbeat(&self, worker: Worker) -> PortResult<()> {
        let (platform, architecture) = validate_worker(&worker)?;
        if matches!(worker.state, WorkerState::Draining | WorkerState::Drained) {
            return Err(PortError {
                code: "invalid_worker",
                message: "drain state is managed by the controller".to_owned(),
            });
        }
        let result = sqlx::query(
            "UPDATE workers SET version = $5, \
             state = CASE WHEN state IN ('draining', 'drained') THEN state ELSE $6 END, \
             capabilities = $7, capacity = $8, last_heartbeat = clock_timestamp(), \
             updated_at = clock_timestamp() \
             WHERE id = $1 AND stable_name = $2 AND platform = $3 AND architecture = $4",
        )
        .bind(worker.id)
        .bind(&worker.stable_name)
        .bind(platform)
        .bind(architecture)
        .bind(&worker.version)
        .bind(state_name(worker.state))
        .bind(Json(&worker.capabilities))
        .bind(Json(&worker.capacity))
        .execute(&self.pool)
        .await
        .map_err(|error| error_with("database_error", error))?;
        if result.rows_affected() != 1 {
            return Err(PortError {
                code: "worker_not_found",
                message: "worker is not registered with this identity and platform".to_owned(),
            });
        }
        Ok(())
    }

    async fn list_workers(&self) -> PortResult<Vec<Worker>> {
        let workers = sqlx::query_as::<_, WorkerRow>(
            "SELECT id, stable_name, platform, architecture, version, state, \
             capabilities, capacity, last_heartbeat FROM workers ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|error| error_with("database_error", error))?;
        workers.into_iter().map(WorkerRow::into_worker).collect()
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, env};

    use arc_domain::{select_worker, PipelineRequirements, PlacementDecision};
    use chrono::Duration;

    use crate::{
        assignments::PgAssignments,
        capacity_reservations::{PgCapacityReservations, ReservationDecision},
        postgres::{connect_postgres, migrate},
    };

    use super::*;

    #[tokio::test]
    #[ignore = "requires an isolated PostgreSQL database; set DATABASE_URL and run ignored tests"]
    async fn registry_persists_identity_and_uses_server_heartbeat_time() {
        let pool = connect_postgres(&env::var("DATABASE_URL").expect("DATABASE_URL is set"))
            .await
            .expect("database connects");
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
            last_heartbeat: Utc::now() + Duration::days(1),
        };
        let before: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&pool)
            .await
            .expect("database clock reads");
        let mut invalid_registration = worker.clone();
        invalid_registration.state = WorkerState::Ready;
        assert_eq!(
            registry
                .register(invalid_registration)
                .await
                .unwrap_err()
                .code,
            "invalid_worker"
        );
        registry.register(worker.clone()).await.expect("register");
        assert_eq!(
            registry.register(worker.clone()).await.unwrap_err().code,
            "worker_conflict"
        );
        let mut other = worker.clone();
        other.id = Uuid::new_v4();
        assert_eq!(
            registry.register(other).await.unwrap_err().code,
            "worker_conflict"
        );

        worker.state = WorkerState::Accepting;
        worker.capacity.assigned_capacity_units = 1;
        registry.heartbeat(worker.clone()).await.expect("heartbeat");
        let reconnected_pool =
            connect_postgres(&env::var("DATABASE_URL").expect("DATABASE_URL is set"))
                .await
                .expect("database reconnects");
        let workers = PgWorkerRegistry::new(reconnected_pool)
            .list_workers()
            .await
            .expect("persisted workers load");
        let after: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&pool)
            .await
            .expect("database clock reads");
        let saved = workers
            .iter()
            .find(|saved| saved.id == id)
            .expect("worker exists");
        assert_eq!(saved.stable_name, worker.stable_name);
        assert_eq!(saved.state, WorkerState::Accepting);
        assert_eq!(saved.capacity.assigned_capacity_units, 1);
        assert!(saved.last_heartbeat >= before && saved.last_heartbeat <= after);
        assert_eq!(
            select_worker(
                std::slice::from_ref(saved),
                &PipelineRequirements {
                    source_adapter: Some("rtp_l16".to_owned()),
                    capacity_units: 3,
                    ..PipelineRequirements::default()
                },
                after,
                Duration::seconds(10)
            ),
            Ok(PlacementDecision::Selected { worker_id: id })
        );

        let mut unknown = worker.clone();
        unknown.id = Uuid::new_v4();
        assert_eq!(
            registry.heartbeat(unknown).await.unwrap_err().code,
            "worker_not_found"
        );
        let mut changed_identity = worker.clone();
        changed_identity.stable_name = "different".to_owned();
        assert_eq!(
            registry.heartbeat(changed_identity).await.unwrap_err().code,
            "worker_not_found"
        );
        let mut changed_platform = worker.clone();
        changed_platform.capabilities.platform = Some(Platform::Windows);
        assert_eq!(
            registry.heartbeat(changed_platform).await.unwrap_err().code,
            "worker_not_found"
        );
        let mut invalid = worker.clone();
        invalid.capacity.cpu_utilization_ratio = Some(f32::NAN);
        assert_eq!(
            registry.heartbeat(invalid).await.unwrap_err().code,
            "invalid_worker"
        );
        let mut invalid_capacity = worker.clone();
        invalid_capacity.capacity.assigned_capacity_units = 5;
        assert_eq!(
            registry.heartbeat(invalid_capacity).await.unwrap_err().code,
            "invalid_worker"
        );
        let mut drain_report = worker.clone();
        drain_report.state = WorkerState::Drained;
        assert_eq!(
            registry.heartbeat(drain_report).await.unwrap_err().code,
            "invalid_worker"
        );

        assert_eq!(
            registry.begin_drain(id).await.unwrap(),
            WorkerState::Draining
        );
        registry
            .heartbeat(worker)
            .await
            .expect("draining worker heartbeat");
        let workers = registry.list_workers().await.expect("list workers");
        assert_eq!(
            workers.iter().find(|saved| saved.id == id).unwrap().state,
            WorkerState::Draining
        );
        sqlx::query("DELETE FROM workers WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .expect("remove test worker");
    }

    #[tokio::test]
    #[ignore = "requires an isolated PostgreSQL database; set DATABASE_URL and run ignored tests"]
    async fn drain_blocks_new_reservations_and_plans_without_discarding_commitments() {
        let url = env::var("DATABASE_URL").expect("DATABASE_URL is set");
        let pool = connect_postgres(&url).await.expect("database connects");
        migrate(&pool).await.expect("migration succeeds");
        let registry = PgWorkerRegistry::new(pool.clone());
        let reservations = PgCapacityReservations::new(pool.clone());
        let assignments = PgAssignments::new(pool.clone());
        let id = Uuid::new_v4();
        let mut worker = Worker {
            id,
            stable_name: format!("worker-{id}"),
            version: "0.1.0".to_owned(),
            state: WorkerState::Registering,
            capabilities: WorkerCapabilities {
                platform: Some(Platform::Linux),
                architecture: Some(Architecture::X86_64),
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
        let requirements = PipelineRequirements {
            capacity_units: 1,
            ..PipelineRequirements::default()
        };
        let age = Duration::seconds(60);
        assert_eq!(
            registry.begin_drain(Uuid::new_v4()).await.unwrap_err().code,
            "worker_not_found"
        );
        registry.register(worker.clone()).await.unwrap();
        assert_eq!(
            registry.begin_drain(id).await.unwrap_err().code,
            "worker_state_conflict"
        );
        worker.state = WorkerState::Accepting;
        registry.heartbeat(worker.clone()).await.unwrap();
        let first = match reservations
            .reserve(Uuid::new_v4(), &requirements, age)
            .await
            .unwrap()
        {
            ReservationDecision::Reserved(value) => value,
            other => panic!("expected reservation, got {other:?}"),
        };
        let assignment = assignments
            .plan(first.id, &requirements, age)
            .await
            .unwrap();
        let pending = match reservations
            .reserve(Uuid::new_v4(), &requirements, age)
            .await
            .unwrap()
        {
            ReservationDecision::Reserved(value) => value,
            other => panic!("expected reservation, got {other:?}"),
        };

        let race_pipeline = Uuid::new_v4();
        let (drain_result, race_result) = tokio::join!(
            registry.begin_drain(id),
            reservations.reserve(race_pipeline, &requirements, age)
        );
        assert_eq!(drain_result.unwrap(), WorkerState::Draining);
        let racing_reservation = match race_result.unwrap() {
            ReservationDecision::Reserved(value) => Some(value),
            ReservationDecision::Unavailable { .. } => None,
        };
        assert_eq!(
            registry.begin_drain(id).await.unwrap(),
            WorkerState::Draining
        );
        registry.heartbeat(worker).await.unwrap();
        let reconnected = PgWorkerRegistry::new(connect_postgres(&url).await.unwrap());
        assert_eq!(
            reconnected
                .list_workers()
                .await
                .unwrap()
                .iter()
                .find(|w| w.id == id)
                .unwrap()
                .state,
            WorkerState::Draining
        );
        assert!(matches!(
            reservations
                .reserve(Uuid::new_v4(), &requirements, age)
                .await
                .unwrap(),
            ReservationDecision::Unavailable { .. }
        ));
        assert_eq!(
            assignments
                .plan(pending.id, &requirements, age)
                .await
                .unwrap_err()
                .code,
            "placement_unavailable"
        );
        assert!(assignments
            .list_planned()
            .await
            .unwrap()
            .contains(&assignment));
        assert!(reservations.list_active().await.unwrap().contains(&pending));
        if let Some(reservation) = racing_reservation {
            assert_eq!(
                assignments
                    .plan(reservation.id, &requirements, age)
                    .await
                    .unwrap_err()
                    .code,
                "placement_unavailable"
            );
            reservations.release(reservation.id).await.unwrap();
        }
        assignments
            .cancel_unstarted(assignment.id, assignment.generation)
            .await
            .unwrap();
        reservations.release(pending.id).await.unwrap();

        sqlx::query("UPDATE workers SET state = 'drained' WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            registry.begin_drain(id).await.unwrap(),
            WorkerState::Drained
        );

        sqlx::query("DELETE FROM pipeline_assignments WHERE worker_id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM worker_capacity_reservations WHERE worker_id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM workers WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
    }
}
