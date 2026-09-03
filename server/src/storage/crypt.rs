use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};

use crate::error::Error;

// What encryption at rest is for, said plainly so nobody reads more into it than
// is there: it protects the bytes on a disk somebody else can read. A stolen
// drive, a leaked backup, a decommissioned volume, a bucket whose provider is
// not you. It does not protect against anyone who has the running server,
// because that process holds the key by construction.

pub const KEY: usize = 32;
pub const SALT: usize = 16;
pub const ID: usize = 4;
pub const TAG: u64 = 16;

// A key is identified by a hash of itself rather than by a number an operator
// assigns. Two things follow, and both are the point: an id can never name a
// different key than the one it was written with, and rotating is appending a
// line rather than remembering which number is next.
pub type KeyId = [u8; ID];

pub struct Keyring {
    // The first key is the one writes use. Every key is accepted for reads,
    // which is what makes rotation something other than re-encrypting the store
    // in one go.
    keys: Vec<([u8; KEY], KeyId)>,
}

impl Keyring {
    pub fn from_source(source: &crate::config::KeySource) -> Result<Self, Error> {
        match source {
            crate::config::KeySource::File(path) => Self::load(path),
            crate::config::KeySource::Command(hook) => Self::exec(hook),
        }
    }

    pub fn load(path: &std::path::Path) -> Result<Self, Error> {
        let contents = std::fs::read_to_string(path).map_err(|error| {
            Error::Storage(std::io::Error::other(format!(
                "the encryption key file at {} could not be read: {error}",
                path.display()
            )))
        })?;

        Self::parse(&contents)
    }

    // The hook runs through the platform shell, because "the command a KMS
    // documents" always carries arguments, and its stdout is read exactly like
    // the key file: hex keys one per line, first line writes. The keys never
    // rest on disk, the audit trail is the source's own, and rotation stays
    // "the source returns a new first line". A failure is spelled out with the
    // command's stderr, because the operator debugging this sees nothing else.
    fn exec(hook: &str) -> Result<Self, Error> {
        let output = shell(hook).output().map_err(|error| {
            Error::Storage(std::io::Error::other(format!(
                "the encryption key command could not be run: {error}"
            )))
        })?;

        if !output.status.success() {
            return Err(Error::Storage(std::io::Error::other(format!(
                "the encryption key command failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ))));
        }

        Self::parse(&String::from_utf8_lossy(&output.stdout))
    }

    pub(super) fn parse(contents: &str) -> Result<Self, Error> {
        let mut keys: Vec<([u8; KEY], KeyId)> = Vec::new();

        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let raw = hex::decode(line)
                .ok()
                .filter(|raw| raw.len() == KEY)
                .ok_or(Error::Misconfigured(
                    "an encryption key must be 32 bytes as 64 hex characters, one key per line",
                ))?;

            let mut key = [0u8; KEY];
            key.copy_from_slice(&raw);
            let id = identify(&key);

            // Two keys answering to the same id would make a stored object
            // ambiguous, and the object cannot say which one it meant. Four
            // bytes of a hash make this vanishingly unlikely and free to check,
            // and a duplicated line is the case that actually happens.
            if keys.iter().any(|(_, known)| *known == id) {
                return Err(Error::Misconfigured(
                    "two encryption keys hash to the same id: the same key is probably listed twice",
                ));
            }

            keys.push((key, id));
        }

        if keys.is_empty() {
            return Err(Error::Misconfigured(
                "the encryption key file holds no keys",
            ));
        }

        Ok(Self { keys })
    }

    pub fn writing(&self) -> ObjectKey {
        let (key, id) = &self.keys[0];

        ObjectKey::derive(key, *id, random_salt())
    }

    pub fn reading(&self, id: KeyId, salt: [u8; SALT]) -> Result<ObjectKey, Error> {
        self.keys
            .iter()
            .find(|(_, known)| *known == id)
            .map(|(key, id)| ObjectKey::derive(key, *id, salt))
            .ok_or(Error::UnknownKey)
    }
}

fn identify(key: &[u8; KEY]) -> KeyId {
    let mut id = [0u8; ID];
    id.copy_from_slice(&blake3::hash(key).as_bytes()[..ID]);
    id
}

fn random_salt() -> [u8; SALT] {
    let mut salt = [0u8; SALT];
    getrandom::fill(&mut salt).expect("the operating system has a random number generator");
    salt
}

// One key per object, derived from the master key and a salt stored with the
// object. It costs a hash per open and buys the thing that matters: a nonce is
// only ever a frame counter, so two objects cannot collide on one however many
// of them a store holds. Deriving per object is what makes that true by
// construction rather than by a birthday bound on a random nonce prefix.
pub struct ObjectKey {
    cipher: ChaCha20Poly1305,
    id: KeyId,
    salt: [u8; SALT],
}

const CONTEXT: &str = "LFSX 2026-08-16 object encryption key";

impl ObjectKey {
    fn derive(master: &[u8; KEY], id: KeyId, salt: [u8; SALT]) -> Self {
        let mut hasher = blake3::Hasher::new_derive_key(CONTEXT);
        hasher.update(master);
        hasher.update(&salt);
        let derived = hasher.finalize();

        Self {
            cipher: ChaCha20Poly1305::new(&Key::from(*derived.as_bytes())),
            id,
            salt,
        }
    }

    pub fn id(&self) -> KeyId {
        self.id
    }

    pub fn salt(&self) -> [u8; SALT] {
        self.salt
    }

    pub fn seal(&self, frame: u32, last: bool, oid: &str, plain: &[u8]) -> Result<Vec<u8>, Error> {
        self.cipher
            .encrypt(
                &nonce(frame),
                Payload {
                    msg: plain,
                    aad: &associated(frame, last, oid),
                },
            )
            .map_err(|_| Error::Storage(std::io::Error::other("a frame could not be encrypted")))
    }

    pub fn open(&self, frame: u32, last: bool, oid: &str, sealed: &[u8]) -> Result<Vec<u8>, Error> {
        self.cipher
            .decrypt(
                &nonce(frame),
                Payload {
                    msg: sealed,
                    aad: &associated(frame, last, oid),
                },
            )
            .map_err(|_| Error::Tampered)
    }
}

fn nonce(frame: u32) -> Nonce {
    let mut bytes = [0u8; 12];
    bytes[8..].copy_from_slice(&frame.to_be_bytes());

    Nonce::from(bytes)
}

// What each frame is bound to, so that a frame is only ever valid where it was
// written. The index stops two frames of one object being swapped; the last-frame
// flag stops an object being truncated to a shorter one that still verifies; the
// object id stops a whole file being moved on top of another, which matters more
// here than usual because the shared content store means one file answers for
// every repository that pushed those bytes.
fn associated(frame: u32, last: bool, oid: &str) -> Vec<u8> {
    let mut aad = Vec::with_capacity(oid.len() + 5);
    aad.extend_from_slice(oid.as_bytes());
    aad.extend_from_slice(&frame.to_le_bytes());
    aad.push(u8::from(last));
    aad
}

#[cfg(test)]
mod tests;

#[cfg(unix)]
fn shell(hook: &str) -> std::process::Command {
    let mut command = std::process::Command::new("sh");
    command.arg("-c").arg(hook);
    command
}

#[cfg(windows)]
fn shell(hook: &str) -> std::process::Command {
    let mut command = std::process::Command::new("cmd");
    command.arg("/C").arg(hook);
    command
}
