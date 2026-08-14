use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("object id is not a lowercase hex sha256 digest")]
    MalformedOid,

    #[error("organisation and repository must be plain names")]
    MalformedNamespace,

    #[error("content hashes to {actual}, which does not match the declared object id {declared}")]
    OidMismatch { declared: String, actual: String },

    #[error("content is {actual} bytes, but {declared} were declared")]
    SizeMismatch { declared: u64, actual: u64 },

    #[error("credentials are required for this repository")]
    Unauthenticated,

    #[error("these credentials do not grant that access to this repository")]
    Forbidden,

    #[error("the forge could not be reached to check permissions")]
    Forge,

    #[error("object not found")]
    NotFound,

    #[error("storage failure: {0}")]
    Storage(#[from] std::io::Error),
}

const CHALLENGE: HeaderValue = HeaderValue::from_static("Basic realm=\"Git LFS\"");

impl Error {
    fn status(&self) -> StatusCode {
        match self {
            Self::MalformedOid
            | Self::MalformedNamespace
            | Self::OidMismatch { .. }
            | Self::SizeMismatch { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Unauthenticated => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Forge => StatusCode::BAD_GATEWAY,
            Self::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = self.status();
        if status.is_server_error() {
            tracing::error!(error = %self, "request failed");
        }

        let body = Json(json!({ "message": self.to_string() }));
        if status == StatusCode::UNAUTHORIZED {
            return (
                status,
                [
                    (header::WWW_AUTHENTICATE, CHALLENGE),
                    (
                        header::HeaderName::from_static("lfs-authenticate"),
                        CHALLENGE,
                    ),
                ],
                body,
            )
                .into_response();
        }

        (status, body).into_response()
    }
}
