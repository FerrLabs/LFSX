use axum::http::{HeaderMap, header};

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use crate::model::Action;
use crate::namespace::Namespace;

mod auth;
mod dashboard;
mod forges;
mod storage;

pub use self::auth::{Auth, GithubApp, Provider};
pub use self::dashboard::Dashboard;
pub use self::forges::Forge;
pub use self::storage::{AzureCredential, Dialect, DiskCache, GcsCredential, KeySource, Storage};

use self::dashboard::dashboard;
use self::storage::encryption_key;

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
    // How many uploads and downloads may hold this server's disk and network
    // open at once. A backstop for the bare deployment with nothing in front:
    // the expensive thing here is a transfer held open, not a request counted,
    // and anything smarter belongs to the reverse proxy.
    pub max_concurrent_transfers: usize,
    pub repo_quota: Option<u64>,
    pub compression: Option<i32>,
    // Never the key itself: a key in the environment is in the pod spec, in
    // `docker inspect`, and in every log that dumps the environment. A file
    // comes from a Kubernetes Secret mount without any of that, and a command
    // is the one interface every KMS, Vault and SOPS already speaks, for the
    // operator whose keys must never rest on disk at all.
    pub encryption_key: Option<KeySource>,
    pub storage: Storage,
    pub auth: Auth,
    pub dashboard: Option<Dashboard>,
    pub forges: Vec<Forge>,
}

const TRANSFER_CAP: usize = 128;
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
            max_concurrent_transfers: transfer_cap(
                std::env::var("LFSX_MAX_CONCURRENT_TRANSFERS")
                    .ok()
                    .as_deref(),
            ),
            repo_quota: bytes("LFSX_REPO_QUOTA"),
            compression: compression(),
            encryption_key: encryption_key(
                std::env::var("LFSX_ENCRYPTION_KEY_FILE").ok().as_deref(),
                std::env::var("LFSX_ENCRYPTION_KEY_COMMAND").ok().as_deref(),
            ),
            storage: Storage::from_env(),
            dashboard: None,
            forges: Vec::new(),
            auth: Auth::from_env(),
        }
        .with_forges()
        .with_dashboard()
    }

    fn with_forges(self) -> Self {
        Self {
            forges: forges::from_env(&self.auth),
            ..self
        }
    }

    fn with_dashboard(self) -> Self {
        Self {
            dashboard: dashboard(
                std::env::var("LFSX_DASHBOARD").ok().as_deref(),
                std::env::var("LFSX_DASHBOARD_DIR").ok().as_deref(),
                std::env::var("LFSX_DASHBOARD_REPO").ok().as_deref(),
                std::env::var("LFSX_DASHBOARD_FORGE").ok().as_deref(),
                &self.auth,
                &self.forges,
            ),
            ..self
        }
    }

    pub fn base_url(&self, headers: &HeaderMap) -> String {
        if let Some(configured) = &self.public_url {
            return configured.clone();
        }

        // Neither of these is this deployment speaking. They are what the caller
        // sent, and what comes out of here is the URL that caller will send the
        // object to, with its credential attached. So both are checked for being
        // the thing they claim to be before either goes into a URL.
        let scheme = headers
            .get("x-forwarded-proto")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(',').next())
            .map(str::trim)
            .filter(|scheme| matches!(*scheme, "http" | "https"))
            .unwrap_or("http");

        let authority = headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|host| is_an_authority(host))
            .unwrap_or("localhost");

        format!("{scheme}://{authority}")
    }

    pub fn object_url(&self, base: &str, ns: &Namespace, oid: &str) -> String {
        format!("{base}/{}/objects/{oid}", ns.url_path())
    }

    pub fn verify_url(&self, base: &str, ns: &Namespace) -> String {
        format!("{base}/{}/objects/verify", ns.url_path())
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

// Is this a host and a port, and nothing else?
//
// A `Host` carrying a `/` or an `@` is not one, and both change where the URL
// built from it points. `real.example@evil.example` resolves to the second name
// with the first read as a username, which turns a header somebody sent into a
// redirect nobody wrote, and the client follows it carrying its token.
//
// Anything that fails this falls back to `localhost`, which is useless to
// everybody and dangerous to nobody. `LFSX_PUBLIC_URL` is the fix, and startup
// says so.
fn is_an_authority(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 255
        && host.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b':' | b'[' | b']')
        })
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
                    "LFSX_COMPRESSION={other} is not a codec this server knows, storing objects as they arrive"
                );
                None
            }
        },
    }
}

// Same posture as the lookup budget: this bounds a cost, so an unparseable
// value falls back to the default with a warning rather than refusing to start.
fn transfer_cap(value: Option<&str>) -> usize {
    match value.map(str::trim).map(str::parse) {
        Some(Ok(cap)) => cap,
        None => TRANSFER_CAP,
        Some(Err(_)) => {
            tracing::warn!(
                "LFSX_MAX_CONCURRENT_TRANSFERS is not a number, keeping the default of {TRANSFER_CAP}"
            );
            TRANSFER_CAP
        }
    }
}

fn bytes(variable: &str) -> Option<u64> {
    let configured = std::env::var(variable).ok()?.trim().parse().ok()?;

    if configured == 0 {
        tracing::warn!("{variable}=0 would refuse every upload, ignoring it");
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
