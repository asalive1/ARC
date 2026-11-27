use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use arc_application::{
    AudioPreview, MediaPipeline, PipelineObservation, PipelineObservedState,
    PipelineSourceIdentity, PipelineSpecification, PortError, PortResult,
};
use arc_domain::{
    estimate_pipeline_requirements, DestinationMediaProfile, MediaPath, SourceMediaProfile,
    SrtMode, WorkerCapabilities,
};
use async_trait::async_trait;
use chrono::Utc;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::{AppSink, AppSinkCallbacks};
use serde::Deserialize;
use tokio::sync::mpsc::{self, error::TrySendError, Receiver};
use uuid::Uuid;

const STATE_CHANGE_TIMEOUT: Duration = Duration::from_secs(5);
const BRANCH_QUEUE_MAX_BUFFERS: u32 = 100;
const PREVIEW_QUEUE_PACKETS: usize = 16;
const PREVIEW_PACKET_BYTES: usize = 4096;
const MAX_ACTIVE_PREVIEWS: usize = 8;

pub struct GstreamerMediaPipeline {
    pipelines: Mutex<PipelineState>,
}

#[derive(Default)]
struct PipelineState {
    pipelines: HashMap<Uuid, ManagedPipeline>,
    source_groups: HashMap<PipelineSourceIdentity, ManagedSourceGroup>,
    previews: HashSet<Uuid>,
    reserved_previews: HashSet<Uuid>,
}

/// A bounded tap of an active shared source. Packets contain interleaved stereo
/// S16LE samples at 48 kHz.
pub struct PcmPreview {
    media: Arc<GstreamerMediaPipeline>,
    pipeline_id: Uuid,
    packets: Receiver<Vec<u8>>,
    dropped_packets: Arc<AtomicU64>,
    decode_error: Arc<Mutex<Option<PortError>>>,
    closed: bool,
}

impl PcmPreview {
    /// Wait asynchronously for audio. Cancellation does not consume a packet.
    pub async fn recv(&mut self) -> PortResult<Vec<u8>> {
        loop {
            self.check()?;
            tokio::select! {
                packet = self.packets.recv() => {
                    self.check()?;
                    return packet.ok_or_else(|| {
                        pipeline_error("preview_stream_ended", "preview audio stream ended unexpectedly")
                    });
                }
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            }
        }
    }

    /// Counts packets dropped at the consumer queue, not upstream queue drops.
    pub fn dropped_packets(&self) -> u64 {
        self.dropped_packets.load(Ordering::Relaxed)
    }

    pub fn check(&self) -> PortResult<()> {
        if let Some(error) = self
            .decode_error
            .lock()
            .map_err(|_| pipeline_error("preview_failed", "preview decoder state is unavailable"))?
            .clone()
        {
            return Err(error);
        }
        let mut state = self.media.lock_pipelines()?;
        let identity = state
            .pipelines
            .get(&self.pipeline_id)
            .and_then(|pipeline| pipeline.source_identity)
            .ok_or_else(|| pipeline_error("preview_closed", "preview is no longer active"))?;
        observe_shared_group(&mut state, identity)?;
        let managed = state
            .pipelines
            .get(&self.pipeline_id)
            .ok_or_else(|| pipeline_error("preview_closed", "preview is no longer active"))?;
        if matches!(managed.observed_state, PipelineObservedState::Failed) {
            return Err(pipeline_error(
                "preview_failed",
                format!("preview failed: {}", managed.health_reasons.join("; ")),
            ));
        }
        Ok(())
    }

    pub fn close(&mut self) -> PortResult<()> {
        if !self.closed {
            let mut state = self.media.lock_pipelines()?;
            retire_pipeline(&mut state, self.pipeline_id)?;
            self.closed = true;
        }
        Ok(())
    }
}

impl Drop for PcmPreview {
    fn drop(&mut self) {
        if let Err(error) = self.close() {
            tracing::error!(code = error.code, message = %error.message, "failed to close PCM preview");
        }
    }
}

#[async_trait]
impl AudioPreview for PcmPreview {
    async fn recv(&mut self) -> PortResult<Vec<u8>> {
        PcmPreview::recv(self).await
    }

    async fn close(&mut self) -> PortResult<()> {
        PcmPreview::close(self)
    }
}

struct ManagedPipeline {
    generation: i64,
    description: String,
    pipeline: Option<gst::Pipeline>,
    source_identity: Option<PipelineSourceIdentity>,
    branch: Option<gst::Bin>,
    tee_pad: Option<gst::Pad>,
    observed_state: PipelineObservedState,
    health_reasons: Vec<String>,
}

struct ManagedSourceGroup {
    description: String,
    pipeline: gst::Pipeline,
    tee: gst::Element,
    active_branches: HashSet<Uuid>,
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
            pipelines: Mutex::new(PipelineState::default()),
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

    /// Attach without opening another source. The tap ends with the last route.
    pub fn start_active_pcm_preview(
        self: &Arc<Self>,
        identity: PipelineSourceIdentity,
        source: &SourceMediaProfile,
    ) -> PortResult<PcmPreview> {
        self.start_pcm_preview(identity, source, false)
    }

    fn start_pcm_preview(
        self: &Arc<Self>,
        identity: PipelineSourceIdentity,
        source: &SourceMediaProfile,
        reserved: bool,
    ) -> PortResult<PcmPreview> {
        let mpeg_ts = matches!(
            source,
            SourceMediaProfile::RtpMp2t { .. } | SourceMediaProfile::SrtMpegTs { .. }
        );
        let description = shared_source_description(source)?;
        let mut state = self.lock_pipelines()?;
        if state.previews.len() >= MAX_ACTIVE_PREVIEWS {
            return Err(pipeline_error(
                "preview_capacity_exhausted",
                "this worker already has the maximum number of previews",
            ));
        }
        let group = state.source_groups.get(&identity);
        if group.is_some_and(|group| group.description != description) {
            return Err(pipeline_error(
                "source_group_configuration_conflict",
                "preview source configuration does not match the active revision",
            ));
        }
        if !reserved
            && !group.is_some_and(|group| {
                group
                    .active_branches
                    .iter()
                    .any(|id| !state.previews.contains(id))
            })
        {
            return Err(pipeline_error(
                "preview_source_inactive",
                "preview requires an active route",
            ));
        }
        let pipeline_id = Uuid::new_v4();
        let decoder = if mpeg_ts {
            "tsdemux name=preview_demux audioconvert name=preview_convert"
        } else {
            "audioconvert name=preview_convert"
        };
        let branch = format!(
            "queue name=preview_input max-size-buffers={BRANCH_QUEUE_MAX_BUFFERS} \
             max-size-bytes=262144 max-size-time=0 leaky=downstream ! {decoder} ! audioresample ! \
             audio/x-raw,format=S16LE,rate=48000,channels=2,layout=interleaved ! \
             appsink name=preview_sink sync=false async=false max-buffers=4 drop=true"
        );
        prepare_shared_pipeline(
            &mut state,
            pipeline_id,
            1,
            "active PCM preview".to_owned(),
            identity,
            description,
            branch,
        )?;
        state.previews.insert(pipeline_id);
        if reserved {
            state.reserved_previews.insert(pipeline_id);
        }
        let decode_error = Arc::new(Mutex::new(None));
        if mpeg_ts {
            let result = state.pipelines[&pipeline_id]
                .branch
                .as_ref()
                .ok_or_else(|| {
                    pipeline_error("preview_prepare_failed", "preview branch is missing")
                })
                .and_then(|branch| configure_mpeg_preview(branch, Arc::clone(&decode_error)));
            if let Err(error) = result {
                retire_pipeline(&mut state, pipeline_id)?;
                return Err(error);
            }
        }
        let (sender, packets) = mpsc::channel(PREVIEW_QUEUE_PACKETS);
        let dropped_packets = Arc::new(AtomicU64::new(0));
        let dropped = Arc::clone(&dropped_packets);
        let sink = state.pipelines[&pipeline_id]
            .branch
            .as_ref()
            .and_then(|branch| branch.by_name("preview_sink"))
            .and_then(|sink| sink.downcast::<AppSink>().ok());
        let Some(sink) = sink else {
            retire_pipeline(&mut state, pipeline_id)?;
            return Err(pipeline_error(
                "preview_prepare_failed",
                "preview branch has no appsink",
            ));
        };
        sink.set_callbacks(
            AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                    let bytes = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                    if bytes.as_slice().len() % 4 != 0 {
                        return Err(gst::FlowError::Error);
                    }
                    let mut chunks = bytes.as_slice().chunks(PREVIEW_PACKET_BYTES);
                    while let Some(packet) = chunks.next() {
                        match sender.try_send(packet.to_vec()) {
                            Ok(()) => {}
                            Err(TrySendError::Full(_)) => {
                                dropped.fetch_add(1 + chunks.len() as u64, Ordering::Relaxed);
                                break;
                            }
                            Err(TrySendError::Closed(_)) => return Err(gst::FlowError::Eos),
                        }
                    }
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );
        if let Err(error) = start_shared_pipeline(&mut state, pipeline_id) {
            retire_pipeline(&mut state, pipeline_id)?;
            return Err(error);
        }
        Ok(PcmPreview {
            media: Arc::clone(self),
            pipeline_id,
            packets,
            dropped_packets,
            decode_error,
            closed: false,
        })
    }

    fn lock_pipelines(&self) -> PortResult<std::sync::MutexGuard<'_, PipelineState>> {
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
    async fn start_active_preview(
        self: Arc<Self>,
        identity: PipelineSourceIdentity,
        source: &SourceMediaProfile,
    ) -> PortResult<Box<dyn AudioPreview>> {
        Ok(Box::new(self.start_active_pcm_preview(identity, source)?))
    }

    async fn start_reserved_preview(
        self: Arc<Self>,
        identity: PipelineSourceIdentity,
        source: &SourceMediaProfile,
    ) -> PortResult<Box<dyn AudioPreview>> {
        Ok(Box::new(self.start_pcm_preview(identity, source, true)?))
    }

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
        let shared = if let Some(identity) = specification.source_identity {
            if identity.source_id.is_nil() || identity.source_revision <= 0 {
                return Err(pipeline_error(
                    "invalid_pipeline_source_identity",
                    "shared source identity requires a source ID and positive revision",
                ));
            }
            Some((
                identity,
                shared_source_description(&route.source)?,
                shared_branch_description(&route.source, &route.destination)?,
            ))
        } else {
            None
        };
        let independent_pipeline = if shared.is_none() {
            Some(parse_pipeline(&description)?)
        } else {
            None
        };

