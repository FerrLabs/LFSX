use super::*;

const SECRET: &[u8] = b"a key the tests share with nobody";

fn alice() -> Viewer {
    Viewer::Token("alice".into())
}

#[test]
fn a_sealed_session_opens_with_the_key_that_sealed_it() {
    let sealed = seal(SECRET, alice(), None).unwrap();

    assert_eq!(
        open(SECRET, &sealed).map(|claims| claims.viewer),
        Some(alice())
    );
}

#[test]
fn a_session_sealed_with_another_key_does_not_open() {
    let sealed = seal(b"some other server's key", alice(), None).unwrap();

    assert!(open(SECRET, &sealed).is_none());
}

#[test]
fn a_session_whose_claims_were_changed_does_not_open() {
    let sealed = seal(SECRET, alice(), None).unwrap();
    let (_, signature) = sealed.split_once('.').unwrap();
    let forged = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&Claims {
            viewer: Viewer::Forge("mallory".into()),
            issued: None,
            expires: now() + 3600,
        })
        .unwrap(),
    );

    assert!(open(SECRET, &format!("{forged}.{signature}")).is_none());
}

#[test]
fn an_expired_session_does_not_open() {
    let claims = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&Claims {
            viewer: alice(),
            issued: None,
            expires: now() - 1,
        })
        .unwrap(),
    );
    let mut mac = mac(SECRET);
    mac.update(claims.as_bytes());
    let sealed = format!(
        "{claims}.{}",
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    );

    assert!(open(SECRET, &sealed).is_none());
}

#[test]
fn garbage_does_not_open() {
    for sealed in ["", ".", "nodot", "a.b", "!!!.???"] {
        assert!(open(SECRET, sealed).is_none(), "{sealed}");
    }
}

#[test]
fn the_session_cookie_is_found_among_others() {
    let mut headers = HeaderMap::new();
    headers.insert(
        COOKIE,
        HeaderValue::from_static("theme=dark; lfsx_session=abc.def; other=1"),
    );

    assert_eq!(presented(&headers), Some("abc.def"));
}

#[test]
fn a_cookie_that_only_starts_like_the_session_one_is_not_taken_for_it() {
    let mut headers = HeaderMap::new();
    headers.insert(COOKIE, HeaderValue::from_static("lfsx_session_old=abc.def"));

    assert_eq!(presented(&headers), None);
}
