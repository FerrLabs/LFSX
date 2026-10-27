use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

struct Held {
    token: String,
    until: Instant,
}

#[derive(Default)]
pub struct ForgeSessions {
    held: Mutex<HashMap<String, Held>>,
}

impl ForgeSessions {
    pub fn open(&self, token: String, lifetime: Duration) -> String {
        let mut raw = [0u8; 32];
        getrandom::fill(&mut raw).expect("the operating system has a random number generator");
        let id = URL_SAFE_NO_PAD.encode(raw);
        let now = Instant::now();

        let mut held = self.held.lock().unwrap();
        held.retain(|_, session| session.until > now);
        held.insert(
            id.clone(),
            Held {
                token,
                until: now + lifetime,
            },
        );

        id
    }

    pub fn token(&self, id: &str) -> Option<String> {
        self.held
            .lock()
            .unwrap()
            .get(id)
            .filter(|session| session.until > Instant::now())
            .map(|session| session.token.clone())
    }

    pub fn close(&self, id: &str) {
        self.held.lock().unwrap().remove(id);
    }
}

impl std::fmt::Debug for ForgeSessions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForgeSessions").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_opened_session_gives_back_its_token_until_closed() {
        let sessions = ForgeSessions::default();
        let id = sessions.open("secret".into(), Duration::from_secs(60));

        assert_eq!(sessions.token(&id).as_deref(), Some("secret"));
        sessions.close(&id);
        assert_eq!(sessions.token(&id), None);
    }

    #[test]
    fn an_unknown_id_has_no_token() {
        assert_eq!(ForgeSessions::default().token("nope"), None);
    }

    #[test]
    fn an_expired_session_has_no_token() {
        let sessions = ForgeSessions::default();
        let id = sessions.open("secret".into(), Duration::ZERO);

        assert_eq!(sessions.token(&id), None);
    }

    #[test]
    fn each_session_gets_its_own_id() {
        let sessions = ForgeSessions::default();
        let first = sessions.open("a".into(), Duration::from_secs(60));
        let second = sessions.open("a".into(), Duration::from_secs(60));

        assert_ne!(first, second);
    }
}
