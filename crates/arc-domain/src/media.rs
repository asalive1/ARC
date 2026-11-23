use std::{error::Error, fmt, net::IpAddr};

use serde::{Deserialize, Serialize};

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
    RtpMp2t {
        bind_address: IpAddr,
        port: u16,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaPath {
    AlsaL16ToRtpL16,
    RtpL24ToRtpL24,
    RtpMp2tToSrtMpegTs,
}

impl SourceMediaProfile {
    pub const fn adapter_id(&self) -> &'static str {
        match self {
            Self::AlsaL16 { .. } => "alsa_l16",
            Self::RtpMp2t { .. } => "rtp_mp2t",
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
            Self::RtpL24ToRtpL24 => "rtp_l24_to_rtp_l24",
            Self::RtpMp2tToSrtMpegTs => "rtp_mp2t_to_srt_mpeg_ts",
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
            Self::InvalidHost => "destination host must not be empty or contain whitespace",
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
        (SourceMediaProfile::RtpL24 { .. }, DestinationMediaProfile::RtpL24 { .. }) => {
            Ok(MediaPath::RtpL24ToRtpL24)
        }
        (SourceMediaProfile::RtpMp2t { .. }, DestinationMediaProfile::SrtMpegTs { .. }) => {
            Ok(MediaPath::RtpMp2tToSrtMpegTs)
        }
        _ => Err(MediaProfileError::UnsupportedConversion),
    }
}

pub fn validate_source_profile(source: &SourceMediaProfile) -> Result<(), MediaProfileError> {
    match source {
        SourceMediaProfile::AlsaL16 { device } if device.trim().is_empty() => {
            Err(MediaProfileError::InvalidAlsaDevice)
        }
        SourceMediaProfile::AlsaL16 { .. } => Ok(()),
        SourceMediaProfile::RtpMp2t { port: 0, .. } => Err(MediaProfileError::InvalidPort),
        SourceMediaProfile::RtpMp2t { .. } => Ok(()),
        SourceMediaProfile::RtpL24 { port: 0, .. } => Err(MediaProfileError::InvalidPort),
        SourceMediaProfile::RtpL24 { payload_type, .. } if !(96..=127).contains(payload_type) => {
            Err(MediaProfileError::InvalidRtpPayloadType)
        }
        SourceMediaProfile::RtpL24 { .. } => Ok(()),
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
}
