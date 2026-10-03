mod access;
pub mod session;
pub mod tokens;

use std::path::Path;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use prometheus::core::Collector;
use serde::Serialize;
use tower_http::services::{ServeDir, ServeFile};

use crate::config::{Auth, Config, Dialect, Provider, Storage};
use crate::error::Error;
use crate::metrics::Metrics;
use crate::state::Shared;

#[derive(Debug, Serialize)]
pub struct Overview {
    pub version: &'static str,
    pub uptime_seconds: u64,
    pub storage: StorageFigures,
    pub traffic: Traffic,
    pub object_sizes: Vec<SizeBucket>,
    pub cache: Option<CacheFigures>,
    pub settings: Settings,
}

#[derive(Debug, Serialize)]
pub struct StorageFigures {
    pub kind: &'static str,
    pub objects: Option<u64>,
    pub bytes: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct Traffic {
    pub requests: u64,
    pub server_errors: u64,
    pub rejections: u64,
    pub uploaded_bytes: u64,
    pub downloaded_bytes: u64,
    pub transfers_in_flight: i64,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct SizeBucket {
    pub up_to: Option<u64>,
    pub objects: u64,
}

#[derive(Debug, Serialize)]
pub struct CacheFigures {
    pub hits: u64,
    pub misses: u64,
    pub bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct Settings {
    pub auth: &'static str,
    pub allowed: Option<Vec<String>>,
    pub restricted: Vec<String>,
    pub anonymous_read: bool,
    pub max_object_size: Option<u64>,
    pub repo_quota: Option<u64>,
    pub max_concurrent_transfers: usize,
    pub compression: bool,
    pub encryption: bool,
    pub presign: bool,
    pub locking: bool,
    pub gc_grace_seconds: u64,
    pub lock_max_age_seconds: Option<u64>,
    pub forges: Vec<NamedForge>,
}

#[derive(Debug, Serialize)]
pub struct NamedForge {
    pub name: String,
    pub auth: &'static str,
    pub api_url: String,
    pub allowed: Option<Vec<String>>,
}

pub struct Refused(Error);

impl From<Error> for Refused {
    fn from(error: Error) -> Self {
        Self(error)
    }
}

impl IntoResponse for Refused {
    fn into_response(self) -> Response {
        match self.0 {
            Error::Unauthenticated => (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "message": "sign in to the dashboard" })),
            )
                .into_response(),
            error => error.into_response(),
        }
    }
}

pub fn router(dir: &Path) -> Router<Shared> {
    Router::new()
        .route("/-/api/overview", get(overview))
        .route(
            "/-/api/access",
            get(access::read).put(access::write).delete(access::reset),
        )
        .route(
            "/-/api/session",
            get(session::current)
                .post(session::sign_in)
                .delete(session::sign_out),
        )
        .nest_service(
            "/-/dashboard",
            ServeDir::new(dir).fallback(ServeFile::new(dir.join("index.html"))),
        )
}

async fn overview(
    State(state): State<Shared>,
    headers: HeaderMap,
) -> Result<Json<Overview>, Refused> {
    admit(&state, &headers).await?;

    let capacity = state.store.capacity().await;

    Ok(Json(Overview {
        version: env!("CARGO_PKG_VERSION"),
        uptime_seconds: state.started.elapsed().as_secs(),
        storage: StorageFigures {
            kind: storage_kind(&state.config.storage),
            objects: capacity.map(|(objects, _)| objects),
            bytes: capacity.map(|(_, bytes)| bytes),
        },
        traffic: traffic(&state.metrics),
        object_sizes: object_sizes(&state.metrics),
        cache: state.store.cache_stats().map(|stats| CacheFigures {
            hits: stats.hits,
            misses: stats.misses,
            bytes: stats.bytes,
        }),
        settings: settings(&state.config, state.authorizer.access()),
    }))
}

pub(crate) async fn admit(state: &Shared, headers: &HeaderMap) -> Result<(), Error> {
    let Some(dashboard) = &state.config.dashboard else {
        return Err(Error::NotServed);
    };

    if session::viewer(state, headers).await?.is_some() {
        return Ok(());
    }

    match (&state.config.auth, &dashboard.admins) {
        (Auth::Forge { .. }, Some(admins)) => state
            .authorizer
            .forge_permission(headers, admins)
            .await?
            .require_admin(),
        _ => Err(Error::Unauthenticated),
    }
}

