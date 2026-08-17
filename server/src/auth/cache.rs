use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use super::Permission;
use crate::error::Error;
use crate::namespace::Namespace;

type Key = ([u8; 32], String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Granted(Permission),
    Unauthenticated,
    Forbidden,
}

impl Decision {
    pub fn of(outcome: &Result<Permission, Error>) -> Option<Self> {
        match outcome {
            Ok(permission) => Some(Self::Granted(*permission)),
            Err(Error::Unauthenticated) => Some(Self::Unauthenticated),
            Err(Error::Forbidden) => Some(Self::Forbidden),
            Err(_) => None,
        }
    }
}

impl From<Decision> for Result<Permission, Error> {
    fn from(decision: Decision) -> Self {
        match decision {
            Decision::Granted(permission) => Ok(permission),
            Decision::Unauthenticated => Err(Error::Unauthenticated),
            Decision::Forbidden => Err(Error::Forbidden),
        }
    }
}

struct Entry {
    decision: Decision,
    expires_at: Instant,
}

pub struct Cache {
    granted_ttl: Duration,
    rejected_ttl: Duration,
    entries: Mutex<HashMap<Key, Entry>>,
}

impl Cache {
    pub fn new(granted_ttl: Duration, rejected_ttl: Duration) -> Self {
        Self {
            granted_ttl,
            rejected_ttl,
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub fn get(&self, caller: Caller<'_>, ns: &Namespace) -> Option<Decision> {
        let entries = self.entries.lock().expect("permission cache");
        let entry = entries.get(&key(caller, ns))?;

        (entry.expires_at > Instant::now()).then_some(entry.decision)
    }

    pub fn insert(&self, caller: Caller<'_>, ns: &Namespace, decision: Decision) {
        let now = Instant::now();
        let ttl = match decision {
            Decision::Granted(_) => self.granted_ttl,
            _ => self.rejected_ttl,
        };

        let mut entries = self.entries.lock().expect("permission cache");
        entries.retain(|_, entry| entry.expires_at > now);
        entries.insert(
            key(caller, ns),
            Entry {
                decision,
                expires_at: now + ttl,
            },
        );
    }
}

fn key(caller: Caller<'_>, ns: &Namespace) -> Key {
    (fingerprint(caller), ns.to_string())
}

pub struct IdentityCache {
    ttl: Duration,
    entries: Mutex<HashMap<[u8; 32], (String, Instant)>>,
}

impl IdentityCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub fn get(&self, token: &str) -> Option<String> {
        let entries = self.entries.lock().expect("identity cache");
        let (login, expires_at) = entries.get(&fingerprint(Caller::Token(token)))?;

        (*expires_at > Instant::now()).then(|| login.clone())
    }

    pub fn insert(&self, token: &str, login: &str) {
        let now = Instant::now();
        let mut entries = self.entries.lock().expect("identity cache");

        entries.retain(|_, (_, expires_at)| *expires_at > now);
        entries.insert(
            fingerprint(Caller::Token(token)),
            (login.to_owned(), now + self.ttl),
        );
    }
}

// Who is asking, for cache purposes. The two are hashed under different domains
// so an anonymous resolution can never be served to somebody presenting a token,
// nor the reverse: without the tag, a client sending the sentinel as its own
// token would collide with it.
#[derive(Debug, Clone, Copy)]
pub enum Caller<'a> {
    Token(&'a str),
    Anonymous,
}

fn fingerprint(caller: Caller<'_>) -> [u8; 32] {
    let mut hasher = Sha256::new();

    match caller {
        Caller::Token(token) => {
            hasher.update(b"token:");
            hasher.update(token.as_bytes());
        }
        Caller::Anonymous => hasher.update(b"anonymous"),
    }

    hasher.finalize().into()
}
