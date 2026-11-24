use arc_application::{PortError, PortResult, WorkerPlacement, WorkerScheduler};
use arc_domain::{PipelineRequirements, PlacementDecision};
use async_trait::async_trait;
use chrono::Duration;
use sqlx::PgPool;

use crate::{
    capacity_reservations::select_with_reservations,
    worker_registry::{error_with, WorkerRow},
};

pub struct PgWorkerScheduler {
    pool: PgPool,
    max_heartbeat_age: Duration,
}

impl PgWorkerScheduler {
    pub fn new(pool: PgPool, max_heartbeat_age: Duration) -> PortResult<Self> {
        if max_heartbeat_age < Duration::zero() {
            return Err(PortError {
                code: "invalid_placement",
                message: "heartbeat age must be nonnegative".to_owned(),
            });
        }
        Ok(Self {
            pool,
            max_heartbeat_age,
        })
    }

    pub async fn inspect_placement(
        &self,
        requirements: &PipelineRequirements,
    ) -> PortResult<PlacementDecision> {
        if requirements.capacity_units == 0 {
            return Err(PortError {
                code: "invalid_placement",
                message: "placement requires positive capacity".to_owned(),
            });
        }
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| error_with("database_error", error))?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *transaction)
            .await
            .map_err(|error| error_with("database_error", error))?;
        let rows = sqlx::query_as::<_, WorkerRow>(
            "SELECT id, stable_name, platform, architecture, version, state, \
             capabilities, capacity, last_heartbeat FROM workers ORDER BY id",
        )
        .fetch_all(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?;
        let decision =
            select_with_reservations(&mut transaction, rows, requirements, self.max_heartbeat_age)
                .await?;
        transaction
            .commit()
            .await
            .map_err(|error| error_with("database_error", error))?;
        Ok(decision)
    }
}

#[async_trait]
impl WorkerScheduler for PgWorkerScheduler {
    async fn find_placement(
        &self,
        requirements: &PipelineRequirements,
    ) -> PortResult<WorkerPlacement> {
        match self.inspect_placement(requirements).await? {
            PlacementDecision::Selected { worker_id } => Ok(WorkerPlacement { worker_id }),
            PlacementDecision::Unavailable { rejections } => Err(error_with(
                "placement_unavailable",
                format!("{rejections:?}"),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, env, net::IpAddr};

    use arc_application::WorkerRegistry;
    use arc_domain::{
        estimate_pipeline_requirements, Architecture, DestinationMediaProfile, PlacementRejection,
        Platform, SourceMediaProfile, Worker, WorkerCapabilities, WorkerCapacity, WorkerState,
    };
    use chrono::Utc;
    use uuid::Uuid;

    use crate::{
        capacity_reservations::{PgCapacityReservations, ReservationDecision},
        postgres::{connect_postgres, migrate},
        worker_registry::PgWorkerRegistry,
    };

    use super::*;

    #[tokio::test]
    #[ignore = "requires an isolated PostgreSQL database; set DATABASE_URL and run ignored tests"]
    async fn scheduler_accounts_for_reservations_and_worker_state() {
        let url = env::var("DATABASE_URL").expect("DATABASE_URL is set");
        let pool = connect_postgres(&url).await.unwrap();
        migrate(&pool).await.unwrap();
        assert!(PgWorkerScheduler::new(pool.clone(), Duration::seconds(-1)).is_err());
        let scheduler = PgWorkerScheduler::new(pool.clone(), Duration::seconds(60)).unwrap();
        let registry = PgWorkerRegistry::new(pool.clone());
        let reservations = PgCapacityReservations::new(pool.clone());
        let address = IpAddr::from([127, 0, 0, 1]);
        let requirements = estimate_pipeline_requirements(
            &SourceMediaProfile::RtpL16 {
                bind_address: address,
                port: 5000,
                payload_type: 96,
            },
            &DestinationMediaProfile::RtpL16 {
                address,
                port: 5002,
                payload_type: 96,
            },
        )
        .unwrap()
        .requirements;
        assert!(matches!(
            scheduler.inspect_placement(&requirements).await.unwrap(),
            PlacementDecision::Unavailable { rejections } if rejections.is_empty()
        ));
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
        registry.register(worker.clone()).await.unwrap();
        worker.state = WorkerState::Accepting;
        worker.capacity.assigned_capacity_units = 1;
        registry.heartbeat(worker.clone()).await.unwrap();
        assert_eq!(
            scheduler
                .find_placement(&requirements)
                .await
                .unwrap()
                .worker_id,
            id
        );
        let incompatible = PipelineRequirements {
            source_adapter: Some("srt_mpeg_ts".to_owned()),
            ..requirements.clone()
        };
        assert!(matches!(
            scheduler.inspect_placement(&incompatible).await.unwrap(),
            PlacementDecision::Unavailable { rejections }
                if matches!(&rejections[0].reason, PlacementRejection::MissingCapabilities(missing) if missing == &["sourceAdapter:srt_mpeg_ts"])
        ));
        assert_eq!(
            scheduler
                .find_placement(&PipelineRequirements::default())
                .await
                .unwrap_err()
                .code,
            "invalid_placement"
        );
        let reserved = match reservations
            .reserve(Uuid::new_v4(), &requirements, Duration::seconds(60))
            .await
            .unwrap()
        {
            ReservationDecision::Reserved(value) => value,
            other => panic!("expected reservation, got {other:?}"),
        };
        assert!(matches!(
            scheduler.inspect_placement(&requirements).await.unwrap(),
            PlacementDecision::Unavailable { rejections }
                if matches!(rejections[0].reason, PlacementRejection::InsufficientCapacity { required: 2, available: 1 })
        ));
        assert_eq!(
            scheduler
                .find_placement(&requirements)
                .await
                .unwrap_err()
                .code,
            "placement_unavailable"
        );
        reservations.release(reserved.id).await.unwrap();
        assert_eq!(
            scheduler
                .find_placement(&requirements)
                .await
                .unwrap()
                .worker_id,
            id
        );
        sqlx::query("UPDATE workers SET last_heartbeat = clock_timestamp() - interval '1 hour' WHERE id = $1")
                    .bind(id).execute(&pool).await.unwrap();
        assert!(matches!(
            scheduler.inspect_placement(&requirements).await.unwrap(),
            PlacementDecision::Unavailable { rejections }
                if matches!(rejections[0].reason, PlacementRejection::HeartbeatExpired)
        ));
        registry.heartbeat(worker).await.unwrap();
        registry.begin_drain(id).await.unwrap();
        assert!(matches!(
            scheduler.inspect_placement(&requirements).await.unwrap(),
            PlacementDecision::Unavailable { rejections }
                if matches!(rejections[0].reason, PlacementRejection::WorkerNotAccepting)
        ));
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
