use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::{Extension, Json};

use crate::auth::{Actor, Permission};
use crate::error::Error;
use crate::model::{
    CreateLockRequest, ListLocksQuery, ListLocksResponse, LockResponse, UnlockRequest,
    VerifyLocksRequest, VerifyLocksResponse,
};
use crate::namespace::Namespace;
use crate::page;
use crate::state::Shared;

// File locking, which is a Git LFS API in its own right and shares nothing
// with the transfer surface but the namespace it is scoped to.

pub(super) async fn create_lock(
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

pub(super) async fn list_locks(
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

pub(super) async fn verify_locks(
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

pub(super) async fn unlock(
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

    let forced = lock.owner.name != caller;
    if forced {
        if !request.force {
            return Err(Error::Forbidden);
        }
        permission.require_admin()?;
    }

    state.locks.remove(&ns, &id).await?;

    if forced {
        crate::audit::audit_log!(
            actor = caller,
            namespace = %ns,
            path = lock.path,
            owner = lock.owner.name,
            "a lock was force-opened over its owner"
        );
    }

    Ok(Json(LockResponse { lock }))
}
