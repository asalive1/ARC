use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
    time::Duration,
};

use arc_application::{
    MediaPipeline, PipelineObservation, PipelineObservedState, PipelineSourceIdentity,
    PipelineSpecification, PortError, PortResult,
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
const BRANCH_QUEUE_MAX_BUFFERS: u32 = 100;

pub struct GstreamerMediaPipeline {
    pipelines: Mutex<PipelineState>,
}

#[derive(Default)]
struct PipelineState {
    pipelines: HashMap<Uuid, ManagedPipeline>,
    source_groups: HashMap<PipelineSourceIdentity, ManagedSourceGroup>,
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
    let branch = gst::parse::bin_from_description(&branch_description, true).map_err(|error| {
        pipeline_error(
            "gstreamer_pipeline_prepare_failed",
            format!("GStreamer could not prepare the destination branch: {error}"),
        )
    })?;
    branch.set_property("name", format!("arc-branch-{pipeline_id}"));
    if branch.static_pad("sink").is_none() {
        return Err(pipeline_error(
            "gstreamer_pipeline_prepare_failed",
            "destination branch has no ghost sink pad",
        ));
    }

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
        if group.active_branches.is_empty() {
            transition_pipeline(
                &group.pipeline,
                gst::State::Null,
                "gstreamer_pipeline_stop_failed",
            )?;
        }
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
    for (branch_id, reason) in errors {
        let affected = match branch_id {
            Some(branch_id)
                if state
                    .pipelines
                    .get(&branch_id)
                    .is_some_and(|managed| managed.source_identity == Some(identity)) =>
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
    let (state_change, gst_state, _) = group.pipeline.state(Some(gst::ClockTime::ZERO));
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
        ) => format!(
            "{queue} ! srtsink uri={} async=false",
            gst_string(&srt_uri(host, *port, *mode)?)?,
        ),
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
            atomic::{AtomicUsize, Ordering},
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
        let loopback = Ipv4Addr::LOCALHOST;
        let source = gst::ElementFactory::make("srtsrc")
            .property(
                "uri",
                srt_uri(&loopback.to_string(), port, SrtMode::Listener)
                    .expect("SRT listener URI builds"),
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
                "queue ! fakesink sync=false".to_owned(),
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

        // A branch-attributed bus error exercises isolation without failing the shared tee's flow.
        {
            let state = media.lock_pipelines().expect("pipeline state is available");
            let branch = state
                .pipelines
                .get(&failing_pipeline_id)
                .and_then(|managed| managed.branch.as_ref())
                .expect("failing branch exists");
            let pipeline = &state
                .source_groups
                .get(&source_identity)
                .expect("shared source group exists")
                .pipeline;
            let message =
                gst::message::Error::builder(gst::CoreError::Failed, "injected branch failure")
                    .debug("deterministic shared-branch failure test")
                    .src(branch)
                    .build();
            pipeline
                .bus()
                .expect("shared source bus exists")
                .post(message)
                .expect("branch failure message posts");
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
                .any(|reason| reason.contains("injected branch failure")),
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

        media
            .stop(failing_pipeline_id, 1)
            .await
            .expect("failed branch cleans up");
        media
            .stop(sibling_pipeline_id, 1)
            .await
            .expect("healthy sibling cleans up");
    }
}
