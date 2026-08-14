use axum::http::HeaderMap;
use axum::http::header::AUTHORIZATION;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;

pub fn token(headers: &HeaderMap) -> Option<String> {
    let (scheme, credentials) = headers.get(AUTHORIZATION)?.to_str().ok()?.split_once(' ')?;

    if scheme.eq_ignore_ascii_case("bearer") {
        return non_empty(credentials.trim());
    }

    if scheme.eq_ignore_ascii_case("basic") {
        return from_basic(credentials.trim());
    }

    None
}

fn from_basic(credentials: &str) -> Option<String> {
    let decoded = String::from_utf8(STANDARD.decode(credentials).ok()?).ok()?;
    let (username, password) = decoded.split_once(':')?;

    non_empty(password).or_else(|| non_empty(username))
}

fn non_empty(candidate: &str) -> Option<String> {
    (!candidate.is_empty()).then(|| candidate.to_owned())
}
