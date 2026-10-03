use super::{Auth, Provider, allowed, anonymous_read, api_url_from};
use crate::auth::Namespaces;
use crate::namespace::is_forge_name;

#[derive(Debug, Clone)]
pub struct Forge {
    pub name: String,
    pub auth: Auth,
}

pub(super) fn from_env(primary: &Auth) -> Vec<Forge> {
    let Some(names) = std::env::var("LFSX_FORGES")
        .ok()
        .filter(|names| !names.trim().is_empty())
    else {
        return Vec::new();
    };

    parse(&names, primary, |variable| std::env::var(variable).ok())
}

pub(super) fn parse(
    names: &str,
    primary: &Auth,
    read: impl Fn(&str) -> Option<String>,
) -> Vec<Forge> {
    let Auth::Forge {
        cache_ttl,
        rejection_ttl,
        lookup_budget,
        ..
    } = primary
    else {
        panic!(
            "LFSX_FORGES is set and LFSX_AUTH=disabled: with authentication off there is nothing to \
             ask the other forges about"
        );
    };

    let mut forges: Vec<Forge> = Vec::new();
    for name in names
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        if !is_forge_name(name) {
            panic!(
                "LFSX_FORGES names {name}: a forge name is 1 to 32 lowercase letters, digits or \
                 dashes, and not api or dashboard"
            );
        }
        if forges.iter().any(|forge| forge.name == name) {
            panic!("LFSX_FORGES names {name} twice");
        }

        let prefix = format!("LFSX_FORGE_{}_", name.to_uppercase().replace('-', "_"));
        let variable = |suffix: &str| format!("{prefix}{suffix}");
        let value = |suffix: &str| read(&variable(suffix));

        let provider = match value("AUTH").as_deref() {
            Some("github") => Provider::Github,
            Some("gitlab") => Provider::Gitlab,
            Some("gitea") | Some("forgejo") => Provider::Gitea,
            _ => panic!(
                "{} must be github, gitlab, gitea or forgejo",
                variable("AUTH")
            ),
        };

        forges.push(Forge {
            name: name.to_owned(),
            auth: Auth::Forge {
                provider,
                api_url: api_url_from(provider, &variable("API_URL"), value("API_URL").as_deref()),
                cache_ttl: *cache_ttl,
                rejection_ttl: *rejection_ttl,
                lookup_budget: *lookup_budget,
                github_app: None,
                anonymous_read: anonymous_read(value("ANONYMOUS_READ").as_deref()),
                restricted: Namespaces::parse(
                    &variable("RESTRICTED"),
                    value("RESTRICTED").as_deref(),
                ),
                allowed: allowed(&variable("ALLOWED"), value("ALLOWED").as_deref()),
            },
        });
    }

    forges
}

#[cfg(test)]
mod tests;
