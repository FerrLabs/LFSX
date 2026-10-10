use std::path::PathBuf;
use std::time::Duration;

use super::seconds;
use crate::auth::Namespaces;

#[derive(Debug, Clone)]
pub enum Auth {
    Forge {
        provider: Provider,
        api_url: String,
        // A GitHub App identity for the server's own calls, so the anonymous
        // lookup spends the App installation's quota instead of the 60-an-hour
        // unauthenticated one. A file path for the key, never the key itself,
        // same discipline as the encryption key.
        github_app: Option<GithubApp>,
        cache_ttl: Duration,
        rejection_ttl: Duration,
        // Lookups a minute this server will spend on the forge, counted only
        // when neither cache could answer. None is no ceiling at all.
        lookup_budget: Option<u32>,
        // Whether a request with no credentials is resolved against the forge
        // instead of refused. Off unless asked for, because a server that serves
        // strangers should be a decision somebody made.
        anonymous_read: bool,
        // Namespaces whose objects take write access to read, so a repository the
        // forge serves publicly can still keep its assets to the people who could
        // push them.
        restricted: Namespaces,
        allowed: Option<Namespaces>,
    },
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubApp {
    pub app_id: String,
    pub key_file: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Github,
    Gitlab,
    // Gitea and Forgejo, which are one API: Forgejo is a fork of Gitea and
    // answers the same routes, so the only thing that tells two instances apart
    // is the root they are reached at.
    Gitea,
}

impl Provider {
    // None where there is no such thing as the instance.
    //
    // github.com and gitlab.com are where a repository is unless the operator
    // says otherwise, so defaulting there is nearly always right. Gitea is
    // software rather than a place. gitea.com exists, but an operator who names
    // this provider is almost certainly running their own, and quietly resolving
    // their namespaces against a stranger's forge is worse than not starting: a
    // public repository there that happens to share a name would hand out
    // anonymous read on objects it has nothing to do with.
    fn default_api_url(self) -> Option<&'static str> {
        match self {
            Self::Github => Some("https://api.github.com"),
            Self::Gitlab => Some("https://gitlab.com/api/v4"),
            Self::Gitea => None,
        }
    }

    fn api_url_variable(self) -> &'static str {
        match self {
            Self::Github => "LFSX_GITHUB_API_URL",
            Self::Gitlab => "LFSX_GITLAB_API_URL",
            Self::Gitea => "LFSX_GITEA_API_URL",
        }
    }
}

const CACHE_TTL: Duration = Duration::from_secs(60);
const REJECTION_TTL: Duration = Duration::from_secs(10);
// Generous enough that a busy server never meets it, since a lookup is one
// distinct token against one repository per cache lifetime rather than one per
// request, and tight enough that a flood costs ten a second instead of whatever
// the network will carry.
pub(super) const LOOKUP_BUDGET: u32 = 600;
// Opt in, not opt out. Serving objects to a caller with no credentials at all is
// a decision an operator should make on purpose: it costs them the bandwidth of
// anyone who finds the endpoint, on a server whose whole job is to move files
// measured in gigabytes. Nothing confidential is at stake, since a request with
// no credentials is still resolved against the forge and a private repository is
// still refused, but "anyone may pull from you" is not a sensible thing to
// inherit by default.
//
// Only the exact string opens it. A typo, an empty value or a `1` leaves it
// closed, because the failure that matters here is the one that opens the door
// when nobody meant to.
pub(super) fn anonymous_read(value: Option<&str>) -> bool {
    value == Some("true")
}

impl Auth {
    pub(super) fn from_env() -> Self {
        if std::env::var("LFSX_AUTH").as_deref() == Ok("disabled") {
            tracing::warn!(
                "LFSX_AUTH=disabled: every request is accepted, run this on a trusted network only"
            );
            if is_set(std::env::var("LFSX_ALLOWED").ok().as_deref()) {
                tracing::warn!(
                    "LFSX_ALLOWED is set and LFSX_AUTH=disabled, so it does nothing: with no forge \
                     to ask, every repository is served"
                );
            }
            if is_set(std::env::var("LFSX_RESTRICTED").ok().as_deref()) {
                tracing::warn!(
                    "LFSX_RESTRICTED is set and LFSX_AUTH=disabled, so it does nothing: every \
                     caller already holds every right"
                );
            }
            return Self::Disabled;
        }

        let provider = provider(std::env::var("LFSX_AUTH").ok().as_deref());

        Self::Forge {
            provider,
            api_url: api_url(
                provider,
                std::env::var(provider.api_url_variable()).ok().as_deref(),
            ),
            cache_ttl: seconds("LFSX_AUTH_CACHE_TTL").unwrap_or(CACHE_TTL),
            rejection_ttl: seconds("LFSX_AUTH_REJECTION_TTL").unwrap_or(REJECTION_TTL),
            lookup_budget: lookup_budget(std::env::var("LFSX_AUTH_LOOKUP_BUDGET").ok().as_deref()),
            github_app: github_app(provider),
            anonymous_read: anonymous_read(std::env::var("LFSX_ANONYMOUS_READ").ok().as_deref()),
            restricted: Namespaces::parse(
                "LFSX_RESTRICTED",
                std::env::var("LFSX_RESTRICTED").ok().as_deref(),
            ),
            allowed: allowed(
                "LFSX_ALLOWED",
                std::env::var("LFSX_ALLOWED").ok().as_deref(),
            ),
        }
    }
}

