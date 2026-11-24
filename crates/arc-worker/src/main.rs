use std::{env, error::Error, io, sync::Arc, time::Duration};

use arc_application::{MediaPipeline, PipelineSpecification, WorkerRegistry};
use arc_domain::{Architecture, Platform, WorkerState};
use arc_gstreamer::GstreamerMediaPipeline;
use arc_infrastructure::{
    assignments::PgAssignments,
    postgres::connect_postgres,
    worker_fencing::PgAssignmentGenerationStore,
    worker_registry::PgWorkerRegistry,
};
use arc_worker::{local_worker, WorkerControlClient, WorkerControlClientConfig};
use chrono::Utc;
use tokio::time::MissedTickBehavior;
use tracing::{info, warn};
use uuid::Uuid;

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt().json().init();
    run_worker().await
}

async fn run_worker() -> Result<(), Box<dyn Error>> {
    let database_url = required_env("DATABASE_URL")?;
    let worker_id = required_env("ARC_WORKER_ID")?
        .parse::<Uuid>()
        .map_err(|error| invalid_config("ARC_WORKER_ID", error))?;
    let endpoint = required_env("ARC_WORKER_CONTROL_ENDPOINT")?;
    let bearer_token = worker_control_token()?;
    let stable_name =
        env::var("ARC_WORKER_NAME").unwrap_or_else(|_| format!("arc-worker-{worker_id}"));
    let enable_alsa = env::var("ARC_WORKER_ENABLE_ALSA")
        .unwrap_or_else(|_| "false".to_owned())
        .parse::<bool>()
        .map_err(|error| invalid_config("ARC_WORKER_ENABLE_ALSA", error))?;
    let capacity_units = env::var("ARC_WORKER_CAPACITY_UNITS")
        .unwrap_or_else(|_| "2".to_owned())
        .parse::<u32>()
        .map_err(|error| invalid_config("ARC_WORKER_CAPACITY_UNITS", error))?;
    if capacity_units == 0 {
        return Err(
            invalid_config("ARC_WORKER_CAPACITY_UNITS", "must be greater than zero").into(),
        );
    }
    let ingress_bits_per_second =
        optional_u64_env("ARC_WORKER_INGRESS_BITS_PER_SECOND")?;
    let egress_bits_per_second =
        optional_u64_env("ARC_WORKER_EGRESS_BITS_PER_SECOND")?;
    let cpu_utilization_ratio =
        optional_ratio_env("ARC_WORKER_CPU_UTILIZATION_RATIO")?;
    let memory_utilization_ratio =
        optional_ratio_env("ARC_WORKER_MEMORY_UTILIZATION_RATIO")?;

    let media_pipeline = Arc::new(GstreamerMediaPipeline::new().map_err(port_error)?);
    let mut worker = local_worker(
        worker_id,
        stable_name,
        env!("CARGO_PKG_VERSION"),
        media_pipeline.worker_capabilities(enable_alsa),
    );
    worker.capabilities.platform = Some(native_platform()?);
    worker.capabilities.architecture = Some(native_architecture()?);
    worker.capacity.total_capacity_units = capacity_units;
    worker.capacity.ingress_bits_per_second = ingress_bits_per_second;
    worker.capacity.egress_bits_per_second = egress_bits_per_second;
    worker.capacity.cpu_utilization_ratio = cpu_utilization_ratio;
    worker.capacity.memory_utilization_ratio = memory_utilization_ratio;
    if worker.capabilities.source_adapters.is_empty()
        || worker.capabilities.destination_adapters.is_empty()
    {
        return Err(io::Error::other(
            "GStreamer did not provide any complete Phase 1 media adapter capabilities",
        )
        .into());
    }

    let pool = connect_postgres(&database_url).await?;
    let registry = PgWorkerRegistry::new(pool.clone());
    register_or_resume(&registry, &mut worker).await?;
    worker.state = WorkerState::Warming;
    worker.last_heartbeat = Utc::now();
    registry
        .heartbeat(worker.clone())
        .await
        .map_err(port_error)?;

    let assignments = PgAssignments::new(pool.clone());
    let generation_store = Arc::new(PgAssignmentGenerationStore::new(pool));
    let client = WorkerControlClient::new(
        WorkerControlClientConfig {
            endpoint,
            worker_id,
            bearer_token,
            allow_insecure_loopback: false,
        },
        generation_store,
        media_pipeline.clone(),
    )
    .map_err(port_error)?;
    let mut connection_status = client.connection_status();
    let control_task = tokio::spawn(run_control_sessions(client));

    info!(
        %worker_id,
        stable_name = %worker.stable_name,
        source_adapters = ?worker.capabilities.source_adapters,
        destination_adapters = ?worker.capabilities.destination_adapters,
        capacity_units,
        "ARC worker is ready"
    );

    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            result = &mut shutdown => {
                result?;
                break;
            }
            changed = connection_status.changed() => {
                if changed.is_err() {
                    return Err(io::Error::other("worker connection status channel closed").into());
                }
                let connected = *connection_status.borrow();
                worker.state = if connected {
                    WorkerState::Ready
                } else {
                    WorkerState::Warming
                };
                worker.last_heartbeat = Utc::now();
                registry.heartbeat(worker.clone()).await.map_err(port_error)?;
                if connected {
                    reconcile_runtime_assignments(
                        &assignments,
                        media_pipeline.as_ref(),
                        worker_id,
                    )
                    .await
                    .map_err(port_error)?;
                }
            }
            _ = heartbeat.tick() => {
                worker.state = if *connection_status.borrow() {
                    WorkerState::Ready
                } else {
                    WorkerState::Warming
                };
                worker.last_heartbeat = Utc::now();
                registry.heartbeat(worker.clone()).await.map_err(port_error)?;
            }
        }
    }

    control_task.abort();
    let _ = control_task.await;
    worker.state = WorkerState::Offline;
    worker.last_heartbeat = Utc::now();
    registry.heartbeat(worker).await.map_err(port_error)?;
    Ok(())
}

