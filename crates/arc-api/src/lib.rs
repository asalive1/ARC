use axum::{routing::get, Json, Router};
use serde_json::{json, Value};

pub fn router() -> Router {
    Router::new()
        .route("/api/v1/health/live", get(liveness))
        .route("/api/v1/system/capabilities", get(capabilities))
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
