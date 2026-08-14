use crate::error::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Namespace<'a> {
    org: &'a str,
    repo: &'a str,
}

impl<'a> Namespace<'a> {
    pub fn new(org: &'a str, repo: &'a str) -> Result<Self, Error> {
        (is_well_formed(org) && is_well_formed(repo))
            .then_some(Self { org, repo })
            .ok_or(Error::MalformedNamespace)
    }

    pub fn org(&self) -> &str {
        self.org
    }

    pub fn repo(&self) -> &str {
        self.repo
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
