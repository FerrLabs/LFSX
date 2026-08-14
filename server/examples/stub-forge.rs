use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let port = args
        .next()
        .and_then(|raw| raw.parse::<u16>().ok())
        .expect("usage: stub-forge <port> <token>");
    let token = args.next().expect("usage: stub-forge <port> <token>");

    let router = Router::new()
        .route("/repos/{org}/{repo}", get(repository))
        .with_state(Arc::new(token));

    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port)))
        .await
        .expect("bind");

    axum::serve(listener, router).await.expect("serve");
}

async fn repository(State(expected): State<Arc<String>>, headers: HeaderMap) -> Response {
    let presented = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));

    match presented {
        Some(token) if token == expected.as_str() => {
            Json(json!({ "permissions": { "pull": true, "push": true } })).into_response()
        }
        _ => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "message": "Bad credentials" })),
        )
            .into_response(),
    }
}
