use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Extension, Json, Router, middleware};
use futures_util::StreamExt;

use crate::auth::{self, Actor, Permission};
use crate::dashboard::{self, Overview};
use crate::error::Error;
use crate::metrics;
use crate::model::{
    Actions, BatchRequest, BatchResponse, CompressRequest, CreateLockRequest, DedupeRequest,
    ListLocksQuery, ListLocksResponse, LockResponse, ObjectId, ObjectSpec, Operation,
    RetainRequest, StatsResponse, UnlockRequest, VerifyLocksRequest, VerifyLocksResponse,
};
use crate::namespace::Namespace;
use crate::page;
use crate::range::Range;
use crate::state::Shared;
use crate::storage::{Budget, CompressReport, DedupeReport, SweepReport, VerifyReport};

pub fn router(state: Shared) -> Router {
    let objects = Router::new()
        .route("/{org}/{repo}/objects/batch", post(batch))
        .route("/{org}/{repo}/objects/verify", post(verify))
        .route("/{org}/{repo}/objects/retain", post(retain))
        .route("/{org}/{repo}/objects/dedupe", post(dedupe))
        .route("/{org}/{repo}/objects/compress", post(compress))
        .route("/{org}/{repo}/objects/audit", post(audit))
        .route("/{org}/{repo}/objects/stats", get(stats))
        .route("/{org}/{repo}", get(overview))
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
    // Left untouched when the backend cannot measure itself, so the gauges keep
    // whatever they last held rather than being pinned to a zero a dashboard
    // would read as an empty store.
    if let Some((objects, bytes)) = state.store.capacity().await {
        state.metrics.objects_stored.set(objects as i64);
        state.metrics.store_bytes.set(bytes as i64);
    }

    state.metrics.store_scans.set(state.store.scans() as i64);

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
    headers: axum::http::HeaderMap,
    Json(request): Json<BatchRequest>,
) -> Result<Json<BatchResponse>, Error> {
    if request.operation == Operation::Upload {
        permission.require_write()?;
    }

    let base = state.config.base_url(&headers);

    let mut objects = Vec::with_capacity(request.objects.len());
    for id in request.objects {
        objects.push(match request.operation {
            Operation::Download => resolve_download(&state, &base, &ns, id).await,
            Operation::Upload => resolve_upload(&state, &base, &ns, id).await,
        });
    }

    Ok(Json(BatchResponse {
        transfer: negotiate(&request.transfers),
        objects,
    }))
}

// The client advertises what it can speak and the server answers with one of
// them. `basic` is the only adapter this server implements and every client
// supports it, so the answer never changes today — but it is chosen here rather
// than assumed, so adding an adapter is a change in one place instead of a hunt.
fn negotiate(advertised: &[String]) -> &'static str {
    const BASIC: &str = "basic";

    if !advertised.is_empty() && !advertised.iter().any(|transfer| transfer == BASIC) {
        tracing::debug!(
            ?advertised,
            "client advertised no adapter this server implements, answering basic"
        );
    }

    BASIC
}

async fn resolve_download(state: &Shared, base: &str, ns: &Namespace, id: ObjectId) -> ObjectSpec {
    // The marker is what says this repository holds the object, and it is
    // consulted before anything else — including before a signature is cut, so
    // a redirect is never a way around the check that a plain download makes.
    if !state.store.exists(ns, &id.oid).await {
        return ObjectSpec::missing(id);
    }

    // A pre-signed bucket URL is the one href this server hands out that
    // genuinely carries its own credentials, so it is the one case where saying
    // so is true rather than the trap it is everywhere else.
    let (href, authenticated) = match state.store.redirect(&id.oid) {
        Some(signed) => (signed, Some(true)),
        None => (state.config.object_url(base, ns, &id.oid), None),
    };

    ObjectSpec {
        id,
        authenticated,
        actions: Some(Actions {
            download: Some(state.config.action(href)),
            ..Actions::default()
        }),
        error: None,
    }
}

