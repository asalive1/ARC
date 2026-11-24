use std::{error::Error, fmt, net::IpAddr};

use serde::{Deserialize, Serialize};

use crate::worker::{PipelineRequirements, Platform};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum SourceMediaProfile {
    AlsaL16 {
        device: String,
    },
    SrtMpegTs {
        host: String,
        port: u16,
        mode: SrtMode,
    },
    RtpMp2t {
        bind_address: IpAddr,
        port: u16,
    },
    RtpL16 {
        bind_address: IpAddr,
        port: u16,
        payload_type: u8,
    },
    RtpL24 {
        bind_address: IpAddr,
        port: u16,
        payload_type: u8,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SrtMode {
    Caller,
    Listener,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum DestinationMediaProfile {
    RtpL16 {
        address: IpAddr,
        port: u16,
        payload_type: u8,
    },
    RtpL24 {
        address: IpAddr,
        port: u16,
        payload_type: u8,
    },
    SrtMpegTs {
        host: String,
        port: u16,
        mode: SrtMode,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaPath {
    AlsaL16ToRtpL16,
    RtpL16ToRtpL16,
    RtpL24ToRtpL24,
    RtpMp2tToSrtMpegTs,
    SrtMpegTsToSrtMpegTs,
}

impl SourceMediaProfile {
    pub const fn adapter_id(&self) -> &'static str {
        match self {
            Self::AlsaL16 { .. } => "alsa_l16",
            Self::SrtMpegTs { .. } => "srt_mpeg_ts",
            Self::RtpMp2t { .. } => "rtp_mp2t",
            Self::RtpL16 { .. } => "rtp_l16",
            Self::RtpL24 { .. } => "rtp_l24",
        }
    }
}

impl DestinationMediaProfile {
    pub const fn adapter_id(&self) -> &'static str {
        match self {
            Self::RtpL16 { .. } => "rtp_l16",
            Self::RtpL24 { .. } => "rtp_l24",
            Self::SrtMpegTs { .. } => "srt_mpeg_ts",
        }
    }
}

impl MediaPath {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AlsaL16ToRtpL16 => "alsa_l16_to_rtp_l16",
            Self::RtpL16ToRtpL16 => "rtp_l16_to_rtp_l16",
            Self::RtpL24ToRtpL24 => "rtp_l24_to_rtp_l24",
            Self::RtpMp2tToSrtMpegTs => "rtp_mp2t_to_srt_mpeg_ts",
            Self::SrtMpegTsToSrtMpegTs => "srt_mpeg_ts_to_srt_mpeg_ts",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaProfileError {
    InvalidAlsaDevice,
    InvalidHost,
    InvalidPort,
    InvalidRtpPayloadType,
    UnsupportedConversion,
}

impl fmt::Display for MediaProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidAlsaDevice => "ALSA device must not be empty",
            Self::InvalidHost => "SRT host must not be empty or contain whitespace",
            Self::InvalidPort => "media endpoint port must be nonzero",
            Self::InvalidRtpPayloadType => {
                "RTP audio payload type must be in the dynamic range 96..=127"
            }
            Self::UnsupportedConversion => "media conversion is not supported by Phase 1",
        };
        formatter.write_str(message)
    }
}

impl Error for MediaProfileError {}

pub fn plan_media_path(
    source: &SourceMediaProfile,
    destination: &DestinationMediaProfile,
) -> Result<MediaPath, MediaProfileError> {
    validate_source_profile(source)?;
    validate_destination_profile(destination)?;

    match (source, destination) {
        (SourceMediaProfile::AlsaL16 { .. }, DestinationMediaProfile::RtpL16 { .. }) => {
            Ok(MediaPath::AlsaL16ToRtpL16)
        }
        (SourceMediaProfile::RtpL16 { .. }, DestinationMediaProfile::RtpL16 { .. }) => {
            Ok(MediaPath::RtpL16ToRtpL16)
        }
        (SourceMediaProfile::RtpL24 { .. }, DestinationMediaProfile::RtpL24 { .. }) => {
            Ok(MediaPath::RtpL24ToRtpL24)
        }
        (SourceMediaProfile::RtpMp2t { .. }, DestinationMediaProfile::SrtMpegTs { .. }) => {
            Ok(MediaPath::RtpMp2tToSrtMpegTs)
        }
        (SourceMediaProfile::SrtMpegTs { .. }, DestinationMediaProfile::SrtMpegTs { .. }) => {
            Ok(MediaPath::SrtMpegTsToSrtMpegTs)
        }
        _ => Err(MediaProfileError::UnsupportedConversion),
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineEstimate {
    pub media_path: MediaPath,
    pub requirements: PipelineRequirements,
    pub ingress_payload_bits_per_second: Option<u64>,
    pub egress_payload_bits_per_second: Option<u64>,
}

pub fn estimate_pipeline_requirements(
    source: &SourceMediaProfile,
    destination: &DestinationMediaProfile,
) -> Result<PipelineEstimate, MediaProfileError> {
    let media_path = plan_media_path(source, destination)?;
    let (platform, audio_backends, ingress, egress) = match media_path {
        MediaPath::AlsaL16ToRtpL16 => (
            Some(Platform::Linux),
            ["alsa".to_owned()].into_iter().collect(),
            None,
            Some(48_000 * 2 * 16),
        ),
        MediaPath::RtpL16ToRtpL16
        | MediaPath::RtpL24ToRtpL24
        | MediaPath::RtpMp2tToSrtMpegTs
        | MediaPath::SrtMpegTsToSrtMpegTs => (None, Default::default(), None, None),
    };
    Ok(PipelineEstimate {
        media_path,
        requirements: PipelineRequirements {
            source_adapter: Some(source.adapter_id().to_owned()),
            destination_adapter: Some(destination.adapter_id().to_owned()),
            platform,
            audio_backends,
            capacity_units: 2,
            ingress_bits_per_second: ingress,
            egress_bits_per_second: egress,
            ..PipelineRequirements::default()
        },
        ingress_payload_bits_per_second: ingress,
        egress_payload_bits_per_second: egress,
    })
}

pub fn validate_source_profile(source: &SourceMediaProfile) -> Result<(), MediaProfileError> {
    match source {
        SourceMediaProfile::AlsaL16 { device } if device.trim().is_empty() => {
            Err(MediaProfileError::InvalidAlsaDevice)
        }
        SourceMediaProfile::AlsaL16 { .. } => Ok(()),
        SourceMediaProfile::SrtMpegTs { port: 0, .. }
        | SourceMediaProfile::RtpMp2t { port: 0, .. }
        | SourceMediaProfile::RtpL16 { port: 0, .. }
        | SourceMediaProfile::RtpL24 { port: 0, .. } => Err(MediaProfileError::InvalidPort),
        SourceMediaProfile::SrtMpegTs { host, .. }
            if host.trim().is_empty() || host.chars().any(char::is_whitespace) =>
        {
            Err(MediaProfileError::InvalidHost)
        }
        SourceMediaProfile::SrtMpegTs { .. } | SourceMediaProfile::RtpMp2t { .. } => Ok(()),
        SourceMediaProfile::RtpL16 { payload_type, .. }
        | SourceMediaProfile::RtpL24 { payload_type, .. }
            if !(96..=127).contains(payload_type) =>
        {
            Err(MediaProfileError::InvalidRtpPayloadType)
        }
        SourceMediaProfile::RtpL16 { .. } | SourceMediaProfile::RtpL24 { .. } => Ok(()),
    }
}

pub fn validate_destination_profile(
    destination: &DestinationMediaProfile,
) -> Result<(), MediaProfileError> {
    match destination {
        DestinationMediaProfile::RtpL16 { port: 0, .. }
        | DestinationMediaProfile::RtpL24 { port: 0, .. }
        | DestinationMediaProfile::SrtMpegTs { port: 0, .. } => Err(MediaProfileError::InvalidPort),
        DestinationMediaProfile::RtpL16 { payload_type, .. }
        | DestinationMediaProfile::RtpL24 { payload_type, .. }
            if !(96..=127).contains(payload_type) =>
        {
            Err(MediaProfileError::InvalidRtpPayloadType)
        }
        DestinationMediaProfile::SrtMpegTs { host, .. }
            if host.trim().is_empty() || host.chars().any(char::is_whitespace) =>
        {
            Err(MediaProfileError::InvalidHost)
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_mpeg_ts_rtp_to_srt_as_pass_through() {
        let source = SourceMediaProfile::RtpMp2t {
            bind_address: IpAddr::from([0, 0, 0, 0]),
            port: 5000,
        };
        let destination = DestinationMediaProfile::SrtMpegTs {
            host: "127.0.0.1".to_owned(),
            port: 6000,
            mode: SrtMode::Caller,
        };

        assert_eq!(
            plan_media_path(&source, &destination),
            Ok(MediaPath::RtpMp2tToSrtMpegTs)
        );
    }

    #[test]
    fn plans_alsa_l16_to_rtp_l16_without_transcoding() {
        let source = SourceMediaProfile::AlsaL16 {
            device: "hw:1,0".to_owned(),
        };
        let destination = DestinationMediaProfile::RtpL16 {
            address: IpAddr::from([127, 0, 0, 1]),
            port: 5004,
            payload_type: 96,
        };

        assert_eq!(
            plan_media_path(&source, &destination),
            Ok(MediaPath::AlsaL16ToRtpL16)
        );
    }

    #[test]
    fn plans_matching_rtp_l16_profiles() {
        let source = SourceMediaProfile::RtpL16 {
            bind_address: IpAddr::from([0, 0, 0, 0]),
            port: 5004,
            payload_type: 96,
        };
        let destination = DestinationMediaProfile::RtpL16 {
            address: IpAddr::from([127, 0, 0, 1]),
            port: 5006,
            payload_type: 97,
        };

        assert_eq!(
            plan_media_path(&source, &destination),
            Ok(MediaPath::RtpL16ToRtpL16)
        );
    }

    #[test]
    fn plans_matching_rtp_l24_profiles() {
        let source = SourceMediaProfile::RtpL24 {
            bind_address: IpAddr::from([0, 0, 0, 0]),
            port: 5004,
            payload_type: 96,
        };
        let destination = DestinationMediaProfile::RtpL24 {
            address: IpAddr::from([127, 0, 0, 1]),
            port: 5006,
            payload_type: 97,
        };

        assert_eq!(
            plan_media_path(&source, &destination),
            Ok(MediaPath::RtpL24ToRtpL24)
        );
    }

    #[test]
    fn plans_srt_mpeg_ts_input_to_output_as_pass_through() {
        let source = SourceMediaProfile::SrtMpegTs {
            host: "0.0.0.0".to_owned(),
            port: 6000,
            mode: SrtMode::Listener,
        };
        let destination = DestinationMediaProfile::SrtMpegTs {
            host: "127.0.0.1".to_owned(),
            port: 6002,
            mode: SrtMode::Caller,
        };

        assert_eq!(
            plan_media_path(&source, &destination),
            Ok(MediaPath::SrtMpegTsToSrtMpegTs)
        );
    }

    #[test]
    fn rejects_rtp_l24_to_l16_conversion_without_a_defined_path() {
        let source = SourceMediaProfile::RtpL24 {
            bind_address: IpAddr::from([0, 0, 0, 0]),
            port: 5004,
            payload_type: 96,
        };
        let destination = DestinationMediaProfile::RtpL16 {
            address: IpAddr::from([127, 0, 0, 1]),
            port: 5006,
            payload_type: 97,
        };

        assert_eq!(
            plan_media_path(&source, &destination),
            Err(MediaProfileError::UnsupportedConversion)
        );
    }

    #[test]
    fn rejects_conversion_without_a_phase_one_profile() {
        let source = SourceMediaProfile::AlsaL16 {
            device: "hw:1,0".to_owned(),
        };
        let destination = DestinationMediaProfile::SrtMpegTs {
            host: "127.0.0.1".to_owned(),
            port: 6000,
            mode: SrtMode::Caller,
        };

        assert_eq!(
            plan_media_path(&source, &destination),
            Err(MediaProfileError::UnsupportedConversion)
        );
    }

    #[test]
    fn rejects_invalid_endpoint_values_before_planning() {
        let source = SourceMediaProfile::RtpMp2t {
            bind_address: IpAddr::from([0, 0, 0, 0]),
            port: 0,
        };
        let destination = DestinationMediaProfile::SrtMpegTs {
            host: "127.0.0.1".to_owned(),
            port: 6000,
            mode: SrtMode::Caller,
        };

        assert_eq!(
            plan_media_path(&source, &destination),
            Err(MediaProfileError::InvalidPort)
        );
    }

    #[test]
    fn rejects_non_dynamic_rtp_l24_payload_types() {
        let source = SourceMediaProfile::RtpL24 {
            bind_address: IpAddr::from([0, 0, 0, 0]),
            port: 5004,
            payload_type: 33,
        };
        let destination = DestinationMediaProfile::RtpL24 {
            address: IpAddr::from([127, 0, 0, 1]),
            port: 5006,
            payload_type: 128,
        };

        assert_eq!(
            validate_source_profile(&source),
            Err(MediaProfileError::InvalidRtpPayloadType)
        );
        assert_eq!(
            validate_destination_profile(&destination),
            Err(MediaProfileError::InvalidRtpPayloadType)
        );
    }

    #[test]
    fn validates_rtp_l16_payload_type_and_srt_input_host() {
        let rtp_source = SourceMediaProfile::RtpL16 {
            bind_address: IpAddr::from([0, 0, 0, 0]),
            port: 5004,
            payload_type: 33,
        };
        let srt_source = SourceMediaProfile::SrtMpegTs {
            host: "invalid host".to_owned(),
            port: 6000,
            mode: SrtMode::Listener,
        };

        assert_eq!(
            validate_source_profile(&rtp_source),
            Err(MediaProfileError::InvalidRtpPayloadType)
        );
        assert_eq!(
            validate_source_profile(&srt_source),
            Err(MediaProfileError::InvalidHost)
        );
    }

    #[test]
    fn estimates_each_supported_path_without_assuming_unknown_bitrates() {
        let address = IpAddr::from([127, 0, 0, 1]);
        let rtp_l16 = DestinationMediaProfile::RtpL16 {
            address,
            port: 5000,
            payload_type: 96,
        };
        let rtp_l24 = DestinationMediaProfile::RtpL24 {
            address,
            port: 5000,
            payload_type: 96,
        };
        let srt = DestinationMediaProfile::SrtMpegTs {
            host: "127.0.0.1".to_owned(),
            port: 5000,
            mode: SrtMode::Caller,
        };
        let cases = [
            (
                SourceMediaProfile::AlsaL16 {
                    device: "hw:0".to_owned(),
                },
                rtp_l16.clone(),
                MediaPath::AlsaL16ToRtpL16,
                "alsa_l16",
                "rtp_l16",
            ),
            (
                SourceMediaProfile::RtpL16 {
                    bind_address: address,
                    port: 5000,
                    payload_type: 96,
                },
                rtp_l16,
                MediaPath::RtpL16ToRtpL16,
                "rtp_l16",
                "rtp_l16",
            ),
            (
                SourceMediaProfile::RtpL24 {
                    bind_address: address,
                    port: 5000,
                    payload_type: 96,
                },
                rtp_l24,
                MediaPath::RtpL24ToRtpL24,
                "rtp_l24",
                "rtp_l24",
            ),
            (
                SourceMediaProfile::RtpMp2t {
                    bind_address: address,
                    port: 5000,
                },
                srt.clone(),
                MediaPath::RtpMp2tToSrtMpegTs,
                "rtp_mp2t",
                "srt_mpeg_ts",
            ),
            (
                SourceMediaProfile::SrtMpegTs {
                    host: "127.0.0.1".to_owned(),
                    port: 5000,
                    mode: SrtMode::Listener,
                },
                srt,
                MediaPath::SrtMpegTsToSrtMpegTs,
                "srt_mpeg_ts",
                "srt_mpeg_ts",
            ),
        ];
        for (source, destination, path, source_adapter, destination_adapter) in cases {
            let estimate = estimate_pipeline_requirements(&source, &destination).unwrap();
            assert_eq!(
                estimate,
                estimate_pipeline_requirements(&source, &destination).unwrap()
            );
            assert_eq!(estimate.media_path, path);
            assert_eq!(
                estimate.requirements.source_adapter.as_deref(),
                Some(source_adapter)
            );
            assert_eq!(
                estimate.requirements.destination_adapter.as_deref(),
                Some(destination_adapter)
            );
            assert_eq!(estimate.requirements.capacity_units, 2);
            if path == MediaPath::AlsaL16ToRtpL16 {
                assert_eq!(estimate.requirements.platform, Some(Platform::Linux));
                assert_eq!(
                    estimate.requirements.audio_backends,
                    ["alsa".to_owned()].into_iter().collect()
                );
                assert_eq!(estimate.ingress_payload_bits_per_second, None);
                assert_eq!(estimate.egress_payload_bits_per_second, Some(1_536_000));
            } else {
                assert_eq!(estimate.requirements.platform, None);
                assert!(estimate.requirements.audio_backends.is_empty());
                assert_eq!(estimate.ingress_payload_bits_per_second, None);
                assert_eq!(estimate.egress_payload_bits_per_second, None);
            }
        }
    }

    #[test]
    fn estimate_rejects_invalid_or_unsupported_paths() {
        let source = SourceMediaProfile::AlsaL16 {
            device: String::new(),
        };
        let destination = DestinationMediaProfile::SrtMpegTs {
            host: "127.0.0.1".to_owned(),
            port: 5000,
            mode: SrtMode::Caller,
        };
        assert_eq!(
            estimate_pipeline_requirements(&source, &destination),
            Err(MediaProfileError::InvalidAlsaDevice)
        );
        let source = SourceMediaProfile::AlsaL16 {
            device: "hw:0".to_owned(),
        };
        assert_eq!(
            estimate_pipeline_requirements(&source, &destination),
            Err(MediaProfileError::UnsupportedConversion)
        );
    }

    #[test]
    fn estimate_enforces_worker_capabilities_and_capacity() {
        use crate::worker::{
            Architecture, PlacementRejection, Worker, WorkerCapabilities, WorkerCapacity,
            WorkerState,
        };
        use chrono::Utc;
        use uuid::Uuid;

        let source = SourceMediaProfile::AlsaL16 {
            device: "hw:0".to_owned(),
        };
        let destination = DestinationMediaProfile::RtpL16 {
            address: IpAddr::from([127, 0, 0, 1]),
            port: 5004,
            payload_type: 96,
        };
        let requirements = estimate_pipeline_requirements(&source, &destination)
            .unwrap()
            .requirements;
        let mut worker = Worker {
            id: Uuid::new_v4(),
            stable_name: "test-worker".to_owned(),
            version: "0.1.0".to_owned(),
            state: WorkerState::Ready,
            capabilities: WorkerCapabilities {
                source_adapters: ["alsa_l16".to_owned()].into_iter().collect(),
                destination_adapters: ["rtp_l16".to_owned()].into_iter().collect(),
                audio_backends: ["alsa".to_owned()].into_iter().collect(),
                platform: Some(Platform::Linux),
                architecture: Some(Architecture::X86_64),
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
        assert_eq!(worker.placement_rejection(&requirements), None);
        worker.capabilities.audio_backends.clear();
        assert!(matches!(
            worker.placement_rejection(&requirements),
            Some(PlacementRejection::MissingCapabilities(_))
        ));
        worker.capabilities.audio_backends.insert("alsa".to_owned());
        worker.capabilities.platform = Some(Platform::Windows);
        assert!(matches!(
            worker.placement_rejection(&requirements),
            Some(PlacementRejection::MissingCapabilities(_))
        ));
        worker.capabilities.platform = Some(Platform::Linux);
        worker.capacity.assigned_capacity_units = 1;
        assert_eq!(
            worker.placement_rejection(&requirements),
            Some(PlacementRejection::InsufficientCapacity {
                required: 2,
                available: 1
            })
        );
    }
}
