use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::Path;
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use super::*;

#[derive(Default)]
struct Seen {
    credential: Option<String>,
    project: Option<String>,
}

type Canned = (StatusCode, Vec<(&'static str, &'static str)>, &'static str);

fn respond((status, headers, body): &Canned) -> Response {
    let mut response = (*status, *body).into_response();
    for (name, value) in headers {
        response.headers_mut().insert(*name, value.parse().unwrap());
    }
    response
}

async fn forge(
    status: StatusCode,
    headers: Vec<(&'static str, &'static str)>,
    body: &'static str,
) -> (String, Arc<Mutex<Seen>>) {
    crate::tls::install_crypto_provider();
    let canned: Arc<Canned> = Arc::new((status, headers, body));
    let seen = Arc::new(Mutex::new(Seen::default()));

    let project = {
        let canned = canned.clone();
        let seen = seen.clone();
        move |Path(path): Path<String>, sent: HeaderMap| {
            let mut recorded = seen.lock().unwrap();
            recorded.project = Some(path);
            recorded.credential = sent
                .get(AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let response = respond(&canned);
            async move { response }
        }
    };
    let user = move || {
        let response = respond(&canned);
        async move { response }
    };

    let app = Router::new()
        .route("/projects/{path}", get(project))
        .route("/user", get(user));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    (format!("http://{address}"), seen)
}

fn namespace() -> Namespace {
    Namespace::new("FerrLabs", "Blastlands").unwrap()
}

async fn anonymously(
    status: StatusCode,
    headers: Vec<(&'static str, &'static str)>,
) -> Result<Permission, Error> {
    let (api, _) = forge(status, headers, "{}").await;

    public(&reqwest::Client::new(), &api, &namespace()).await
}

async fn with_token(status: StatusCode, body: &'static str) -> Result<Permission, Error> {
    let (api, _) = forge(status, Vec::new(), body).await;

    permission(&reqwest::Client::new(), &api, "a-token", &namespace()).await
}

#[test]
fn a_project_path_is_encoded_as_one_segment() {
    assert_eq!(urlencoding("FerrLabs"), "FerrLabs");
    assert_eq!(urlencoding("Idler-Survivor"), "Idler-Survivor");
    assert_eq!(
        urlencoding("groupe avec espace"),
        "groupe%20avec%20espace",
        "GitLab addresses a project by its encoded path, so anything else must be escaped"
    );
}

#[tokio::test]
async fn an_anonymous_lookup_reads_a_public_project_by_its_path_and_presents_nothing() {
    let (api, seen) = forge(StatusCode::OK, Vec::new(), r#"{"visibility":"public"}"#).await;

    let granted = public(&reqwest::Client::new(), &api, &namespace()).await;

    assert!(matches!(granted, Ok(Permission::Read)), "{granted:?}");
    let seen = seen.lock().unwrap();
    assert_eq!(seen.project.as_deref(), Some("FerrLabs/Blastlands"));
    assert_eq!(
        seen.credential, None,
        "the anonymous question has to be asked anonymously"
    );
}

#[tokio::test]
async fn a_project_gitlab_will_not_admit_anonymously_leaves_git_lfs_asking_for_credentials() {
    for status in [
        StatusCode::NOT_FOUND,
        StatusCode::UNAUTHORIZED,
        StatusCode::INTERNAL_SERVER_ERROR,
    ] {
        let refused = anonymously(status, Vec::new()).await;

        assert!(
            matches!(refused, Err(Error::Unauthenticated)),
            "a 403 would make git-lfs stop asking the credential helper ({status}): {refused:?}"
        );
    }
}

#[tokio::test]
async fn a_throttled_anonymous_lookup_is_a_limit_with_the_wait_gitlab_names() {
    let limited = anonymously(StatusCode::TOO_MANY_REQUESTS, vec![("retry-after", "42")]).await;

    assert!(
        matches!(limited, Err(Error::RateLimited { retry_after: 42 })),
        "{limited:?}"
    );
}

#[tokio::test]
async fn a_token_lookup_tells_a_refusal_from_a_broken_forge() {
    assert!(matches!(
        with_token(StatusCode::UNAUTHORIZED, "{}").await,
        Err(Error::Unauthenticated)
    ));
    for status in [StatusCode::FORBIDDEN, StatusCode::NOT_FOUND] {
        assert!(
            matches!(with_token(status, "{}").await, Err(Error::Forbidden)),
            "{status}"
        );
    }
    let (api, _) = forge(
        StatusCode::TOO_MANY_REQUESTS,
        vec![("retry-after", "7")],
        "{}",
    )
    .await;
    assert!(matches!(
        permission(&reqwest::Client::new(), &api, "a-token", &namespace()).await,
        Err(Error::RateLimited { retry_after: 7 })
    ));
    assert!(matches!(
        with_token(StatusCode::BAD_GATEWAY, "{}").await,
        Err(Error::Forge)
    ));
    assert!(matches!(
        with_token(StatusCode::OK, "not json").await,
        Err(Error::Forge)
    ));
}

#[tokio::test]
async fn a_project_that_declares_no_permissions_grants_nothing() {
    let refused = with_token(StatusCode::OK, r#"{"id":1}"#).await;

    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
}

#[tokio::test]
async fn a_user_answer_without_a_username_is_a_forge_failure() {
    let (api, _) = forge(StatusCode::OK, Vec::new(), r#"{"login":"bryan"}"#).await;

    let identity = login(&reqwest::Client::new(), &api, "a-token").await;

    assert!(matches!(identity, Err(Error::Forge)), "{identity:?}");
}
