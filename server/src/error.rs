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

    #[error(
        "a batch asks about {asked} objects, and this server answers at most {limit} at a time"
    )]
    BatchTooLarge { asked: usize, limit: usize },

    #[error("this repository holds {used} bytes of its {limit} byte budget")]
    OverQuota { used: u64, limit: u64 },

    #[error("this server does not compress objects — set LFSX_COMPRESSION first")]
    CompressionDisabled,

    #[error("this object is encrypted and this server holds no key — set LFSX_ENCRYPTION_KEY_FILE")]
    NotDecryptable,

    #[error("this object was encrypted with a key this server does not hold")]
    UnknownKey,

    #[error("this object failed its integrity check — the bytes on disk are not what was stored")]
    Tampered,

    #[error("{0}")]
    Misconfigured(&'static str),

    #[error("{0}")]
    Unsupported(&'static str),

    #[error("credentials are required for this repository")]
    Unauthenticated,

    #[error("these credentials do not grant that access to this repository")]
    Forbidden,

    #[error("the forge could not be reached to check permissions")]
    Forge,

    // Distinct from `Forge` on purpose. A throttled forge is not a broken one:
    // it is working, it has said when to come back, and the answer has to carry
    // that so a client waits instead of spending the next request on the same
    // exhausted quota.
    #[error("the forge is rate-limiting this server — retry in {retry_after} seconds")]
    RateLimited { retry_after: u64 },

    // And distinct from that one, because the two read the same to a client and
    // mean opposite things to an operator. `RateLimited` is the forge saying it
    // has had enough of this server. This is the server saying it has had enough
    // on the forge's behalf, before the forge is given a reason to refuse
    // everybody at once.
    #[error("this server is not asking the forge again yet, retry in {retry_after} seconds")]
    LookupBudgetSpent { retry_after: u64 },

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
            | Self::SizeMismatch { .. }
            | Self::BatchTooLarge { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            Self::TooLarge { .. } => StatusCode::PAYLOAD_TOO_LARGE,
            Self::OverQuota { .. } => StatusCode::INSUFFICIENT_STORAGE,
            Self::CompressionDisabled => StatusCode::CONFLICT,
            // The object is there and the request was fine; this server cannot
            // serve it, which is a fact about the deployment.
            Self::NotDecryptable | Self::UnknownKey => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Tampered => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Misconfigured(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Unsupported(_) => StatusCode::NOT_IMPLEMENTED,
            Self::Unauthenticated => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::LockHeld(_) => StatusCode::CONFLICT,
            Self::NotFound | Self::LockNotFound => StatusCode::NOT_FOUND,
            Self::Forge => StatusCode::BAD_GATEWAY,
            // Not 502: a bad gateway invites an immediate retry, which is the
            // one thing that must not happen here.
            Self::RateLimited { .. } | Self::LookupBudgetSpent { .. } => {
                StatusCode::SERVICE_UNAVAILABLE
            }
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
            Self::BatchTooLarge { .. } => "batch_too_large",
            Self::OverQuota { .. } => "over_quota",
            Self::CompressionDisabled => "compression_disabled",
            Self::NotDecryptable => "not_decryptable",
            Self::UnknownKey => "unknown_key",
            Self::Tampered => "tampered",
            Self::Misconfigured(_) => "misconfigured",
            Self::Unsupported(_) => "unsupported",
            Self::Unauthenticated => "unauthenticated",
            Self::Forbidden => "forbidden",
            Self::Forge => "forge_unreachable",
            // "the forge is throttling us" and "the forge is broken" are
            // different afternoons, and sharing one label hides which.
            Self::RateLimited { .. } => "forge_rate_limited",
            Self::LookupBudgetSpent { .. } => "lookup_budget_spent",
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

        if let Self::RateLimited { retry_after } | Self::LookupBudgetSpent { retry_after } = &self {
            let mut response = (
                status,
                [(header::RETRY_AFTER, retry_after.to_string())],
                Json(json!({ "message": self.to_string() })),
            )
                .into_response();
            response.extensions_mut().insert(cause);
            return response;
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
