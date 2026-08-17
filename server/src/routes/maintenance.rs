use axum::extract::State;
use axum::{Extension, Json};

use crate::auth::Permission;
use crate::error::Error;
use crate::model::{CompressRequest, DedupeRequest, RetainRequest};
use crate::namespace::Namespace;
use crate::state::Shared;
use crate::storage::{CompressReport, DedupeReport, SweepReport, VerifyReport};

// Operations an operator runs against a repository rather than a client: they
// rewrite or measure what is already stored, and each one asks for rights a
// pushing client is not assumed to have.

pub(super) async fn retain(
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
pub(super) async fn dedupe(
    State(state): State<Shared>,
    Extension(ns): Extension<Namespace>,
    Extension(permission): Extension<Permission>,
    Json(request): Json<DedupeRequest>,
) -> Result<Json<DedupeReport>, Error> {
    permission.require_admin()?;

    let report = state.store.dedupe(&ns, request.dry_run).await?;

    Ok(Json(report))
}

pub(super) async fn compress(
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
pub(super) async fn audit(
    State(state): State<Shared>,
    Extension(ns): Extension<Namespace>,
    Extension(permission): Extension<Permission>,
) -> Result<Json<VerifyReport>, Error> {
    permission.require_admin()?;

    let report = state.store.verify(&ns).await?;

    Ok(Json(report))
}
