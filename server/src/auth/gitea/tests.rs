use std::sync::{Arc, Mutex};

use axum::Router;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;

use super::*;

type Seen = Arc<Mutex<Option<String>>>;

// A stand-in for the forge that answers one canned response and records the
// credential it was presented with, so each case below is the payload shape or
// the status under test and nothing else.
async fn forge(
    status: StatusCode,
    headers: Vec<(&'static str, &'static str)>,
    body: &'static str,
) -> (String, Seen) {
    let seen: Seen = Arc::new(Mutex::new(None));
    let recorded = seen.clone();

    let answer = move |sent: HeaderMap| {
        let recorded = recorded.clone();
        let headers = headers.clone();

        async move {
            *recorded.lock().unwrap() = sent
                .get(AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);

            let mut response = (status, body).into_response();
            for (name, value) in headers {
                response.headers_mut().insert(name, value.parse().unwrap());
            }

            response
        }
    };

    let app = Router::new()
        .route("/repos/{org}/{repo}", get(answer.clone()))
        .route("/user", get(answer));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    (format!("http://{address}"), seen)
}

fn namespace() -> Namespace {
    Namespace::new("FerrLabs", "Blastlands").unwrap()
}

async fn asked(status: StatusCode, body: &'static str) -> Result<Permission, Error> {
    crate::tls::install_crypto_provider();

    let (api, _) = forge(status, Vec::new(), body).await;

    permission(&reqwest::Client::new(), &api, "a-token", &namespace()).await
}

async fn asked_with(
    status: StatusCode,
    headers: Vec<(&'static str, &'static str)>,
) -> Result<Permission, Error> {
    crate::tls::install_crypto_provider();

    let (api, _) = forge(status, headers, "{}").await;

    permission(&reqwest::Client::new(), &api, "a-token", &namespace()).await
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

// The interop detail most likely to be wrong, so it is pinned rather than
// assumed. Gitea documents `token` for the access tokens a user creates, and an
// older instance accepts nothing else.
#[tokio::test]
async fn the_token_is_presented_in_the_scheme_gitea_documents() {
    crate::tls::install_crypto_provider();

    let (api, seen) = forge(StatusCode::OK, Vec::new(), r#"{"permissions":{}}"#).await;

    permission(
        &reqwest::Client::new(),
        &api,
        "65eaa9c8ef52460d22a93307",
        &namespace(),
    )
    .await
    .unwrap();

    assert_eq!(
        seen.lock().unwrap().as_deref(),
        Some("token 65eaa9c8ef52460d22a93307")
    );
}

// A repository the token cannot see is a 404 rather than a refusal, so a body at
// all has already established read. Refusing on a block that grants nothing is
// how this went wrong for GitHub twice, and the answer there is the answer here.
#[tokio::test]
async fn a_body_at_all_grants_read() {
    for body in [
        r#"{"permissions":{"admin":false,"push":false,"pull":false}}"#,
        r#"{"private":true}"#,
    ] {
        assert_eq!(
            asked(StatusCode::OK, body).await.unwrap(),
            Permission::Read,
            "{body}"
        );
    }
}

// And read only. Nothing in either payload says the token may write, and a job
// uploading objects it was never granted is the failure worth keeping impossible.
#[tokio::test]
async fn a_body_that_grants_nothing_never_grants_write() {
    for body in [
        r#"{"permissions":{"admin":false,"push":false,"pull":false}}"#,
        r#"{"private":true}"#,
    ] {
        let granted = asked(StatusCode::OK, body).await.unwrap();

        assert!(granted.require_write().is_err(), "{body}: {granted:?}");
    }
}

#[tokio::test]
async fn a_repository_the_token_cannot_see_is_refused() {
    let refused = asked(StatusCode::NOT_FOUND, r#"{"message":"Not Found"}"#).await;

    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
}

// The mapping the issue called the part to get right. A throttled forge is
// working and has said when to come back, and answering `Forbidden` would tell a
// user with full rights that they have none.
#[tokio::test]
async fn a_throttled_forge_is_a_limit_and_not_a_refusal() {
    let limited = asked_with(StatusCode::TOO_MANY_REQUESTS, vec![("retry-after", "45")]).await;

    assert!(
        matches!(limited, Err(Error::RateLimited { retry_after: 45 })),
        "{limited:?}"
    );
}

// Neither Gitea nor Forgejo rate-limits its own API, so this arrives from the
// proxy in front of it, and nginx answers 503 unless it was told otherwise. The
// status is the proxy's choice; the `Retry-After` is what makes it a limit.
#[tokio::test]
async fn a_proxy_answering_503_with_retry_after_is_a_limit_too() {
    let limited = asked_with(StatusCode::SERVICE_UNAVAILABLE, vec![("retry-after", "30")]).await;

    assert!(
        matches!(limited, Err(Error::RateLimited { retry_after: 30 })),
        "{limited:?}"
    );
}

// And a 503 with nothing to say is an instance that is down rather than busy. It
// stays a forge failure, because telling a client to wait a minute for an outage
// that will last an hour is not better information.
#[tokio::test]
async fn a_503_that_says_nothing_stays_a_forge_failure() {
    let failed = asked_with(StatusCode::SERVICE_UNAVAILABLE, Vec::new()).await;

    assert!(matches!(failed, Err(Error::Forge)), "{failed:?}");
}

#[tokio::test]
async fn an_anonymous_lookup_reads_a_public_repository_and_asks_for_credentials_otherwise() {
    crate::tls::install_crypto_provider();

    let (api, _) = forge(StatusCode::OK, Vec::new(), r#"{"private":false}"#).await;
    assert_eq!(
        public(&reqwest::Client::new(), &api, &namespace())
            .await
            .unwrap(),
        Permission::Read
    );

    let (api, seen) = forge(StatusCode::NOT_FOUND, Vec::new(), "{}").await;
    let refused = public(&reqwest::Client::new(), &api, &namespace()).await;

    assert!(
        matches!(refused, Err(Error::Unauthenticated)),
        "a private repository has to leave git-lfs asking the credential helper: {refused:?}"
    );
    assert_eq!(
        *seen.lock().unwrap(),
        None,
        "the anonymous question has to be asked anonymously"
    );
}

#[tokio::test]
async fn a_login_is_read_from_the_field_gitea_sends() {
    crate::tls::install_crypto_provider();

    let (api, _) = forge(
        StatusCode::OK,
        Vec::new(),
        r#"{"id":1,"login":"bryan","full_name":"","email":""}"#,
    )
    .await;

    assert_eq!(
        login(&reqwest::Client::new(), &api, "a-token")
            .await
            .unwrap(),
        "bryan"
    );
}
