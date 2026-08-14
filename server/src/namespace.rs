use crate::error::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Namespace {
    org: String,
    repo: String,
}

impl Namespace {
    pub fn new(org: impl Into<String>, repo: impl Into<String>) -> Result<Self, Error> {
        let (org, repo) = (org.into(), repo.into());

        (is_well_formed(&org) && is_well_formed(&repo))
            .then_some(Self { org, repo })
            .ok_or(Error::MalformedNamespace)
    }

    pub fn org(&self) -> &str {
        &self.org
    }

    pub fn repo(&self) -> &str {
        &self.repo
    }
}

impl std::fmt::Display for Namespace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.org, self.repo)
    }
}

fn is_well_formed(segment: &str) -> bool {
    !segment.is_empty()
        && segment.len() <= 100
        && segment != "."
        && segment != ".."
        && segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests;
