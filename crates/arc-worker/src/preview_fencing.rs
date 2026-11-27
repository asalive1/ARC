use std::{sync::Arc, time::Duration};

use arc_application::{PortError, PortResult};
use arc_infrastructure::{capacity_reservations::PgCapacityReservations, postgres::PgPool};
use arc_worker::{PreviewSessionFinalizer, WorkerControlClient, WorkerProcessFence};
use uuid::Uuid;

pub struct PreviewFenceRuntime {
    _fence: WorkerProcessFence,
    incarnation: Uuid,
    finalizer: Arc<PgPreviewSessionFinalizer>,
}

struct PgPreviewSessionFinalizer {
    reservations: PgCapacityReservations,
}

const FINALIZATION_TIMEOUT: Duration = Duration::from_secs(10);

#[async_trait::async_trait]
impl PreviewSessionFinalizer for PgPreviewSessionFinalizer {
    async fn finish(&self, worker_id: Uuid, session_id: Uuid) -> PortResult<()> {
        tokio::time::timeout(
            FINALIZATION_TIMEOUT,
            self.reservations
                .finish_preview_session(worker_id, session_id),
        )
        .await
        .map_err(|_| PortError {
            code: "preview_finalization_timeout",
            message: "preview session finalization timed out".to_owned(),
        })?
        .map(|_| ())
    }
}

impl PreviewFenceRuntime {
    pub async fn acquire(
        pool: PgPool,
        worker_id: Uuid,
        fence: WorkerProcessFence,
        allow_initial_enrollment: bool,
    ) -> PortResult<Self> {
        let incarnation = Uuid::new_v4();
        let reservations = PgCapacityReservations::new(pool);
        reservations
            .recover_fenced_previews(worker_id, incarnation, fence.identity(), |previous| {
                match previous {
                    Some(previous) if fence.previous_process_terminated(previous)? => Ok(()),
                    Some(_) => Err(PortError {
                        code: "worker_fence_previous_alive",
                        message: "the previous worker process has not exited".to_owned(),
                    }),
                    None if allow_initial_enrollment => Ok(()),
                    None => Err(PortError {
                        code: "worker_fence_enrollment_required",
                        message: "initial enrollment requires verification that unfenced workers are stopped"
                            .to_owned(),
                    }),
                }
            })
            .await?;
        Ok(Self {
            _fence: fence,
            incarnation,
            finalizer: Arc::new(PgPreviewSessionFinalizer { reservations }),
        })
    }

    pub fn configure_client(&self, client: WorkerControlClient) -> PortResult<WorkerControlClient> {
        Ok(client
            .with_process_incarnation(self.incarnation)?
            .with_preview_finalizer(self.finalizer.clone()))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::{BufRead, BufReader},
        path::Path,
        process::{Command, Stdio},
        time::Duration,
    };

    use arc_application::WorkerRegistry;
    use arc_domain::{Architecture, Platform, WorkerCapabilities};
    use arc_infrastructure::{
        postgres::{connect_postgres, migrate},
        worker_registry::PgWorkerRegistry,
    };
    use arc_worker::local_worker;

    use super::*;

    #[tokio::test]
    #[ignore = "helper for the host-process and PostgreSQL recovery test"]
    async fn fenced_worker_child() {
        let pool = connect_postgres(&std::env::var("DATABASE_URL").unwrap())
            .await
            .unwrap();
        let worker_id = std::env::var("ARC_TEST_FENCE_WORKER")
            .unwrap()
            .parse()
            .unwrap();
        let lock = std::env::var("ARC_TEST_FENCE_LOCK").unwrap();
        let host_id = std::env::var("ARC_TEST_FENCE_HOST_ID").unwrap();
        let host_proc = std::env::var("ARC_TEST_HOST_PROC").unwrap();
        let fence = WorkerProcessFence::acquire(
            Path::new(&lock),
            Path::new(&host_proc),
            Path::new(&host_id),
        )
        .unwrap();
        let _runtime = PreviewFenceRuntime::acquire(pool, worker_id, fence, true)
            .await
            .unwrap();
        println!("FENCED_WORKER_READY");
        tokio::time::sleep(Duration::from_secs(60)).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires an isolated PostgreSQL database and a host proc mount"]
    async fn fencing_enrolls_and_recovers_only_after_process_exit() {
        let pool = connect_postgres(&std::env::var("DATABASE_URL").unwrap())
            .await
            .unwrap();
        migrate(&pool).await.unwrap();
        let worker_id = Uuid::new_v4();
        let mut worker = local_worker(
            worker_id,
            format!("fenced-worker-{worker_id}"),
            "test",
            WorkerCapabilities {
                platform: Some(Platform::Linux),
                architecture: Some(Architecture::X86_64),
                ..WorkerCapabilities::default()
            },
        );
        worker.capacity.total_capacity_units = 2;
        PgWorkerRegistry::new(pool.clone())
            .register(worker)
            .await
            .unwrap();
        let directory = std::env::temp_dir();
        let lock = directory.join(format!("arc-preview-lock-{worker_id}"));
        let host_id = directory.join(format!("arc-preview-host-{worker_id}"));
        let host_proc = std::env::var("ARC_TEST_HOST_PROC").unwrap();
        fs::write(&lock, b"").unwrap();
        fs::write(&host_id, Uuid::new_v4().to_string()).unwrap();

        let fence = WorkerProcessFence::acquire(&lock, Path::new(&host_proc), &host_id).unwrap();
        let error = PreviewFenceRuntime::acquire(pool.clone(), worker_id, fence, false)
            .await
            .err()
            .expect("unverified initial enrollment must fail");
        assert_eq!(error.code, "worker_fence_enrollment_required");

        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "preview_fencing::tests::fenced_worker_child",
                "--nocapture",
            ])
            .env("ARC_TEST_FENCE_WORKER", worker_id.to_string())
            .env("ARC_TEST_FENCE_LOCK", &lock)
            .env("ARC_TEST_FENCE_HOST_ID", &host_id)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let output = child.stdout.take().unwrap();
        let readiness = tokio::task::spawn_blocking(move || {
            let mut output = BufReader::new(output);
            loop {
                let mut line = String::new();
                assert_ne!(
                    output.read_line(&mut line).unwrap(),
                    0,
                    "worker child exited"
                );
                if line.contains("FENCED_WORKER_READY") {
                    break;
                }
            }
        });
        match tokio::time::timeout(Duration::from_secs(10), readiness).await {
            Ok(result) => result.unwrap(),
            Err(_) => {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("worker child did not become ready");
            }
        }
        assert_eq!(
            WorkerProcessFence::acquire(&lock, Path::new(&host_proc), &host_id)
                .err()
                .unwrap()
                .code,
            "worker_fence_unavailable"
        );
        child.kill().unwrap();
        child.wait().unwrap();
        let fence = WorkerProcessFence::acquire(&lock, Path::new(&host_proc), &host_id).unwrap();
        let runtime = PreviewFenceRuntime::acquire(pool.clone(), worker_id, fence, false)
            .await
            .unwrap();
        runtime
            .finalizer
            .finish(worker_id, Uuid::new_v4())
            .await
            .unwrap();
        drop(runtime);

        let fence = WorkerProcessFence::acquire(&lock, Path::new(&host_proc), &host_id).unwrap();
        let error = PreviewFenceRuntime::acquire(pool, worker_id, fence, false)
            .await
            .err()
            .expect("a still-live previous process must not be recovered");
        assert_eq!(error.code, "worker_fence_previous_alive");
        fs::remove_file(lock).unwrap();
        fs::remove_file(host_id).unwrap();
    }
}
