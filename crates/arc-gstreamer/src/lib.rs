use std::{collections::HashMap, sync::Mutex, time::Duration};

use arc_application::{
    MediaPipeline, PipelineObservation, PipelineObservedState, PipelineSpecification, PortError,
    PortResult,
};
use arc_domain::{
    estimate_pipeline_requirements, DestinationMediaProfile, MediaPath, SourceMediaProfile,
    SrtMode, WorkerCapabilities,
};
use async_trait::async_trait;
use chrono::Utc;
use gstreamer as gst;
use gstreamer::prelude::*;
use serde::Deserialize;
use uuid::Uuid;

const STATE_CHANGE_TIMEOUT: Duration = Duration::from_secs(5);

pub struct GstreamerMediaPipeline {
    pipelines: Mutex<HashMap<Uuid, ManagedPipeline>>,
}

struct ManagedPipeline {
    generation: i64,
    description: String,
    pipeline: Option<gst::Pipeline>,
    observed_state: PipelineObservedState,
    health_reasons: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PipelineRouteConfiguration {
    source: SourceMediaProfile,
    destination: DestinationMediaProfile,
}

impl GstreamerMediaPipeline {
    pub fn new() -> PortResult<Self> {
        gst::init().map_err(|error| {
            pipeline_error(
                "gstreamer_initialization_failed",
                format!("GStreamer initialization failed: {error}"),
            )
        })?;
        Ok(Self {
            pipelines: Mutex::new(HashMap::new()),
        })
    }

    pub fn worker_capabilities(&self, enable_alsa: bool) -> WorkerCapabilities {
        let mut capabilities = WorkerCapabilities::default();
        let available = |elements: &[&str]| {
            elements
                .iter()
                .all(|element| gst::ElementFactory::find(element).is_some())
        };

        if enable_alsa && available(&["alsasrc"]) {
            capabilities.source_adapters.insert("alsa_l16".to_owned());
            capabilities.audio_backends.insert("alsa".to_owned());
        }
        if available(&["udpsrc", "rtpjitterbuffer", "rtpL16depay"]) {
            capabilities.source_adapters.insert("rtp_l16".to_owned());
        }
        if available(&["udpsrc", "rtpjitterbuffer", "rtpL24depay"]) {
            capabilities.source_adapters.insert("rtp_l24".to_owned());
        }
        if available(&["udpsrc", "rtpjitterbuffer", "rtpmp2tdepay"]) {
            capabilities.source_adapters.insert("rtp_mp2t".to_owned());
        }
        if available(&["srtsrc"]) {
            capabilities
                .source_adapters
                .insert("srt_mpeg_ts".to_owned());
        }
        if available(&["rtpL16pay", "udpsink"]) {
            capabilities
                .destination_adapters
                .insert("rtp_l16".to_owned());
        }
        if available(&["rtpL24pay", "udpsink"]) {
            capabilities
                .destination_adapters
                .insert("rtp_l24".to_owned());
        }
        if available(&["srtsink"]) {
            capabilities
                .destination_adapters
                .insert("srt_mpeg_ts".to_owned());
        }
        capabilities
    }

    fn lock_pipelines(
        &self,
    ) -> PortResult<std::sync::MutexGuard<'_, HashMap<Uuid, ManagedPipeline>>> {
        self.pipelines.lock().map_err(|_| {
            pipeline_error(
                "media_pipeline_state_unavailable",
                "GStreamer pipeline state lock is poisoned",
            )
        })
    }
}

