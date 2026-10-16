use std::path::PathBuf;

use super::{Auth, ConfigError, Forge};
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
) -> Result<Option<Dashboard>, ConfigError> {
    if enabled != Some("true") {
        return Ok(None);
    }

    let forge = forge.filter(|forge| !forge.is_empty());
    if let Some(forge) = forge
        && !forges.iter().any(|named| named.name == forge)
    {
        return Err(ConfigError::new(
            "LFSX_DASHBOARD_FORGE",
            forge,
            "a forge listed in LFSX_FORGES",
        ));
    }

    let admins = repo
        .filter(|repo| !repo.is_empty())
        .map(|repo| {
            repo.split_once('/')
                .and_then(|(org, name)| match forge {
                    Some(forge) => Namespace::on(forge, org, name).ok(),
                    None => Namespace::new(org, name).ok(),
                })
                .ok_or_else(|| ConfigError::new("LFSX_DASHBOARD_REPO", repo, "org/repo"))
        })
        .transpose()?;
    if admins.is_none() && matches!(auth, Auth::Forge { .. }) {
        return Err(ConfigError::new(
            "LFSX_DASHBOARD_REPO",
            "",
            "set: the dashboard is shown to the admins of that repository, and nobody else",
        ));
    }

    Ok(Some(Dashboard {
        dir: dir
            .filter(|dir| !dir.is_empty())
            .unwrap_or(DASHBOARD_DIR)
            .into(),
        admins,
    }))
}
