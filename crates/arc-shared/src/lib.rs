use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const WORKER_PROTOCOL_VERSION: u16 = 2;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerCommandEnvelope<T> {
    pub protocol_version: u16,
    pub command_id: Uuid,
    pub worker_id: Uuid,
    pub pipeline_id: Uuid,
    pub assignment_id: Uuid,
    pub generation: i64,
    pub payload: T,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerSessionMessageKind {
    Hello,
    Welcome,
    CommandAck,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerSessionHello {
    pub kind: WorkerSessionMessageKind,
    pub protocol_version: u16,
    pub worker_id: Uuid,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerSessionWelcome {
    pub kind: WorkerSessionMessageKind,
    pub protocol_version: u16,
    pub worker_id: Uuid,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerCommandOutcome {
    Completed,
    Stale,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerCommandAcknowledgement {
    pub kind: WorkerSessionMessageKind,
    pub protocol_version: u16,
    pub worker_id: Uuid,
    pub command_id: Uuid,
    pub outcome: WorkerCommandOutcome,
    pub current_generation: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

impl WorkerCommandAcknowledgement {
    pub fn validate_for_worker(
        &self,
        authenticated_worker_id: Uuid,
    ) -> Result<(), WorkerSessionValidationError> {
        if self.kind != WorkerSessionMessageKind::CommandAck {
            return Err(WorkerSessionValidationError::InvalidMessageKind);
        }
        if self.protocol_version != WORKER_PROTOCOL_VERSION {
            return Err(WorkerSessionValidationError::UnsupportedProtocolVersion);
        }
        if self.command_id.is_nil() {
            return Err(WorkerSessionValidationError::InvalidCommandIdentity);
        }
        if self.worker_id.is_nil() || self.worker_id != authenticated_worker_id {
            return Err(WorkerSessionValidationError::InvalidWorkerIdentity);
        }
        match (self.outcome, self.current_generation) {
            (WorkerCommandOutcome::Completed, None)
                if self.error_code.is_none() && self.error_message.is_none() => {}
            (WorkerCommandOutcome::Stale, Some(generation))
                if generation > 0 && self.error_code.is_none() && self.error_message.is_none() => {}
            (WorkerCommandOutcome::Failed, None)
                if self
                    .error_code
                    .as_ref()
                    .is_some_and(|code| !code.is_empty() && code.len() <= 128)
                    && self
                        .error_message
                        .as_ref()
                        .is_some_and(|message| !message.is_empty() && message.len() <= 4096) => {}
            _ => return Err(WorkerSessionValidationError::InvalidCommandIdentity),
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerSessionValidationError {
    InvalidMessageKind,
    UnsupportedProtocolVersion,
    InvalidWorkerIdentity,
    InvalidCommandIdentity,
}

impl WorkerSessionHello {
    pub fn validate_for_worker(
        &self,
        authenticated_worker_id: Uuid,
    ) -> Result<(), WorkerSessionValidationError> {
        if self.kind != WorkerSessionMessageKind::Hello {
            return Err(WorkerSessionValidationError::InvalidMessageKind);
        }
        if self.protocol_version != WORKER_PROTOCOL_VERSION {
            return Err(WorkerSessionValidationError::UnsupportedProtocolVersion);
        }
        if self.worker_id.is_nil() || self.worker_id != authenticated_worker_id {
            return Err(WorkerSessionValidationError::InvalidWorkerIdentity);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerCommandValidationError {
    UnsupportedProtocolVersion,
    InvalidIdentity,
    InvalidGeneration,
    WorkerIdentityMismatch,
}

impl<T> WorkerCommandEnvelope<T> {
    pub fn validate_for_worker(
        &self,
        authenticated_worker_id: Uuid,
    ) -> Result<(), WorkerCommandValidationError> {
        if self.protocol_version != WORKER_PROTOCOL_VERSION {
            return Err(WorkerCommandValidationError::UnsupportedProtocolVersion);
        }
        if self.command_id.is_nil()
            || self.worker_id.is_nil()
            || self.pipeline_id.is_nil()
            || self.assignment_id.is_nil()
        {
            return Err(WorkerCommandValidationError::InvalidIdentity);
        }
        if self.generation <= 0 {
            return Err(WorkerCommandValidationError::InvalidGeneration);
        }
        if self.worker_id != authenticated_worker_id {
            return Err(WorkerCommandValidationError::WorkerIdentityMismatch);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerEventEnvelope<T> {
    pub protocol_version: u16,
    pub event_id: Uuid,
    pub worker_id: Uuid,
    pub assignment_id: Option<Uuid>,
    pub generation: Option<i64>,
    pub payload: T,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn worker_command_envelope_has_explicit_protocol_version() {
        let value = serde_json::to_value(WorkerCommandEnvelope {
            protocol_version: WORKER_PROTOCOL_VERSION,
            command_id: Uuid::nil(),
            worker_id: Uuid::nil(),
            pipeline_id: Uuid::nil(),
            assignment_id: Uuid::nil(),
            generation: 1,
            payload: json!({ "kind": "stop" }),
        })
        .expect("envelope serializes");

        assert_eq!(value["protocolVersion"], WORKER_PROTOCOL_VERSION);
        assert_eq!(value["generation"], 1);
        assert!(value.get("pipelineId").is_some());
        assert!(value.get("workerId").is_some());
    }

    #[test]
    fn worker_session_hello_requires_the_authenticated_identity_and_protocol() {
        let worker_id = Uuid::new_v4();
        let hello = WorkerSessionHello {
            kind: WorkerSessionMessageKind::Hello,
            protocol_version: WORKER_PROTOCOL_VERSION,
            worker_id,
        };
        assert_eq!(hello.validate_for_worker(worker_id), Ok(()));
        assert_eq!(
            hello.validate_for_worker(Uuid::new_v4()),
            Err(WorkerSessionValidationError::InvalidWorkerIdentity)
        );
    }

    #[test]
    fn worker_command_acknowledgement_is_correlated_to_the_authenticated_session() {
        let worker_id = Uuid::new_v4();
        let acknowledgement = WorkerCommandAcknowledgement {
            kind: WorkerSessionMessageKind::CommandAck,
            protocol_version: WORKER_PROTOCOL_VERSION,
            worker_id,
            command_id: Uuid::new_v4(),
            outcome: WorkerCommandOutcome::Completed,
            current_generation: None,
            error_code: None,
            error_message: None,
        };
        assert_eq!(acknowledgement.validate_for_worker(worker_id), Ok(()));
        assert_eq!(
            acknowledgement.validate_for_worker(Uuid::new_v4()),
            Err(WorkerSessionValidationError::InvalidWorkerIdentity)
        );

        let value = serde_json::to_value(acknowledgement).expect("ack serializes");
        assert_eq!(value["kind"], "command_ack");
        assert_eq!(value["outcome"], "completed");
        assert!(value.get("commandId").is_some());
    }

    #[test]
    fn failed_worker_acknowledgement_requires_bounded_error_details() {
        let worker_id = Uuid::new_v4();
        let acknowledgement = WorkerCommandAcknowledgement {
            kind: WorkerSessionMessageKind::CommandAck,
            protocol_version: WORKER_PROTOCOL_VERSION,
            worker_id,
            command_id: Uuid::new_v4(),
            outcome: WorkerCommandOutcome::Failed,
            current_generation: None,
            error_code: Some("media_start_failed".to_owned()),
            error_message: Some("pipeline could not enter playing state".to_owned()),
        };
        assert_eq!(acknowledgement.validate_for_worker(worker_id), Ok(()));

        let mut invalid = acknowledgement;
        invalid.error_message = None;
        assert_eq!(
            invalid.validate_for_worker(worker_id),
            Err(WorkerSessionValidationError::InvalidCommandIdentity)
        );
    }
}