async fn reconcile_runtime_assignments(
    assignments: &PgAssignments,
    media_pipeline: &dyn MediaPipeline,
    worker_id: Uuid,
) -> Result<(), arc_application::PortError> {
    for snapshot in assignments
        .list_runtime_for_worker(worker_id)
        .await?
    {
        let Some(route) = snapshot.assignment.route else {
            continue;
        };
        let specification = PipelineSpecification {
            pipeline_id: snapshot.assignment.pipeline_id,
            assignment_generation: snapshot.assignment.generation,
            requirements: snapshot.assignment.requirements,
            configuration: route.configuration,
        };

        if snapshot.desired_state == "running" {
            media_pipeline.prepare(&specification).await?;
            media_pipeline
                .start(specification.pipeline_id, specification.assignment_generation)
                .await?;
            assignments
                .mark_started(
                    snapshot.assignment.id,
                    snapshot.assignment.generation,
                )
                .await?;
        } else if snapshot.desired_state == "stopped"
            && snapshot.observed_state == "stopping"
        {
            media_pipeline
                .stop(
                    specification.pipeline_id,
                    specification.assignment_generation,
                )
                .await?;
            assignments
                .mark_stopped(
                    snapshot.assignment.id,
                    snapshot.assignment.generation,
                )
                .await?;
        }
    }
    Ok(())
}

async fn register_or_resume(
    registry: &PgWorkerRegistry,
    worker: &mut arc_domain::Worker,
) -> Result<(), Box<dyn Error>> {
    let existing = registry
        .list_workers()
        .await
        .map_err(port_error)?
        .into_iter()
        .find(|registered| registered.id == worker.id);
    if let Some(existing) = existing {
        if existing.stable_name != worker.stable_name
            || existing.capabilities.platform != worker.capabilities.platform
            || existing.capabilities.architecture != worker.capabilities.architecture
        {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "registered worker identity does not match this worker configuration",
            )
            .into());
        }
        if existing.capacity.assigned_capacity_units > worker.capacity.total_capacity_units {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "configured capacity is below the registered assignment capacity",
            )
            .into());
        }
        worker.capacity.assigned_capacity_units = existing.capacity.assigned_capacity_units;
        info!(worker_id = %worker.id, "resuming registered worker identity");
        return Ok(());
    }
    registry
        .register(worker.clone())
        .await
        .map_err(port_error)?;
    Ok(())
}

