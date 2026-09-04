use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Extension, Json, Router, middleware};

use crate::auth::{self, Permission};
use crate::dashboard::{self, Overview};
use crate::error::Error;
use crate::metrics;
use crate::model::StatsResponse;
use crate::namespace::Namespace;
use crate::state::Shared;

mod locks;
mod maintenance;
mod objects;

pub fn router(state: Shared) -> Router {
    let objects = Router::new()
        .route("/{org}/{repo}/objects/batch", post(objects::batch))
        .route("/{org}/{repo}/objects/verify", post(objects::verify))
        .route("/{org}/{repo}/objects/retain", post(maintenance::retain))
        .route("/{org}/{repo}/objects/dedupe", post(maintenance::dedupe))
        .route(
            "/{org}/{repo}/objects/compress",
            post(maintenance::compress),
        )
        .route("/{org}/{repo}/objects/audit", post(maintenance::audit))
        .route("/{org}/{repo}/objects/stats", get(stats))
        .route("/{org}/{repo}", get(overview))
        .route(
            "/{org}/{repo}/objects/{oid}",
            put(objects::upload).get(objects::download),
        )
        .route(
            "/{org}/{repo}/locks",
            post(locks::create_lock).get(locks::list_locks),
        )
        .route("/{org}/{repo}/locks/verify", post(locks::verify_locks))
        .route("/{org}/{repo}/locks/{id}/unlock", post(locks::unlock))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::authorize,
        ));

    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/ready", get(ready))
        .route("/metrics", get(scrape))
        .merge(objects)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            metrics::record,
        ))
        .with_state(state)
}

async fn scrape(State(state): State<Shared>) -> Response {
    // Left untouched when the backend cannot measure itself, so the gauges keep
    // whatever they last held rather than being pinned to a zero a dashboard
    // would read as an empty store.
    if let Some((objects, bytes)) = state.store.capacity().await {
        state.metrics.objects_stored.set(objects as i64);
        state.metrics.store_bytes.set(bytes as i64);
    }

    state.metrics.store_scans.set(state.store.scans() as i64);

    if let Some(transfers) = &state.transfers {
        let in_flight = state.config.max_concurrent_transfers - transfers.available_permits();
        state.metrics.transfers_in_flight.set(in_flight as i64);
    }

    // The cache counts its own hits, so the scrape carries the difference
    // rather than the total: both sides are monotonic, which makes the delta the
    // right thing to add to a counter.
    if let Some(cache) = state.store.cache_stats().await {
        let metrics = &state.metrics;
        metrics
            .cache_hits
            .inc_by(cache.hits.saturating_sub(metrics.cache_hits.get()));
        metrics
            .cache_misses
            .inc_by(cache.misses.saturating_sub(metrics.cache_misses.get()));
        metrics.cache_bytes.set(cache.bytes as i64);
    }

    state.metrics.render().into_response()
}

async fn ready(State(state): State<Shared>) -> Response {
    match state.store.writable().await {
        Ok(()) => "ready".into_response(),
        Err(error) => {
            tracing::error!(%error, "this instance cannot serve and is answering not ready");
            (StatusCode::SERVICE_UNAVAILABLE, error.to_string()).into_response()
        }
    }
}

async fn stats(
    State(state): State<Shared>,
    Extension(ns): Extension<Namespace>,
) -> Result<Json<StatsResponse>, Error> {
    let (objects, bytes) = state.store.usage_of(&ns).await;

    Ok(Json(StatsResponse {
        objects,
        bytes,
        locks: state.locks.list(&ns).await?.len(),
    }))
}

async fn overview(
    State(state): State<Shared>,
    Extension(ns): Extension<Namespace>,
    Extension(permission): Extension<Permission>,
) -> Result<Response, Error> {
    let (objects, bytes) = state.store.usage_of(&ns).await;
    let page = dashboard::render(&Overview {
        namespace: ns.clone(),
        objects,
        bytes,
        locks: state.locks.list(&ns).await?,
        // Asked of the store rather than the config, so the page cannot tint a
        // lock as takeable that the store would refuse to hand over.
        lock_max_age: state.locks.max_age(),
        writable: permission.require_write().is_ok(),
    });

    Ok(([(header::CONTENT_TYPE, "text/html; charset=utf-8")], page).into_response())
}
