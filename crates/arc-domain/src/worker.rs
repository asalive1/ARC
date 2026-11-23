use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Linux,
    Windows,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Architecture {
    X86_64,
    Aarch64,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerCapabilities {
    pub source_adapters: BTreeSet<String>,
    pub destination_adapters: BTreeSet<String>,
    pub codecs: BTreeSet<String>,
    pub encoders: BTreeSet<String>,
    pub payload_formats: BTreeSet<String>,
    pub signal_parsers: BTreeSet<String>,
    pub signal_generators: BTreeSet<String>,
    pub audio_backends: BTreeSet<String>,
    pub hardware_acceleration: BTreeSet<String>,
    pub platform: Option<Platform>,
    pub architecture: Option<Architecture>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerCapacity {
    pub total_capacity_units: u32,
    pub assigned_capacity_units: u32,
    pub cpu_utilization_ratio: Option<f32>,
    pub memory_utilization_ratio: Option<f32>,
    pub ingress_bits_per_second: Option<u64>,
    pub egress_bits_per_second: Option<u64>,
}

impl WorkerCapacity {
    pub fn available_capacity_units(&self) -> u32 {
        self.total_capacity_units
            .saturating_sub(self.assigned_capacity_units)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerState {
    Registering,
    Warming,
    Ready,
    Accepting,
    Draining,
    Drained,
    Degraded,
    Failed,
    Offline,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Worker {
    pub id: Uuid,
    pub version: String,
    pub state: WorkerState,
    pub capabilities: WorkerCapabilities,
    pub capacity: WorkerCapacity,
    pub last_heartbeat: DateTime<Utc>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineRequirements {
    pub source_adapter: Option<String>,
    pub destination_adapter: Option<String>,
    pub codecs: BTreeSet<String>,
    pub encoders: BTreeSet<String>,
    pub payload_formats: BTreeSet<String>,
    pub signal_parsers: BTreeSet<String>,
    pub signal_generators: BTreeSet<String>,
    pub audio_backends: BTreeSet<String>,
    pub hardware_acceleration: BTreeSet<String>,
    pub platform: Option<Platform>,
    pub architecture: Option<Architecture>,
    pub capacity_units: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlacementRejection {
    WorkerNotAccepting,
    MissingCapabilities(Vec<String>),
    InsufficientCapacity { required: u32, available: u32 },
}

impl Worker {
    pub fn placement_rejection(
        &self,
        requirements: &PipelineRequirements,
    ) -> Option<PlacementRejection> {
        if !matches!(self.state, WorkerState::Ready | WorkerState::Accepting) {
            return Some(PlacementRejection::WorkerNotAccepting);
        }

        let mut missing = Vec::new();
        if let Some(adapter) = &requirements.source_adapter {
            if !self.capabilities.source_adapters.contains(adapter) {
                missing.push(format!("sourceAdapter:{adapter}"));
            }
        }
        if let Some(adapter) = &requirements.destination_adapter {
            if !self.capabilities.destination_adapters.contains(adapter) {
                missing.push(format!("destinationAdapter:{adapter}"));
            }
        }
        missing.extend(
            requirements
                .codecs
                .difference(&self.capabilities.codecs)
                .map(|value| format!("codec:{value}")),
        );
        missing.extend(
            requirements
                .encoders
                .difference(&self.capabilities.encoders)
                .map(|value| format!("encoder:{value}")),
        );
        missing.extend(
            requirements
                .payload_formats
                .difference(&self.capabilities.payload_formats)
                .map(|value| format!("payloadFormat:{value}")),
        );
        missing.extend(
            requirements
                .signal_parsers
                .difference(&self.capabilities.signal_parsers)
                .map(|value| format!("signalParser:{value}")),
        );
        missing.extend(
            requirements
                .signal_generators
                .difference(&self.capabilities.signal_generators)
                .map(|value| format!("signalGenerator:{value}")),
        );
        missing.extend(
            requirements
                .audio_backends
                .difference(&self.capabilities.audio_backends)
                .map(|value| format!("audioBackend:{value}")),
        );
        missing.extend(
            requirements
                .hardware_acceleration
                .difference(&self.capabilities.hardware_acceleration)
                .map(|value| format!("hardwareAcceleration:{value}")),
        );
        if requirements
            .platform
            .is_some_and(|required| self.capabilities.platform != Some(required))
        {
            missing.push("platform".to_owned());
        }
        if requirements
            .architecture
            .is_some_and(|required| self.capabilities.architecture != Some(required))
        {
            missing.push("architecture".to_owned());
        }
        if !missing.is_empty() {
            return Some(PlacementRejection::MissingCapabilities(missing));
        }

        let available = self.capacity.available_capacity_units();
        if available < requirements.capacity_units {
            return Some(PlacementRejection::InsufficientCapacity {
                required: requirements.capacity_units,
                available,
            });
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worker(state: WorkerState, capacity: u32) -> Worker {
        Worker {
            id: Uuid::nil(),
            version: "0.1.0".to_owned(),
            state,
            capabilities: WorkerCapabilities {
                source_adapters: BTreeSet::from(["rtp".to_owned()]),
                destination_adapters: BTreeSet::from(["rtp".to_owned()]),
                codecs: BTreeSet::from(["aac-lc".to_owned()]),
                platform: Some(Platform::Linux),
                architecture: Some(Architecture::X86_64),
                ..WorkerCapabilities::default()
            },
            capacity: WorkerCapacity {
                total_capacity_units: capacity,
                assigned_capacity_units: 0,
                cpu_utilization_ratio: None,
                memory_utilization_ratio: None,
                ingress_bits_per_second: None,
                egress_bits_per_second: None,
            },
            last_heartbeat: Utc::now(),
        }
    }

    #[test]
    fn compatible_worker_is_eligible_for_placement() {
        let requirements = PipelineRequirements {
            source_adapter: Some("rtp".to_owned()),
            destination_adapter: Some("rtp".to_owned()),
            codecs: BTreeSet::from(["aac-lc".to_owned()]),
            capacity_units: 2,
            ..PipelineRequirements::default()
        };

        assert_eq!(
            worker(WorkerState::Accepting, 4).placement_rejection(&requirements),
            None
        );
    }

    #[test]
    fn draining_worker_is_not_eligible_for_new_placement() {
        let requirements = PipelineRequirements {
            capacity_units: 1,
            ..PipelineRequirements::default()
        };

        assert_eq!(
            worker(WorkerState::Draining, 4).placement_rejection(&requirements),
            Some(PlacementRejection::WorkerNotAccepting)
        );
    }

    #[test]
    fn capacity_shortfall_is_reported_structurally() {
        let requirements = PipelineRequirements {
            capacity_units: 3,
            ..PipelineRequirements::default()
        };

        assert_eq!(
            worker(WorkerState::Ready, 2).placement_rejection(&requirements),
            Some(PlacementRejection::InsufficientCapacity {
                required: 3,
                available: 2,
            })
        );
    }
}
