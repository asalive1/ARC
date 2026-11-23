use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const WORKER_PROTOCOL_VERSION: u16 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerCommandEnvelope<T> {
    pub protocol_version: u16,
    pub command_id: Uuid,
    pub assignment_id: Uuid,
    pub generation: i64,
    pub payload: T,
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
            assignment_id: Uuid::nil(),
            generation: 1,
            payload: json!({ "kind": "stop" }),
        })
        .expect("envelope serializes");

        assert_eq!(value["protocolVersion"], WORKER_PROTOCOL_VERSION);
        assert_eq!(value["generation"], 1);
    }
}
