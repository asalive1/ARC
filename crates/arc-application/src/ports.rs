use std::{collections::BTreeSet, net::IpAddr};

use arc_domain::{PipelineRequirements, Worker, WorkerCapabilities};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type PortResult<T> = Result<T, PortError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortError {
    pub code: &'static str,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkerCommandIdentity {
    pub command_id: Uuid,
    pub worker_id: Uuid,
    pub pipeline_id: Uuid,
    pub assignment_id: Uuid,
    pub generation: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandFenceDecision {
    Process,
    Duplicate,
    Stale { current_generation: i64 },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterConfiguration {
    pub resource_id: Uuid,
    pub revision: i64,
    pub adapter_id: String,
    pub configuration: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineSpecification {
    pub pipeline_id: Uuid,
    pub assignment_generation: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_identity: Option<PipelineSourceIdentity>,
    pub requirements: PipelineRequirements,
    pub configuration: serde_json::Value,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineSourceIdentity {
    pub source_id: Uuid,
    pub source_revision: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineObservedState {
    Preparing,
    Running,
    Degraded,
    Failed,
    Stopping,
    Stopped,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineObservation {
    pub pipeline_id: Uuid,
    pub assignment_generation: i64,
    pub state: PipelineObservedState,
    pub health_reasons: Vec<String>,
    pub observed_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalEvent {
    pub event_id: Uuid,
    pub source_id: Uuid,
    pub event_type: String,
    pub logical_state: Option<bool>,
    pub media_timestamp: Option<u64>,
    pub received_at: DateTime<Utc>,
    pub attributes: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkInterface {
    pub stable_id: String,
    pub name: String,
    pub addresses: Vec<IpAddr>,
    pub mtu: Option<u32>,
    pub link_up: bool,
    pub administrative_up: bool,
    pub multicast_capable: bool,
    pub link_speed_bits_per_second: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineAssignment {
    pub assignment_id: Uuid,
    pub pipeline_id: Uuid,
    pub worker_id: Uuid,
    pub generation: i64,
    pub desired_state: String,
    pub observed_state: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerPlacement {
    pub worker_id: Uuid,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapacityRequestState {
    Requested,
    WaitingForCapacity,
    Rejected,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapacityRequest {
    pub request_id: Uuid,
    pub state: CapacityRequestState,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEvent {
    pub event_id: Uuid,
    pub sequence: Option<i64>,
    pub event_type: String,
    pub resource_id: Option<Uuid>,
    pub occurred_at: DateTime<Utc>,
    pub payload: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupRequest {
    pub backup_id: Uuid,
    pub requested_at: DateTime<Utc>,
    pub include_audit: bool,
}

#[async_trait]
pub trait SourceAdapter: Send + Sync {
    fn adapter_id(&self) -> &'static str;
    async fn validate(&self, configuration: &AdapterConfiguration) -> PortResult<()>;
    async fn start(&self, configuration: &AdapterConfiguration) -> PortResult<()>;
    async fn stop(&self, resource_id: Uuid) -> PortResult<()>;
}

#[async_trait]
pub trait DestinationAdapter: Send + Sync {
    fn adapter_id(&self) -> &'static str;
    async fn validate(&self, configuration: &AdapterConfiguration) -> PortResult<()>;
    async fn start(&self, configuration: &AdapterConfiguration) -> PortResult<()>;
    async fn stop(&self, resource_id: Uuid) -> PortResult<()>;
}

#[async_trait]
pub trait MediaPipeline: Send + Sync {
    async fn prepare(&self, specification: &PipelineSpecification) -> PortResult<()>;
    async fn start(&self, pipeline_id: Uuid, generation: i64) -> PortResult<()>;
    async fn stop(&self, pipeline_id: Uuid, generation: i64) -> PortResult<()>;
    async fn observe(&self, pipeline_id: Uuid) -> PortResult<PipelineObservation>;
}

pub trait SignalExtractor: Send + Sync {
    fn extract(&self, source_id: Uuid, payload: &[u8]) -> PortResult<Vec<SignalEvent>>;
}

#[async_trait]
pub trait SignalGenerator: Send + Sync {
    async fn generate(&self, event: &SignalEvent) -> PortResult<()>;
}

#[async_trait]
pub trait NetworkInterfaceProvider: Send + Sync {
    async fn list_interfaces(&self) -> PortResult<Vec<NetworkInterface>>;
}

#[async_trait]
pub trait StateRepository: Send + Sync {
    async fn save_assignment(&self, assignment: &PipelineAssignment) -> PortResult<()>;
    async fn load_assignments(&self) -> PortResult<Vec<PipelineAssignment>>;
}

#[async_trait]
pub trait SecretProvider: Send + Sync {
    async fn resolve_reference(&self, reference: &str) -> PortResult<String>;
}

#[async_trait]
pub trait WorkerAuthenticator: Send + Sync {
    async fn authenticate(&self, worker_id: Uuid, bearer_token: &str) -> PortResult<()>;
}

#[async_trait]
pub trait AssignmentGenerationStore: Send + Sync {
    /// Atomically reject stale generations and identify completed commands.
    async fn begin_command(
        &self,
        command: WorkerCommandIdentity,
    ) -> PortResult<CommandFenceDecision>;

    /// Mark successful work complete; an incomplete command remains retryable.
    async fn complete_command(&self, command_id: Uuid) -> PortResult<()>;
}

#[async_trait]
pub trait EventPublisher: Send + Sync {
    async fn publish(&self, event: DomainEvent) -> PortResult<()>;
}

#[async_trait]
pub trait WorkerRegistry: Send + Sync {
    async fn register(&self, worker: Worker) -> PortResult<()>;
    async fn heartbeat(&self, worker: Worker) -> PortResult<()>;
    async fn list_workers(&self) -> PortResult<Vec<Worker>>;
}

#[async_trait]
pub trait WorkerScheduler: Send + Sync {
    async fn find_placement(
        &self,
        requirements: &PipelineRequirements,
    ) -> PortResult<WorkerPlacement>;
}

#[async_trait]
pub trait CapacityManager: Send + Sync {
    async fn request_capacity(
        &self,
        requirements: &PipelineRequirements,
        allow_waiting: bool,
    ) -> PortResult<CapacityRequest>;
}

#[async_trait]
pub trait BackupProvider: Send + Sync {
    async fn create_backup(&self, request: &BackupRequest) -> PortResult<String>;
}

pub fn capability_set(values: impl IntoIterator<Item = String>) -> BTreeSet<String> {
    values.into_iter().collect()
}

pub fn capabilities_for_adapter(adapter_id: &str) -> WorkerCapabilities {
    WorkerCapabilities {
        source_adapters: BTreeSet::from([adapter_id.to_owned()]),
        ..WorkerCapabilities::default()
    }
}
