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

    #[error("object exceeds the {limit} byte limit this server accepts")]
    TooLarge { limit: u64 },

    #[error("this repository holds {used} bytes of its {limit} byte budget")]
    OverQuota { used: u64, limit: u64 },

    #[error("this server does not compress objects — set LFSX_COMPRESSION first")]
    CompressionDisabled,

    #[error("{0}")]
    Misconfigured(&'static str),

    #[error("credentials are required for this repository")]
    Unauthenticated,

    #[error("these credentials do not grant that access to this repository")]
    Forbidden,

    #[error("the forge could not be reached to check permissions")]
    Forge,

    #[error("lock path must not be empty")]
    MalformedLockPath,

    #[error("the file is already locked")]
    LockHeld(Box<crate::locks::Lock>),

    #[error("lock not found")]
    LockNotFound,

    #[error("object not found")]
    NotFound,

    #[error("storage failure: {0}")]
    Storage(#[from] std::io::Error),

    #[error("could not serialise: {0}")]
    Serialisation(#[from] serde_json::Error),
}

const CHALLENGE: HeaderValue = HeaderValue::from_static("Basic realm=\"Git LFS\"");

impl Error {
    fn status(&self) -> StatusCode {
        match self {
            Self::MalformedOid
            | Self::MalformedLockPath
            | Self::MalformedNamespace
            | Self::OidMismatch { .. }
            | Self::SizeMismatch { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            Self::TooLarge { .. } => StatusCode::PAYLOAD_TOO_LARGE,
            Self::OverQuota { .. } => StatusCode::INSUFFICIENT_STORAGE,
            Self::CompressionDisabled => StatusCode::CONFLICT,
            Self::Misconfigured(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Unauthenticated => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::LockHeld(_) => StatusCode::CONFLICT,
            Self::NotFound | Self::LockNotFound => StatusCode::NOT_FOUND,
            Self::Forge => StatusCode::BAD_GATEWAY,
            Self::Storage(_) | Self::Serialisation(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl Error {
    fn cause(&self) -> &'static str {
        match self {
            Self::MalformedOid => "malformed_oid",
            Self::MalformedNamespace => "malformed_namespace",
            Self::MalformedLockPath => "malformed_lock_path",
            Self::OidMismatch { .. } => "oid_mismatch",
            Self::SizeMismatch { .. } => "size_mismatch",
            Self::TooLarge { .. } => "too_large",
            Self::OverQuota { .. } => "over_quota",
            Self::CompressionDisabled => "compression_disabled",
            Self::Misconfigured(_) => "misconfigured",
            Self::Unauthenticated => "unauthenticated",
            Self::Forbidden => "forbidden",
            Self::Forge => "forge_unreachable",
            Self::LockHeld(_) => "lock_held",
            Self::LockNotFound => "lock_not_found",
            Self::NotFound => "not_found",
            Self::Storage(_) => "storage",
            Self::Serialisation(_) => "serialisation",
        }
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = self.status();
        let cause = crate::metrics::Cause(self.cause());

        if status.is_server_error() {
            tracing::error!(error = %self, "request failed");
        }

        if let Self::LockHeld(lock) = &self {
            let mut response = (
                status,
                Json(json!({ "lock": lock, "message": self.to_string() })),
            )
                .into_response();
            response.extensions_mut().insert(cause);
            return response;
        }

        let body = Json(json!({ "message": self.to_string() }));
        let mut response = if status == StatusCode::UNAUTHORIZED {
            (
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
                .into_response()
        } else {
            (status, body).into_response()
        };

        response.extensions_mut().insert(cause);
        response
    }
}
