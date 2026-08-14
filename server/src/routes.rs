use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Extension, Json, Router, middleware};
use tokio_util::io::ReaderStream;

use crate::auth::{self, Actor, Permission};
use crate::error::Error;
use crate::metrics;
use crate::model::{
    Actions, BatchRequest, BatchResponse, CreateLockRequest, ListLocksQuery, ListLocksResponse,
    LockResponse, ObjectId, ObjectSpec, Operation, RetainRequest, UnlockRequest,
    VerifyLocksResponse,
};
use crate::namespace::Namespace;
use crate::state::Shared;
use crate::storage::SweepReport;

pub fn router(state: Shared) -> Router {
    let objects = Router::new()
        .route("/{org}/{repo}/objects/batch", post(batch))
        .route("/{org}/{repo}/objects/verify", post(verify))
        .route("/{org}/{repo}/objects/retain", post(retain))
        .route("/{org}/{repo}/objects/{oid}", put(upload).get(download))
        .route("/{org}/{repo}/locks", post(create_lock).get(list_locks))
        .route("/{org}/{repo}/locks/verify", post(verify_locks))
        .route("/{org}/{repo}/locks/{id}/unlock", post(unlock))
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
    let (objects, bytes) = state.store.usage().await;
    state.metrics.objects_stored.set(objects as i64);
    state.metrics.store_bytes.set(bytes as i64);

    state.metrics.render().into_response()
}

async fn ready(State(state): State<Shared>) -> Response {
    match state.store.writable().await {
        Ok(()) => "ready".into_response(),
        Err(error) => {
            tracing::error!(%error, "storage root is not writable");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "storage root is not writable",
            )
                .into_response()
        }
    }
}

async fn batch(
    State(state): State<Shared>,
    Extension(ns): Extension<Namespace>,
    Extension(permission): Extension<Permission>,
    Json(request): Json<BatchRequest>,
) -> Result<Json<BatchResponse>, Error> {
    if request.operation == Operation::Upload {
        permission.require_write()?;
    }

    let mut objects = Vec::with_capacity(request.objects.len());
    for id in request.objects {
        objects.push(match request.operation {
            Operation::Download => resolve_download(&state, &ns, id).await,
            Operation::Upload => resolve_upload(&state, &ns, id).await,
        });
    }

    Ok(Json(BatchResponse {
        transfer: "basic",
        objects,
    }))
}

async fn resolve_download(state: &Shared, ns: &Namespace, id: ObjectId) -> ObjectSpec {
    if !state.store.exists(ns, &id.oid).await {
        return ObjectSpec::missing(id);
    }

    let href = state.config.object_url(ns, &id.oid);
    ObjectSpec {
        id,
        actions: Some(Actions {
            download: Some(state.config.action(href)),
            ..Actions::default()
        }),
        error: None,
    }
}

async fn resolve_upload(state: &Shared, ns: &Namespace, id: ObjectId) -> ObjectSpec {
    if state.store.exists(ns, &id.oid).await {
        return ObjectSpec {
            id,
            actions: None,
            error: None,
        };
    }

    let upload = state.config.object_url(ns, &id.oid);
    let verify = state.config.verify_url(ns);
    ObjectSpec {
        id,
        actions: Some(Actions {
            upload: Some(state.config.action(upload)),
            verify: Some(state.config.action(verify)),
            ..Actions::default()
        }),
        error: None,
    }
}

async fn upload(
    State(state): State<Shared>,
    Extension(ns): Extension<Namespace>,
    Extension(permission): Extension<Permission>,
    Path((.., oid)): Path<(String, String, String)>,
    headers: axum::http::HeaderMap,
    body: Body,
) -> Result<StatusCode, Error> {
    permission.require_write()?;

    let size = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok());

    let written = state
        .store
        .write(&ns, &oid, size, body.into_data_stream())
        .await?;

    state.metrics.uploaded_bytes.inc_by(written);
    state.metrics.object_size.observe(written as f64);

    Ok(StatusCode::OK)
}

