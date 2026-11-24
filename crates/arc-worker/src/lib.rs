use arc_domain::{Worker, WorkerCapabilities, WorkerState};
use chrono::Utc;
use uuid::Uuid;

pub use arc_application::{
    BackupProvider, CapacityManager, DestinationAdapter, EventPublisher, MediaPipeline,
    NetworkInterfaceProvider, SecretProvider, SignalExtractor, SignalGenerator, SourceAdapter,
    StateRepository, WorkerRegistry, WorkerScheduler,
};
pub use arc_shared::{WorkerCommandEnvelope, WorkerEventEnvelope, WORKER_PROTOCOL_VERSION};

pub fn local_worker(
    id: Uuid,
    stable_name: impl Into<String>,
    version: impl Into<String>,
    capabilities: WorkerCapabilities,
) -> Worker {
    Worker {
        id,
        stable_name: stable_name.into(),
        version: version.into(),
        state: WorkerState::Registering,
        capabilities,
        capacity: arc_domain::WorkerCapacity {
            total_capacity_units: 0,
            assigned_capacity_units: 0,
            cpu_utilization_ratio: None,
            memory_utilization_ratio: None,
            ingress_bits_per_second: None,
            egress_bits_per_second: None,
        },
        last_heartbeat: Utc::now(),
    }
}