#[async_trait]
impl MediaPipeline for GstreamerMediaPipeline {
    async fn prepare(&self, specification: &PipelineSpecification) -> PortResult<()> {
        let route: PipelineRouteConfiguration =
            serde_json::from_value(specification.configuration.clone()).map_err(|error| {
                pipeline_error(
                    "invalid_pipeline_configuration",
                    format!("pipeline route configuration is invalid: {error}"),
                )
            })?;
        let estimate = estimate_pipeline_requirements(&route.source, &route.destination)
            .map_err(|error| pipeline_error("invalid_media_profile", error.to_string()))?;
        if estimate.requirements != specification.requirements {
            return Err(pipeline_error(
                "pipeline_requirements_mismatch",
                "pipeline requirements do not match the pinned route profiles",
            ));
        }
        let description = pipeline_description(&route.source, &route.destination)?;
        let element = gst::parse::launch(&description).map_err(|error| {
            pipeline_error(
                "gstreamer_pipeline_prepare_failed",
                format!("GStreamer could not prepare the media graph: {error}"),
            )
        })?;
        let pipeline = element.downcast::<gst::Pipeline>().map_err(|_| {
            pipeline_error(
                "gstreamer_pipeline_prepare_failed",
                "GStreamer media graph did not produce a pipeline",
            )
        })?;

        let mut pipelines = self.lock_pipelines()?;
        if let Some(current) = pipelines.get_mut(&specification.pipeline_id) {
            if specification.assignment_generation < current.generation {
                return Err(pipeline_error(
                    "stale_pipeline_generation",
                    "pipeline preparation belongs to an older assignment generation",
                ));
            }
            if specification.assignment_generation == current.generation {
                if !current.description.is_empty() && current.description != description {
                    return Err(pipeline_error(
                        "pipeline_generation_conflict",
                        "one assignment generation cannot prepare different media graphs",
                    ));
                }
                if current.pipeline.is_some() {
                    return Ok(());
                }
            }
            if specification.assignment_generation > current.generation {
                if let Some(current_pipeline) = &current.pipeline {
                    transition_pipeline(
                        current_pipeline,
                        gst::State::Null,
                        "gstreamer_pipeline_stop_failed",
                    )?;
                }
            }
        }

        pipelines.insert(
            specification.pipeline_id,
            ManagedPipeline {
                generation: specification.assignment_generation,
                description,
                pipeline: Some(pipeline),
                observed_state: PipelineObservedState::Preparing,
                health_reasons: Vec::new(),
            },
        );
        Ok(())
    }

    async fn start(&self, pipeline_id: Uuid, generation: i64) -> PortResult<()> {
        let mut pipelines = self.lock_pipelines()?;
        let managed = pipelines.get_mut(&pipeline_id).ok_or_else(|| {
            pipeline_error(
                "pipeline_not_prepared",
                "pipeline must be prepared before it can start",
            )
        })?;
        if generation != managed.generation {
            return Err(pipeline_error(
                "stale_pipeline_generation",
                "pipeline start does not match the prepared generation",
            ));
        }
        let pipeline = managed.pipeline.clone().ok_or_else(|| {
            pipeline_error(
                "pipeline_not_prepared",
                "stopped pipeline must be prepared again before it can start",
            )
        })?;
        if let Err(error) = transition_pipeline(
            &pipeline,
            gst::State::Playing,
            "gstreamer_pipeline_start_failed",
        ) {
            if let Err(cleanup_error) = pipeline.set_state(gst::State::Null) {
                tracing::warn!(%cleanup_error, "failed to stop GStreamer pipeline after start failure");
            }
            managed.observed_state = PipelineObservedState::Failed;
            managed.health_reasons = vec![error.message.clone()];
            return Err(error);
        }
        managed.observed_state = PipelineObservedState::Running;
        managed.health_reasons.clear();
        Ok(())
    }

    async fn stop(&self, pipeline_id: Uuid, generation: i64) -> PortResult<()> {
        let mut pipelines = self.lock_pipelines()?;
        let Some(managed) = pipelines.get_mut(&pipeline_id) else {
            pipelines.insert(
                pipeline_id,
                ManagedPipeline {
                    generation,
                    description: String::new(),
                    pipeline: None,
                    observed_state: PipelineObservedState::Stopped,
                    health_reasons: Vec::new(),
                },
            );
            return Ok(());
        };
        if generation < managed.generation {
            return Err(pipeline_error(
                "stale_pipeline_generation",
                "pipeline stop belongs to an older assignment generation",
            ));
        }
        if let Some(pipeline) = managed.pipeline.clone() {
            transition_pipeline(
                &pipeline,
                gst::State::Null,
                "gstreamer_pipeline_stop_failed",
            )?;
        }
        managed.generation = generation;
        managed.pipeline = None;
        managed.observed_state = PipelineObservedState::Stopped;
        managed.health_reasons.clear();
        Ok(())
    }

