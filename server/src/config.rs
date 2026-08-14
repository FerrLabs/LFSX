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
    pub auth: Auth,
}

#[derive(Debug, Clone)]
pub enum Auth {
    Github {
        api_url: String,
        cache_ttl: Duration,
        rejection_ttl: Duration,
    },
    Disabled,
}

const GITHUB_API_URL: &str = "https://api.github.com";
const CACHE_TTL: Duration = Duration::from_secs(60);
const REJECTION_TTL: Duration = Duration::from_secs(10);
const GC_GRACE: Duration = Duration::from_secs(14 * 24 * 60 * 60);

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
            expires_in: self.action_lifetime,
        }
    }
}

impl Auth {
    fn from_env() -> Self {
        if std::env::var("LFSX_AUTH").as_deref() == Ok("disabled") {
            tracing::warn!(
                "LFSX_AUTH=disabled — every request is accepted, run this on a trusted network only"
            );
            return Self::Disabled;
        }

        let api_url = std::env::var("LFSX_GITHUB_API_URL")
            .unwrap_or_else(|_| GITHUB_API_URL.to_owned())
            .trim_end_matches('/')
            .to_owned();

        let cache_ttl = seconds("LFSX_AUTH_CACHE_TTL").unwrap_or(CACHE_TTL);

        let rejection_ttl = seconds("LFSX_AUTH_REJECTION_TTL").unwrap_or(REJECTION_TTL);

        Self::Github {
            api_url,
            cache_ttl,
            rejection_ttl,
        }
    }
}

fn seconds(variable: &str) -> Option<Duration> {
    std::env::var(variable)
        .ok()
        .and_then(|raw| raw.parse().ok())
        .map(Duration::from_secs)
}