        let mut state = self.lock_pipelines()?;
        if let Some(current) = state.pipelines.get(&specification.pipeline_id) {
            if specification.assignment_generation < current.generation {
                return Err(pipeline_error(
                    "stale_pipeline_generation",
                    "pipeline preparation belongs to an older assignment generation",
                ));
            }
            if specification.assignment_generation == current.generation {
                if matches!(current.observed_state, PipelineObservedState::Stopped) {
                    return Err(pipeline_error(
                        "stale_pipeline_generation",
                        "a stopped assignment generation cannot be prepared again",
                    ));
                }
                if (!current.description.is_empty() && current.description != description)
                    || current.source_identity != specification.source_identity
                {
                    return Err(pipeline_error(
                        "pipeline_generation_conflict",
                        "one assignment generation cannot prepare different media graphs",
                    ));
                }
                if current.pipeline.is_some() || current.branch.is_some() {
                    return Ok(());
                }
            }
            if specification.assignment_generation > current.generation {
                retire_pipeline(&mut state, specification.pipeline_id)?;
            }
        }

        if let Some((identity, source_description, branch_description)) = shared {
            prepare_shared_pipeline(
                &mut state,
                specification.pipeline_id,
                specification.assignment_generation,
                description,
                identity,
                source_description,
                branch_description,
            )?;
        } else {
            state.pipelines.insert(
                specification.pipeline_id,
                ManagedPipeline {
                    generation: specification.assignment_generation,
                    description,
                    pipeline: independent_pipeline,
                    source_identity: None,
                    branch: None,
                    tee_pad: None,
                    observed_state: PipelineObservedState::Preparing,
                    health_reasons: Vec::new(),
                },
            );
        }
        Ok(())
    }

    async fn start(&self, pipeline_id: Uuid, generation: i64) -> PortResult<()> {
        let mut state = self.lock_pipelines()?;
        let managed = state.pipelines.get(&pipeline_id).ok_or_else(|| {
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
        if managed.source_identity.is_some() {
            return start_shared_pipeline(&mut state, pipeline_id);
        }
        let managed = state.pipelines.get_mut(&pipeline_id).ok_or_else(|| {
            pipeline_error(
                "pipeline_not_prepared",
                "pipeline disappeared before it could start",
            )
        })?;
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
        let mut state = self.lock_pipelines()?;
        if state
            .pipelines
            .get(&pipeline_id)
            .is_some_and(|managed| generation < managed.generation)
        {
            return Err(pipeline_error(
                "stale_pipeline_generation",
                "pipeline stop belongs to an older assignment generation",
            ));
        }
        let Some(mut managed) = state.pipelines.remove(&pipeline_id) else {
            state.pipelines.insert(
                pipeline_id,
                ManagedPipeline {
                    generation,
                    description: String::new(),
                    pipeline: None,
                    source_identity: None,
                    branch: None,
                    tee_pad: None,
                    observed_state: PipelineObservedState::Stopped,
                    health_reasons: Vec::new(),
                },
            );
            return Ok(());
        };
        if managed.source_identity.is_some() {
            if let Err(error) = detach_shared_branch(&mut state, pipeline_id, &mut managed) {
                state.pipelines.insert(pipeline_id, managed);
                return Err(error);
            }
            managed.branch = None;
            managed.tee_pad = None;
        }
        if let Some(pipeline) = managed.pipeline.clone() {
            if let Err(error) = transition_pipeline(
                &pipeline,
                gst::State::Null,
                "gstreamer_pipeline_stop_failed",
            ) {
                state.pipelines.insert(pipeline_id, managed);
                return Err(error);
            }
        }
        managed.generation = generation;
        managed.pipeline = None;
        managed.observed_state = PipelineObservedState::Stopped;
        managed.health_reasons.clear();
        let source_identity = managed.source_identity;
        state.pipelines.insert(pipeline_id, managed);
        if let Some(identity) = source_identity {
            prune_source_group(&mut state, identity);
        }
        Ok(())
    }

    async fn observe(&self, pipeline_id: Uuid) -> PortResult<PipelineObservation> {
        let mut state = self.lock_pipelines()?;
        let managed = state.pipelines.get(&pipeline_id).ok_or_else(|| {
            pipeline_error(
                "pipeline_not_found",
                "pipeline has not been prepared on this worker",
            )
        })?;
        let source_identity = managed.source_identity;
        let is_stopped = matches!(managed.observed_state, PipelineObservedState::Stopped);
        if let Some(identity) = source_identity.filter(|_| !is_stopped) {
            observe_shared_group(&mut state, identity)?;
        }
        let managed = state.pipelines.get_mut(&pipeline_id).ok_or_else(|| {
            pipeline_error(
                "pipeline_not_found",
                "pipeline state disappeared during observation",
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
        if let Ok(state) = self.pipelines.get_mut() {
            for managed in state.pipelines.values_mut() {
                if let Some(pipeline) = &managed.pipeline {
                    if let Err(error) = pipeline.set_state(gst::State::Null) {
                        tracing::warn!(%error, "failed to stop GStreamer pipeline on shutdown");
                    }
                }
            }
            for group in state.source_groups.values() {
                if let Err(error) = group.pipeline.set_state(gst::State::Null) {
                    tracing::warn!(%error, "failed to stop shared GStreamer source on shutdown");
                }
            }
        }
    }
}

fn configure_mpeg_preview(
    branch: &gst::Bin,
    error: Arc<Mutex<Option<PortError>>>,
) -> PortResult<()> {
    let demux = branch.by_name("preview_demux").ok_or_else(|| {
        pipeline_error(
            "preview_prepare_failed",
            "preview MPEG-TS demuxer is missing",
        )
    })?;
    let convert = branch.by_name("preview_convert").ok_or_else(|| {
        pipeline_error(
            "preview_prepare_failed",
            "preview audio converter is missing",
        )
    })?;
    let linked = Arc::new(AtomicBool::new(false));
    let pad_linked = Arc::clone(&linked);
    let pad_error = Arc::clone(&error);
    let branch = branch.downgrade();
    demux.connect_pad_added(move |_demux, pad| {
        let Some(caps) = pad.current_caps() else {
            return;
        };
        let Some(format) = caps.structure(0) else {
            return;
        };
        let decoder = match format.name().as_str() {
            "audio/x-opus" => "opusdec",
            "audio/mpeg" if format.get::<i32>("mpegversion") == Ok(1) => {
                "mpegaudioparse ! mpg123audiodec"
            }
            "audio/mpeg" if matches!(format.get::<i32>("mpegversion"), Ok(2 | 4)) => {
                "aacparse ! avdec_aac"
            }
            _ => return,
        };
        if pad_linked.swap(true, Ordering::AcqRel) {
            return;
        }
        let result = (|| -> PortResult<()> {
            let branch = branch.upgrade().ok_or_else(|| {
                pipeline_error("preview_closed", "preview branch has already closed")
            })?;
            let decoder = gst::parse::bin_from_description(decoder, true).map_err(|error| {
                pipeline_error(
                    "preview_codec_unsupported",
                    format!("preview audio decoder is unavailable: {error}"),
                )
            })?;
            branch.add(&decoder).map_err(|error| {
                pipeline_error(
                    "preview_failed",
                    format!("cannot add preview decoder: {error}"),
                )
            })?;
            let sink = decoder.static_pad("sink").ok_or_else(|| {
                pipeline_error("preview_failed", "preview decoder has no input pad")
            })?;
            let output = decoder.static_pad("src").ok_or_else(|| {
                pipeline_error("preview_failed", "preview decoder has no output pad")
            })?;
            let target = convert.static_pad("sink").ok_or_else(|| {
                pipeline_error("preview_failed", "preview converter has no input pad")
            })?;
            output.link(&target).map_err(|error| {
                pipeline_error(
                    "preview_failed",
                    format!("cannot link preview converter: {error}"),
                )
            })?;
            decoder.sync_state_with_parent().map_err(|error| {
                pipeline_error(
                    "preview_failed",
                    format!("cannot start preview decoder: {error}"),
                )
            })?;
            pad.link(&sink).map_err(|error| {
                pipeline_error(
                    "preview_failed",
                    format!("cannot link preview decoder: {error}"),
                )
            })?;
            Ok(())
        })();
        if let Err(error) = result {
            set_decode_error(&pad_error, error);
        }
    });
    demux.connect_no_more_pads(move |_| {
        if !linked.load(Ordering::Acquire) {
            set_decode_error(
                &error,
                pipeline_error(
                    "preview_codec_unsupported",
                    "MPEG-TS preview supports Opus, MPEG-1 audio and AAC; no supported audio track was found",
                ),
            );
        }
    });
    Ok(())
}

fn set_decode_error(state: &Mutex<Option<PortError>>, error: PortError) {
    match state.lock() {
        Ok(mut state) => {
            if state.is_none() {
                *state = Some(error);
            }
        }
        Err(error) => tracing::error!(%error, "preview decoder state was poisoned"),
    }
}

fn parse_pipeline(description: &str) -> PortResult<gst::Pipeline> {
    let element = gst::parse::launch(description).map_err(|error| {
        pipeline_error(
            "gstreamer_pipeline_prepare_failed",
            format!("GStreamer could not prepare the media graph: {error}"),
        )
    })?;
    element.downcast::<gst::Pipeline>().map_err(|_| {
        pipeline_error(
            "gstreamer_pipeline_prepare_failed",
            "GStreamer media graph did not produce a pipeline",
        )
    })
}

fn prepare_shared_pipeline(
    state: &mut PipelineState,
    pipeline_id: Uuid,
    generation: i64,
    description: String,
    identity: PipelineSourceIdentity,
    source_description: String,
    branch_description: String,
) -> PortResult<()> {
    let branch = gst::parse::bin_from_description(&branch_description, false).map_err(|error| {
        pipeline_error(
            "gstreamer_pipeline_prepare_failed",
            format!("GStreamer could not prepare the destination branch: {error}"),
        )
    })?;
    branch.set_property("name", format!("arc-branch-{pipeline_id}"));
    let target = branch
        .by_name("preview_input")
        .and_then(|input| input.static_pad("sink"))
        .or_else(|| branch.find_unlinked_pad(gst::PadDirection::Sink))
        .ok_or_else(|| {
            pipeline_error(
                "gstreamer_pipeline_prepare_failed",
                "destination branch has no unlinked sink pad",
            )
        })?;
    let flow_failed = Arc::new(AtomicBool::new(false));
    let list_flow_failed = Arc::clone(&flow_failed);
    let sink = gst::GhostPad::builder_with_target(&target)
        .map_err(|error| {
            pipeline_error(
                "gstreamer_pipeline_prepare_failed",
                format!("GStreamer could not prepare the branch sink pad: {error}"),
            )
        })?
        .name("sink")
        .chain_function(move |pad, parent, buffer| {
            isolate_branch_flow(
                parent,
                gst::ProxyPad::chain_default(pad, parent, buffer),
                &flow_failed,
            )
        })
        .chain_list_function(move |pad, parent, buffers| {
            isolate_branch_flow(
                parent,
                gst::ProxyPad::chain_list_default(pad, parent, buffers),
                &list_flow_failed,
            )
        })
        .build();
    branch.add_pad(&sink).map_err(|error| {
        pipeline_error(
            "gstreamer_pipeline_prepare_failed",
            format!("GStreamer could not add the branch sink pad: {error}"),
        )
    })?;

    if let Some(group) = state.source_groups.get(&identity) {
        if group.description != source_description {
            return Err(pipeline_error(
                "source_group_configuration_conflict",
                "one source revision cannot prepare different source configurations",
            ));
        }
    } else {
        let pipeline = parse_pipeline(&source_description)?;
        let tee = pipeline.by_name("source_tee").ok_or_else(|| {
            pipeline_error(
                "gstreamer_pipeline_prepare_failed",
                "shared source graph is missing its fan-out element",
            )
        })?;
        state.source_groups.insert(
            identity,
            ManagedSourceGroup {
                description: source_description,
                pipeline,
                tee,
                active_branches: HashSet::new(),
            },
        );
    }

    state.pipelines.insert(
        pipeline_id,
        ManagedPipeline {
            generation,
            description,
            pipeline: None,
            source_identity: Some(identity),
            branch: Some(branch),
            tee_pad: None,
            observed_state: PipelineObservedState::Preparing,
            health_reasons: Vec::new(),
        },
    );
    Ok(())
}

fn isolate_branch_flow(
    parent: Option<&gst::Object>,
    result: Result<gst::FlowSuccess, gst::FlowError>,
    failed: &AtomicBool,
) -> Result<gst::FlowSuccess, gst::FlowError> {
    match result {
        Ok(success) => Ok(success),
        // A branch flushing during teardown must not flush the shared source.
        Err(gst::FlowError::Flushing) => Ok(gst::FlowSuccess::Ok),
        Err(error) => {
            let Some(branch) = parent.and_then(|object| object.downcast_ref::<gst::Bin>()) else {
                return Err(error);
            };
            // Report the failure on this branch, but keep tee's upstream flow alive.
            if !failed.swap(true, Ordering::AcqRel) {
                gst::element_error!(
                    branch,
                    gst::StreamError::Failed,
                    ("destination branch stopped accepting media: {error}")
                );
            }
            Ok(gst::FlowSuccess::Ok)
        }
    }
}

fn start_shared_pipeline(state: &mut PipelineState, pipeline_id: Uuid) -> PortResult<()> {
    let (identity, branch) = {
        let managed = state.pipelines.get(&pipeline_id).ok_or_else(|| {
            pipeline_error(
                "pipeline_not_prepared",
                "pipeline must be prepared before it can start",
            )
        })?;
        let identity = managed.source_identity.ok_or_else(|| {
            pipeline_error(
                "pipeline_not_prepared",
                "pipeline has no shared source identity",
            )
        })?;
        let branch = managed.branch.clone().ok_or_else(|| {
            pipeline_error(
                "pipeline_not_prepared",
                "stopped pipeline must be prepared again before it can start",
            )
        })?;
        (identity, branch)
    };
    let group = state.source_groups.get_mut(&identity).ok_or_else(|| {
        pipeline_error(
            "pipeline_not_prepared",
            "shared source has not been prepared on this worker",
        )
    })?;
    if group.active_branches.contains(&pipeline_id) {
        return Ok(());
    }
    let was_empty = group.active_branches.is_empty();
    let tee_sink = group.tee.static_pad("sink").ok_or_else(|| {
        pipeline_error(
            "gstreamer_branch_state_unavailable",
            "shared source fan-out has no sink pad",
        )
    })?;
    // Exclude in-flight buffers and serialized events while changing tee pads.
    let _stream_lock = if was_empty {
        None
    } else {
        Some(tee_sink.stream_lock())
    };
    group.pipeline.add(&branch).map_err(|error| {
        pipeline_error(
            "gstreamer_branch_start_failed",
            format!("GStreamer could not add destination branch: {error}"),
        )
    })?;
    let Some(tee_pad) = group.tee.request_pad_simple("src_%u") else {
        let _ = group.pipeline.remove(&branch);
        return Err(pipeline_error(
            "gstreamer_branch_start_failed",
            "GStreamer could not allocate a source fan-out pad",
        ));
    };
    let Some(branch_sink) = branch.static_pad("sink") else {
        group.tee.release_request_pad(&tee_pad);
        let _ = group.pipeline.remove(&branch);
        return Err(pipeline_error(
            "gstreamer_branch_start_failed",
            "destination branch has no ghost sink pad",
        ));
    };
    if let Err(error) = tee_pad.link(&branch_sink) {
        group.tee.release_request_pad(&tee_pad);
        let _ = group.pipeline.remove(&branch);
        return Err(pipeline_error(
            "gstreamer_branch_start_failed",
            format!("GStreamer could not link the destination branch: {error}"),
        ));
    }

    let activation = if was_empty {
        transition_pipeline(
            &group.pipeline,
            gst::State::Playing,
            "gstreamer_pipeline_start_failed",
        )
    } else {
        branch.sync_state_with_parent().map_err(|error| {
            pipeline_error(
                "gstreamer_branch_start_failed",
                format!("GStreamer could not start the destination branch: {error}"),
            )
        })
    };
    if let Err(error) = activation {
        let _ = branch.set_state(gst::State::Null);
        let _ = tee_pad.unlink(&branch_sink);
        group.tee.release_request_pad(&tee_pad);
        let _ = group.pipeline.remove(&branch);
        if was_empty {
            if let Err(cleanup_error) = group.pipeline.set_state(gst::State::Null) {
                tracing::warn!(%cleanup_error, "failed to stop shared source after branch start failure");
            }
        }
        if let Some(managed) = state.pipelines.get_mut(&pipeline_id) {
            managed.observed_state = PipelineObservedState::Failed;
            managed.health_reasons = vec![error.message.clone()];
        }
        return Err(error);
    }
    group.active_branches.insert(pipeline_id);
    let managed = state.pipelines.get_mut(&pipeline_id).ok_or_else(|| {
        pipeline_error(
            "pipeline_not_prepared",
            "pipeline state disappeared during branch activation",
        )
    })?;
    managed.tee_pad = Some(tee_pad);
    managed.observed_state = PipelineObservedState::Running;
    managed.health_reasons.clear();
    Ok(())
}

fn detach_shared_branch(
    state: &mut PipelineState,
    pipeline_id: Uuid,
    managed: &mut ManagedPipeline,
) -> PortResult<()> {
    let Some(identity) = managed.source_identity else {
        return Ok(());
    };
    if !state.previews.contains(&pipeline_id) {
        if let Some(group) = state.source_groups.get(&identity) {
            let last_route = group.active_branches.contains(&pipeline_id)
                && !group
                    .active_branches
                    .iter()
                    .any(|id| *id != pipeline_id && !state.previews.contains(id));
            if last_route {
                let previews: Vec<_> = group
                    .active_branches
                    .iter()
                    .filter(|id| {
                        state.previews.contains(id) && !state.reserved_previews.contains(id)
                    })
                    .copied()
                    .collect();
                for preview in previews {
                    retire_pipeline(state, preview)?;
                }
            }
        }
    }
    let Some(group) = state.source_groups.get_mut(&identity) else {
        return Err(pipeline_error(
            "shared_source_not_found",
            "shared source group disappeared before its branch stopped",
        ));
    };
    let Some(branch) = managed.branch.as_ref() else {
        return Ok(());
    };
    if group.active_branches.contains(&pipeline_id) {
        let tee_pad = managed.tee_pad.as_ref().ok_or_else(|| {
            pipeline_error(
                "gstreamer_branch_state_unavailable",
                "active destination branch has no source pad",
            )
        })?;
        let branch_sink = branch.static_pad("sink").ok_or_else(|| {
            pipeline_error(
                "gstreamer_branch_state_unavailable",
                "active destination branch has no ghost sink pad",
            )
        })?;
        let tee_sink = group.tee.static_pad("sink").ok_or_else(|| {
            pipeline_error(
                "gstreamer_branch_state_unavailable",
                "shared source fan-out has no sink pad",
            )
        })?;
        let stream_lock = tee_sink.stream_lock();
        branch.set_state(gst::State::Null).map_err(|error| {
            pipeline_error(
                "gstreamer_pipeline_stop_failed",
                format!("GStreamer could not stop destination branch: {error}"),
            )
        })?;
        tee_pad.unlink(&branch_sink).map_err(|error| {
            pipeline_error(
                "gstreamer_pipeline_stop_failed",
                format!("GStreamer could not unlink destination branch: {error}"),
            )
        })?;
        group.tee.release_request_pad(tee_pad);
        group.pipeline.remove(branch).map_err(|error| {
            pipeline_error(
                "gstreamer_pipeline_stop_failed",
                format!("GStreamer could not remove destination branch: {error}"),
            )
        })?;
        group.active_branches.remove(&pipeline_id);
        drop(stream_lock);
    }
    if group.active_branches.is_empty() {
        transition_pipeline(
            &group.pipeline,
            gst::State::Null,
            "gstreamer_pipeline_stop_failed",
        )?;
    }
    Ok(())
}

fn retire_pipeline(state: &mut PipelineState, pipeline_id: Uuid) -> PortResult<()> {
    let Some(mut managed) = state.pipelines.remove(&pipeline_id) else {
        return Ok(());
    };
    let source_identity = managed.source_identity;
    if source_identity.is_some() {
        if let Err(error) = detach_shared_branch(state, pipeline_id, &mut managed) {
            state.pipelines.insert(pipeline_id, managed);
            return Err(error);
        }
    }
    if let Some(pipeline) = managed.pipeline.as_ref() {
        if let Err(error) =
            transition_pipeline(pipeline, gst::State::Null, "gstreamer_pipeline_stop_failed")
        {
            state.pipelines.insert(pipeline_id, managed);
            return Err(error);
        }
    }
    if let Some(identity) = source_identity {
        prune_source_group(state, identity);
    }
    state.previews.remove(&pipeline_id);
    state.reserved_previews.remove(&pipeline_id);
    Ok(())
}

fn prune_source_group(state: &mut PipelineState, identity: PipelineSourceIdentity) {
    let still_referenced = state
        .pipelines
        .values()
        .any(|managed| managed.source_identity == Some(identity) && managed.branch.is_some());
    if !still_referenced
        && state
            .source_groups
            .get(&identity)
            .is_some_and(|group| group.active_branches.is_empty())
    {
        state.source_groups.remove(&identity);
    }
}

fn observe_shared_group(
    state: &mut PipelineState,
    identity: PipelineSourceIdentity,
) -> PortResult<()> {
    let errors = {
        let group = state.source_groups.get(&identity).ok_or_else(|| {
            pipeline_error(
                "shared_source_not_found",
                "shared source group is not available on this worker",
            )
        })?;
        drain_pipeline_errors(&group.pipeline)
    };
    let had_errors = !errors.is_empty();
    for (branch_id, reason) in errors {
        let affected = match branch_id {
            Some(branch_id)
                if state
                    .source_groups
                    .get(&identity)
                    .is_some_and(|group| group.active_branches.contains(&branch_id)) =>
            {
                vec![branch_id]
            }
            Some(_) => continue,
            None => state
                .source_groups
                .get(&identity)
                .map(|group| group.active_branches.iter().copied().collect())
                .unwrap_or_default(),
        };
        for branch_id in affected {
            let Some(mut managed) = state.pipelines.remove(&branch_id) else {
                continue;
            };
            managed.observed_state = PipelineObservedState::Failed;
            managed.health_reasons = vec![reason.clone()];
            if let Err(error) = detach_shared_branch(state, branch_id, &mut managed) {
                state.pipelines.insert(branch_id, managed);
                return Err(error);
            }
            state.pipelines.insert(branch_id, managed);
        }
    }
    let group = state.source_groups.get(&identity).ok_or_else(|| {
        pipeline_error(
            "shared_source_not_found",
            "shared source group is not available on this worker",
        )
    })?;
    let (mut state_change, mut gst_state, _) = group.pipeline.state(Some(gst::ClockTime::ZERO));
    if state_change.is_err()
        && gst_state == gst::State::Playing
        && had_errors
        && !group.active_branches.is_empty()
    {
        // A removed child's error leaves the bin's state result cached as failed.
        // Reaffirm Playing without cycling the still-streaming shared source.
        group
            .pipeline
            .set_state(gst::State::Playing)
            .map_err(|error| {
                pipeline_error(
                    "gstreamer_pipeline_observation_failed",
                    format!(
                        "shared source could not acknowledge the isolated branch error: {error}"
                    ),
                )
            })?;
        (state_change, gst_state, _) = group.pipeline.state(Some(gst::ClockTime::ZERO));
    }
    state_change.map_err(|error| {
        pipeline_error(
            "gstreamer_pipeline_observation_failed",
            format!("shared source state could not be read: {error}"),
        )
    })?;
    if gst_state == gst::State::Playing {
        for branch_id in &group.active_branches {
            if let Some(managed) = state.pipelines.get_mut(branch_id) {
                if !matches!(managed.observed_state, PipelineObservedState::Failed) {
                    managed.observed_state = PipelineObservedState::Running;
                    managed.health_reasons.clear();
                }
            }
        }
    }
    Ok(())
}

fn drain_pipeline_errors(pipeline: &gst::Pipeline) -> Vec<(Option<Uuid>, String)> {
    let Some(bus) = pipeline.bus() else {
        return Vec::new();
    };
    let mut errors = Vec::new();
    while let Some(message) = bus.pop_filtered(&[gst::MessageType::Error]) {
        if let gst::MessageView::Error(error) = message.view() {
            let source = message
                .src()
                .map(|source| source.name().to_string())
                .unwrap_or_else(|| "unknown".to_owned());
            errors.push((
                branch_id_for_message(&message),
                format!("GStreamer element {source} failed: {}", error.error()),
            ));
        }
    }
    errors
}

fn branch_id_for_message(message: &gst::Message) -> Option<Uuid> {
    let mut object = message.src().cloned();
    while let Some(current) = object {
        let name = current.name();
        if let Some(value) = name.strip_prefix("arc-branch-") {
            return Uuid::parse_str(value).ok();
        }
        object = current.parent();
    }
    None
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

fn shared_source_description(source: &SourceMediaProfile) -> PortResult<String> {
    let description = match source {
        SourceMediaProfile::AlsaL16 { device } => format!(
            "alsasrc device={} ! \
             audio/x-raw,format=S16LE,rate=48000,channels=2 ! \
             audioconvert ! audio/x-raw,format=S16BE,rate=48000,channels=2 ! \
             tee name=source_tee",
            gst_string(device)?,
        ),
        SourceMediaProfile::RtpL16 {
            bind_address,
            port,
            payload_type,
        } => rtp_source_description(
            bind_address.to_string(),
            *port,
            "L16",
            *payload_type,
            "rtpL16depay",
        )?,
        SourceMediaProfile::RtpL24 {
            bind_address,
            port,
            payload_type,
        } => rtp_source_description(
            bind_address.to_string(),
            *port,
            "L24",
            *payload_type,
            "rtpL24depay",
        )?,
        SourceMediaProfile::RtpMp2t { bind_address, port } => format!(
            "udpsrc address={} port={port} caps={} ! \
             rtpjitterbuffer ! rtpmp2tdepay ! tee name=source_tee",
            gst_string(&bind_address.to_string())?,
            gst_string(
                "application/x-rtp,media=(string)video,clock-rate=(int)90000,\
                 encoding-name=(string)MP2T,payload=(int)33"
            )?,
        ),
        SourceMediaProfile::SrtMpegTs { host, port, mode } => format!(
            "srtsrc uri={} ! tee name=source_tee",
            gst_string(&srt_uri(host, *port, *mode)?)?,
        ),
    };
    Ok(description)
}

fn shared_branch_description(
    source: &SourceMediaProfile,
    destination: &DestinationMediaProfile,
) -> PortResult<String> {
    let queue = format!(
        "queue max-size-buffers={BRANCH_QUEUE_MAX_BUFFERS} \
         max-size-bytes=0 max-size-time=0 leaky=downstream"
    );
    let description = match (
        arc_domain::plan_media_path(source, destination),
        destination,
    ) {
        (
            Ok(MediaPath::AlsaL16ToRtpL16 | MediaPath::RtpL16ToRtpL16),
            DestinationMediaProfile::RtpL16 {
                address,
                port,
                payload_type,
            },
        ) => format!(
            "{queue} ! rtpL16pay pt={payload_type} ! \
             udpsink host={} port={port} sync=false async=false",
            gst_string(&address.to_string())?,
        ),
        (
            Ok(MediaPath::RtpL24ToRtpL24),
            DestinationMediaProfile::RtpL24 {
                address,
                port,
                payload_type,
            },
        ) => format!(
            "{queue} ! rtpL24pay pt={payload_type} ! \
             udpsink host={} port={port} sync=false async=false",
            gst_string(&address.to_string())?,
        ),
        (
            Ok(MediaPath::RtpMp2tToSrtMpegTs | MediaPath::SrtMpegTsToSrtMpegTs),
            DestinationMediaProfile::SrtMpegTs { host, port, mode },
        ) => {
            // A disconnected sink must not block serialized queries on the shared tee.
            format!(
                "{queue} ! srtsink uri={} async=false wait-for-connection=false",
                gst_string(&srt_uri(host, *port, *mode)?)?,
            )
        }
        _ => {
            return Err(pipeline_error(
                "unsupported_media_path",
                "media path does not have a shared-source GStreamer branch",
            ))
        }
    };
    Ok(description)
}

fn rtp_source_description(
    bind_address: String,
    source_port: u16,
    encoding: &str,
    payload_type: u8,
    depayloader: &str,
) -> PortResult<String> {
    let caps = format!(
        "application/x-rtp,media=(string)audio,clock-rate=(int)48000,\
         encoding-name=(string){encoding},channels=(int)2,payload=(int){payload_type}"
    );
    Ok(format!(
        "udpsrc address={} port={source_port} caps={} ! \
         rtpjitterbuffer ! {depayloader} ! tee name=source_tee",
        gst_string(&bind_address)?,
        gst_string(&caps)?,
    ))
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
    use std::{
        net::{IpAddr, Ipv4Addr, UdpSocket},
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc,
        },
        time::Instant,
    };

    use super::*;
    use gstreamer_app::{AppSink, AppSinkCallbacks};

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
            let shared_source = shared_source_description(&source).unwrap();
            assert!(shared_source.contains("tee name=source_tee"));
            gst::parse::launch(&shared_source).unwrap_or_else(|error| {
                panic!("invalid shared GStreamer source {shared_source}: {error}")
            });
            let branch = shared_branch_description(&source, &destination).unwrap();
            assert!(branch.contains("max-size-buffers=100"));
            assert!(branch.contains("leaky=downstream"));
            gst::parse::bin_from_description(&branch, true)
                .unwrap_or_else(|error| panic!("invalid destination branch {branch}: {error}"));
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
            pipelines: Mutex::new(PipelineState::default()),
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

    fn srt_receiver(port: u16) -> (gst::Pipeline, AppSink) {
        srt_receiver_with_mode(port, SrtMode::Listener)
    }

    fn srt_receiver_with_mode(port: u16, mode: SrtMode) -> (gst::Pipeline, AppSink) {
        let loopback = Ipv4Addr::LOCALHOST;
        let source = gst::ElementFactory::make("srtsrc")
            .property(
                "uri",
                srt_uri(&loopback.to_string(), port, mode).expect("SRT receiver URI builds"),
            )
            .build()
            .expect("SRT listener element builds");
        let sink = AppSink::builder().sync(false).build();
        let pipeline = gst::Pipeline::new();
        pipeline
            .add_many([&source, sink.upcast_ref::<gst::Element>()])
            .expect("SRT receiver elements are added");
        source
            .link(sink.upcast_ref::<gst::Element>())
            .expect("SRT receiver elements link");
        pipeline
            .set_state(gst::State::Playing)
            .expect("SRT receiver starts");
        (pipeline, sink)
    }

    fn send_transport_stream(
        sender: &UdpSocket,
        loopback: Ipv4Addr,
        port: u16,
        marker: u8,
    ) -> Vec<u8> {
        let mut transport_stream = Vec::with_capacity(188 * 7);
        for packet_index in 0..7u8 {
            let mut packet = vec![marker.wrapping_add(packet_index); 188];
            packet[0] = 0x47;
            packet[1] = 0x01;
            packet[2] = 0x00;
            packet[3] = 0x10 | packet_index;
            transport_stream.extend_from_slice(&packet);
        }
        for sequence in 1..=4u16 {
            let mut datagram = Vec::with_capacity(12 + transport_stream.len());
            datagram.extend_from_slice(&[0x80, 33]);
            datagram.extend_from_slice(&(u16::from(marker) * 4 + sequence).to_be_bytes());
            datagram.extend_from_slice(
                &(u32::from(marker) * 14_400 + u32::from(sequence) * 3_600).to_be_bytes(),
            );
            datagram.extend_from_slice(&0x4152_4331_u32.to_be_bytes());
            datagram.extend_from_slice(&transport_stream);
            sender
                .send_to(&datagram, (loopback, port))
                .expect("RTP fixture packet sends");
        }
        transport_stream
    }

    fn receiver_contains(sink: &AppSink, expected: &[u8]) -> bool {
        let mut received = Vec::new();
        for _ in 0..10 {
            let Some(sample) = sink.try_pull_sample(gst::ClockTime::from_mseconds(250)) else {
                continue;
            };
            let buffer = sample
                .buffer()
                .expect("SRT receiver sample has a buffer")
                .map_readable()
                .expect("SRT receiver buffer is readable");
            received.extend_from_slice(buffer.as_slice());
            if received
                .windows(expected.len())
                .any(|candidate| candidate == expected)
            {
                return true;
            }
        }
        false
    }

    #[tokio::test]
    async fn active_pcm_preview_is_bounded_and_does_not_keep_the_source_alive() {
        let media = Arc::new(GstreamerMediaPipeline::new().expect("GStreamer initializes"));
        let loopback = Ipv4Addr::LOCALHOST;
        let reservation = UdpSocket::bind((loopback, 0)).expect("RTP port reserves");
        let port = reservation.local_addr().unwrap().port();
        drop(reservation);
        let source = SourceMediaProfile::RtpL16 {
            bind_address: loopback.into(),
            port,
            payload_type: 96,
        };
        let identity = PipelineSourceIdentity {
            source_id: Uuid::new_v4(),
            source_revision: 1,
        };
        let provider: Arc<dyn MediaPipeline> = media.clone();
        assert_eq!(
            Arc::clone(&provider)
                .start_active_preview(identity, &source)
                .await
                .err()
                .unwrap()
                .code,
            "preview_source_inactive"
        );
        assert_eq!(
            media
                .start_active_pcm_preview(identity, &source)
                .err()
                .unwrap()
                .code,
            "preview_source_inactive"
        );
        let route_id = Uuid::new_v4();
        let sibling_id = Uuid::new_v4();
        {
            let mut state = media.lock_pipelines().unwrap();
            prepare_shared_pipeline(
                &mut state,
                route_id,
                1,
                "test route".to_owned(),
                identity,
                shared_source_description(&source).unwrap(),
                "queue ! appsink name=route_sink sync=false async=false".to_owned(),
            )
            .unwrap();
            prepare_shared_pipeline(
                &mut state,
                sibling_id,
                1,
                "second test route".to_owned(),
                identity,
                shared_source_description(&source).unwrap(),
                "queue ! appsink name=sibling_sink sync=false async=false".to_owned(),
            )
            .unwrap();
        }
        assert_eq!(
            media
                .start_active_pcm_preview(identity, &source)
                .err()
                .unwrap()
                .code,
            "preview_source_inactive"
        );
        let count = Arc::new(AtomicUsize::new(0));
        let sibling_count = Arc::new(AtomicUsize::new(0));
        {
            let state = media.lock_pipelines().unwrap();
            for (id, name, counter) in [
                (route_id, "route_sink", &count),
                (sibling_id, "sibling_sink", &sibling_count),
            ] {
                let sink = state.pipelines[&id]
                    .branch
                    .as_ref()
                    .unwrap()
                    .by_name(name)
                    .unwrap()
                    .downcast::<AppSink>()
                    .unwrap();
                let counter = Arc::clone(counter);
                sink.set_callbacks(
                    AppSinkCallbacks::builder()
                        .new_sample(move |sink| {
                            sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                            counter.fetch_add(1, Ordering::Relaxed);
                            Ok(gst::FlowSuccess::Ok)
                        })
                        .build(),
                );
            }
        }
        media.start(route_id, 1).await.unwrap();
        media.start(sibling_id, 1).await.unwrap();
        let changed_source = SourceMediaProfile::RtpL16 {
            bind_address: loopback.into(),
            port,
            payload_type: 97,
        };
        assert_eq!(
            media
                .start_active_pcm_preview(identity, &changed_source)
                .err()
                .unwrap()
                .code,
            "source_group_configuration_conflict"
        );
        let mut preview = media.start_active_pcm_preview(identity, &source).unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), preview.recv())
                .await
                .is_err()
        );
        let sender = parse_pipeline(&format!(
            "audiotestsrc is-live=true ! audio/x-raw,format=S16BE,rate=48000,channels=2 ! \
             rtpL16pay pt=96 ! udpsink host=127.0.0.1 port={port} sync=false"
        ))
        .unwrap();
        transition_pipeline(&sender, gst::State::Playing, "test_start_failed").unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while count.load(Ordering::Relaxed) == 0 || sibling_count.load(Ordering::Relaxed) == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("both routes receive audio before preview churn");
        let mut additional = Vec::new();
        for _ in 1..MAX_ACTIVE_PREVIEWS {
            additional.push(media.start_active_pcm_preview(identity, &source).unwrap());
        }
        assert_eq!(
            media
                .start_active_pcm_preview(identity, &source)
                .err()
                .unwrap()
                .code,
            "preview_capacity_exhausted"
        );
        for extra in &mut additional {
            extra.close().unwrap();
        }
        let packet = tokio::time::timeout(Duration::from_secs(5), preview.recv())
            .await
            .expect("preview receives audio in time")
            .expect("preview receives audio");
        assert!(!packet.is_empty() && packet.len() <= PREVIEW_PACKET_BYTES);
        assert_eq!(packet.len() % 4, 0);
        assert!(packet.iter().any(|byte| *byte != 0));
        let mut abstract_preview = Arc::clone(&provider)
            .start_active_preview(identity, &source)
            .await
            .unwrap();
        let packet = tokio::time::timeout(Duration::from_secs(5), abstract_preview.recv())
            .await
            .expect("worker preview receives audio in time")
            .expect("worker preview receives audio");
        assert!(!packet.is_empty() && packet.len() <= PREVIEW_PACKET_BYTES);
        assert_eq!(packet.len() % 4, 0);
        abstract_preview.close().await.unwrap();
        {
            let state = media.lock_pipelines().unwrap();
            let caps = state.pipelines[&preview.pipeline_id]
                .branch
                .as_ref()
                .unwrap()
                .by_name("preview_sink")
                .unwrap()
                .static_pad("sink")
                .unwrap()
                .current_caps()
                .unwrap();
            let audio = caps.structure(0).unwrap();
            assert_eq!(audio.get::<&str>("format").unwrap(), "S16LE");
            assert_eq!(audio.get::<i32>("rate").unwrap(), 48_000);
            assert_eq!(audio.get::<i32>("channels").unwrap(), 2);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while preview.dropped_packets() == 0 {
            assert!(
                Instant::now() < deadline,
                "preview did not apply backpressure"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let before = count.load(Ordering::Relaxed);
        while count.load(Ordering::Relaxed) <= before {
            assert!(
                Instant::now() < deadline,
                "route stopped while preview was full"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        preview.close().unwrap();
        assert!(matches!(
            media.observe(route_id).await.unwrap().state,
            PipelineObservedState::Running
        ));

        let mut failing_preview = media.start_active_pcm_preview(identity, &source).unwrap();
        {
            let state = media.lock_pipelines().unwrap();
            let branch = state.pipelines[&failing_preview.pipeline_id]
                .branch
                .as_ref()
                .unwrap();
            let bus = state.source_groups[&identity].pipeline.bus().unwrap();
            bus.post(
                gst::message::Error::builder(gst::CoreError::Failed, "preview fault")
                    .src(branch)
                    .build(),
            )
            .unwrap();
        }
        let error = failing_preview
            .recv()
            .await
            .expect_err("preview fault is reported");
        assert_eq!(error.code, "preview_failed");
        assert!(error.message.contains("preview fault"));
        assert!(matches!(
            media.observe(route_id).await.unwrap().state,
            PipelineObservedState::Running
        ));
        failing_preview.close().unwrap();

        let mut preview = media.start_active_pcm_preview(identity, &source).unwrap();
        tokio::time::timeout(Duration::from_secs(5), preview.recv())
            .await
            .expect("second preview receives audio in time")
            .expect("second preview receives audio");
        media.stop(route_id, 1).await.unwrap();
        let before = sibling_count.load(Ordering::Relaxed);
        tokio::time::timeout(Duration::from_secs(5), async {
            while sibling_count.load(Ordering::Relaxed) <= before {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("sibling route keeps receiving after the other route stops");
        assert!(matches!(
            media.observe(sibling_id).await.unwrap().state,
            PipelineObservedState::Running
        ));
        tokio::time::timeout(Duration::from_secs(5), preview.recv())
            .await
            .expect("preview receives audio in time")
            .expect("preview survives one route stopping");
        transition_pipeline(&sender, gst::State::Null, "test_stop_failed").unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                match tokio::time::timeout(Duration::from_millis(100), preview.recv()).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => panic!("preview failed while draining: {error:?}"),
                    Err(_) => break,
                }
            }
        })
        .await
        .expect("preview audio stops after the sender stops");
        let (received, stopped) = tokio::join!(
            tokio::time::timeout(Duration::from_secs(3), preview.recv()),
            async {
                tokio::time::sleep(Duration::from_millis(50)).await;
                media.stop(sibling_id, 1).await
            }
        );
        stopped.unwrap();
        assert_eq!(
            received
                .expect("receiver observes route stop")
                .unwrap_err()
                .code,
            "preview_closed"
        );
        assert_eq!(preview.check().unwrap_err().code, "preview_closed");
        assert!(media.lock_pipelines().unwrap().source_groups.is_empty());
    }

    #[tokio::test]
    async fn reserved_preview_shares_an_idle_source_with_routes_and_other_previews() {
        let media = Arc::new(GstreamerMediaPipeline::new().unwrap());
        let reservation = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = reservation.local_addr().unwrap().port();
        drop(reservation);
        let identity = PipelineSourceIdentity {
            source_id: Uuid::new_v4(),
            source_revision: 1,
        };
        let source = SourceMediaProfile::RtpL16 {
            bind_address: Ipv4Addr::LOCALHOST.into(),
            port,
            payload_type: 96,
        };
        let mut preview = media.start_pcm_preview(identity, &source, true).unwrap();
        let provider: Arc<dyn MediaPipeline> = media.clone();
        let mut second = provider
            .start_reserved_preview(identity, &source)
            .await
            .unwrap();
        let changed_source = SourceMediaProfile::RtpL16 {
            bind_address: Ipv4Addr::LOCALHOST.into(),
            port,
            payload_type: 97,
        };
        assert_eq!(
            media
                .start_pcm_preview(identity, &changed_source, true)
                .err()
                .unwrap()
                .code,
            "source_group_configuration_conflict"
        );
        assert_eq!(media.lock_pipelines().unwrap().source_groups.len(), 1);
        let sender = parse_pipeline(&format!(
            "audiotestsrc is-live=true ! audio/x-raw,format=S16BE,rate=48000,channels=2 ! \
             rtpL16pay pt=96 ! udpsink host=127.0.0.1 port={port} sync=false"
        ))
        .unwrap();
        transition_pipeline(&sender, gst::State::Playing, "test_start_failed").unwrap();
        for tap in [&mut preview as &mut dyn AudioPreview, second.as_mut()] {
            let packet = tokio::time::timeout(Duration::from_secs(5), tap.recv())
                .await
                .unwrap()
                .unwrap();
            assert!(packet.iter().any(|byte| *byte != 0));
        }

        let route_id = Uuid::new_v4();
        let received = Arc::new(AtomicUsize::new(0));
        {
            let mut state = media.lock_pipelines().unwrap();
            prepare_shared_pipeline(
                &mut state,
                route_id,
                1,
                "test route".to_owned(),
                identity,
                shared_source_description(&source).unwrap(),
                "queue ! appsink name=route_sink sync=false async=false".to_owned(),
            )
            .unwrap();
            let sink = state.pipelines[&route_id]
                .branch
                .as_ref()
                .unwrap()
                .by_name("route_sink")
                .unwrap()
                .downcast::<AppSink>()
                .unwrap();
            let received = Arc::clone(&received);
            sink.set_callbacks(
                AppSinkCallbacks::builder()
                    .new_sample(move |sink| {
                        sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                        received.fetch_add(1, Ordering::Relaxed);
                        Ok(gst::FlowSuccess::Ok)
                    })
                    .build(),
            );
        }
        media.start(route_id, 1).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while received.load(Ordering::Relaxed) == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("route receives audio from the preview source");
        let mut active_preview = media.start_active_pcm_preview(identity, &source).unwrap();
        second.close().await.unwrap();
        let before = received.load(Ordering::Relaxed);
        tokio::time::timeout(Duration::from_secs(5), async {
            while received.load(Ordering::Relaxed) <= before {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("route continues after a preview closes");

        media.stop(route_id, 1).await.unwrap();
        assert_eq!(active_preview.check().unwrap_err().code, "preview_closed");
        active_preview.close().unwrap();
        while preview.packets.try_recv().is_ok() {}
        let packet = tokio::time::timeout(Duration::from_secs(5), preview.recv())
            .await
            .expect("preview continues after the final route stops")
            .unwrap();
        assert!(packet.iter().any(|byte| *byte != 0));
        preview.close().unwrap();
        assert!(media.lock_pipelines().unwrap().source_groups.is_empty());
        transition_pipeline(&sender, gst::State::Null, "test_stop_failed").unwrap();
    }

    #[test]
    fn retiring_an_inactive_branch_stops_its_unreferenced_source() {
        let media = GstreamerMediaPipeline::new().unwrap();
        let identity = PipelineSourceIdentity {
            source_id: Uuid::new_v4(),
            source_revision: 1,
        };
        let branch = Uuid::new_v4();
        let mut state = media.lock_pipelines().unwrap();
        prepare_shared_pipeline(
            &mut state,
            branch,
            1,
            "test route".to_owned(),
            identity,
            "audiotestsrc is-live=true ! tee name=source_tee allow-not-linked=true".to_owned(),
            "queue ! fakesink sync=false async=false".to_owned(),
        )
        .unwrap();
        let source = state.source_groups[&identity].pipeline.clone();
        transition_pipeline(&source, gst::State::Playing, "test_start_failed").unwrap();
        retire_pipeline(&mut state, branch).unwrap();
        assert_eq!(source.current_state(), gst::State::Null);
        assert!(state.source_groups.is_empty());
    }

    #[tokio::test]
    async fn preview_decodes_mpeg_ts_opus_without_interrupting_the_route() {
        test_mpeg_ts_preview("opusenc", true, false, false).await;
    }

    #[tokio::test]
    async fn preview_decodes_srt_mpeg_ts_without_interrupting_the_route() {
        test_mpeg_ts_preview("opusenc", true, true, false).await;
    }

    #[tokio::test]
    async fn preview_decodes_mpeg_ts_mpeg_audio_without_interrupting_the_route() {
        test_mpeg_ts_preview("lamemp3enc ! mpegaudioparse", true, false, false).await;
    }

    #[tokio::test]
    async fn preview_decodes_mpeg_ts_aac_without_interrupting_the_route() {
        test_mpeg_ts_preview("voaacenc ! aacparse", true, false, false).await;
    }

    #[tokio::test]
    async fn preview_decodes_srt_mpeg_ts_aac_without_interrupting_the_route() {
        test_mpeg_ts_preview("voaacenc ! aacparse", true, true, false).await;
    }

    #[tokio::test]
    async fn preview_attaches_to_an_already_streaming_srt_source() {
        test_mpeg_ts_preview_with_streaming_source("opusenc").await;
    }

    #[tokio::test]
    async fn aac_preview_attaches_to_an_already_streaming_srt_source() {
        test_mpeg_ts_preview_with_streaming_source("voaacenc ! aacparse").await;
    }

    #[tokio::test]
    async fn preview_on_streaming_srt_with_disconnected_destination() {
        test_srt_preview_with_disconnected_destination(SrtMode::Caller).await;
    }

    #[tokio::test]
    async fn preview_on_streaming_srt_with_no_listener_client() {
        test_srt_preview_with_disconnected_destination(SrtMode::Listener).await;
    }

    async fn test_srt_preview_with_disconnected_destination(destination_mode: SrtMode) {
        let media = Arc::new(GstreamerMediaPipeline::new().unwrap());
        let ports = [
            UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap(),
            UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap(),
        ];
        let source_port = ports[0].local_addr().unwrap().port();
        let destination_port = ports[1].local_addr().unwrap().port();
        drop(ports);
        let identity = PipelineSourceIdentity {
            source_id: Uuid::new_v4(),
            source_revision: 1,
        };
        let source = SourceMediaProfile::SrtMpegTs {
            host: Ipv4Addr::LOCALHOST.to_string(),
            port: source_port,
            mode: SrtMode::Listener,
        };
        let route_id = Uuid::new_v4();
        let count = Arc::new(AtomicUsize::new(0));
        {
            let mut state = media.lock_pipelines().unwrap();
            prepare_shared_pipeline(
                &mut state,
                route_id,
                1,
                "test route".to_owned(),
                identity,
                shared_source_description(&source).unwrap(),
                shared_branch_description(
                    &source,
                    &DestinationMediaProfile::SrtMpegTs {
                        host: Ipv4Addr::LOCALHOST.to_string(),
                        port: destination_port,
                        mode: destination_mode,
                    },
                )
                .unwrap(),
            )
            .unwrap();
            let counter = Arc::clone(&count);
            state.source_groups[&identity]
                .tee
                .static_pad("sink")
                .unwrap()
                .add_probe(gst::PadProbeType::BUFFER, move |_, _| {
                    counter.fetch_add(1, Ordering::Relaxed);
                    gst::PadProbeReturn::Ok
                });
        }
        media.start(route_id, 1).await.unwrap();
        let sender = parse_pipeline(&format!(
            "audiotestsrc is-live=true ! audio/x-raw,rate=48000,channels=2 ! \
             voaacenc ! aacparse ! mpegtsmux ! \
             srtsink uri=\"srt://127.0.0.1:{source_port}?mode=caller\" sync=false async=false"
        ))
        .unwrap();
        transition_pipeline(&sender, gst::State::Playing, "test_start_failed").unwrap();
        tokio::time::timeout(Duration::from_secs(8), async {
            while count.load(Ordering::Relaxed) == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("SRT source receives audio before preview starts");
        tokio::time::sleep(Duration::from_secs(3)).await;
        let mut preview = media.start_pcm_preview(identity, &source, true).unwrap();
        let packet = tokio::time::timeout(Duration::from_secs(8), preview.recv())
            .await
            .expect("preview responds")
            .expect("preview decodes audio");
        assert!(packet.iter().any(|byte| *byte != 0));
        preview.close().unwrap();
        let mut preview = media.start_pcm_preview(identity, &source, true).unwrap();
        let receiver_mode = match destination_mode {
            SrtMode::Caller => SrtMode::Listener,
            SrtMode::Listener => SrtMode::Caller,
        };
        let (receiver, sink) = srt_receiver_with_mode(destination_port, receiver_mode);
        let routed = sink
            .try_pull_sample(gst::ClockTime::from_seconds(12))
            .expect("SRT destination receives media after reconnect");
        let bytes = routed.buffer().unwrap().map_readable().unwrap();
        assert!(bytes.as_slice().chunks(188).any(|packet| packet[0] == 0x47));
        while sink.try_pull_sample(gst::ClockTime::ZERO).is_some() {}
        preview.close().unwrap();
        let routed_after_close = sink
            .try_pull_sample(gst::ClockTime::from_seconds(5))
            .expect("SRT destination continues after preview closes");
        assert!(routed_after_close.buffer().unwrap().size() > 0);
        media.stop(route_id, 1).await.unwrap();
        transition_pipeline(&receiver, gst::State::Null, "test_stop_failed").unwrap();
        transition_pipeline(&sender, gst::State::Null, "test_stop_failed").unwrap();
    }

    #[tokio::test]
    async fn no_signal_srt_and_rtp_previews_close_without_blocking() {
        let media = Arc::new(GstreamerMediaPipeline::new().unwrap());
        let ports = [
            UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap(),
            UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap(),
            UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap(),
        ];
        let srt_port = ports[0].local_addr().unwrap().port();
        let rtp_port = ports[1].local_addr().unwrap().port();
        let destination_port = ports[2].local_addr().unwrap().port();
        drop(ports);
        let srt_identity = PipelineSourceIdentity {
            source_id: Uuid::new_v4(),
            source_revision: 1,
        };
        let srt = SourceMediaProfile::SrtMpegTs {
            host: Ipv4Addr::LOCALHOST.to_string(),
            port: srt_port,
            mode: SrtMode::Listener,
        };
        let route_id = Uuid::new_v4();
        {
            let mut state = media.lock_pipelines().unwrap();
            prepare_shared_pipeline(
                &mut state,
                route_id,
                1,
                "test route".to_owned(),
                srt_identity,
                shared_source_description(&srt).unwrap(),
                format!(
                    "queue ! srtsink uri=\"srt://127.0.0.1:{destination_port}?mode=caller\" async=false"
                ),
            )
            .unwrap();
        }
        media.start(route_id, 1).await.unwrap();
        let mut srt_preview = media.start_pcm_preview(srt_identity, &srt, true).unwrap();
        let rtp = SourceMediaProfile::RtpL24 {
            bind_address: Ipv4Addr::LOCALHOST.into(),
            port: rtp_port,
            payload_type: 96,
        };
        let mut rtp_preview = media
            .start_pcm_preview(
                PipelineSourceIdentity {
                    source_id: Uuid::new_v4(),
                    source_revision: 1,
                },
                &rtp,
                true,
            )
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), srt_preview.recv())
                .await
                .is_err()
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(100), rtp_preview.recv())
                .await
                .is_err()
        );
        srt_preview.close().unwrap();
        rtp_preview.close().unwrap();
        media.stop(route_id, 1).await.unwrap();
    }

    #[tokio::test]
    async fn reserved_preview_shares_an_idle_srt_source_with_a_route() {
        test_mpeg_ts_preview("voaacenc ! aacparse", true, true, true).await;
    }

    #[tokio::test]
    async fn preview_reports_unsupported_mpeg_ts_audio_without_interrupting_the_route() {
        test_mpeg_ts_preview(
            "capssetter caps=\"audio/x-lpcm,width=(int)16,rate=(int)48000,channels=(int)2,\
             dynamic_range=(int)0,emphasis=(boolean)false,mute=(boolean)false\" \
             replace=true join=false",
            false,
            false,
            false,
        )
        .await;
    }

    async fn test_mpeg_ts_preview(encoder: &str, supported: bool, srt: bool, reserved: bool) {
        test_mpeg_ts_preview_scenario(encoder, supported, srt, reserved, false).await;
    }

    async fn test_mpeg_ts_preview_with_streaming_source(encoder: &str) {
        test_mpeg_ts_preview_scenario(encoder, true, true, false, true).await;
    }

    async fn test_mpeg_ts_preview_scenario(
        encoder: &str,
        supported: bool,
        srt: bool,
        reserved: bool,
        streaming_before_preview: bool,
    ) {
        let media = Arc::new(GstreamerMediaPipeline::new().unwrap());
        let reservation = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = reservation.local_addr().unwrap().port();
        drop(reservation);
        let identity = PipelineSourceIdentity {
            source_id: Uuid::new_v4(),
            source_revision: 1,
        };
        let source = if srt {
            SourceMediaProfile::SrtMpegTs {
                host: Ipv4Addr::LOCALHOST.to_string(),
                port,
                mode: SrtMode::Listener,
            }
        } else {
            SourceMediaProfile::RtpMp2t {
                bind_address: Ipv4Addr::LOCALHOST.into(),
                port,
            }
        };
        assert_eq!(
            media
                .start_active_pcm_preview(identity, &source)
                .err()
                .unwrap()
                .code,
            "preview_source_inactive"
        );
        let idle_preview = if reserved {
            Some(media.start_pcm_preview(identity, &source, true).unwrap())
        } else {
            None
        };
        let route_id = Uuid::new_v4();
        let count = Arc::new(AtomicUsize::new(0));
        {
            let mut state = media.lock_pipelines().unwrap();
            prepare_shared_pipeline(
                &mut state,
                route_id,
                1,
                "test route".to_owned(),
                identity,
                shared_source_description(&source).unwrap(),
                "queue ! appsink name=route_sink sync=false async=false".to_owned(),
            )
            .unwrap();
            let sink = state.pipelines[&route_id]
                .branch
                .as_ref()
                .unwrap()
                .by_name("route_sink")
                .unwrap()
                .downcast::<AppSink>()
                .unwrap();
            let count = Arc::clone(&count);
            sink.set_callbacks(
                AppSinkCallbacks::builder()
                    .new_sample(move |sink| {
                        sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                        count.fetch_add(1, Ordering::Relaxed);
                        Ok(gst::FlowSuccess::Ok)
                    })
                    .build(),
            );
        }
        media.start(route_id, 1).await.unwrap();
        let output = if srt {
            format!("srtsink uri=\"srt://127.0.0.1:{port}?mode=caller\" sync=false async=false")
        } else {
            format!("rtpmp2tpay ! udpsink host=127.0.0.1 port={port} sync=false")
        };
        let sender = parse_pipeline(&format!(
            "audiotestsrc is-live=true ! audio/x-raw,rate=48000,channels=2 ! \
             {encoder} ! mpegtsmux ! {output}"
        ))
        .unwrap();
        let mut preview = if !streaming_before_preview {
            Some(match idle_preview {
                Some(preview) => preview,
                None => media.start_active_pcm_preview(identity, &source).unwrap(),
            })
        } else {
            None
        };
        transition_pipeline(&sender, gst::State::Playing, "test_start_failed").unwrap();
        if streaming_before_preview {
            tokio::time::timeout(Duration::from_secs(8), async {
                while count.load(Ordering::Relaxed) == 0 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("route receives audio before preview starts");
            preview = Some(media.start_active_pcm_preview(identity, &source).unwrap());
        }
        let mut preview = preview.expect("preview started");
        let result = tokio::time::timeout(Duration::from_secs(8), preview.recv()).await;
        let during_preview = count.load(Ordering::Relaxed);
        tokio::time::timeout(Duration::from_secs(5), async {
            while count.load(Ordering::Relaxed) <= during_preview {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("route continues while preview is active");
        let before = count.load(Ordering::Relaxed);
        preview.close().unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while count.load(Ordering::Relaxed) <= before {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("route continues after preview closes");
        media.stop(route_id, 1).await.unwrap();
        transition_pipeline(&sender, gst::State::Null, "test_stop_failed").unwrap();
        let result = result.expect("MPEG-TS preview responds in time");
        if supported {
            assert!(result
                .expect("preview decodes audio")
                .iter()
                .any(|byte| *byte != 0));
        } else {
            assert_eq!(result.unwrap_err().code, "preview_codec_unsupported");
        }
    }

    #[tokio::test]
    async fn shares_one_rtp_source_and_keeps_sibling_branch_running_after_stop() {
        let media = GstreamerMediaPipeline::new().expect("GStreamer initializes");
        let loopback = Ipv4Addr::LOCALHOST;
        let rtp_reservation = UdpSocket::bind((loopback, 0)).expect("RTP source port reserves");
        let rtp_port = rtp_reservation
            .local_addr()
            .expect("RTP source port is available")
            .port();
        let srt_reservations = [
            UdpSocket::bind((loopback, 0)).expect("first SRT port reserves"),
            UdpSocket::bind((loopback, 0)).expect("second SRT port reserves"),
        ];
        let srt_ports = srt_reservations
            .iter()
            .map(|socket| {
                socket
                    .local_addr()
                    .expect("SRT listener port is available")
                    .port()
            })
            .collect::<Vec<_>>();
        drop((rtp_reservation, srt_reservations));
        let (first_receiver, first_sink) = srt_receiver(srt_ports[0]);
        let (second_receiver, second_sink) = srt_receiver(srt_ports[1]);

        let source = SourceMediaProfile::RtpMp2t {
            bind_address: loopback.into(),
            port: rtp_port,
        };
        let first_destination = DestinationMediaProfile::SrtMpegTs {
            host: loopback.to_string(),
            port: srt_ports[0],
            mode: SrtMode::Caller,
        };
        let second_destination = DestinationMediaProfile::SrtMpegTs {
            host: loopback.to_string(),
            port: srt_ports[1],
            mode: SrtMode::Caller,
        };
        let estimate = estimate_pipeline_requirements(&source, &first_destination)
            .expect("RTP MPEG-TS to SRT requirements are supported");
        let first_pipeline_id = Uuid::new_v4();
        let second_pipeline_id = Uuid::new_v4();
        let source_identity = PipelineSourceIdentity {
            source_id: Uuid::new_v4(),
            source_revision: 1,
        };
        let first_specification = PipelineSpecification {
            pipeline_id: first_pipeline_id,
            assignment_generation: 1,
            source_identity: Some(source_identity),
            requirements: estimate.requirements.clone(),
            configuration: serde_json::json!({
                "source": source,
                "destination": first_destination,
            }),
        };
        let second_specification = PipelineSpecification {
            pipeline_id: second_pipeline_id,
            assignment_generation: 1,
            source_identity: Some(source_identity),
            requirements: estimate.requirements.clone(),
            configuration: serde_json::json!({
                "source": source,
                "destination": second_destination,
            }),
        };
        media
            .prepare(&first_specification)
            .await
            .expect("RTP-to-SRT pipeline prepares");
        media
            .prepare(&second_specification)
            .await
            .expect("second destination branch prepares on the shared source");
        media
            .start(first_pipeline_id, 1)
            .await
            .expect("RTP-to-SRT pipeline starts");
        media
            .start(second_pipeline_id, 1)
            .await
            .expect("second destination branch starts without another source graph");
        let sender = UdpSocket::bind((loopback, 0)).expect("RTP sender socket binds");
        let first_stream = send_transport_stream(&sender, loopback, rtp_port, 1);
        assert!(receiver_contains(&first_sink, &first_stream));
        assert!(receiver_contains(&second_sink, &first_stream));

        media
            .stop(first_pipeline_id, 1)
            .await
            .expect("first destination branch stops");
        let error = media
            .prepare(&first_specification)
            .await
            .expect_err("a delayed Start cannot prepare a stopped generation");
        assert_eq!(error.code, "stale_pipeline_generation");
        assert!(matches!(
            media
                .observe(second_pipeline_id)
                .await
                .expect("sibling observation succeeds")
                .state,
            PipelineObservedState::Running
        ));
        let second_stream = send_transport_stream(&sender, loopback, rtp_port, 40);
        assert!(receiver_contains(&second_sink, &second_stream));

        media
            .stop(second_pipeline_id, 1)
            .await
            .expect("second destination branch stops");
        first_receiver
            .set_state(gst::State::Null)
            .expect("first SRT receiver stops");
        second_receiver
            .set_state(gst::State::Null)
            .expect("second SRT receiver stops");
    }

    #[tokio::test]
    async fn isolates_a_failed_shared_branch_and_keeps_sibling_receiving() {
        assert_shared_branch_failure_isolated(false).await;
    }

    #[tokio::test]
    async fn isolates_an_element_flow_error_before_observation() {
        assert_shared_branch_failure_isolated(true).await;
    }

    #[tokio::test]
    async fn a_shared_source_flow_error_fails_all_its_branches() {
        let media = GstreamerMediaPipeline::new().expect("GStreamer initializes");
        let identity = PipelineSourceIdentity {
            source_id: Uuid::new_v4(),
            source_revision: 1,
        };
        let routes = [Uuid::new_v4(), Uuid::new_v4()];
        {
            let mut state = media.lock_pipelines().expect("pipeline state is available");
            for pipeline_id in routes {
                prepare_shared_pipeline(
                    &mut state,
                    pipeline_id,
                    1,
                    "shared source failure".to_owned(),
                    identity,
                    "videotestsrc is-live=true ! identity error-after=20 ! tee name=source_tee"
                        .to_owned(),
                    "queue ! fakesink sync=false async=false".to_owned(),
                )
                .expect("branch prepares");
            }
        }
        for pipeline_id in routes {
            media.start(pipeline_id, 1).await.expect("branch starts");
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let observation = media
                .observe(routes[0])
                .await
                .expect("observation succeeds");
            if matches!(observation.state, PipelineObservedState::Failed) {
                break;
            }
            assert!(Instant::now() < deadline, "source failure was not observed");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        for pipeline_id in routes {
            let observation = media
                .observe(pipeline_id)
                .await
                .expect("observation succeeds");
            assert!(matches!(observation.state, PipelineObservedState::Failed));
            assert!(observation
                .health_reasons
                .iter()
                .any(|reason| reason.contains("Failed after iterations")));
            media.stop(pipeline_id, 1).await.expect("branch cleans up");
        }
        assert!(media.lock_pipelines().unwrap().source_groups.is_empty());
    }

    async fn assert_shared_branch_failure_isolated(element_error: bool) {
        let media = GstreamerMediaPipeline::new().expect("GStreamer initializes");
        let source_identity = PipelineSourceIdentity {
            source_id: Uuid::new_v4(),
            source_revision: 1,
        };
        let source_description = "videotestsrc is-live=true ! tee name=source_tee".to_owned();
        let failing_pipeline_id = Uuid::new_v4();
        let sibling_pipeline_id = Uuid::new_v4();
        {
            let mut state = media.lock_pipelines().expect("pipeline state is available");
            prepare_shared_pipeline(
                &mut state,
                failing_pipeline_id,
                1,
                "injected branch failure".to_owned(),
                source_identity,
                source_description.clone(),
                if element_error {
                    "queue ! identity name=fault error-after=20 ! fakesink sync=false async=false"
                } else {
                    "queue ! fakesink sync=false"
                }
                .to_owned(),
            )
            .expect("failing branch prepares");
            prepare_shared_pipeline(
                &mut state,
                sibling_pipeline_id,
                1,
                "healthy sibling".to_owned(),
                source_identity,
                source_description,
                "queue ! appsink name=healthy_sink sync=false".to_owned(),
            )
            .expect("healthy sibling prepares");
        }

        let received_buffers = Arc::new(AtomicUsize::new(0));
        let sibling_sink = {
            let state = media.lock_pipelines().expect("pipeline state is available");
            state
                .pipelines
                .get(&sibling_pipeline_id)
                .and_then(|managed| managed.branch.as_ref())
                .and_then(|branch| branch.by_name("healthy_sink"))
                .expect("healthy appsink exists")
                .downcast::<AppSink>()
                .expect("healthy sink is an appsink")
        };
        let callback_count = Arc::clone(&received_buffers);
        sibling_sink.set_callbacks(
            AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    callback_count.fetch_add(1, Ordering::Relaxed);
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        let source_stream_starts = Arc::new(AtomicUsize::new(0));
        let failure_posted = Arc::new(AtomicBool::new(false));
        let (branch, bus) = {
            let state = media.lock_pipelines().expect("pipeline state is available");
            let branch = state.pipelines[&failing_pipeline_id]
                .branch
                .clone()
                .expect("failing branch exists");
            let group = &state.source_groups[&source_identity];
            let stream_starts = Arc::clone(&source_stream_starts);
            group
                .tee
                .static_pad("sink")
                .expect("shared tee sink exists")
                .add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, move |_, info| {
                    if info
                        .event()
                        .is_some_and(|event| event.type_() == gst::EventType::StreamStart)
                    {
                        stream_starts.fetch_add(1, Ordering::Relaxed);
                    }
                    gst::PadProbeReturn::Ok
                })
                .expect("source stream probe attaches");
            (
                branch,
                group.pipeline.bus().expect("shared source bus exists"),
            )
        };
        if element_error {
            let posted = Arc::clone(&failure_posted);
            bus.set_sync_handler(move |_, message| {
                if matches!(message.view(), gst::MessageView::Error(_))
                    && branch_id_for_message(message) == Some(failing_pipeline_id)
                {
                    posted.store(true, Ordering::Release);
                }
                gst::BusSyncReply::Pass
            });
        }

        media
            .start(sibling_pipeline_id, 1)
            .await
            .expect("healthy sibling starts");
        media
            .start(failing_pipeline_id, 1)
            .await
            .expect("failing branch starts");

        let deadline = Instant::now() + Duration::from_secs(5);
        while received_buffers.load(Ordering::Relaxed) == 0 {
            assert!(
                Instant::now() < deadline,
                "healthy sibling did not receive initial buffers"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        if !element_error {
            let message =
                gst::message::Error::builder(gst::CoreError::Failed, "injected branch failure")
                    .debug("deterministic shared-branch failure test")
                    .src(&branch)
                    .build();
            bus.post(message).expect("branch failure message posts");
        }

        if element_error {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !failure_posted.load(Ordering::Acquire) {
                assert!(
                    Instant::now() < deadline,
                    "the element did not generate an error"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let buffers_before_observation = received_buffers.load(Ordering::Relaxed);
            while received_buffers.load(Ordering::Relaxed) < buffers_before_observation + 10 {
                assert!(
                    Instant::now() < deadline,
                    "the sibling stopped receiving before the controller observed the failure"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }

        let failure_deadline = Instant::now() + Duration::from_secs(5);
        let failure = loop {
            let observation = media
                .observe(failing_pipeline_id)
                .await
                .expect("failed branch observation succeeds");
            if matches!(observation.state, PipelineObservedState::Failed) {
                break observation;
            }
            assert!(
                Instant::now() < failure_deadline,
                "branch failure was not observed"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert!(
            failure
                .health_reasons
                .iter()
                .any(|reason| reason.contains(if element_error {
                    "Failed after iterations"
                } else {
                    "injected branch failure"
                })),
            "the failed branch's GStreamer error is reported: {:?}",
            failure.health_reasons
        );

        let buffers_at_failure = received_buffers.load(Ordering::Relaxed);
        let deadline = Instant::now() + Duration::from_secs(5);
        while received_buffers.load(Ordering::Relaxed) <= buffers_at_failure {
            assert!(
                Instant::now() < deadline,
                "healthy sibling stopped receiving buffers after branch failure"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(matches!(
            media
                .observe(sibling_pipeline_id)
                .await
                .expect("healthy sibling observation succeeds")
                .state,
            PipelineObservedState::Running
        ));
        assert_eq!(
            source_stream_starts.load(Ordering::Relaxed),
            1,
            "isolating a branch must not restart the shared source"
        );

        media
            .stop(failing_pipeline_id, 1)
            .await
            .expect("failed branch cleans up");
        media
            .stop(sibling_pipeline_id, 1)
            .await
            .expect("healthy sibling cleans up");
        assert!(media.lock_pipelines().unwrap().source_groups.is_empty());
    }
}
