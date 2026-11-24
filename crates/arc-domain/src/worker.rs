use std::collections::BTreeSet;

use chrono::{DateTime, Duration, Utc};
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
    pub stable_name: String,
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
    pub ingress_bits_per_second: Option<u64>,
    pub egress_bits_per_second: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "code", content = "details", rename_all = "snake_case")]
pub enum PlacementRejection {
    WorkerNotAccepting,
    HeartbeatExpired,
    HeartbeatInFuture,
    MissingCapabilities(Vec<String>),
    InsufficientCapacity { required: u32, available: u32 },
    InsufficientIngressBandwidth { required: u64, available: u64 },
    InsufficientEgressBandwidth { required: u64, available: u64 },
    CpuPressure,
    MemoryPressure,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerRejection {
    pub worker_id: Uuid,
    pub reason: PlacementRejection,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlacementDecision {
    Selected { worker_id: Uuid },
    Unavailable { rejections: Vec<WorkerRejection> },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlacementError {
    NegativeHeartbeatAge,
}

pub fn select_worker(
    workers: &[Worker],
    requirements: &PipelineRequirements,
    now: DateTime<Utc>,
    max_heartbeat_age: Duration,
) -> Result<PlacementDecision, PlacementError> {
    if max_heartbeat_age < Duration::zero() {
        return Err(PlacementError::NegativeHeartbeatAge);
    }

    let mut selected = None;
    let mut rejections = Vec::new();
    for worker in workers {
        let rejection = if worker.last_heartbeat > now {
            Some(PlacementRejection::HeartbeatInFuture)
        } else if now.signed_duration_since(worker.last_heartbeat) > max_heartbeat_age {
            Some(PlacementRejection::HeartbeatExpired)
        } else {
            worker.placement_rejection(requirements)
        };
        if let Some(reason) = rejection {
            rejections.push(WorkerRejection {
                worker_id: worker.id,
                reason,
            });
        } else if selected.is_none_or(|id| worker.id < id) {
            selected = Some(worker.id);
        }
    }

    if let Some(worker_id) = selected {
        Ok(PlacementDecision::Selected { worker_id })
    } else {
        rejections.sort_by_key(|rejection| rejection.worker_id);
        Ok(PlacementDecision::Unavailable { rejections })
    }
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
        if let Some(required) = requirements.ingress_bits_per_second {
            if self.capacity.ingress_bits_per_second.is_some_and(|available| required > available) {
                return Some(PlacementRejection::InsufficientIngressBandwidth {
                    required,
                    available: self.capacity.ingress_bits_per_second.unwrap_or(0),
                });
            }
        }
        if let Some(required) = requirements.egress_bits_per_second {
            if self.capacity.egress_bits_per_second.is_some_and(|available| required > available) {
                return Some(PlacementRejection::InsufficientEgressBandwidth {
                    required,
                    available: self.capacity.egress_bits_per_second.unwrap_or(0),
                });
            }
        }
        if self
            .capacity
            .cpu_utilization_ratio
            .is_some_and(|ratio| ratio >= 0.90)
        {
            return Some(PlacementRejection::CpuPressure);
        }
        if self
            .capacity
            .memory_utilization_ratio
            .is_some_and(|ratio| ratio >= 0.90)
        {
            return Some(PlacementRejection::MemoryPressure);
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
            stable_name: "test-worker".to_owned(),
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

    #[test]
    fn bandwidth_and_resource_pressure_rejections_are_structural() {
        let requirements = PipelineRequirements {
            source_adapter: Some("rtp".to_owned()),
            destination_adapter: Some("rtp".to_owned()),
            capacity_units: 1,
            ingress_bits_per_second: Some(2_000_000),
            egress_bits_per_second: Some(3_000_000),
            ..PipelineRequirements::default()
        };
        let mut limited = worker(WorkerState::Accepting, 4);
        limited.capacity.ingress_bits_per_second = Some(1_000_000);
        limited.capacity.egress_bits_per_second = Some(4_000_000);
        assert_eq!(
            limited.placement_rejection(&requirements),
            Some(PlacementRejection::InsufficientIngressBandwidth {
                required: 2_000_000,
                available: 1_000_000,
            })
        );

        let mut pressured = worker(WorkerState::Accepting, 4);
        pressured.capacity.egress_bits_per_second = Some(4_000_000);
        pressured.capacity.cpu_utilization_ratio = Some(0.91);
        assert_eq!(
            pressured.placement_rejection(&requirements),
            Some(PlacementRejection::CpuPressure)
        );
    }

    #[test]
    fn selection_is_deterministic_and_skips_stale_or_ineligible_workers() {
        let now = Utc::now();
        let requirements = PipelineRequirements {
            source_adapter: Some("rtp".to_owned()),
            capacity_units: 2,
            ..PipelineRequirements::default()
        };
        let mut stale = worker(WorkerState::Ready, 4);
        stale.id = Uuid::from_u128(1);
        stale.last_heartbeat = now - Duration::seconds(11);
        let mut draining = worker(WorkerState::Draining, 4);
        draining.id = Uuid::from_u128(2);
        draining.last_heartbeat = now;
        let mut eligible = worker(WorkerState::Accepting, 4);
        eligible.id = Uuid::from_u128(4);
        eligible.last_heartbeat = now - Duration::seconds(10);
        let mut first = eligible.clone();
        first.id = Uuid::from_u128(3);

        assert_eq!(
            select_worker(
                &[eligible, draining, stale, first],
                &requirements,
                now,
                Duration::seconds(10),
            ),
            Ok(PlacementDecision::Selected {
                worker_id: Uuid::from_u128(3),
            })
        );
    }

    #[test]
    fn selection_reports_missing_capability_capacity_and_heartbeat_failures() {
        let now = Utc::now();
        let requirements = PipelineRequirements {
            source_adapter: Some("rtp".to_owned()),
            capacity_units: 2,
            ..PipelineRequirements::default()
        };
        let mut future = worker(WorkerState::Ready, 4);
        future.id = Uuid::from_u128(4);
        future.last_heartbeat = now + Duration::seconds(1);
        let mut stale = worker(WorkerState::Ready, 4);
        stale.id = Uuid::from_u128(3);
        stale.last_heartbeat = now - Duration::seconds(11);
        let mut low_capacity = worker(WorkerState::Ready, 1);
        low_capacity.id = Uuid::from_u128(2);
        low_capacity.last_heartbeat = now;
        let mut incompatible = worker(WorkerState::Ready, 4);
        incompatible.id = Uuid::from_u128(1);
        incompatible.last_heartbeat = now;
        incompatible.capabilities.source_adapters.clear();

        assert_eq!(
            select_worker(
                &[future, stale, low_capacity, incompatible],
                &requirements,
                now,
                Duration::seconds(10),
            ),
            Ok(PlacementDecision::Unavailable {
                rejections: vec![
                    WorkerRejection {
                        worker_id: Uuid::from_u128(1),
                        reason: PlacementRejection::MissingCapabilities(vec![
                            "sourceAdapter:rtp".to_owned(),
                        ]),
                    },
                    WorkerRejection {
                        worker_id: Uuid::from_u128(2),
                        reason: PlacementRejection::InsufficientCapacity {
                            required: 2,
                            available: 1,
                        },
                    },
                    WorkerRejection {
                        worker_id: Uuid::from_u128(3),
                        reason: PlacementRejection::HeartbeatExpired,
                    },
                    WorkerRejection {
                        worker_id: Uuid::from_u128(4),
                        reason: PlacementRejection::HeartbeatInFuture,
                    },
                ],
            })
        );
    }

    #[test]
    fn selection_handles_empty_inventory_and_rejects_negative_heartbeat_age() {
        let requirements = PipelineRequirements::default();
        let now = Utc::now();
        assert_eq!(
            select_worker(&[], &requirements, now, Duration::zero()),
            Ok(PlacementDecision::Unavailable {
                rejections: Vec::new(),
            })
        );
        assert_eq!(
            select_worker(&[], &requirements, now, Duration::seconds(-1)),
            Err(PlacementError::NegativeHeartbeatAge)
        );
    }
}
