use axum::extract::State;
use axum::http::HeaderMap;
use axum::{Extension, Json};

use crate::audit::audit_log;
use crate::auth::{Actor, Permission};
use crate::error::Error;
use crate::model::{CompressRequest, DedupeRequest, RetainRequest};
use crate::namespace::Namespace;
use crate::state::Shared;
use crate::storage::{CompressReport, DedupeReport, SweepReport, VerifyReport};

// Operations an operator runs against a repository rather than a client: they
// rewrite or measure what is already stored, and each one asks for rights a
// pushing client is not assumed to have.

// The one operation here that unlinks files, so the real run asks for the
// rights of someone the forge treats as an administrator, same as force-opening
// a lock. The dry run stays at push rights: it is a read of what collection
// would free, and the contributor who wants that number is exactly who should
// be able to see it without being able to act on it.
pub(super) async fn retain(
    State(state): State<Shared>,
    Extension(ns): Extension<Namespace>,
    Extension(permission): Extension<Permission>,
    headers: HeaderMap,
    Json(request): Json<RetainRequest>,
) -> Result<Json<SweepReport>, Error> {
    permission.require_write()?;
    if !request.dry_run {
        permission.require_admin()?;
    }
    let actor = attributed(&state, &headers, request.dry_run).await?;

    let retained = request.oids.into_iter().collect();
    let report = state
        .store
        .sweep(&ns, &retained, state.config.gc_grace, request.dry_run)
        .await?;

    if let Some(Actor(actor)) = actor {
        audit_log!(
            actor,
            namespace = %ns,
            swept = report.swept,
            bytes = report.bytes,
            within_grace = report.within_grace,
            "a retain sweep unlinked what the keep list did not name"
        );
    }

    Ok(Json(report))
}

// Folding a repository's objects into the shared store rewrites what is on
// disk, so it asks for the rights of someone who could delete them instead.
pub(super) async fn dedupe(
    State(state): State<Shared>,
    Extension(ns): Extension<Namespace>,
    Extension(permission): Extension<Permission>,
    headers: HeaderMap,
    Json(request): Json<DedupeRequest>,
) -> Result<Json<DedupeReport>, Error> {
    permission.require_admin()?;
    let actor = attributed(&state, &headers, request.dry_run).await?;

    let report = state.store.dedupe(&ns, request.dry_run).await?;

    if let Some(Actor(actor)) = actor {
        audit_log!(
            actor,
            namespace = %ns,
            adopted = report.adopted,
            linked = report.linked,
            reclaimed = report.reclaimed,
            refused = report.refused,
            "a repository's objects were folded into the shared store"
        );
    }

    Ok(Json(report))
}

pub(super) async fn compress(
    State(state): State<Shared>,
    Extension(ns): Extension<Namespace>,
    Extension(permission): Extension<Permission>,
    headers: HeaderMap,
    Json(request): Json<CompressRequest>,
) -> Result<Json<CompressReport>, Error> {
    permission.require_admin()?;
    let actor = attributed(&state, &headers, request.dry_run).await?;

    let report = state.store.compress(&ns, request.dry_run).await?;

    if let Some(Actor(actor)) = actor {
        audit_log!(
            actor,
            namespace = %ns,
            compressed = report.compressed,
            before = report.before,
            after = report.after,
            "a repository's stored objects were rewritten compressed"
        );
    }

    Ok(Json(report))
}

// Who a real run answers to. Resolved before the mutation, because a
// privileged operation that cannot be attributed should not happen; a dry run
// is a read and stays silent.
async fn attributed(
    state: &Shared,
    headers: &HeaderMap,
    dry_run: bool,
) -> Result<Option<Actor>, Error> {
    if dry_run {
        return Ok(None);
    }

    Ok(Some(state.authorizer.actor(headers).await?))
}

// Reading every object back through the path a download takes is the only check
// that still means something once the file on disk is not the object. It is a
// read of the whole repository, so it asks for the rights of someone who would
// be entitled to read it all anyway.
pub(super) async fn audit(
    State(state): State<Shared>,
    Extension(ns): Extension<Namespace>,
    Extension(permission): Extension<Permission>,
) -> Result<Json<VerifyReport>, Error> {
    permission.require_admin()?;

    let report = state.store.verify(&ns).await?;

    Ok(Json(report))
}