fn storage_kind(storage: &Storage) -> &'static str {
    match storage {
        Storage::Local => "local",
        Storage::Bucket { dialect, .. } => match dialect {
            Dialect::S3 { .. } => "s3",
            Dialect::Azure { .. } => "azure",
            Dialect::Gcs { .. } => "gcs",
        },
    }
}

fn counted(collector: &impl Collector, keep: impl Fn(&[(&str, &str)]) -> bool) -> u64 {
    collector
        .collect()
        .iter()
        .flat_map(|family| family.get_metric())
        .filter(|metric| {
            let labels: Vec<(&str, &str)> = metric
                .get_label()
                .iter()
                .map(|label| (label.name(), label.value()))
                .collect();
            keep(&labels)
        })
        .map(|metric| metric.get_counter().get_value() as u64)
        .sum()
}

pub(crate) fn traffic(metrics: &Metrics) -> Traffic {
    Traffic {
        requests: counted(&metrics.requests, |_| true),
        server_errors: counted(&metrics.requests, |labels| {
            labels
                .iter()
                .any(|(name, value)| *name == "status" && value.starts_with('5'))
        }),
        rejections: counted(&metrics.rejections, |_| true),
        uploaded_bytes: metrics.uploaded_bytes.get(),
        downloaded_bytes: metrics.downloaded_bytes.get(),
        transfers_in_flight: metrics.transfers_in_flight.get(),
    }
}

pub(crate) fn object_sizes(metrics: &Metrics) -> Vec<SizeBucket> {
    let families = metrics.object_size.collect();
    let Some(histogram) = families
        .first()
        .and_then(|family| family.get_metric().first())
        .map(|metric| metric.get_histogram())
    else {
        return Vec::new();
    };

    let mut buckets = Vec::new();
    let mut below = 0;
    for bucket in histogram.get_bucket() {
        let cumulative = bucket.cumulative_count();
        buckets.push(SizeBucket {
            up_to: Some(bucket.upper_bound() as u64),
            objects: cumulative - below,
        });
        below = cumulative;
    }
    buckets.push(SizeBucket {
        up_to: None,
        objects: histogram.get_sample_count() - below,
    });

    buckets
}

pub fn start(state: Shared) {
    access::keep_fresh(state);
}

pub(crate) fn settings(config: &Config, live: Option<crate::auth::Access>) -> Settings {
    let auth = match &config.auth {
        Auth::Disabled => "disabled",
        Auth::Forge { provider, .. } => provider_name(*provider),
    };
    let (allowed, restricted, anonymous_read) = match live {
        None => (None, Vec::new(), true),
        Some(access) => (
            access.allowed.as_ref().map(|allowed| allowed.entries()),
            access.restricted.entries(),
            access.anonymous_read,
        ),
    };
    let (presign, locking) = match &config.storage {
        Storage::Local => (false, true),
        Storage::Bucket {
            presign, locking, ..
        } => (*presign, *locking),
    };

    Settings {
        auth,
        allowed,
        restricted,
        anonymous_read,
        max_object_size: config.max_object_size,
        repo_quota: config.repo_quota,
        max_concurrent_transfers: config.max_concurrent_transfers,
        compression: config.compression.is_some(),
        encryption: config.encryption_key.is_some(),
        presign,
        locking,
        gc_grace_seconds: config.gc_grace.as_secs(),
        lock_max_age_seconds: config.lock_max_age.map(|age| age.as_secs()),
        forges: config
            .forges
            .iter()
            .filter_map(|forge| match &forge.auth {
                Auth::Forge {
                    provider,
                    api_url,
                    allowed,
                    ..
                } => Some(NamedForge {
                    name: forge.name.clone(),
                    auth: provider_name(*provider),
                    api_url: api_url.clone(),
                    allowed: allowed.as_ref().map(|allowed| allowed.entries()),
                }),
                Auth::Disabled => None,
            })
            .collect(),
    }
}

fn provider_name(provider: Provider) -> &'static str {
    match provider {
        Provider::Github => "github",
        Provider::Gitlab => "gitlab",
        Provider::Gitea => "gitea",
    }
}