    async fn observe(&self, pipeline_id: Uuid) -> PortResult<PipelineObservation> {
        let mut pipelines = self.lock_pipelines()?;
        let managed = pipelines.get_mut(&pipeline_id).ok_or_else(|| {
            pipeline_error(
                "pipeline_not_found",
                "pipeline has not been prepared on this worker",
            )
        })?;
        if let Some(pipeline) = managed.pipeline.clone() {
            if let Some(reason) = bus_error(&pipeline) {
                if let Err(error) = pipeline.set_state(gst::State::Null) {
                    tracing::warn!(%error, %pipeline_id, "failed to stop errored GStreamer pipeline");
                }
                managed.pipeline = None;
                managed.observed_state = PipelineObservedState::Failed;
                managed.health_reasons = vec![reason];
            } else {
                let (state_change, state, _) = pipeline.state(Some(gst::ClockTime::ZERO));
                state_change.map_err(|error| {
                    pipeline_error(
                        "gstreamer_pipeline_observation_failed",
                        format!("GStreamer pipeline state could not be read: {error}"),
                    )
                })?;
                match state {
                    gst::State::Playing => {
                        managed.observed_state = PipelineObservedState::Running;
                        managed.health_reasons.clear();
                    }
                    gst::State::Null
                        if matches!(managed.observed_state, PipelineObservedState::Failed) => {}
                    gst::State::Null | gst::State::Ready => {
                        managed.observed_state = PipelineObservedState::Preparing;
                    }
                    gst::State::Paused => {
                        managed.observed_state = PipelineObservedState::Degraded;
                        managed.health_reasons =
                            vec!["GStreamer pipeline is unexpectedly paused".to_owned()];
                    }
                    _ => {
                        managed.observed_state = PipelineObservedState::Degraded;
                        managed.health_reasons =
                            vec!["GStreamer pipeline is in an unknown state".to_owned()];
                    }
                }
            }
        }
        Ok(PipelineObservation {
            pipeline_id,
            assignment_generation: managed.generation,
            state: managed.observed_state.clone(),
            health_reasons: managed.health_reasons.clone(),
            observed_at: Utc::now(),
        })
    }
}

impl Drop for GstreamerMediaPipeline {
    fn drop(&mut self) {
        if let Ok(pipelines) = self.pipelines.get_mut() {
            for managed in pipelines.values_mut() {
                if let Some(pipeline) = &managed.pipeline {
                    if let Err(error) = pipeline.set_state(gst::State::Null) {
                        tracing::warn!(%error, "failed to stop GStreamer pipeline on shutdown");
                    }
                }
            }
        }
    }
}

fn transition_pipeline(
    pipeline: &gst::Pipeline,
    target: gst::State,
    error_code: &'static str,
) -> PortResult<()> {
    pipeline.set_state(target).map_err(|error| {
        pipeline_error(
            error_code,
            format!("GStreamer pipeline could not enter {target:?}: {error}"),
        )
    })?;
    let (state_change, state, _) = pipeline.state(Some(gst::ClockTime::from_seconds(
        STATE_CHANGE_TIMEOUT.as_secs(),
    )));
    state_change.map_err(|error| {
        pipeline_error(
            error_code,
            format!("GStreamer pipeline failed while entering {target:?}: {error}"),
        )
    })?;
    if state != target {
        let timeout_code = if target == gst::State::Playing {
            "gstreamer_pipeline_start_timeout"
        } else {
            "gstreamer_pipeline_stop_timeout"
        };
        return Err(pipeline_error(
            timeout_code,
            format!("GStreamer pipeline reached {state:?} instead of {target:?}"),
        ));
    }
    if target == gst::State::Playing {
        if let Some(reason) = bus_error(pipeline) {
            return Err(pipeline_error(error_code, reason));
        }
    }
    Ok(())
}