async fn resolve_upload(state: &Shared, base: &str, ns: &Namespace, id: ObjectId) -> ObjectSpec {
    // The size is declared before a single byte moves, so an object over the
    // ceiling is refused here rather than after the client has spent an hour
    // uploading it. The error rides on the object, not the batch: the rest of
    // the push goes ahead.
    if let Some(limit) = state
        .config
        .max_object_size
        .filter(|limit| id.size > *limit)
    {
        return ObjectSpec::too_large(id, limit);
    }

    // An object the repository already holds costs it no room, so it is never
    // refused for want of budget.
    if state.store.exists(ns, &id.oid).await {
        return ObjectSpec {
            id,
            authenticated: None,
            actions: None,
            error: None,
        };
    }

    if let Some(limit) = state.config.repo_quota {
        let (_, used) = state.store.usage_of(ns).await;

        if used + id.size > limit {
            return ObjectSpec::over_quota(id, used, limit);
        }
    }

    let verify = state.config.verify_url(base, ns);

    // A pre-signed upload goes to a key only this repository was handed a
    // signature for, with the object's digest bound into that signature. So the
    // store refuses anything that does not hash to the object, and the fact that
    // bytes arrived under this repository's key is what proves it had them. The
    // shared content key would have taken bytes from anyone allowed to write,
    // and then nothing would distinguish a repository that has an object from
    // one that merely knows its digest.
    if let Some(signed) = state.store.presigned_upload(ns, &id.oid) {
        return ObjectSpec {
            id,
            authenticated: Some(true),
            actions: Some(Actions {
                upload: Some(state.config.signed_action(signed.href, signed.headers)),
                // The client must come back: nothing measured these bytes, so the
                // size, the ceiling and the budget are checked here, and only
                // then does the object become this repository's.
                verify: Some(state.config.action(verify)),
                ..Actions::default()
            }),
            error: None,
        };
    }

    let upload = state.config.object_url(base, ns, &id.oid);
    ObjectSpec {
        id,
        authenticated: None,
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

    // Negotiation already refused what does not fit, but a client is free to
    // skip it and PUT straight here — including without declaring a size, which
    // is why the budget rides along into the transfer rather than being checked
    // once against a number the client chose.
    let budget = match state.config.repo_quota {
        Some(limit) if !state.store.exists(&ns, &oid).await => {
            let (_, used) = state.store.usage_of(&ns).await;
            let budget = Budget { used, limit };

            if budget.exceeded_by(size.unwrap_or_default()) {
                return Err(budget.refusal());
            }

            Some(budget)
        }
        _ => None,
    };

    let written = state
        .store
        .write(&ns, &oid, size, budget, body.into_data_stream())
        .await?;

    state.metrics.uploaded_bytes.inc_by(written);
    state.metrics.object_size.observe(written as f64);

    Ok(StatusCode::OK)
}

async fn download(
    State(state): State<Shared>,
    Extension(ns): Extension<Namespace>,
    Path((.., oid)): Path<(String, String, String)>,
    headers: axum::http::HeaderMap,
) -> Result<Response, Error> {
    let object = state.store.open(&ns, &oid).await?;
    let size = object.size();

    let requested = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok());

    let range = Range::parse(requested, size);
    if range == Range::Unsatisfiable {
        return Ok((
            StatusCode::RANGE_NOT_SATISFIABLE,
            [(header::CONTENT_RANGE, format!("bytes */{size}"))],
        )
            .into_response());
    }

    let start = match range {
        Range::Slice { start, .. } => start,
        _ => 0,
    };

    let length = range.length(size);
    let counted = state.clone();
    let chunks = object.stream(start, length).await?;
    let body = Body::from_stream(chunks.inspect(move |chunk| {
        if let Ok(bytes) = chunk {
            counted.metrics.downloaded_bytes.inc_by(bytes.len() as u64);
        }
    }));

    let mut response = (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
            ),
            (header::ACCEPT_RANGES, HeaderValue::from_static("bytes")),
            (header::CONTENT_LENGTH, HeaderValue::from(length)),
        ],
        body,
    )
        .into_response();

    if let Range::Slice { start, end } = range {
        response
            .headers_mut()
            .insert(header::CONTENT_RANGE, content_range(start, end, size));
        *response.status_mut() = StatusCode::PARTIAL_CONTENT;
    }

    Ok(response)
}

fn content_range(start: u64, end: u64, size: u64) -> HeaderValue {
    HeaderValue::from_str(&format!("bytes {start}-{end}/{size}"))
        .unwrap_or_else(|_| HeaderValue::from_static("bytes */0"))
}

