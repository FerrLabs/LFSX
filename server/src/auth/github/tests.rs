use axum::Router;
use axum::http::StatusCode;
use axum::routing::get;

use super::*;

// A stand-in for the forge that answers one canned body, so each case below is
// the payload shape under test and nothing else.
async fn forge(status: StatusCode, body: &'static str) -> String {
    let app = Router::new().route(
        "/repos/{org}/{repo}",
        get(move || async move { (status, body) }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    format!("http://{addr}")
}

async fn asked(status: StatusCode, body: &'static str) -> Result<Permission, Error> {
    crate::tls::install_crypto_provider();

    let api = forge(status, body).await;
    let ns = Namespace::new("FerrLabs", "Blastlands").unwrap();

    permission(&reqwest::Client::new(), &api, "a-token", &ns).await
}

#[tokio::test]
async fn a_permissions_block_decides_the_level() {
    for (body, expected) in [
        (
            r#"{"permissions":{"admin":true,"push":true,"pull":true}}"#,
            Permission::Admin,
        ),
        (
            r#"{"permissions":{"admin":false,"push":true,"pull":true}}"#,
            Permission::Write,
        ),
        (
            r#"{"permissions":{"admin":false,"push":false,"pull":true}}"#,
            Permission::Read,
        ),
    ] {
        assert_eq!(asked(StatusCode::OK, body).await.unwrap(), expected);
    }
}

// A block that grants nothing is a refusal, and stays one: the caller was told
// about this repository and told it may do nothing with it.
#[tokio::test]
async fn a_block_granting_nothing_is_still_refused() {
    let refused = asked(
        StatusCode::OK,
        r#"{"permissions":{"admin":false,"push":false,"pull":false}}"#,
    )
    .await;

    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
}

// What `GITHUB_TOKEN` looks like from here: a GitHub App installation token gets
// the repository with no permissions block at all. Refusing it failed every CI
// job on a private repository with a message about credentials that were fine.
#[tokio::test]
async fn a_payload_with_no_permissions_block_is_read() {
    let granted = asked(StatusCode::OK, r#"{"private":true}"#).await;

    assert_eq!(
        granted.unwrap(),
        Permission::Read,
        "the forge answers 404 to a token that cannot see the repository, so a body at all is proof of read"
    );
}

// And only read. Nothing in that payload says the token may write, and a job
// that pushes objects it was never granted is the failure worth avoiding.
#[tokio::test]
async fn no_permissions_block_never_grants_write() {
    let granted = asked(StatusCode::OK, r#"{"private":true}"#).await.unwrap();

    assert!(granted.require_write().is_err(), "{granted:?}");
}

#[tokio::test]
async fn a_repository_the_token_cannot_see_is_refused() {
    let refused = asked(StatusCode::NOT_FOUND, r#"{"message":"Not Found"}"#).await;

    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
}
