use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Extension, Json, Router, middleware};
use tokio_util::io::ReaderStream;

use crate::auth::{self, Permission};
use crate::error::Error;
use crate::model::{
    Actions, BatchRequest, BatchResponse, ObjectId, ObjectSpec, Operation, RetainRequest,
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
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::authorize,
        ));

    Router::new()
        .route("/health", get(|| async { "ok" }))
        .merge(objects)
        .with_state(state)
}

async fn batch(
    State(state): State<Shared>,
    Path((org, repo)): Path<(String, String)>,
    Extension(permission): Extension<Permission>,
    Json(request): Json<BatchRequest>,
) -> Result<Json<BatchResponse>, Error> {
    let ns = Namespace::new(&org, &repo)?;
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

async fn resolve_download(state: &Shared, ns: &Namespace<'_>, id: ObjectId) -> ObjectSpec {
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

async fn resolve_upload(state: &Shared, ns: &Namespace<'_>, id: ObjectId) -> ObjectSpec {
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
    Path((org, repo, oid)): Path<(String, String, String)>,
    Extension(permission): Extension<Permission>,
    headers: axum::http::HeaderMap,
    body: Body,
) -> Result<StatusCode, Error> {
    permission.require_write()?;

    let size = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok());

    let ns = Namespace::new(&org, &repo)?;

    state
        .store
        .write(&ns, &oid, size, body.into_data_stream())
        .await?;

    Ok(StatusCode::OK)
}

async fn download(
    State(state): State<Shared>,
    Path((org, repo, oid)): Path<(String, String, String)>,
) -> Result<Response, Error> {
    let ns = Namespace::new(&org, &repo)?;

    let (file, size) = state.store.open(&ns, &oid).await?;
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

async fn retain(
    State(state): State<Shared>,
    Path((org, repo)): Path<(String, String)>,
    Extension(permission): Extension<Permission>,
    Json(request): Json<RetainRequest>,
) -> Result<Json<SweepReport>, Error> {
    permission.require_write()?;

    let ns = Namespace::new(&org, &repo)?;
    let retained = request.oids.into_iter().collect();

    let report = state
        .store
        .sweep(&ns, &retained, state.config.gc_grace, request.dry_run)
        .await?;

    Ok(Json(report))
}

async fn verify(
    State(state): State<Shared>,
    Path((org, repo)): Path<(String, String)>,
    Extension(permission): Extension<Permission>,
    Json(id): Json<ObjectId>,
) -> Result<StatusCode, Error> {
    permission.require_write()?;

    let ns = Namespace::new(&org, &repo)?;

    state
        .store
        .exists(&ns, &id.oid)
        .await
        .then_some(StatusCode::OK)
        .ok_or(Error::NotFound)
}