pub(super) fn is_set(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.trim().is_empty())
}

pub(super) fn allowed(variable: &str, value: Option<&str>) -> Option<Namespaces> {
    is_set(value).then(|| Namespaces::parse(variable, value))
}

// Both variables or neither. One without the other is a configuration that
// says two things at once, and an operator who set up an App meant to have its
// quota, so the mistake is refused at boot instead of quietly ignored.
pub(super) fn github_app(provider: Provider) -> Option<GithubApp> {
    github_app_from(provider, "LFSX_", "LFSX_AUTH", |variable| {
        std::env::var(variable).ok()
    })
}

pub(super) fn github_app_from(
    provider: Provider,
    prefix: &str,
    auth_variable: &str,
    read: impl Fn(&str) -> Option<String>,
) -> Option<GithubApp> {
    let id_variable = format!("{prefix}GITHUB_APP_ID");
    let key_variable = format!("{prefix}GITHUB_APP_KEY_FILE");
    let id = read(&id_variable).filter(|id| !id.is_empty());
    let key_file = read(&key_variable).filter(|path| !path.is_empty());

    match (id, key_file) {
        (None, None) => None,
        (Some(app_id), Some(key_file)) => {
            if provider != Provider::Github {
                tracing::warn!(
                    "{id_variable} is set but {auth_variable} is not github, so it does nothing"
                );
                return None;
            }
            Some(GithubApp {
                app_id,
                key_file: PathBuf::from(key_file),
            })
        }
        _ => panic!(
            "{id_variable} and {key_variable} come together: one without the other is half an \
             identity, and guessing which half was meant is worse than stopping"
        ),
    }
}

// Zero is the one value that cannot mean what it says. A ceiling of no lookups
// is a server that refuses every caller it has not already seen, so it is read as
// the operator turning the ceiling off, which is the only other thing they could
// have meant. Anything unparseable is the default rather than a refusal to start:
// this bounds a cost, and getting it wrong should not take the server down.
pub(super) fn lookup_budget(value: Option<&str>) -> Option<u32> {
    match value.map(str::trim).map(str::parse::<u32>) {
        Some(Ok(0)) => None,
        Some(Ok(budget)) => Some(budget),
        Some(Err(_)) | None => Some(LOOKUP_BUDGET),
    }
}

// Anything unrecognised is GitHub, which is what an operator who set nothing
// almost certainly meant. Forgejo is named alongside Gitea because they are one
// API, and somebody running Forgejo should not have to know it began as a fork.
pub(super) fn provider(value: Option<&str>) -> Provider {
    match value {
        Some("gitlab") => Provider::Gitlab,
        Some("gitea") | Some("forgejo") => Provider::Gitea,
        _ => Provider::Github,
    }
}

// The trailing slash matters: every route is built by appending to this, so one
// left on the end produces `//repos/...`, which some forges answer and others do
// not, and the ones that do not answer 404 for a repository that is right there.
pub(super) fn api_url(provider: Provider, configured: Option<&str>) -> String {
    api_url_from(provider, provider.api_url_variable(), configured)
}

pub(super) fn api_url_from(provider: Provider, variable: &str, configured: Option<&str>) -> String {
    configured
        .map(str::to_owned)
        .or_else(|| provider.default_api_url().map(str::to_owned))
        .unwrap_or_else(|| {
            panic!(
                "{variable} must be set: a self-hosted forge has no default API root, and guessing \
                 one would resolve your repositories against somebody else's"
            )
        })
        .trim_end_matches('/')
        .to_owned()
}
