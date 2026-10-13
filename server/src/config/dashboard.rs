use std::path::PathBuf;

use super::{Auth, Forge};
use crate::namespace::Namespace;

#[derive(Debug, Clone)]
pub struct Dashboard {
    pub dir: PathBuf,
    pub admins: Option<Namespace>,
}

pub(super) const DASHBOARD_DIR: &str = "/usr/share/lfsx/dashboard";

pub(super) fn dashboard(
    enabled: Option<&str>,
    dir: Option<&str>,
    repo: Option<&str>,
    forge: Option<&str>,
    auth: &Auth,
    forges: &[Forge],
) -> Option<Dashboard> {
    if enabled != Some("true") {
        return None;
    }

    let forge = forge.filter(|forge| !forge.is_empty());
    if let Some(forge) = forge
        && !forges.iter().any(|named| named.name == forge)
    {
        panic!("LFSX_DASHBOARD_FORGE names {forge}, which LFSX_FORGES does not list");
    }

    let admins = repo.filter(|repo| !repo.is_empty()).map(|repo| {
        repo.split_once('/')
            .and_then(|(org, name)| match forge {
                Some(forge) => Namespace::on(forge, org, name).ok(),
                None => Namespace::new(org, name).ok(),
            })
            .unwrap_or_else(|| panic!("LFSX_DASHBOARD_REPO is not org/repo: {repo}"))
    });
    if admins.is_none() && matches!(auth, Auth::Forge { .. }) {
        panic!(
            "LFSX_DASHBOARD=true needs LFSX_DASHBOARD_REPO: the dashboard is shown to the admins of \
             that repository, and nobody else"
        );
    }

    Some(Dashboard {
        dir: dir
            .filter(|dir| !dir.is_empty())
            .unwrap_or(DASHBOARD_DIR)
            .into(),
        admins,
    })
}
