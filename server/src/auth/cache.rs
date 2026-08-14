use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use super::Permission;
use crate::namespace::Namespace;

type Key = ([u8; 32], String);

struct Entry {
    permission: Permission,
    expires_at: Instant,
}

pub struct Cache {
    ttl: Duration,
    entries: Mutex<HashMap<Key, Entry>>,
}

impl Cache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub fn get(&self, token: &str, ns: &Namespace<'_>) -> Option<Permission> {
        let entries = self.entries.lock().expect("permission cache");
        let entry = entries.get(&key(token, ns))?;

        (entry.expires_at > Instant::now()).then_some(entry.permission)
    }

    pub fn insert(&self, token: &str, ns: &Namespace<'_>, permission: Permission) {
        let now = Instant::now();
        let mut entries = self.entries.lock().expect("permission cache");

        entries.retain(|_, entry| entry.expires_at > now);
        entries.insert(
            key(token, ns),
            Entry {
                permission,
                expires_at: now + self.ttl,
            },
        );
    }
}

fn key(token: &str, ns: &Namespace<'_>) -> Key {
    (
        Sha256::digest(token.as_bytes()).into(),
        format!("{}/{}", ns.org(), ns.repo()),
    )
}