pub fn pipeline_description(
    source: &SourceMediaProfile,
    destination: &DestinationMediaProfile,
) -> PortResult<String> {
    let path = arc_domain::plan_media_path(source, destination)
        .map_err(|error| pipeline_error("invalid_media_profile", error.to_string()))?;
    let description = match (path, source, destination) {
        (
            MediaPath::AlsaL16ToRtpL16,
            SourceMediaProfile::AlsaL16 { device },
            DestinationMediaProfile::RtpL16 {
                address,
                port,
                payload_type,
            },
        ) => format!(
            "alsasrc device={} ! \
             audio/x-raw,format=S16LE,rate=48000,channels=2 ! \
             audioconvert ! audio/x-raw,format=S16BE,rate=48000,channels=2 ! \
             rtpL16pay pt={payload_type} ! \
             udpsink host={} port={port} sync=false async=false",
            gst_string(device)?,
            gst_string(&address.to_string())?,
        ),
        (
            MediaPath::RtpL16ToRtpL16,
            SourceMediaProfile::RtpL16 {
                bind_address,
                port: source_port,
                payload_type: source_payload_type,
            },
            DestinationMediaProfile::RtpL16 {
                address,
                port: destination_port,
                payload_type: destination_payload_type,
            },
        ) => rtp_audio_description(
            bind_address.to_string(),
            *source_port,
            "L16",
            *source_payload_type,
            "rtpL16depay",
            "rtpL16pay",
            *destination_payload_type,
            address.to_string(),
            *destination_port,
        )?,
        (
            MediaPath::RtpL24ToRtpL24,
            SourceMediaProfile::RtpL24 {
                bind_address,
                port: source_port,
                payload_type: source_payload_type,
            },
            DestinationMediaProfile::RtpL24 {
                address,
                port: destination_port,
                payload_type: destination_payload_type,
            },
        ) => rtp_audio_description(
            bind_address.to_string(),
            *source_port,
            "L24",
            *source_payload_type,
            "rtpL24depay",
            "rtpL24pay",
            *destination_payload_type,
            address.to_string(),
            *destination_port,
        )?,
        (
            MediaPath::RtpMp2tToSrtMpegTs,
            SourceMediaProfile::RtpMp2t { bind_address, port },
            DestinationMediaProfile::SrtMpegTs {
                host,
                port: destination_port,
                mode,
            },
        ) => format!(
            "udpsrc address={} port={port} caps={} ! \
             rtpjitterbuffer ! rtpmp2tdepay ! srtsink uri={} async=false",
            gst_string(&bind_address.to_string())?,
            gst_string(
                "application/x-rtp,media=(string)video,clock-rate=(int)90000,\
                 encoding-name=(string)MP2T,payload=(int)33"
            )?,
            gst_string(&srt_uri(host, *destination_port, *mode)?)?,
        ),
        (
            MediaPath::SrtMpegTsToSrtMpegTs,
            SourceMediaProfile::SrtMpegTs {
                host: source_host,
                port: source_port,
                mode: source_mode,
            },
            DestinationMediaProfile::SrtMpegTs {
                host: destination_host,
                port: destination_port,
                mode: destination_mode,
            },
        ) => format!(
            "srtsrc uri={} ! srtsink uri={} async=false",
            gst_string(&srt_uri(source_host, *source_port, *source_mode)?)?,
            gst_string(&srt_uri(
                destination_host,
                *destination_port,
                *destination_mode
            )?)?,
        ),
        _ => {
            return Err(pipeline_error(
                "unsupported_media_path",
                "media path does not have a GStreamer implementation",
            ))
        }
    };
    Ok(description)
}

fn rtp_audio_description(
    bind_address: String,
    source_port: u16,
    encoding: &str,
    source_payload_type: u8,
    depayloader: &str,
    payloader: &str,
    destination_payload_type: u8,
    destination_address: String,
    destination_port: u16,
) -> PortResult<String> {
    let caps = format!(
        "application/x-rtp,media=(string)audio,clock-rate=(int)48000,\
         encoding-name=(string){encoding},channels=(int)2,payload=(int){source_payload_type}"
    );
    Ok(format!(
        "udpsrc address={} port={source_port} caps={} ! \
         rtpjitterbuffer ! {depayloader} ! {payloader} pt={destination_payload_type} ! \
         udpsink host={} port={destination_port} sync=false async=false",
        gst_string(&bind_address)?,
        gst_string(&caps)?,
        gst_string(&destination_address)?,
    ))
}

fn srt_uri(host: &str, port: u16, mode: SrtMode) -> PortResult<String> {
    let mut uri = url::Url::parse("srt://localhost").expect("static SRT URI is valid");
    let host = if host.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    uri.set_host(Some(&host))
        .map_err(|_| pipeline_error("invalid_media_profile", "SRT host is not a valid URI host"))?;
    uri.set_port(Some(port))
        .map_err(|_| pipeline_error("invalid_media_profile", "SRT port is not a valid URI port"))?;
    uri.query_pairs_mut().append_pair(
        "mode",
        match mode {
            SrtMode::Caller => "caller",
            SrtMode::Listener => "listener",
        },
    );
    Ok(uri.to_string())
}

