use arc_domain::{
    plan_media_path, validate_destination_profile, validate_source_profile,
    DestinationMediaProfile, MediaProfileError, SourceMediaProfile,
};
use arc_infrastructure::catalog::{self, DestinationRecord, SourceRecord};
use arc_infrastructure::postgres::PgPool;
use axum::{
    extract::Extension,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub fn router() -> Router {
    Router::new()
        .route("/api/v1/health/live", get(liveness))
        .route("/api/v1/system/capabilities", get(capabilities))
        .route("/api/v1/routes/plan", post(plan_route))
}

pub fn router_with_database(pool: PgPool) -> Router {
    router()
        .route("/api/v1/sources", get(list_sources).post(create_source))
        .route(
            "/api/v1/destinations",
            get(list_destinations).post(create_destination),
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
}

async fn list_sources(
    Extension(pool): Extension<PgPool>,
) -> Result<Json<Vec<SourceRecord>>, ApiError> {
    catalog::list_sources(&pool)
        .await
        .map(Json)
        .map_err(|error| ApiError::database("list sources", error))
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

async fn list_destinations(
    Extension(pool): Extension<PgPool>,
) -> Result<Json<Vec<DestinationRecord>>, ApiError> {
    catalog::list_destinations(&pool)
        .await
        .map(Json)
        .map_err(|error| ApiError::database("list destinations", error))
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

async fn plan_route(
    Json(request): Json<PlanRouteRequest>,
) -> Result<Json<PlanRouteResponse>, ApiError> {
    let media_path = plan_media_path(&request.source, &request.destination)
        .map_err(ApiError::invalid_media_profile)?;
    Ok(Json(PlanRouteResponse {
        media_path: media_path.as_str(),
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
