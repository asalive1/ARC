use arc_application::PortResult;
use arc_domain::{
    estimate_pipeline_requirements, DestinationMediaProfile, PipelineEstimate, SourceMediaProfile,
    WorkerRejection,
};
use chrono::Duration;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    assignments::{PgAssignments, PlannedAssignment},
    capacity_reservations::{reserve_in_transaction, ReservationDecision},
    worker_registry::error_with,
};

#[derive(Clone, Debug, PartialEq)]
pub struct PlannedPipeline {
    pub estimate: PipelineEstimate,
    pub assignment: PlannedAssignment,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PipelinePlanningDecision {
    Planned(PlannedPipeline),
    Unavailable { rejections: Vec<WorkerRejection> },
}

pub struct PgPipelinePlanner {
    pool: PgPool,
}

impl PgPipelinePlanner {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn plan(
        &self,
        pipeline_id: Uuid,
        source: &SourceMediaProfile,
        destination: &DestinationMediaProfile,
        max_heartbeat_age: Duration,
    ) -> PortResult<PipelinePlanningDecision> {
        let estimate = estimate_pipeline_requirements(source, destination)
            .map_err(|error| error_with("invalid_media_profile", error))?;
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| error_with("database_error", error))?;
        let decision = match reserve_in_transaction(
            &mut transaction,
            pipeline_id,
            &estimate.requirements,
            max_heartbeat_age,
        )
        .await?
        {
            ReservationDecision::Reserved(reservation) => {
                let assignment = PgAssignments::plan_in_transaction(
                    &mut transaction,
                    reservation.id,
                    &estimate.requirements,
                    max_heartbeat_age,
                )
                .await?;
                PipelinePlanningDecision::Planned(PlannedPipeline {
                    estimate,
                    assignment,
                })
            }
            ReservationDecision::Unavailable { rejections } => {
                PipelinePlanningDecision::Unavailable { rejections }
            }
        };
        transaction
            .commit()
            .await
            .map_err(|error| error_with("database_error", error))?;
        Ok(decision)
    }
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
        let source = SourceMediaProfile::RtpL16 {
            bind_address: address,
            port: 5000,
            payload_type: 96,
        };
        let destination = DestinationMediaProfile::RtpL16 {
            address,
            port: 5002,
            payload_type: 96,
        };
        let age = Duration::seconds(60);
        let invalid = SourceMediaProfile::RtpL16 {
            bind_address: address,
            port: 0,
            payload_type: 96,
        };
        assert_eq!(
            planner
                .plan(Uuid::new_v4(), &invalid, &destination, age)
                .await
                .unwrap_err()
                .code,
            "invalid_media_profile"
        );
        assert_eq!(
            planner
                .plan(Uuid::nil(), &source, &destination, age)
                .await
                .unwrap_err()
                .code,
            "invalid_reservation"
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
                .plan(conflicted_pipeline, &source, &destination, age)
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
            planner.plan(first_pipeline, &source, &destination, age),
            planner.plan(second_pipeline, &source, &destination, age),
        );
        let planned = match (first.unwrap(), second.unwrap()) {
            (
                PipelinePlanningDecision::Planned(plan),
                PipelinePlanningDecision::Unavailable { rejections },
            )
            | (
                PipelinePlanningDecision::Unavailable { rejections },
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
        let reconnected = PgAssignments::new(connect_postgres(&url).await.unwrap());
        assert_eq!(
            reconnected.list_planned().await.unwrap(),
            vec![planned.assignment.clone()]
        );
        assert_eq!(
            reservations.list_active().await.unwrap()[0].id,
            planned.assignment.reservation_id
        );
        assert_eq!(
            planner
                .plan(planned.assignment.pipeline_id, &source, &destination, age)
                .await
                .unwrap_err()
                .code,
            "reservation_conflict"
        );
        assignments
            .cancel_unstarted(planned.assignment.id, planned.assignment.generation)
            .await
            .unwrap();
        let replacement = match planner
            .plan(planned.assignment.pipeline_id, &source, &destination, age)
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
            planner.plan(duplicate_pipeline, &source, &destination, age),
            planner.plan(duplicate_pipeline, &source, &destination, age),
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
        registry.begin_drain(worker_id).await.unwrap();
        assert!(matches!(
            planner.plan(Uuid::new_v4(), &source, &destination, age).await.unwrap(),
            PipelinePlanningDecision::Unavailable { rejections }
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
    }
}
