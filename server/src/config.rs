use std::net::SocketAddr;
use std::path::PathBuf;

use crate::model::Action;
use crate::storage::Namespace;

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: SocketAddr,
    pub storage_root: PathBuf,
    pub public_url: String,
    pub action_lifetime: u32,
}

impl Config {
    pub fn from_env() -> Self {
        let bind = std::env::var("LFSX_BIND")
            .ok()
            .and_then(|raw| raw.parse().ok())
            .unwrap_or_else(|| SocketAddr::from(([0, 0, 0, 0], 8080)));

        let storage_root = std::env::var("LFSX_STORAGE_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/var/lib/lfsx"));

        let public_url = std::env::var("LFSX_PUBLIC_URL")
            .unwrap_or_else(|_| format!("http://{bind}"))
            .trim_end_matches('/')
            .to_owned();

        Self {
            bind,
            storage_root,
            public_url,
            action_lifetime: 1800,
        }
    }

    pub fn object_url(&self, ns: &Namespace<'_>, oid: &str) -> String {
        format!("{}/{}/{}/objects/{oid}", self.public_url, ns.org, ns.repo)
    }

    pub fn verify_url(&self, ns: &Namespace<'_>) -> String {
        format!("{}/{}/{}/objects/verify", self.public_url, ns.org, ns.repo)
    }

    pub fn action(&self, href: String) -> Action {
        Action {
            href,
            expires_in: self.action_lifetime,
        }
    }
}
