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

fn bucket_store(root: &tempfile::TempDir, endpoint: &str) -> Store {
    Store::bucket(
        crate::storage::s3::tests::store(endpoint),
        crate::storage::LocalStore::new(root.path()),
    )
}

#[tokio::test]
async fn an_empty_key_file_is_refused_rather_than_used_as_the_key() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::local(crate::storage::LocalStore::new(root.path()));
    store.write_meta(KEY, Vec::new()).await.unwrap();

    assert!(matches!(
        kept_key(&store).await,
        Err(Error::Misconfigured(_))
    ));
}

#[tokio::test]
async fn a_new_key_is_32_bytes_and_is_the_one_kept() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::local(crate::storage::LocalStore::new(root.path()));

    let key = kept_key(&store).await.unwrap();

    assert_eq!(key.len(), KEY_LENGTH);
    assert_eq!(kept_key(&store).await.unwrap(), key);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replicas_creating_the_key_at_once_end_with_the_same_one() {
    let (endpoint, _objects) = crate::storage::s3::tests::bucket().await;
    let root = tempfile::tempdir().unwrap();
    let stores: Vec<_> = (0..8).map(|_| bucket_store(&root, &endpoint)).collect();

    let keys = futures_util::future::join_all(stores.iter().map(kept_key)).await;
    let keys: Vec<_> = keys.into_iter().map(Result::unwrap).collect();

    assert!(keys.iter().all(|key| key == &keys[0]));
}
