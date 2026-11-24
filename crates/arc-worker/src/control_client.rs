use std::{
    net::IpAddr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use arc_application::{
    AssignmentGenerationStore, MediaPipeline, PipelineSpecification, PortError, PortResult,
};
use arc_shared::{
    WorkerCommandEnvelope, WorkerSessionHello, WorkerSessionMessageKind, WorkerSessionWelcome,
    WORKER_PROTOCOL_VERSION,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{
        client::IntoClientRequest, http::HeaderValue, protocol::WebSocketConfig, Message,
    },
};
use uuid::Uuid;

use crate::receive_worker_command;

const MAX_WORKER_CONTROL_MESSAGE_SIZE: usize = 64 * 1024;
const WORKER_CONTROL_HELLO_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkerCommand {
    Start {
        specification: PipelineSpecification,
    },
    Stop,
}

#[derive(Clone, Debug)]
pub struct WorkerControlClientConfig {
    pub endpoint: String,
    pub worker_id: Uuid,
    pub bearer_token: String,
    pub allow_insecure_loopback: bool,
}

#[derive(Clone)]
pub struct WorkerControlClient {
    config: WorkerControlClientConfig,
    generation_store: Arc<dyn AssignmentGenerationStore>,
    media_pipeline: Arc<dyn MediaPipeline>,
    connection_status: watch::Sender<bool>,
    session_was_established: Arc<AtomicBool>,
}

impl WorkerControlClient {
    pub fn new(
        config: WorkerControlClientConfig,
        generation_store: Arc<dyn AssignmentGenerationStore>,
        media_pipeline: Arc<dyn MediaPipeline>,
    ) -> PortResult<Self> {
        if config.worker_id.is_nil()
            || config.bearer_token.is_empty()
            || config
                .bearer_token
                .bytes()
                .any(|byte| byte.is_ascii_whitespace())
        {
            return Err(worker_client_error(
                "invalid_worker_control_config",
                "worker identity and bearer credential must be valid",
            ));
        }
        let (connection_status, _) = watch::channel(false);
        let client = Self {
            config,
            generation_store,
            media_pipeline,
            connection_status,
            session_was_established: Arc::new(AtomicBool::new(false)),
        };
        client.websocket_request()?;
        Ok(client)
    }

    pub async fn run_session(&self) -> PortResult<()> {
        let _status_guard = ConnectionStatusGuard(self.connection_status.clone());
        let request = self.websocket_request()?;
        let mut websocket_config = WebSocketConfig::default();
        websocket_config.max_message_size = Some(MAX_WORKER_CONTROL_MESSAGE_SIZE);
        websocket_config.max_frame_size = Some(MAX_WORKER_CONTROL_MESSAGE_SIZE);
        let (mut socket, _) = connect_async_with_config(request, Some(websocket_config), false)
            .await
            .map_err(|_| {
                worker_client_error(
                    "worker_control_connection_failed",
                    "worker control connection or handshake failed",
                )
            })?;

        let hello = WorkerSessionHello {
            kind: WorkerSessionMessageKind::Hello,
            protocol_version: WORKER_PROTOCOL_VERSION,
            worker_id: self.config.worker_id,
        };
        let hello = serde_json::to_string(&hello).map_err(|error| {
            worker_client_error(
                "worker_control_protocol_error",
                format!("worker hello could not be serialized: {error}"),
            )
        })?;
        socket
            .send(Message::Text(hello.into()))
            .await
            .map_err(|_| {
                worker_client_error(
                    "worker_control_connection_failed",
                    "worker hello could not be sent",
                )
            })?;

        let welcome = tokio::time::timeout(WORKER_CONTROL_HELLO_TIMEOUT, socket.next())
            .await
            .map_err(|_| {
                worker_client_error(
                    "worker_control_hello_timeout",
                    "worker control welcome was not received in time",
                )
            })?
            .ok_or_else(|| {
                worker_client_error(
                    "worker_control_connection_ended",
                    "worker control connection ended before welcome",
                )
            })?
            .map_err(|_| {
                worker_client_error(
                    "worker_control_connection_failed",
                    "worker control welcome could not be read",
                )
            })?;
        let Message::Text(welcome) = welcome else {
            return Err(worker_client_error(
                "worker_control_protocol_error",
                "worker control server did not send a text welcome",
            ));
        };
        let welcome: WorkerSessionWelcome =
            serde_json::from_str(welcome.as_str()).map_err(|error| {
                worker_client_error(
                    "worker_control_protocol_error",
                    format!("worker welcome is invalid: {error}"),
                )
            })?;
        if welcome.kind != WorkerSessionMessageKind::Welcome
            || welcome.protocol_version != WORKER_PROTOCOL_VERSION
            || welcome.worker_id != self.config.worker_id
        {
            return Err(worker_client_error(
                "worker_control_protocol_error",
                "worker welcome does not match the authenticated session",
            ));
        }
        self.connection_status.send_replace(true);
        self.session_was_established.store(true, Ordering::Release);

        while let Some(message) = socket.next().await {
            match message.map_err(|_| {
                worker_client_error(
                    "worker_control_connection_failed",
                    "worker control message could not be read",
                )
            })? {
                Message::Text(text) => {
                    let command: WorkerCommandEnvelope<WorkerCommand> =
                        serde_json::from_str(text.as_str()).map_err(|error| {
                            worker_client_error(
                                "invalid_worker_command",
                                format!("worker command is invalid: {error}"),
                            )
                        })?;
                    validate_pipeline_command(&command)?;
                    let acknowledgement = receive_worker_command(
                        self.generation_store.as_ref(),
                        self.config.worker_id,
                        command,
                        |command| {
                            let media_pipeline = Arc::clone(&self.media_pipeline);
                            async move {
                                execute_pipeline_command(media_pipeline.as_ref(), command).await
                            }
                        },
                    )
                    .await?;
                    let acknowledgement =
                        serde_json::to_string(&acknowledgement).map_err(|error| {
                            worker_client_error(
                                "worker_control_protocol_error",
                                format!("command acknowledgement could not be serialized: {error}"),
                            )
                        })?;
                    socket
                        .send(Message::Text(acknowledgement.into()))
                        .await
                        .map_err(|_| {
                            worker_client_error(
                                "worker_control_connection_failed",
                                "command acknowledgement could not be sent",
                            )
                        })?;
                }
                Message::Ping(payload) => {
                    socket.send(Message::Pong(payload)).await.map_err(|_| {
                        worker_client_error(
                            "worker_control_connection_failed",
                            "worker ping response could not be sent",
                        )
                    })?;
                }
                Message::Pong(_) => {}
                Message::Close(_) => return Ok(()),
                Message::Binary(_) => {
                    return Err(worker_client_error(
                        "worker_control_protocol_error",
                        "binary worker control messages are unsupported",
                    ));
                }
                Message::Frame(_) => {
                    return Err(worker_client_error(
                        "worker_control_protocol_error",
                        "raw WebSocket frames are unsupported",
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn connection_status(&self) -> watch::Receiver<bool> {
        self.connection_status.subscribe()
    }

    pub fn consume_session_was_established(&self) -> bool {
        self.session_was_established.swap(false, Ordering::AcqRel)
    }

    fn websocket_request(&self) -> PortResult<tokio_tungstenite::tungstenite::http::Request<()>> {
        let mut request = self
            .config
            .endpoint
            .as_str()
            .into_client_request()
            .map_err(|_| {
                worker_client_error(
                    "invalid_worker_control_config",
                    "worker control endpoint is not a valid WebSocket URL",
                )
            })?;
        let scheme = request.uri().scheme_str();
        let is_secure = scheme == Some("wss");
        let is_local_insecure = scheme == Some("ws")
            && self.config.allow_insecure_loopback
            && request
                .uri()
                .host()
                .and_then(|host| host.parse::<IpAddr>().ok())
                .is_some_and(|address| address.is_loopback());
        if !is_secure && !is_local_insecure {
            return Err(worker_client_error(
                "insecure_worker_control_endpoint",
                "worker control requires WSS except for explicitly allowed loopback tests",
            ));
        }
        let authorization = HeaderValue::from_str(&format!("Bearer {}", self.config.bearer_token))
            .map_err(|_| {
                worker_client_error(
                    "invalid_worker_control_config",
                    "worker bearer credential is not a valid HTTP header value",
                )
            })?;
        let worker_id =
            HeaderValue::from_str(&self.config.worker_id.to_string()).map_err(|_| {
                worker_client_error(
                    "invalid_worker_control_config",
                    "worker identity is not a valid HTTP header value",
                )
            })?;
        request.headers_mut().insert("authorization", authorization);
        request.headers_mut().insert("x-worker-id", worker_id);
        request.headers_mut().insert(
            "x-forwarded-proto",
            HeaderValue::from_static("https"),
        );
        Ok(request)
    }
}

struct ConnectionStatusGuard(watch::Sender<bool>);

impl Drop for ConnectionStatusGuard {
    fn drop(&mut self) {
        self.0.send_replace(false);
    }
}

pub async fn execute_pipeline_command(
    media_pipeline: &dyn MediaPipeline,
    command: WorkerCommandEnvelope<WorkerCommand>,
) -> PortResult<()> {
    validate_pipeline_command(&command)?;
    match command.payload {
        WorkerCommand::Start { specification } => {
            media_pipeline.prepare(&specification).await?;
            media_pipeline
                .start(command.pipeline_id, command.generation)
                .await
        }
        WorkerCommand::Stop => {
            media_pipeline
                .stop(command.pipeline_id, command.generation)
                .await
        }
    }
}

fn validate_pipeline_command(command: &WorkerCommandEnvelope<WorkerCommand>) -> PortResult<()> {
    if let WorkerCommand::Start { specification } = &command.payload {
        if specification.pipeline_id != command.pipeline_id
            || specification.assignment_generation != command.generation
        {
            return Err(worker_client_error(
                "invalid_pipeline_command",
                "start specification does not match its command identity",
            ));
        }
    }
    Ok(())
}

fn worker_client_error(code: &'static str, message: impl Into<String>) -> PortError {
    PortError {
        code,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeSet, HashMap},
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc, Mutex,
        },
    };

    use arc_application::{
        CommandFenceDecision, PipelineObservation, PipelineObservedState, PipelineSpecification,
        WorkerCommandIdentity,
    };
    use arc_domain::PipelineRequirements;
    use arc_shared::WorkerCommandAcknowledgement;
    use tokio::{net::TcpListener, task::JoinHandle};
    use tokio_tungstenite::{
        accept_hdr_async,
        tungstenite::{
            handshake::server::{Request, Response},
            Message,
        },
    };

    use super::*;

    #[derive(Default)]
    struct TestMediaPipeline {
        prepared: AtomicUsize,
        started: AtomicUsize,
        stopped: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl MediaPipeline for TestMediaPipeline {
        async fn prepare(&self, _: &PipelineSpecification) -> PortResult<()> {
            self.prepared.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn start(&self, _: Uuid, _: i64) -> PortResult<()> {
            self.started.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn stop(&self, _: Uuid, _: i64) -> PortResult<()> {
            self.stopped.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn observe(&self, pipeline_id: Uuid) -> PortResult<PipelineObservation> {
            Ok(PipelineObservation {
                pipeline_id,
                assignment_generation: 1,
                state: PipelineObservedState::Running,
                health_reasons: Vec::new(),
                observed_at: chrono::Utc::now(),
            })
        }
    }

    #[derive(Default)]
    struct TestGenerationStore {
        pipelines: Mutex<HashMap<Uuid, StoredPipeline>>,
    }

    #[derive(Default)]
    struct StoredPipeline {
        generation: i64,
        worker_id: Uuid,
        assignment_id: Uuid,
        commands: HashMap<Uuid, (WorkerCommandIdentity, bool)>,
    }

    #[async_trait::async_trait]
    impl AssignmentGenerationStore for TestGenerationStore {
        async fn begin_command(
            &self,
            command: WorkerCommandIdentity,
        ) -> PortResult<CommandFenceDecision> {
            let mut pipelines = self.pipelines.lock().unwrap();
            let pipeline = pipelines.entry(command.pipeline_id).or_default();
            if command.generation < pipeline.generation {
                return Ok(CommandFenceDecision::Stale {
                    current_generation: pipeline.generation,
                });
            }
            if command.generation == pipeline.generation
                && pipeline.generation > 0
                && (command.worker_id != pipeline.worker_id
                    || command.assignment_id != pipeline.assignment_id)
            {
                return Err(PortError {
                    code: "assignment_generation_conflict",
                    message: "one generation cannot identify multiple assignments".to_owned(),
                });
            }
            if command.generation > pipeline.generation {
                pipeline.generation = command.generation;
                pipeline.worker_id = command.worker_id;
                pipeline.assignment_id = command.assignment_id;
            }
            match pipeline.commands.get(&command.command_id) {
                Some((recorded, _)) if *recorded != command => Err(PortError {
                    code: "command_identity_conflict",
                    message: "command ID was already used for a different identity".to_owned(),
                }),
                Some((_, true)) => Ok(CommandFenceDecision::Duplicate),
                Some((_, false)) => Ok(CommandFenceDecision::Process),
                None => {
                    pipeline
                        .commands
                        .insert(command.command_id, (command, false));
                    Ok(CommandFenceDecision::Process)
                }
            }
        }

        async fn complete_command(&self, command_id: Uuid) -> PortResult<()> {
            for pipeline in self.pipelines.lock().unwrap().values_mut() {
                if let Some((_, completed)) = pipeline.commands.get_mut(&command_id) {
                    *completed = true;
                    return Ok(());
                }
            }
            Err(PortError {
                code: "command_not_found",
                message: "worker command was not recorded".to_owned(),
            })
        }
    }

    fn specification(pipeline_id: Uuid, generation: i64) -> PipelineSpecification {
        PipelineSpecification {
            pipeline_id,
            assignment_generation: generation,
            requirements: PipelineRequirements {
                source_adapter: None,
                destination_adapter: None,
                codecs: BTreeSet::new(),
                encoders: BTreeSet::new(),
                payload_formats: BTreeSet::new(),
                signal_parsers: BTreeSet::new(),
                signal_generators: BTreeSet::new(),
                audio_backends: BTreeSet::new(),
                hardware_acceleration: BTreeSet::new(),
                platform: None,
                architecture: None,
                capacity_units: 1,
                ingress_bits_per_second: None,
                egress_bits_per_second: None,
            },
            configuration: serde_json::json!({}),
        }
    }

    fn command(
        worker_id: Uuid,
        pipeline_id: Uuid,
        assignment_id: Uuid,
        generation: i64,
        payload: WorkerCommand,
    ) -> WorkerCommandEnvelope<WorkerCommand> {
        WorkerCommandEnvelope {
            protocol_version: WORKER_PROTOCOL_VERSION,
            command_id: Uuid::new_v4(),
            worker_id,
            pipeline_id,
            assignment_id,
            generation,
            payload,
        }
    }

    async fn receive_acknowledgement(
        socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    ) -> WorkerCommandAcknowledgement {
        let message = socket
            .next()
            .await
            .expect("worker sends acknowledgement")
            .expect("acknowledgement is readable");
        let Message::Text(message) = message else {
            panic!("expected command acknowledgement");
        };
        serde_json::from_str(message.as_str()).expect("acknowledgement is valid JSON")
    }

    async fn send_command(
        socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
        command: WorkerCommandEnvelope<WorkerCommand>,
    ) -> WorkerCommandAcknowledgement {
        socket
            .send(Message::Text(
                serde_json::to_string(&command)
                    .expect("command serializes")
                    .into(),
            ))
            .await
            .expect("server sends command");
        receive_acknowledgement(socket).await
    }

    async fn worker_test_server(
        worker_id: Uuid,
        pipeline_id: Uuid,
        assignment_id: Uuid,
        store: Arc<TestGenerationStore>,
    ) -> (String, JoinHandle<Vec<WorkerCommandAcknowledgement>>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test server binds");
        let address = listener.local_addr().expect("server address exists");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("worker connects");
            let mut socket = accept_hdr_async(stream, |request: &Request, response: Response| {
                assert_eq!(
                    request.headers()["x-worker-id"].to_str().unwrap(),
                    worker_id.to_string()
                );
                assert_eq!(
                    request.headers()["authorization"].to_str().unwrap(),
                    "Bearer local-worker-token"
                );
                Ok(response)
            })
            .await
            .expect("worker WebSocket handshake succeeds");
            let hello = socket.next().await.expect("worker sends hello").unwrap();
            let Message::Text(hello) = hello else {
                panic!("expected worker hello");
            };
            let hello: WorkerSessionHello =
                serde_json::from_str(hello.as_str()).expect("hello is valid JSON");
            let welcome = WorkerSessionWelcome {
                kind: WorkerSessionMessageKind::Welcome,
                protocol_version: WORKER_PROTOCOL_VERSION,
                worker_id: hello.worker_id,
            };
            socket
                .send(Message::Text(
                    serde_json::to_string(&welcome)
                        .expect("welcome serializes")
                        .into(),
                ))
                .await
                .expect("welcome sends");
            let mut acknowledgements = Vec::new();
            acknowledgements.push(
                send_command(
                    &mut socket,
                    command(
                        worker_id,
                        pipeline_id,
                        assignment_id,
                        1,
                        WorkerCommand::Start {
                            specification: specification(pipeline_id, 1),
                        },
                    ),
                )
                .await,
            );
            acknowledgements.push(
                send_command(
                    &mut socket,
                    command(
                        worker_id,
                        pipeline_id,
                        assignment_id,
                        1,
                        WorkerCommand::Stop,
                    ),
                )
                .await,
            );
            store
                .begin_command(WorkerCommandIdentity {
                    command_id: Uuid::new_v4(),
                    worker_id,
                    pipeline_id,
                    assignment_id: Uuid::new_v4(),
                    generation: 2,
                })
                .await
                .expect("new assignment generation is recorded");
            acknowledgements.push(
                send_command(
                    &mut socket,
                    command(
                        worker_id,
                        pipeline_id,
                        assignment_id,
                        1,
                        WorkerCommand::Start {
                            specification: specification(pipeline_id, 1),
                        },
                    ),
                )
                .await,
            );
            socket
                .close(None)
                .await
                .expect("test server closes session");
            acknowledgements
        });
        (format!("ws://{address}"), server)
    }

    fn client(
        endpoint: String,
        worker_id: Uuid,
        store: Arc<TestGenerationStore>,
        media_pipeline: Arc<TestMediaPipeline>,
        allow_insecure_loopback: bool,
    ) -> WorkerControlClient {
        WorkerControlClient::new(
            WorkerControlClientConfig {
                endpoint,
                worker_id,
                bearer_token: "local-worker-token".to_owned(),
                allow_insecure_loopback,
            },
            store,
            media_pipeline,
        )
        .expect("worker control client configuration is valid")
    }

    #[test]
    fn worker_client_rejects_plaintext_endpoints_outside_loopback() {
        let error = WorkerControlClient::new(
            WorkerControlClientConfig {
                endpoint: "ws://192.0.2.1/api/v1/workers/control".to_owned(),
                worker_id: Uuid::new_v4(),
                bearer_token: "local-worker-token".to_owned(),
                allow_insecure_loopback: true,
            },
            Arc::new(TestGenerationStore::default()),
            Arc::new(TestMediaPipeline::default()),
        )
        .err()
        .expect("non-loopback plaintext endpoint is rejected");
        assert_eq!(error.code, "insecure_worker_control_endpoint");
    }

    #[tokio::test]
    async fn start_command_rejects_a_specification_with_a_different_generation() {
        let media_pipeline = TestMediaPipeline::default();
        let pipeline_id = Uuid::new_v4();
        let error = execute_pipeline_command(
            &media_pipeline,
            command(
                Uuid::new_v4(),
                pipeline_id,
                Uuid::new_v4(),
                1,
                WorkerCommand::Start {
                    specification: specification(pipeline_id, 2),
                },
            ),
        )
        .await
        .expect_err("mismatched specification is rejected");

        assert_eq!(error.code, "invalid_pipeline_command");
        assert_eq!(media_pipeline.prepared.load(Ordering::SeqCst), 0);
        assert_eq!(media_pipeline.started.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn failed_command_handler_does_not_return_a_completed_acknowledgement() {
        let store = TestGenerationStore::default();
        let worker_id = Uuid::new_v4();
        let command = command(
            worker_id,
            Uuid::new_v4(),
            Uuid::new_v4(),
            1,
            WorkerCommand::Stop,
        );
        let identity = WorkerCommandIdentity {
            command_id: command.command_id,
            worker_id: command.worker_id,
            pipeline_id: command.pipeline_id,
            assignment_id: command.assignment_id,
            generation: command.generation,
        };
        let error = receive_worker_command(&store, worker_id, command, |_| async {
            Err(PortError {
                code: "media_pipeline_failed",
                message: "injected test failure".to_owned(),
            })
        })
        .await
        .expect_err("failed handler does not produce an acknowledgement");

        assert_eq!(error.code, "media_pipeline_failed");
        assert_eq!(
            store.begin_command(identity).await.unwrap(),
            CommandFenceDecision::Process
        );
    }

    #[tokio::test]
    async fn worker_client_runs_start_and_stop_handlers_and_acknowledges_after_completion() {
        let worker_id = Uuid::new_v4();
        let pipeline_id = Uuid::new_v4();
        let assignment_id = Uuid::new_v4();
        let store = Arc::new(TestGenerationStore::default());
        let media_pipeline = Arc::new(TestMediaPipeline::default());
        let (endpoint, server) =
            worker_test_server(worker_id, pipeline_id, assignment_id, store.clone()).await;
        let client = client(
            endpoint,
            worker_id,
            store.clone(),
            media_pipeline.clone(),
            true,
        );
        let observer = client.clone();
        let client_task = tokio::spawn(async move { client.run_session().await });
        let acknowledgements = server.await.expect("test server joins");
        client_task
            .await
            .expect("worker client task joins")
            .expect("worker session completes");
        assert!(observer.consume_session_was_established());
        assert_eq!(acknowledgements.len(), 3);
        assert_eq!(
            acknowledgements[0].outcome,
            arc_shared::WorkerCommandOutcome::Completed
        );
        assert_eq!(
            acknowledgements[1].outcome,
            arc_shared::WorkerCommandOutcome::Completed
        );
        assert_eq!(
            acknowledgements[2].outcome,
            arc_shared::WorkerCommandOutcome::Stale
        );
        assert_eq!(acknowledgements[2].current_generation, Some(2));
        assert_eq!(media_pipeline.prepared.load(Ordering::SeqCst), 1);
        assert_eq!(media_pipeline.started.load(Ordering::SeqCst), 1);
        assert_eq!(media_pipeline.stopped.load(Ordering::SeqCst), 1);
    }
}
