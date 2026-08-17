use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use futures_util::StreamExt;

use crate::auth::Permission;
use crate::error::Error;
use crate::model::{Actions, BatchRequest, BatchResponse, ObjectId, ObjectSpec, Operation};
use crate::namespace::Namespace;
use crate::range::Range;
use crate::state::Shared;
use crate::storage::Budget;

// The Git LFS transfer surface: what a client negotiates, what it sends and
// what it gets back. Everything here is driven by a client running `git push`
// or `git pull` and nothing else calls it.

pub(super) async fn batch(
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

pub(super) async fn upload(
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

pub(super) async fn download(
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
pub(super) async fn verify(
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
