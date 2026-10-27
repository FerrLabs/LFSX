use std::path::Path;

use super::ConfigError;

#[derive(Clone, PartialEq, Eq)]
pub struct SessionKey([u8; 32]);

impl SessionKey {
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for SessionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionKey(..)")
    }
}

const EXPECTED: &str = "a file holding a 32 byte key as 64 hex characters";

pub(super) fn session_key(file: Option<&str>) -> Result<Option<SessionKey>, ConfigError> {
    let Some(path) = file.filter(|path| !path.is_empty()) else {
        return Ok(None);
    };

    let contents = std::fs::read_to_string(Path::new(path))
        .map_err(|_| ConfigError::new("LFSX_SESSION_KEY_FILE", path, EXPECTED))?;

    hex::decode(contents.trim())
        .ok()
        .and_then(|raw| <[u8; 32]>::try_from(raw).ok())
        .map(|bytes| Some(SessionKey::new(bytes)))
        .ok_or_else(|| ConfigError::new("LFSX_SESSION_KEY_FILE", path, EXPECTED))
}
