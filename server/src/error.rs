use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("object id is not a lowercase hex sha256 digest")]
    MalformedOid,

    #[error("content hashes to {actual}, which does not match the declared object id {declared}")]
    OidMismatch { declared: String, actual: String },

    #[error("content is {actual} bytes, but {declared} were declared")]
    SizeMismatch { declared: u64, actual: u64 },

    #[error("object not found")]
    NotFound,

    #[error("storage failure: {0}")]
    Storage(#[from] std::io::Error),
}

impl Error {
    fn status(&self) -> StatusCode {
        match self {
            Self::MalformedOid | Self::OidMismatch { .. } | Self::SizeMismatch { .. } => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = self.status();
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!(error = %self, "request failed");
        }
        (status, Json(json!({ "message": self.to_string() }))).into_response()
    }
}
