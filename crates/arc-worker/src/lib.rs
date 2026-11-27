use arc_application::{PortError, PortResult};
use arc_domain::{Worker, WorkerCapabilities, WorkerState};
use arc_shared::{WorkerCommandOutcome, WorkerCommandValidationError, WorkerSessionMessageKind};
use chrono::Utc;
use uuid::Uuid;

use std::future::Future;

mod control_client;
mod preview;
#[cfg(target_os = "linux")]
mod process_fence;
pub use control_client::{
    execute_pipeline_command, PreviewSessionFinalizer, WorkerCommand, WorkerControlClient,
    WorkerControlClientConfig, WorkerPreviewControl, WorkerPreviewEventKind, WorkerPreviewFinished,
    MAX_PREVIEW_ERROR_CODE_BYTES, MAX_PREVIEW_ERROR_MESSAGE_BYTES,
};
pub use preview::{relay_preview, send_preview_over_websocket, PreviewEnd, PreviewRelayReport};
#[cfg(target_os = "linux")]
pub use process_fence::{ProcessFenceIdentity, WorkerProcessFence};

pub use arc_application::{
    BackupProvider, CapacityManager, DestinationAdapter, EventPublisher, MediaPipeline,
    NetworkInterfaceProvider, PipelineSpecification, SecretProvider, SignalExtractor,
    SignalGenerator, SourceAdapter, StateRepository, WorkerRegistry, WorkerScheduler,
};
pub use arc_shared::{
    WorkerCommandAcknowledgement, WorkerCommandEnvelope, WorkerEventEnvelope,
    WORKER_PROTOCOL_VERSION,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandDispatchResult {
    Applied,
    Duplicate,
    Stale { current_generation: i64 },
}

pub use arc_application::{AssignmentGenerationStore, CommandFenceDecision, WorkerCommandIdentity};

pub async fn dispatch_if_fresh<S, T, F, Fut>(
    store: &S,
    authenticated_worker_id: Uuid,
    command: WorkerCommandEnvelope<T>,
    handler: F,
) -> PortResult<CommandDispatchResult>
where
    S: AssignmentGenerationStore + ?Sized,
    F: FnOnce(WorkerCommandEnvelope<T>) -> Fut,
    Fut: Future<Output = PortResult<()>>,
{
    command
        .validate_for_worker(authenticated_worker_id)
        .map_err(command_validation_error)?;
    let identity = WorkerCommandIdentity {
        command_id: command.command_id,
        worker_id: command.worker_id,
        pipeline_id: command.pipeline_id,
        assignment_id: command.assignment_id,
        generation: command.generation,
    };
    match store.begin_command(identity).await? {
        CommandFenceDecision::Duplicate => Ok(CommandDispatchResult::Duplicate),
        CommandFenceDecision::Stale { current_generation } => {
            Ok(CommandDispatchResult::Stale { current_generation })
        }
        CommandFenceDecision::Process => {
            let command_id = command.command_id;
            handler(command).await?;
            store.complete_command(command_id).await?;
            Ok(CommandDispatchResult::Applied)
        }
    }
}

pub async fn receive_worker_command<S, T, F, Fut>(
    store: &S,
    authenticated_worker_id: Uuid,
    command: WorkerCommandEnvelope<T>,
    handler: F,
) -> PortResult<WorkerCommandAcknowledgement>
where
    S: AssignmentGenerationStore + ?Sized,
    F: FnOnce(WorkerCommandEnvelope<T>) -> Fut,
    Fut: Future<Output = PortResult<()>>,
{
    let worker_id = command.worker_id;
    let command_id = command.command_id;
    command
        .validate_for_worker(authenticated_worker_id)
        .map_err(command_validation_error)?;
    let identity = WorkerCommandIdentity {
        command_id,
        worker_id,
        pipeline_id: command.pipeline_id,
        assignment_id: command.assignment_id,
        generation: command.generation,
    };
    let (outcome, current_generation, error_code, error_message) =
        match store.begin_command(identity).await? {
            CommandFenceDecision::Duplicate => (WorkerCommandOutcome::Completed, None, None, None),
            CommandFenceDecision::Stale { current_generation } => (
                WorkerCommandOutcome::Stale,
                Some(current_generation),
                None,
                None,
            ),
            CommandFenceDecision::Process => match handler(command).await {
                Ok(()) => {
                    store.complete_command(command_id).await?;
                    (WorkerCommandOutcome::Completed, None, None, None)
                }
                Err(error) => (
                    WorkerCommandOutcome::Failed,
                    None,
                    Some(error.code.to_owned()),
                    Some(error.message.chars().take(512).collect()),
                ),
            },
        };
    Ok(WorkerCommandAcknowledgement {
        kind: WorkerSessionMessageKind::CommandAck,
        protocol_version: WORKER_PROTOCOL_VERSION,
        worker_id,
        command_id,
        outcome,
        current_generation,
        error_code,
        error_message,
    })
}

fn command_validation_error(error: WorkerCommandValidationError) -> PortError {
    PortError {
        code: "invalid_worker_command",
        message: format!("{error:?}"),
    }
}

pub fn local_worker(
    id: Uuid,
    stable_name: impl Into<String>,
    version: impl Into<String>,
    capabilities: WorkerCapabilities,
) -> Worker {
    Worker {
        id,
        stable_name: stable_name.into(),
        version: version.into(),
        state: WorkerState::Registering,
        capabilities,
        capacity: arc_domain::WorkerCapacity {
            total_capacity_units: 0,
            assigned_capacity_units: 0,
            cpu_utilization_ratio: None,
            memory_utilization_ratio: None,
            ingress_bits_per_second: None,
            egress_bits_per_second: None,
        },
        last_heartbeat: Utc::now(),
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Mutex};

    use super::*;

    #[derive(Default)]
    struct TestGenerationStore {
        pipelines: Mutex<HashMap<Uuid, StoredPipeline>>,
    }

    #[derive(Default)]
    struct StoredPipeline {
        generation: i64,
        assignment_id: Uuid,
        commands: HashMap<Uuid, bool>,
    }

    #[async_trait::async_trait]
    impl AssignmentGenerationStore for TestGenerationStore {
        async fn begin_command(
            &self,
            command: WorkerCommandIdentity,
        ) -> PortResult<CommandFenceDecision> {
            let mut pipelines = self.pipelines.lock().unwrap();
            let state = pipelines.entry(command.pipeline_id).or_default();
            if command.generation < state.generation {
                return Ok(CommandFenceDecision::Stale {
                    current_generation: state.generation,
                });
            }
            if command.generation == state.generation
                && !state.assignment_id.is_nil()
                && command.assignment_id != state.assignment_id
            {
                return Err(PortError {
                    code: "assignment_generation_conflict",
                    message: "one generation cannot identify multiple assignments".to_owned(),
                });
            }
            if command.generation > state.generation {
                state.generation = command.generation;
                state.assignment_id = command.assignment_id;
                state.commands.clear();
            }
            match state.commands.get(&command.command_id) {
                Some(true) => Ok(CommandFenceDecision::Duplicate),
                Some(false) => Ok(CommandFenceDecision::Process),
                None => {
                    state.commands.insert(command.command_id, false);
                    Ok(CommandFenceDecision::Process)
                }
            }
        }

        async fn complete_command(&self, command_id: Uuid) -> PortResult<()> {
            for pipeline in self.pipelines.lock().unwrap().values_mut() {
                if let Some(completed) = pipeline.commands.get_mut(&command_id) {
                    *completed = true;
                    return Ok(());
                }
            }
            Err(PortError {
                code: "command_not_found",
                message: "command was not recorded before completion".to_owned(),
            })
        }
    }

    fn command(
        worker_id: Uuid,
        pipeline_id: Uuid,
        assignment_id: Uuid,
        generation: i64,
        command_id: Uuid,
    ) -> WorkerCommandEnvelope<serde_json::Value> {
        WorkerCommandEnvelope {
            protocol_version: WORKER_PROTOCOL_VERSION,
            command_id,
            worker_id,
            pipeline_id,
            assignment_id,
            generation,
            payload: serde_json::json!({ "kind": "start" }),
        }
    }

    #[tokio::test]
    async fn command_dispatch_fences_stale_generations_and_deduplicates_completed_commands() {
        let store = TestGenerationStore::default();
        let worker_id = Uuid::new_v4();
        let pipeline_id = Uuid::new_v4();
        let first_assignment = Uuid::new_v4();
        let first_command = Uuid::new_v4();
        let applied = Mutex::new(0);

        assert_eq!(
            dispatch_if_fresh(
                &store,
                worker_id,
                command(worker_id, pipeline_id, first_assignment, 4, first_command),
                |_| async {
                    *applied.lock().unwrap() += 1;
                    Ok(())
                },
            )
            .await
            .unwrap(),
            CommandDispatchResult::Applied
        );
        assert_eq!(
            dispatch_if_fresh(
                &store,
                worker_id,
                command(worker_id, pipeline_id, first_assignment, 4, first_command),
                |_| async {
                    *applied.lock().unwrap() += 1;
                    Ok(())
                },
            )
            .await
            .unwrap(),
            CommandDispatchResult::Duplicate
        );
        assert_eq!(
            dispatch_if_fresh(
                &store,
                worker_id,
                command(worker_id, pipeline_id, Uuid::new_v4(), 3, Uuid::new_v4()),
                |_| async {
                    *applied.lock().unwrap() += 1;
                    Ok(())
                },
            )
            .await
            .unwrap(),
            CommandDispatchResult::Stale {
                current_generation: 4
            }
        );
        assert_eq!(*applied.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn worker_receiver_acknowledges_only_completed_or_rejected_work() {
        let store = TestGenerationStore::default();
        let worker_id = Uuid::new_v4();
        let pipeline_id = Uuid::new_v4();
        let assignment_id = Uuid::new_v4();
        let command_id = Uuid::new_v4();
        let handled = Mutex::new(0);

        let acknowledgement = receive_worker_command(
            &store,
            worker_id,
            command(worker_id, pipeline_id, assignment_id, 4, command_id),
            |_| async {
                *handled.lock().unwrap() += 1;
                Ok(())
            },
        )
        .await
        .expect("command completes");
        assert_eq!(acknowledgement.outcome, WorkerCommandOutcome::Completed);
        assert_eq!(acknowledgement.current_generation, None);

        let duplicate = receive_worker_command(
            &store,
            worker_id,
            command(worker_id, pipeline_id, assignment_id, 4, command_id),
            |_| async {
                *handled.lock().unwrap() += 1;
                Ok(())
            },
        )
        .await
        .expect("completed retry is acknowledged");
        assert_eq!(duplicate.outcome, WorkerCommandOutcome::Completed);

        let stale = receive_worker_command(
            &store,
            worker_id,
            command(worker_id, pipeline_id, Uuid::new_v4(), 3, Uuid::new_v4()),
            |_| async {
                *handled.lock().unwrap() += 1;
                Ok(())
            },
        )
        .await
        .expect("stale command is rejected");
        assert_eq!(stale.outcome, WorkerCommandOutcome::Stale);
        assert_eq!(stale.current_generation, Some(4));
        assert_eq!(*handled.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn worker_receiver_acknowledges_execution_failure_without_completing_command() {
        let store = TestGenerationStore::default();
        let worker_id = Uuid::new_v4();
        let pipeline_id = Uuid::new_v4();
        let assignment_id = Uuid::new_v4();
        let command_id = Uuid::new_v4();
        let acknowledgement = receive_worker_command(
            &store,
            worker_id,
            command(worker_id, pipeline_id, assignment_id, 1, command_id),
            |_| async {
                Err(PortError {
                    code: "media_start_failed",
                    message: "pipeline failed to start".to_owned(),
                })
            },
        )
        .await
        .expect("execution failure is acknowledged");

        assert_eq!(acknowledgement.outcome, WorkerCommandOutcome::Failed);
        assert_eq!(acknowledgement.current_generation, None);
        assert_eq!(
            acknowledgement.error_code.as_deref(),
            Some("media_start_failed")
        );
        assert_eq!(
            acknowledgement.error_message.as_deref(),
            Some("pipeline failed to start")
        );

        let retried = receive_worker_command(
            &store,
            worker_id,
            command(worker_id, pipeline_id, assignment_id, 1, command_id),
            |_| async { Ok(()) },
        )
        .await
        .expect("incomplete command can be retried");
        assert_eq!(retried.outcome, WorkerCommandOutcome::Completed);
    }

    #[tokio::test]
    async fn rejected_worker_identity_never_reaches_the_command_handler() {
        let store = TestGenerationStore::default();
        let worker_id = Uuid::new_v4();
        let wrong_worker_id = Uuid::new_v4();
        let called = Mutex::new(false);
        let result = dispatch_if_fresh(
            &store,
            worker_id,
            command(
                wrong_worker_id,
                Uuid::new_v4(),
                Uuid::new_v4(),
                1,
                Uuid::new_v4(),
            ),
            |_| async {
                *called.lock().unwrap() = true;
                Ok(())
            },
        )
        .await;
        assert_eq!(result.unwrap_err().code, "invalid_worker_command");
        assert!(!*called.lock().unwrap());
    }

    #[test]
    fn worker_command_envelope_requires_a_positive_generation() {
        let worker_id = Uuid::new_v4();
        let invalid = WorkerCommandEnvelope {
            protocol_version: WORKER_PROTOCOL_VERSION,
            command_id: Uuid::new_v4(),
            worker_id,
            pipeline_id: Uuid::new_v4(),
            assignment_id: Uuid::new_v4(),
            generation: 0,
            payload: (),
        };
        assert_eq!(
            invalid.validate_for_worker(worker_id),
            Err(WorkerCommandValidationError::InvalidGeneration)
        );
    }
}
