use crate::error::Error;

const FORGE_SEPARATOR: char = '~';

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Namespace {
    forge: Option<String>,
    org: String,
    repo: String,
    stored_org: String,
}

impl Namespace {
    pub fn new(org: impl Into<String>, repo: impl Into<String>) -> Result<Self, Error> {
        let (org, repo) = (org.into(), repo.into());

        (is_well_formed(&org) && is_well_formed(&repo))
            .then(|| Self {
                forge: None,
                stored_org: org.clone(),
                org,
                repo,
            })
            .ok_or(Error::MalformedNamespace)
    }

    pub fn on(
        forge: impl Into<String>,
        org: impl Into<String>,
        repo: impl Into<String>,
    ) -> Result<Self, Error> {
        let forge = forge.into();
        if !is_forge_name(&forge) {
            return Err(Error::MalformedNamespace);
        }
        let ns = Self::new(org, repo)?;

        Ok(Self {
            stored_org: format!("{forge}{FORGE_SEPARATOR}{}", ns.org),
            forge: Some(forge),
            ..ns
        })
    }

    pub fn forge(&self) -> Option<&str> {
        self.forge.as_deref()
    }

    pub fn org(&self) -> &str {
        &self.org
    }

    pub fn repo(&self) -> &str {
        &self.repo
    }

    pub fn stored_org(&self) -> &str {
        &self.stored_org
    }

    pub fn upstream(&self) -> String {
        format!("{}/{}", self.org, self.repo)
    }

    pub fn url_path(&self) -> String {
        match &self.forge {
            Some(forge) => format!("-/{forge}/{}/{}", self.org, self.repo),
            None => self.upstream(),
        }
    }
}

impl std::fmt::Display for Namespace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.stored_org, self.repo)
    }
}

pub fn is_forge_name(name: &str) -> bool {
    (1..=32).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !matches!(name, "api" | "dashboard")
}

fn is_well_formed(segment: &str) -> bool {
    !segment.is_empty()
        && segment.len() <= 100
        && !segment.starts_with('.')
        && segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests;