// The client's report that an upload finished. For a transfer that came through
// this server it confirms what is already known. For a pre-signed one it is the
// first time the server sees the object at all, so it is where everything the
// streaming path enforces on the way past has to be enforced instead.
async fn verify(
    State(state): State<Shared>,
    Extension(ns): Extension<Namespace>,
    Extension(permission): Extension<Permission>,
    Json(id): Json<ObjectId>,
) -> Result<StatusCode, Error> {
    permission.require_write()?;

    if state.store.exists(&ns, &id.oid).await {
        return Ok(StatusCode::OK);
    }

    let Some(arrived) = state.store.uploaded_size(&ns, &id.oid).await? else {
        return Err(Error::NotFound);
    };

    // The digest needs no checking: the store refused every byte that did not
    // hash to it. The size does, because nothing counted it, and the client
    // declared it before anything moved.
    if arrived != id.size {
        return Err(Error::SizeMismatch {
            declared: id.size,
            actual: arrived,
        });
    }

    if let Some(limit) = state
        .config
        .max_object_size
        .filter(|limit| arrived > *limit)
    {
        return Err(Error::TooLarge { limit });
    }

    if let Some(limit) = state.config.repo_quota {
        let (_, used) = state.store.usage_of(&ns).await;
        let budget = Budget { used, limit };

        if budget.exceeded_by(arrived) {
            return Err(budget.refusal());
        }
    }

    // Every check has passed, so the object becomes this repository's. The bytes
    // never crossed this server, which is why lfsx_uploaded_bytes does not move:
    // counting a figure nothing here measured would make that counter mean two
    // different things.
    state.store.adopt(&ns, &id.oid).await?;
    state.metrics.object_size.observe(arrived as f64);

    Ok(StatusCode::OK)
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

// Folding a repository's objects into the shared store rewrites what is on
// disk, so it asks for the rights of someone who could delete them instead.
async fn dedupe(
    State(state): State<Shared>,
    Extension(ns): Extension<Namespace>,
    Extension(permission): Extension<Permission>,
    Json(request): Json<DedupeRequest>,
) -> Result<Json<DedupeReport>, Error> {
    permission.require_admin()?;

    let report = state.store.dedupe(&ns, request.dry_run).await?;

    Ok(Json(report))
}

async fn compress(
    State(state): State<Shared>,
    Extension(ns): Extension<Namespace>,
    Extension(permission): Extension<Permission>,
    Json(request): Json<CompressRequest>,
) -> Result<Json<CompressReport>, Error> {
    permission.require_admin()?;

    let report = state.store.compress(&ns, request.dry_run).await?;

    Ok(Json(report))
}

// Reading every object back through the path a download takes is the only check
// that still means something once the file on disk is not the object. It is a
// read of the whole repository, so it asks for the rights of someone who would
// be entitled to read it all anyway.
async fn audit(
    State(state): State<Shared>,
    Extension(ns): Extension<Namespace>,
    Extension(permission): Extension<Permission>,
) -> Result<Json<VerifyReport>, Error> {
    permission.require_admin()?;

    let report = state.store.verify(&ns).await?;

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
    let matching: Vec<_> = state
        .locks
        .list(&ns)
        .await?
        .into_iter()
        .filter(|lock| query.path.as_ref().is_none_or(|path| *path == lock.path))
        .filter(|lock| query.id.as_ref().is_none_or(|id| *id == lock.id))
        .collect();

    let page = page::paginate(matching, query.cursor.as_deref(), query.limit);

    Ok(Json(ListLocksResponse {
        locks: page.locks,
        next_cursor: page.next_cursor,
    }))
}

async fn verify_locks(
    State(state): State<Shared>,
    Extension(ns): Extension<Namespace>,
    headers: axum::http::HeaderMap,
    Json(request): Json<VerifyLocksRequest>,
) -> Result<Json<VerifyLocksResponse>, Error> {
    let Actor(caller) = state.authorizer.actor(&headers).await?;

    // Paginated over the whole list before it is split, so a cursor means the
    // same position on both sides and a client walking pages sees each lock once.
    let page = page::paginate(
        state.locks.list(&ns).await?,
        request.cursor.as_deref(),
        request.limit,
    );
    let (ours, theirs) = page
        .locks
        .into_iter()
        .partition(|lock| lock.owner.name == caller);

    Ok(Json(VerifyLocksResponse {
        ours,
        theirs,
        next_cursor: page.next_cursor,
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