async fn run_control_sessions(client: WorkerControlClient) {
    let mut reconnect_delay = Duration::from_secs(1);
    loop {
        match client.run_session().await {
            Ok(()) => warn!("worker control session ended; reconnecting"),
            Err(error) => warn!(
                code = error.code,
                message = %error.message,
                "worker control session failed; reconnecting"
            ),
        }

        tokio::time::sleep(reconnect_delay).await;
        reconnect_delay = if client.consume_session_was_established() {
            Duration::from_secs(1)
        } else {
            (reconnect_delay * 2).min(MAX_RECONNECT_DELAY)
        };
    }
}

fn worker_control_token() -> io::Result<String> {
    let token = if let Ok(path) = env::var("ARC_WORKER_CONTROL_TOKEN_FILE") {
        if path.trim().is_empty() {
            return Err(invalid_config(
                "ARC_WORKER_CONTROL_TOKEN_FILE",
                "must not be empty",
            ));
        }
        std::fs::read_to_string(&path)
            .map_err(|error| invalid_config(
                "ARC_WORKER_CONTROL_TOKEN_FILE",
                &format!("could not read secret file: {error}"),
            ))?
            .trim()
            .to_owned()
    } else {
        required_env("ARC_WORKER_CONTROL_TOKEN")?
    };
    if token.len() != 64
        || !token.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(invalid_config(
            "ARC_WORKER_CONTROL_TOKEN",
            "must be a 64-character hexadecimal token",
        ));
    }
    Ok(token)
}

fn optional_u64_env(name: &str) -> io::Result<Option<u64>> {
    let Some(value) = env::var(name).ok() else {
        return Ok(None);
    };
    if value.trim().is_empty() {
        return Ok(None);
    }
    value
        .trim()
        .parse::<u64>()
        .map(Some)
        .map_err(|error| invalid_config(name, error))
}

fn optional_ratio_env(name: &str) -> io::Result<Option<f32>> {
    let Some(value) = env::var(name).ok() else {
        return Ok(None);
    };
    if value.trim().is_empty() {
        return Ok(None);
    }
    let ratio = value
        .trim()
        .parse::<f32>()
        .map_err(|error| invalid_config(name, error))?;
    if !(0.0..=1.0).contains(&ratio) {
        return Err(invalid_config(name, "must be between 0.0 and 1.0"));
    }
    Ok(Some(ratio))
}

fn native_architecture() -> io::Result<Architecture> {
    match env::consts::ARCH {
        "x86_64" => Ok(Architecture::X86_64),
        "aarch64" => Ok(Architecture::Aarch64),
        _ => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("unsupported worker architecture: {}", env::consts::ARCH),
        )),
    }
}

fn native_platform() -> io::Result<Platform> {
    match env::consts::OS {
        "linux" => Ok(Platform::Linux),
        "windows" => Ok(Platform::Windows),
        _ => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("unsupported worker platform: {}", env::consts::OS),
        )),
    }
}

async fn shutdown_signal() -> io::Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result,
            _ = terminate.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await
    }
}

fn required_env(name: &str) -> io::Result<String> {
    env::var(name)
        .map_err(|_| invalid_config(name, "must be set"))
        .and_then(|value| {
            if value.trim().is_empty() {
                Err(invalid_config(name, "must not be empty"))
            } else {
                Ok(value)
            }
        })
}

fn invalid_config(name: &str, error: impl std::fmt::Display) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("invalid {name}: {error}"),
    )
}

fn port_error(error: arc_application::PortError) -> io::Error {
    io::Error::other(format!("{}: {}", error.code, error.message))
}