async fn download(
    State(state): State<Shared>,
    Extension(ns): Extension<Namespace>,
    Path((.., oid)): Path<(String, String, String)>,
) -> Result<Response, Error> {
    let (file, size) = state.store.open(&ns, &oid).await?;
    state.metrics.downloaded_bytes.inc_by(size);
    let body = Body::from_stream(ReaderStream::new(file));

    Ok((
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
            ),
            (header::CONTENT_LENGTH, HeaderValue::from(size)),
        ],
        body,
    )
        .into_response())
}

async fn verify(
    State(state): State<Shared>,
    Extension(ns): Extension<Namespace>,
    Extension(permission): Extension<Permission>,
    Json(id): Json<ObjectId>,
) -> Result<StatusCode, Error> {
    permission.require_write()?;

    state
        .store
        .exists(&ns, &id.oid)
        .await
        .then_some(StatusCode::OK)
        .ok_or(Error::NotFound)
}

async fn retain(
    State(state): State<Shared>,
    Extension(ns): Extension<Namespace>,
    Extension(permission): Extension<Permission>,
    Json(request): Json<RetainRequest>,
) -> Result<Json<SweepReport>, Error> {
    permission.require_write()?;

    let retained = request.oids.into_iter().collect();
    let report = state
        .store
        .sweep(&ns, &retained, state.config.gc_grace, request.dry_run)
        .await?;

    Ok(Json(report))
}

async fn create_lock(
    State(state): State<Shared>,
    Extension(ns): Extension<Namespace>,
    Extension(permission): Extension<Permission>,
    headers: axum::http::HeaderMap,
    Json(request): Json<CreateLockRequest>,
) -> Result<(StatusCode, Json<LockResponse>), Error> {
    permission.require_write()?;

    let Actor(owner) = state.authorizer.actor(&headers).await?;
    let lock = state.locks.create(&ns, &request.path, &owner).await?;

    Ok((StatusCode::CREATED, Json(LockResponse { lock })))
}

async fn list_locks(
    State(state): State<Shared>,
    Extension(ns): Extension<Namespace>,
    Query(query): Query<ListLocksQuery>,
) -> Result<Json<ListLocksResponse>, Error> {
    let locks = state
        .locks
        .list(&ns)
        .await?
        .into_iter()
        .filter(|lock| query.path.as_ref().is_none_or(|path| *path == lock.path))
        .filter(|lock| query.id.as_ref().is_none_or(|id| *id == lock.id))
        .collect();

    Ok(Json(ListLocksResponse {
        locks,
        next_cursor: "",
    }))
}

async fn verify_locks(
    State(state): State<Shared>,
    Extension(ns): Extension<Namespace>,
    headers: axum::http::HeaderMap,
) -> Result<Json<VerifyLocksResponse>, Error> {
    let Actor(caller) = state.authorizer.actor(&headers).await?;
    let (ours, theirs) = state
        .locks
        .list(&ns)
        .await?
        .into_iter()
        .partition(|lock| lock.owner.name == caller);

    Ok(Json(VerifyLocksResponse {
        ours,
        theirs,
        next_cursor: "",
    }))
}

async fn unlock(
    State(state): State<Shared>,
    Extension(ns): Extension<Namespace>,
    Extension(permission): Extension<Permission>,
    Path((.., id)): Path<(String, String, String)>,
    headers: axum::http::HeaderMap,
    Json(request): Json<UnlockRequest>,
) -> Result<Json<LockResponse>, Error> {
    permission.require_write()?;

    let lock = state
        .locks
        .get(&ns, &id)
        .await?
        .ok_or(Error::LockNotFound)?;
    let Actor(caller) = state.authorizer.actor(&headers).await?;

    if lock.owner.name != caller {
        if !request.force {
            return Err(Error::Forbidden);
        }
        permission.require_admin()?;
    }

    state.locks.remove(&ns, &id).await?;

    Ok(Json(LockResponse { lock }))
}
