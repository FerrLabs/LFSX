use axum::http::{HeaderMap, header};

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use crate::model::Action;
use crate::namespace::Namespace;

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: SocketAddr,
    pub storage_root: PathBuf,
    pub public_url: Option<String>,
    pub action_lifetime: u32,
    pub gc_grace: Duration,
    pub staging_max_age: Duration,
    // How long a lock may go untouched before anyone can take it. Unset means
    // never, which is what happened before this existed and what a team that has
    // not thought about it yet should keep getting.
    pub lock_max_age: Option<Duration>,
    pub max_object_size: Option<u64>,
    pub repo_quota: Option<u64>,
    pub compression: Option<i32>,
    // A path rather than the key itself: a key in the environment is in the pod
    // spec, in `docker inspect`, and in every log that dumps the environment. A
    // file comes from a Kubernetes Secret mount or FerrVault without any of that.
    pub encryption_key_file: Option<PathBuf>,
    pub storage: Storage,
    pub auth: Auth,
}

#[derive(Debug, Clone)]
pub enum Storage {
    Local,
    // Endpoint, bucket and credentials all have to be there: a bucket the server
    // cannot reach is a server that answers every push with an error, and
    // discovering that on the first upload rather than at boot is the wrong
    // order.
    Bucket {
        endpoint: String,
        bucket: String,
        region: String,
        access_key: String,
        secret_key: String,
        path_style: bool,
        // Whether a download is redirected to the bucket instead of streamed
        // through this server. Off by default: the streamed path is the one
        // that counts bytes, serves ranges and holds the ceiling, and an
        // operator should choose to give those up rather than discover it.
        presign: bool,
    },
}

impl Storage {
    fn from_env() -> Self {
        if std::env::var("LFSX_STORAGE").as_deref() != Ok("s3") {
            return Self::Local;
        }

        let required = |name: &str| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| panic!("LFSX_STORAGE=s3 needs {name}"))
        };

        Self::Bucket {
            endpoint: required("LFSX_S3_ENDPOINT"),
            bucket: required("LFSX_S3_BUCKET"),
            region: std::env::var("LFSX_S3_REGION").unwrap_or_else(|_| "us-east-1".into()),
            access_key: required("LFSX_S3_ACCESS_KEY"),
            secret_key: required("LFSX_S3_SECRET_KEY"),
            path_style: std::env::var("LFSX_S3_PATH_STYLE").as_deref() != Ok("false"),
            presign: std::env::var("LFSX_S3_PRESIGN").as_deref() == Ok("true"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Auth {
    Forge {
        provider: Provider,
        api_url: String,
        cache_ttl: Duration,
        rejection_ttl: Duration,
        // Whether a request with no credentials is resolved against the forge
        // instead of refused. On by default, because that is what cloning a
        // public repository does everywhere else.
        anonymous_read: bool,
    },
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Github,
    Gitlab,
}

impl Provider {
    fn default_api_url(self) -> &'static str {
        match self {
            Self::Github => "https://api.github.com",
            Self::Gitlab => "https://gitlab.com/api/v4",
        }
    }

    fn api_url_variable(self) -> &'static str {
        match self {
            Self::Github => "LFSX_GITHUB_API_URL",
            Self::Gitlab => "LFSX_GITLAB_API_URL",
        }
    }
}

const CACHE_TTL: Duration = Duration::from_secs(60);
const REJECTION_TTL: Duration = Duration::from_secs(10);
const GC_GRACE: Duration = Duration::from_secs(14 * 24 * 60 * 60);
const STAGING_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

impl Config {
    pub fn from_env() -> Self {
        let bind = std::env::var("LFSX_BIND")
            .ok()
            .and_then(|raw| raw.parse().ok())
            .unwrap_or_else(|| SocketAddr::from(([0, 0, 0, 0], 8080)));

        let storage_root = std::env::var("LFSX_STORAGE_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/var/lib/lfsx"));

        let public_url = std::env::var("LFSX_PUBLIC_URL")
            .ok()
            .filter(|url| !url.is_empty())
            .map(|url| url.trim_end_matches('/').to_owned());