fn gst_string(value: &str) -> PortResult<String> {
    if value.chars().any(char::is_control) {
        return Err(pipeline_error(
            "invalid_media_profile",
            "media configuration contains a control character",
        ));
    }
    Ok(format!(
        "\"{}\"",
        value.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

fn bus_error(pipeline: &gst::Pipeline) -> Option<String> {
    let bus = pipeline.bus()?;
    let message = bus.pop_filtered(&[gst::MessageType::Error])?;
    match message.view() {
        gst::MessageView::Error(error) => {
            let source = message
                .src()
                .map(|source| source.name().to_string())
                .unwrap_or_else(|| "unknown".to_owned());
            Some(format!(
                "GStreamer element {source} failed: {}",
                error.error()
            ))
        }
        _ => Some("GStreamer pipeline reported an unexpected bus message".to_owned()),
    }
}

fn pipeline_error(code: &'static str, message: impl Into<String>) -> PortError {
    PortError {
        code,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, UdpSocket};

    use super::*;
    use gstreamer_app::AppSink;

    #[test]
    fn builds_and_parses_typed_phase_one_pipeline_descriptions() {
        gst::init().expect("GStreamer initializes");
        let rtp_source = IpAddr::from([0, 0, 0, 0]);
        let output = IpAddr::from([127, 0, 0, 1]);
        let cases = [
            (
                SourceMediaProfile::AlsaL16 {
                    device: "hw:1,0".to_owned(),
                },
                DestinationMediaProfile::RtpL16 {
                    address: output,
                    port: 5004,
                    payload_type: 96,
                },
                "alsasrc device=\"hw:1,0\"",
            ),
            (
                SourceMediaProfile::RtpL16 {
                    bind_address: rtp_source,
                    port: 5004,
                    payload_type: 96,
                },
                DestinationMediaProfile::RtpL16 {
                    address: output,
                    port: 5006,
                    payload_type: 97,
                },
                "rtpL16depay ! rtpL16pay pt=97",
            ),
            (
                SourceMediaProfile::RtpL24 {
                    bind_address: rtp_source,
                    port: 5004,
                    payload_type: 96,
                },
                DestinationMediaProfile::RtpL24 {
                    address: output,
                    port: 5006,
                    payload_type: 97,
                },
                "rtpL24depay ! rtpL24pay pt=97",
            ),
            (
                SourceMediaProfile::RtpMp2t {
                    bind_address: rtp_source,
                    port: 5000,
                },
                DestinationMediaProfile::SrtMpegTs {
                    host: "127.0.0.1".to_owned(),
                    port: 6000,
                    mode: SrtMode::Caller,
                },
                "rtpmp2tdepay ! srtsink",
            ),
            (
                SourceMediaProfile::SrtMpegTs {
                    host: "0.0.0.0".to_owned(),
                    port: 6000,
                    mode: SrtMode::Listener,
                },
                DestinationMediaProfile::SrtMpegTs {
                    host: "127.0.0.1".to_owned(),
                    port: 6002,
                    mode: SrtMode::Caller,
                },
                "srtsrc uri=",
            ),
        ];

        for (source, destination, expected) in cases {
            let description = pipeline_description(&source, &destination).unwrap();
            assert!(description.contains(expected));
            gst::parse::launch(&description)
                .unwrap_or_else(|error| panic!("invalid GStreamer graph {description}: {error}"));
        }
    }

    #[test]
    fn escapes_gstreamer_property_values_and_rejects_control_characters() {
        assert_eq!(gst_string("audio\\device").unwrap(), "\"audio\\\\device\"");
        assert_eq!(gst_string("audio\"device").unwrap(), "\"audio\\\"device\"");
        assert_eq!(
            gst_string("audio\ndevice").unwrap_err().code,
            "invalid_media_profile"
        );
    }

    #[test]
    fn srt_uris_encode_ipv6_and_transfer_mode() {
        let uri = srt_uri("::1", 6000, SrtMode::Listener).unwrap();
        assert_eq!(uri, "srt://[::1]:6000?mode=listener");
    }

    #[test]
    fn capabilities_are_limited_to_installed_gstreamer_elements() {
        let _ = gst::init();
        let capabilities = GstreamerMediaPipeline {
            pipelines: Mutex::new(HashMap::new()),
        }
        .worker_capabilities(false);
        assert!(!capabilities.source_adapters.contains("alsa_l16"));
        assert_eq!(
            capabilities.source_adapters.contains("rtp_l16"),
            ["udpsrc", "rtpjitterbuffer", "rtpL16depay"]
                .into_iter()
                .all(|element| gst::ElementFactory::find(element).is_some())
        );
        assert_eq!(
            capabilities.destination_adapters.contains("srt_mpeg_ts"),
            gst::ElementFactory::find("srtsink").is_some()
        );
    }

    #[tokio::test]
    async fn forwards_rtp_mpeg_ts_to_an_srt_listener() {
        let media = GstreamerMediaPipeline::new().expect("GStreamer initializes");
        let loopback = Ipv4Addr::LOCALHOST;
        let rtp_reservation = UdpSocket::bind((loopback, 0)).expect("RTP source port reserves");
        let rtp_port = rtp_reservation
            .local_addr()
            .expect("RTP source port is available")
            .port();
        let srt_reservation = UdpSocket::bind((loopback, 0)).expect("SRT listener port reserves");
        let srt_port = srt_reservation
            .local_addr()
            .expect("SRT listener port is available")
            .port();
        drop((rtp_reservation, srt_reservation));

        let receiver_source = gst::ElementFactory::make("srtsrc")
            .property(
                "uri",
                srt_uri(&loopback.to_string(), srt_port, SrtMode::Listener)
                    .expect("SRT listener URI builds"),
            )
            .build()
            .expect("SRT listener element builds");
        let receiver_sink = AppSink::builder().sync(false).build();
        let receiver_sink_element = receiver_sink.upcast_ref::<gst::Element>();
        let receiver_pipeline = gst::Pipeline::new();
        receiver_pipeline
            .add_many([&receiver_source, receiver_sink_element])
            .expect("SRT receiver elements are added");
        receiver_source
            .link(receiver_sink_element)
            .expect("SRT receiver elements link");
        receiver_pipeline
            .set_state(gst::State::Playing)
            .expect("SRT receiver starts");

        let source = SourceMediaProfile::RtpMp2t {
            bind_address: loopback.into(),
            port: rtp_port,
        };
        let destination = DestinationMediaProfile::SrtMpegTs {
            host: loopback.to_string(),
            port: srt_port,
            mode: SrtMode::Caller,
        };
        let estimate = estimate_pipeline_requirements(&source, &destination)
            .expect("RTP MPEG-TS to SRT requirements are supported");
        let pipeline_id = Uuid::new_v4();
        let specification = PipelineSpecification {
            pipeline_id,
            assignment_generation: 1,
            requirements: estimate.requirements,
            configuration: serde_json::json!({
                "source": source,
                "destination": destination,
            }),
        };
        media
            .prepare(&specification)
            .await
            .expect("RTP-to-SRT pipeline prepares");
        media
            .start(pipeline_id, 1)
            .await
            .expect("RTP-to-SRT pipeline starts");

        let mut transport_stream = Vec::with_capacity(188 * 7);
        for packet_index in 0..7u8 {
            let mut packet = vec![packet_index; 188];
            packet[0] = 0x47;
            packet[1] = 0x01;
            packet[2] = 0x00;
            packet[3] = 0x10 | packet_index;
            transport_stream.extend_from_slice(&packet);
        }
        let sender = UdpSocket::bind((loopback, 0)).expect("RTP sender socket binds");
        for sequence in 1..=4u16 {
            let mut datagram = Vec::with_capacity(12 + transport_stream.len());
            datagram.extend_from_slice(&[0x80, 33]);
            datagram.extend_from_slice(&sequence.to_be_bytes());
            datagram.extend_from_slice(&(u32::from(sequence) * 3_600).to_be_bytes());
            datagram.extend_from_slice(&0x4152_4331_u32.to_be_bytes());
            datagram.extend_from_slice(&transport_stream);
            sender
                .send_to(&datagram, (loopback, rtp_port))
                .expect("RTP fixture packet sends");
        }

        let mut received = Vec::new();
        for _ in 0..5 {
            let Some(sample) = receiver_sink.try_pull_sample(gst::ClockTime::from_seconds(1))
            else {
                continue;
            };
            let buffer = sample
                .buffer()
                .expect("SRT receiver sample has a buffer")
                .map_readable()
                .expect("SRT receiver buffer is readable");
            received.extend_from_slice(buffer.as_slice());
            if received
                .windows(transport_stream.len())
                .any(|candidate| candidate == transport_stream.as_slice())
            {
                break;
            }
        }
        assert!(
            received
                .windows(transport_stream.len())
                .any(|candidate| candidate == transport_stream.as_slice()),
            "SRT output did not contain the original MPEG-TS packets; received {} bytes",
            received.len()
        );

        media
            .stop(pipeline_id, 1)
            .await
            .expect("RTP-to-SRT pipeline stops");
        receiver_pipeline
            .set_state(gst::State::Null)
            .expect("SRT receiver stops");
    }
}
