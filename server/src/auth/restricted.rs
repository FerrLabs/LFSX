use crate::namespace::Namespace;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Repo {
    Any,
    Prefix(String),
    Exact(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Pattern {
    org: String,
    repo: Repo,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Restricted(Vec<Pattern>);

impl Restricted {
    pub fn parse(value: Option<&str>) -> Self {
        let mut patterns = Vec::new();

        for entry in value.unwrap_or_default().split(',') {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }

            match Pattern::parse(entry) {
                Some(pattern) => patterns.push(pattern),
                // Dropped rather than widened, because an entry nobody can read as
                // org/repo must never become the whole organisation. Said out loud
                // because the direction it fails in is open: a typo leaves the
                // objects served to anyone the forge grants pull, and the boot line
                // below stays quiet when nothing parsed at all.
                None => tracing::warn!(
                    entry,
                    "LFSX_RESTRICTED entry is not org/repo and was ignored, so that \
                     repository keeps the permissions the forge gives it"
                ),
            }
        }

        Self(patterns)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn covers(&self, ns: &Namespace) -> bool {
        self.0.iter().any(|pattern| pattern.covers(ns))
    }
}

impl Pattern {
    fn parse(entry: &str) -> Option<Self> {
        let (org, repo) = entry.split_once('/')?;
        if org.is_empty() || repo.is_empty() {
            return None;
        }

        let repo = match repo.strip_suffix('*') {
            Some("") => Repo::Any,
            Some(prefix) => Repo::Prefix(prefix.to_lowercase()),
            None => Repo::Exact(repo.to_lowercase()),
        };

        Some(Self {
            org: org.to_lowercase(),
            repo,
        })
    }

    fn covers(&self, ns: &Namespace) -> bool {
        if !ns.org().eq_ignore_ascii_case(&self.org) {
            return false;
        }

        match &self.repo {
            Repo::Any => true,
            Repo::Prefix(prefix) => ns.repo().to_lowercase().starts_with(prefix),
            Repo::Exact(repo) => ns.repo().eq_ignore_ascii_case(repo),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ns(org: &str, repo: &str) -> Namespace {
        Namespace::new(org, repo).unwrap()
    }

    #[test]
    fn nothing_configured_covers_nothing() {
        let restricted = Restricted::parse(None);

        assert!(restricted.is_empty());
        assert!(!restricted.covers(&ns("acme", "assets")));
    }

    #[test]
    fn an_exact_entry_covers_only_that_repository() {
        let restricted = Restricted::parse(Some("acme/assets"));

        assert!(restricted.covers(&ns("acme", "assets")));
        assert!(!restricted.covers(&ns("acme", "assets-public")));
        assert!(!restricted.covers(&ns("other", "assets")));
    }

    #[test]
    fn a_trailing_star_covers_the_prefix_it_names() {
        let restricted = Restricted::parse(Some("acme/game-*"));

        assert!(restricted.covers(&ns("acme", "game-art")));
        assert!(restricted.covers(&ns("acme", "game-")));
        assert!(!restricted.covers(&ns("acme", "game")));
        assert!(!restricted.covers(&ns("acme", "tools")));
    }

    #[test]
    fn a_bare_star_covers_the_whole_organisation() {
        let restricted = Restricted::parse(Some("acme/*"));

        assert!(restricted.covers(&ns("acme", "anything")));
        assert!(!restricted.covers(&ns("acmecorp", "anything")));
    }

    #[test]
    fn entries_are_matched_without_regard_to_case() {
        let restricted = Restricted::parse(Some("ACME/Assets,acme/Game-*"));

        assert!(restricted.covers(&ns("acme", "assets")));
        assert!(restricted.covers(&ns("Acme", "ASSETS")));
        assert!(restricted.covers(&ns("acme", "GAME-art")));
    }

    #[test]
    fn several_entries_are_read_and_whitespace_around_them_is_not() {
        let restricted = Restricted::parse(Some(" acme/assets , acme/game-* "));

        assert!(restricted.covers(&ns("acme", "assets")));
        assert!(restricted.covers(&ns("acme", "game-art")));
    }

    #[test]
    fn an_entry_that_names_no_repository_is_dropped_rather_than_widened() {
        let restricted = Restricted::parse(Some("acme,,/assets,acme/,acme/assets"));

        assert_eq!(restricted.0.len(), 1);
        assert!(restricted.covers(&ns("acme", "assets")));
        assert!(!restricted.covers(&ns("acme", "anything")));
    }
}
