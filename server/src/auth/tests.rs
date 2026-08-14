use std::thread::sleep;
use std::time::Duration;

use axum::http::HeaderMap;
use axum::http::header::AUTHORIZATION;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use super::*;

fn headers(value: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, value.parse().unwrap());
    headers
}

fn basic(raw: &str) -> String {
    format!("Basic {}", STANDARD.encode(raw))
}

#[test]
fn the_password_half_of_basic_credentials_is_the_token() {
    let token = credentials::token(&headers(&basic("bryan:ghp_secret")));

    assert_eq!(token.as_deref(), Some("ghp_secret"));
}

#[test]
fn a_token_sent_as_the_username_is_still_found() {
    let token = credentials::token(&headers(&basic("ghp_secret:")));

    assert_eq!(token.as_deref(), Some("ghp_secret"));
}

#[test]
fn bearer_credentials_are_accepted() {
    let token = credentials::token(&headers("Bearer ghp_secret"));

    assert_eq!(token.as_deref(), Some("ghp_secret"));
}

#[test]
fn unusable_credentials_yield_no_token() {
    for value in [
        "Basic not-base64!!",
        "Basic ",
        "Digest nonce=1",
        "ghp_secret",
        &basic("no-colon-here"),
        &basic(":"),
    ] {
        assert!(
            credentials::token(&headers(value)).is_none(),
            "{value} was read as a token"
        );
    }
}

#[test]
fn a_cached_permission_is_scoped_to_its_token_and_repository() {
    let cache = Cache::new(Duration::from_secs(60));
    let ns = Namespace::new("FerrLabs", "LFSX").unwrap();
    cache.insert("writer", &ns, Permission::Write);

    assert_eq!(cache.get("writer", &ns), Some(Permission::Write));
    assert_eq!(cache.get("someone-else", &ns), None);
    assert_eq!(
        cache.get("writer", &Namespace::new("FerrLabs", "Other").unwrap()),
        None
    );
}

#[test]
fn a_cached_permission_stops_being_served_once_it_expires() {
    let cache = Cache::new(Duration::from_millis(20));
    let ns = Namespace::new("FerrLabs", "LFSX").unwrap();
    cache.insert("writer", &ns, Permission::Write);

    sleep(Duration::from_millis(40));

    assert_eq!(cache.get("writer", &ns), None);
}

#[test]
fn only_write_permission_satisfies_a_write() {
    assert!(Permission::Write.require_write().is_ok());
    assert!(Permission::Read.require_write().is_err());
}