        Self {
            bind,
            storage_root,
            public_url,
            action_lifetime: 1800,
            gc_grace: seconds("LFSX_GC_GRACE").unwrap_or(GC_GRACE),
            staging_max_age: seconds("LFSX_STAGING_MAX_AGE").unwrap_or(STAGING_MAX_AGE),
            lock_max_age: seconds("LFSX_LOCK_MAX_AGE"),
            max_object_size: bytes("LFSX_MAX_OBJECT_SIZE"),
            repo_quota: bytes("LFSX_REPO_QUOTA"),
            compression: compression(),
            encryption_key_file: std::env::var("LFSX_ENCRYPTION_KEY_FILE")
                .ok()
                .filter(|path| !path.is_empty())
                .map(PathBuf::from),
            storage: Storage::from_env(),
            auth: Auth::from_env(),
        }
    }

    pub fn base_url(&self, headers: &HeaderMap) -> String {
        if let Some(configured) = &self.public_url {
            return configured.clone();
        }

        let scheme = headers
            .get("x-forwarded-proto")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(',').next())
            .map(str::trim)
            .filter(|scheme| !scheme.is_empty())
            .unwrap_or("http");

        let authority = headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|host| !host.is_empty())
            .unwrap_or("localhost");

        format!("{scheme}://{authority}")
    }

    pub fn object_url(&self, base: &str, ns: &Namespace, oid: &str) -> String {
        format!("{base}/{ns}/objects/{oid}")
    }

    pub fn verify_url(&self, base: &str, ns: &Namespace) -> String {
        format!("{base}/{ns}/objects/verify")
    }

    pub fn action(&self, href: String) -> Action {
        Action {
            href,
            header: None,
            expires_in: self.action_lifetime,
        }
    }

    pub fn signed_action(&self, href: String, headers: Vec<(String, String)>) -> Action {
        Action {
            href,
            header: Some(headers.into_iter().collect()),
            expires_in: self.action_lifetime,
        }
    }
}

// Opt in, not opt out. Serving objects to a caller with no credentials at all is
// a decision an operator should make on purpose: it costs them the bandwidth of
// anyone who finds the endpoint, on a server whose whole job is to move files
// measured in gigabytes. Nothing confidential is at stake, since a request with
// no credentials is still resolved against the forge and a private repository is
// still refused, but "anyone may pull from you" is not a sensible thing to
// inherit by default.
//
// Only the exact string opens it. A typo, an empty value or a `1` leaves it
// closed, because the failure that matters here is the one that opens the door
// when nobody meant to.
fn anonymous_read(value: Option<&str>) -> bool {
    value == Some("true")
}

impl Auth {
    fn from_env() -> Self {
        if std::env::var("LFSX_AUTH").as_deref() == Ok("disabled") {
            tracing::warn!(
                "LFSX_AUTH=disabled — every request is accepted, run this on a trusted network only"
            );
            return Self::Disabled;
        }

        let provider = match std::env::var("LFSX_AUTH").as_deref() {
            Ok("gitlab") => Provider::Gitlab,
            _ => Provider::Github,
        };

        let api_url = std::env::var(provider.api_url_variable())
            .unwrap_or_else(|_| provider.default_api_url().to_owned())
            .trim_end_matches('/')
            .to_owned();

        Self::Forge {
            provider,
            api_url,
            cache_ttl: seconds("LFSX_AUTH_CACHE_TTL").unwrap_or(CACHE_TTL),
            rejection_ttl: seconds("LFSX_AUTH_REJECTION_TTL").unwrap_or(REJECTION_TTL),
            anonymous_read: anonymous_read(std::env::var("LFSX_ANONYMOUS_READ").ok().as_deref()),
        }
    }
}

// Unset means unlimited, which is what a server on its own volume wants. Zero
// would refuse every push, so it is read as a typo rather than as a policy
// nobody would choose deliberately.
// zstd level 3 is the default because it is the one that costs nothing you can
// measure: it compresses faster than a spinning disk writes, and the meshes and
// uncompressed raster that make up most of an LFS store give most of their
// ground at any level. Higher levels are there for a store that is short on
// room rather than on time.
fn compression() -> Option<i32> {
    match std::env::var("LFSX_COMPRESSION").ok()?.trim() {
        "" | "none" | "off" => None,
        "zstd" => Some(3),
        other => match other
            .strip_prefix("zstd:")
            .and_then(|level| level.parse().ok())
        {
            Some(level @ 1..=19) => Some(level),
            _ => {
                tracing::warn!(
                    "LFSX_COMPRESSION={other} is not a codec this server knows — storing objects as they arrive"
                );
                None
            }
        },
    }
}

fn bytes(variable: &str) -> Option<u64> {
    let configured = std::env::var(variable).ok()?.trim().parse().ok()?;

    if configured == 0 {
        tracing::warn!("{variable}=0 would refuse every upload — ignoring it");
        return None;
    }

    Some(configured)
}

fn seconds(variable: &str) -> Option<Duration> {
    std::env::var(variable)
        .ok()
        .and_then(|raw| raw.parse().ok())
        .map(Duration::from_secs)
}

#[cfg(test)]
mod tests;
