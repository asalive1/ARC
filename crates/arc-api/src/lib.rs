use arc_domain::{
    estimate_pipeline_requirements, validate_destination_profile, validate_source_profile,
    DestinationMediaProfile, MediaProfileError, SourceMediaProfile,
};
use arc_infrastructure::catalog::{
    self, DestinationRecord, DraftValidationError, ResourceUpdateError, SourceRecord,
};
use arc_infrastructure::postgres::PgPool;
use axum::{
    extract::{Extension, Path},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

pub fn router() -> Router {
    Router::new()
        .route("/api/v1/health/live", get(liveness))
        .route("/api/v1/system/capabilities", get(capabilities))
        .route("/api/v1/routes/plan", post(plan_route))
        .route(
            "/api/v1/sources/{source_id}/validate",
            post(validate_source_draft),
        )
        .route(
            "/api/v1/destinations/{destination_id}/validate",
            post(validate_destination_draft),
        )
}

pub fn router_with_database(pool: PgPool) -> Router {
    router()
        .route("/api/v1/sources", get(list_sources).post(create_source))
        .route("/api/v1/sources/{source_id}", put(update_source))
        .route(
            "/api/v1/destinations",
            get(list_destinations).post(create_destination),
        )
        .route(
            "/api/v1/destinations/{destination_id}",
            put(update_destination),
        )
        .layer(Extension(pool))
}

async fn liveness() -> Json<Value> {
    Json(json!({ "status": "alive" }))
}

async fn capabilities() -> Json<Value> {
    Json(json!({
        "apiVersion": "1.0",
        "workers": []
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateSourceRequest {
    friendly_name: String,
    profile: SourceMediaProfile,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateDestinationRequest {
    friendly_name: String,
    profile: DestinationMediaProfile,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlanRouteRequest {
    source: SourceMediaProfile,
    destination: DestinationMediaProfile,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanRouteResponse {
    media_path: &'static str,
    estimate: RouteResourceEstimate,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RouteResourceEstimate {
    capacity_units: u32,
    ingress_payload_bits_per_second: Option<u64>,
    egress_payload_bits_per_second: Option<u64>,
}

async fn list_sources(
    Extension(pool): Extension<PgPool>,
) -> Result<Json<Vec<SourceRecord>>, ApiError> {
    catalog::list_sources(&pool)
        .await
        .map(Json)
        .map_err(|error| ApiError::database("list sources", error))
}

async fn validate_source_draft(
    Path(source_id): Path<Uuid>,
    Extension(pool): Extension<PgPool>,
) -> Result<Json<SourceRecord>, ApiError> {
    catalog::validate_source_draft(&pool, source_id)
        .await
        .map(Json)
        .map_err(ApiError::draft_validation)
}

async fn create_source(
    Extension(pool): Extension<PgPool>,
    Json(request): Json<CreateSourceRequest>,
) -> Result<(StatusCode, Json<SourceRecord>), ApiError> {
    validate_friendly_name(&request.friendly_name)?;
    validate_source_profile(&request.profile).map_err(ApiError::invalid_media_profile)?;
    let configuration =
        serde_json::to_value(&request.profile).map_err(ApiError::serialize_profile)?;

    let source = catalog::create_source(
        &pool,
        request.friendly_name.trim(),
        request.profile.adapter_id(),
        configuration,
    )
    .await
    .map_err(|error| ApiError::database("create source", error))?;

    Ok((StatusCode::CREATED, Json(source)))
}

async fn update_source(
    Path(source_id): Path<Uuid>,
    Extension(pool): Extension<PgPool>,
    Json(request): Json<CreateSourceRequest>,
) -> Result<Json<SourceRecord>, ApiError> {
    validate_friendly_name(&request.friendly_name)?;
    validate_source_profile(&request.profile).map_err(ApiError::invalid_media_profile)?;
    let configuration =
        serde_json::to_value(&request.profile).map_err(ApiError::serialize_profile)?;

    catalog::update_source(
        &pool,
        source_id,
        request.friendly_name.trim(),
        request.profile.adapter_id(),
        configuration,
    )
    .await
    .map(Json)
    .map_err(ApiError::resource_update)
}

async fn list_destinations(
    Extension(pool): Extension<PgPool>,
) -> Result<Json<Vec<DestinationRecord>>, ApiError> {
    catalog::list_destinations(&pool)
        .await
        .map(Json)
        .map_err(|error| ApiError::database("list destinations", error))
}

async fn validate_destination_draft(
    Path(destination_id): Path<Uuid>,
    Extension(pool): Extension<PgPool>,
) -> Result<Json<DestinationRecord>, ApiError> {
    catalog::validate_destination_draft(&pool, destination_id)
        .await
        .map(Json)
        .map_err(ApiError::draft_validation)
}

async fn create_destination(
    Extension(pool): Extension<PgPool>,
    Json(request): Json<CreateDestinationRequest>,
) -> Result<(StatusCode, Json<DestinationRecord>), ApiError> {
    validate_friendly_name(&request.friendly_name)?;
    validate_destination_profile(&request.profile).map_err(ApiError::invalid_media_profile)?;
    let configuration =
        serde_json::to_value(&request.profile).map_err(ApiError::serialize_profile)?;

    let destination = catalog::create_destination(
        &pool,
        request.friendly_name.trim(),
        request.profile.adapter_id(),
        configuration,
    )
    .await
    .map_err(|error| ApiError::database("create destination", error))?;

    Ok((StatusCode::CREATED, Json(destination)))
}

async fn update_destination(
    Path(destination_id): Path<Uuid>,
    Extension(pool): Extension<PgPool>,
    Json(request): Json<CreateDestinationRequest>,
) -> Result<Json<DestinationRecord>, ApiError> {
    validate_friendly_name(&request.friendly_name)?;
    validate_destination_profile(&request.profile).map_err(ApiError::invalid_media_profile)?;
    let configuration =
        serde_json::to_value(&request.profile).map_err(ApiError::serialize_profile)?;

    catalog::update_destination(
        &pool,
        destination_id,
        request.friendly_name.trim(),
        request.profile.adapter_id(),
        configuration,
    )
    .await
    .map(Json)
    .map_err(ApiError::resource_update)
}

async fn plan_route(
    Json(request): Json<PlanRouteRequest>,
) -> Result<Json<PlanRouteResponse>, ApiError> {
    let estimate = estimate_pipeline_requirements(&request.source, &request.destination)
        .map_err(ApiError::invalid_media_profile)?;
    Ok(Json(PlanRouteResponse {
        media_path: estimate.media_path.as_str(),
        estimate: RouteResourceEstimate {
            capacity_units: estimate.requirements.capacity_units,
            ingress_payload_bits_per_second: estimate.ingress_payload_bits_per_second,
            egress_payload_bits_per_second: estimate.egress_payload_bits_per_second,
        },
    }))
}

fn validate_friendly_name(value: &str) -> Result<(), ApiError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 120 {
        return Err(ApiError::invalid_request(
            "friendlyName must contain 1 to 120 non-whitespace characters",
        ));
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorResponse {
    code: &'static str,
    message: String,
}

struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code: "invalid_request",
            message: message.into(),
        }
    }

    fn invalid_media_profile(error: MediaProfileError) -> Self {
        Self::invalid_request(error.to_string())
    }

    fn draft_validation(error: DraftValidationError) -> Self {
        match error {
            DraftValidationError::NotFound => Self {
                status: StatusCode::NOT_FOUND,
                code: "not_found",
                message: "The requested source or destination does not exist".to_owned(),
            },
            DraftValidationError::NotDraft => Self {
                status: StatusCode::CONFLICT,
                code: "invalid_lifecycle",
                message: "Only draft resources can be validated".to_owned(),
            },
            DraftValidationError::Database(error) => Self::database("validate draft", error),
        }
    }

    fn resource_update(error: ResourceUpdateError) -> Self {
        match error {
            ResourceUpdateError::NotFound => Self {
                status: StatusCode::NOT_FOUND,
                code: "not_found",
                message: "The requested source or destination does not exist".to_owned(),
            },
            ResourceUpdateError::Enabled => Self {
                status: StatusCode::CONFLICT,
                code: "resource_enabled",
                message: "Enabled sources and destinations cannot be edited".to_owned(),
            },
            ResourceUpdateError::Database(error) => Self::database("update resource", error),
        }
    }

    fn serialize_profile(error: serde_json::Error) -> Self {
        tracing::error!(%error, "media profile serialization failed");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal_error",
            message: "Media profile could not be stored".to_owned(),
        }
    }

    fn database(operation: &'static str, error: impl std::fmt::Display) -> Self {
        tracing::error!(%error, operation, "catalog database operation failed");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "database_error",
            message: "Catalog database operation failed".to_owned(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorResponse {
                code: self.code,
                message: self.message,
            }),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
    };
    use serde_json::Value;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    };
    use tower::ServiceExt;

    use super::router;

    #[tokio::test]
    async fn liveness_route_returns_alive() {
        let response = router()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/health/live")
                    .body(Body::empty())
                    .expect("request builds"),
            )
            .await
            .expect("router handles request");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body reads");
        let value: Value = serde_json::from_slice(&body).expect("response is JSON");
        assert_eq!(value["status"], "alive");
    }

    #[tokio::test]
    async fn capability_route_returns_worker_collection() {
        let response = router()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/system/capabilities")
                    .body(Body::empty())
                    .expect("request builds"),
            )
            .await
            .expect("router handles request");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body reads");
        let value: Value = serde_json::from_slice(&body).expect("response is JSON");
        assert_eq!(value["apiVersion"], "1.0");
        assert_eq!(value["workers"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn route_planner_accepts_the_selected_rtp_to_srt_profile() {
        let response = router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/plan")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "source": {
                                "kind": "rtp_mp2t",
                                "bindAddress": "0.0.0.0",
                                "port": 5000
                            },
                            "destination": {
                                "kind": "srt_mpeg_ts",
                                "host": "127.0.0.1",
                                "port": 6000,
                                "mode": "caller"
                            }
                        })
                        .to_string(),
                    ))
                    .expect("request builds"),
            )
            .await
            .expect("router handles request");

        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body reads");
        assert_eq!(
            status,
            StatusCode::OK,
            "unexpected error response: {}",
            String::from_utf8_lossy(&body)
        );
        let value: Value = serde_json::from_slice(&body).expect("response is JSON");
        assert_eq!(value["mediaPath"], "rtp_mp2t_to_srt_mpeg_ts");
        assert_eq!(
            value["estimate"],
            serde_json::json!({
                "capacityUnits": 2,
                "ingressPayloadBitsPerSecond": null,
                "egressPayloadBitsPerSecond": null
            })
        );
    }

    #[tokio::test]
    async fn route_planner_reports_known_alsa_payload_rate() {
        let response = router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/plan")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "source": { "kind": "alsa_l16", "device": "hw:1,0" },
                            "destination": {
                                "kind": "rtp_l16",
                                "address": "127.0.0.1",
                                "port": 5004,
                                "payloadType": 96
                            }
                        })
                        .to_string(),
                    ))
                    .expect("request builds"),
            )
            .await
            .expect("router handles request");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body reads");
        let value: Value = serde_json::from_slice(&body).expect("response is JSON");
        assert_eq!(
            value,
            serde_json::json!({
                "mediaPath": "alsa_l16_to_rtp_l16",
                "estimate": {
                    "capacityUnits": 2,
                    "ingressPayloadBitsPerSecond": null,
                    "egressPayloadBitsPerSecond": 1536000
                }
            })
        );
    }

    #[tokio::test]
    async fn route_planner_accepts_matching_rtp_l16_profiles() {
        let response = router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/plan")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "source": {
                                "kind": "rtp_l16",
                                "bindAddress": "0.0.0.0",
                                "port": 5004,
                                "payloadType": 96
                            },
                            "destination": {
                                "kind": "rtp_l16",
                                "address": "127.0.0.1",
                                "port": 5006,
                                "payloadType": 97
                            }
                        })
                        .to_string(),
                    ))
                    .expect("request builds"),
            )
            .await
            .expect("router handles request");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body reads");
        let value: Value = serde_json::from_slice(&body).expect("response is JSON");
        assert_eq!(value["mediaPath"], "rtp_l16_to_rtp_l16");
    }

    #[tokio::test]
    async fn route_planner_accepts_srt_mpeg_ts_input_to_output() {
        let response = router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/plan")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "source": {
                                "kind": "srt_mpeg_ts",
                                "host": "0.0.0.0",
                                "port": 6000,
                                "mode": "listener"
                            },
                            "destination": {
                                "kind": "srt_mpeg_ts",
                                "host": "127.0.0.1",
                                "port": 6002,
                                "mode": "caller"
                            }
                        })
                        .to_string(),
                    ))
                    .expect("request builds"),
            )
            .await
            .expect("router handles request");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body reads");
        let value: Value = serde_json::from_slice(&body).expect("response is JSON");
        assert_eq!(value["mediaPath"], "srt_mpeg_ts_to_srt_mpeg_ts");
    }

    #[tokio::test]
    async fn route_planner_accepts_matching_rtp_l24_profiles() {
        let response = router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/plan")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "source": {
                                "kind": "rtp_l24",
                                "bindAddress": "0.0.0.0",
                                "port": 5004,
                                "payloadType": 96
                            },
                            "destination": {
                                "kind": "rtp_l24",
                                "address": "127.0.0.1",
                                "port": 5006,
                                "payloadType": 97
                            }
                        })
                        .to_string(),
                    ))
                    .expect("request builds"),
            )
            .await
            .expect("router handles request");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body reads");
        let value: Value = serde_json::from_slice(&body).expect("response is JSON");
        assert_eq!(value["mediaPath"], "rtp_l24_to_rtp_l24");
    }

    #[tokio::test]
    async fn route_planner_rejects_an_unsupported_conversion() {
        let response = router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/routes/plan")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "source": {
                                "kind": "alsa_l16",
                                "device": "hw:1,0"
                            },
                            "destination": {
                                "kind": "srt_mpeg_ts",
                                "host": "127.0.0.1",
                                "port": 6000,
                                "mode": "caller"
                            }
                        })
                        .to_string(),
                    ))
                    .expect("request builds"),
            )
            .await
            .expect("router handles request");

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn http_server_serves_liveness_over_a_socket() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("ephemeral listener binds");
        let address = listener.local_addr().expect("listener has an address");
        let server = tokio::spawn(async move {
            axum::serve(listener, router())
                .await
                .expect("test server serves requests");
        });

        let mut stream = TcpStream::connect(address)
            .await
            .expect("test server accepts a connection");
        stream
            .write_all(
                b"GET /api/v1/health/live HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            )
            .await
            .expect("request writes");
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .await
            .expect("response reads");
        server.abort();

        let response = String::from_utf8(response).expect("HTTP response is UTF-8");
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("{\"status\":\"alive\"}"));
    }
}
