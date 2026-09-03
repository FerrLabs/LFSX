use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::{get, post};

use super::App;
use crate::auth::Permission;
use crate::auth::github;
use crate::error::Error;
use crate::namespace::Namespace;

fn key_pem() -> &'static str {
    static PEM: OnceLock<String> = OnceLock::new();
    PEM.get_or_init(|| {
        use rsa::pkcs1::EncodeRsaPrivateKey;
        let key =
            rsa::RsaPrivateKey::new(&mut rand_core::OsRng, 2048).expect("a throwaway test key");
        key.to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
            .expect("the test key renders as PEM")
            .to_string()
    })
}

fn app_under_test(root: &tempfile::TempDir) -> App {
    let key_file = root.path().join("app.pem");
    std::fs::write(&key_file, key_pem()).unwrap();
    App::load("41", &key_file)
}

#[derive(Clone)]
struct Forge {
    installed: bool,
    private: bool,
    minted: Arc<AtomicUsize>,
    repo_auth: Arc<std::sync::Mutex<Vec<Option<String>>>>,
}

async fn forge(installed: bool, private: bool) -> (String, Forge) {
    let state = Forge {
        installed,
        private,
        minted: Arc::new(AtomicUsize::new(0)),
        repo_auth: Arc::new(std::sync::Mutex::new(Vec::new())),
    };

    let router = Router::new()
        .route(
            "/repos/{org}/{repo}/installation",
            get(
                |State(forge): State<Forge>, headers: HeaderMap| async move {
                    assert!(
                        headers
                            .get("authorization")
                            .is_some_and(|auth| auth.to_str().unwrap().starts_with("Bearer ")),
                        "the installation lookup has to identify as the App"
                    );
                    if forge.installed {
                        Ok(axum::Json(serde_json::json!({ "id": 7 })))
                    } else {
                        Err(axum::http::StatusCode::NOT_FOUND)
                    }
                },
            ),
        )
        .route(
            "/app/installations/7/access_tokens",
            post(|State(forge): State<Forge>| async move {
                forge.minted.fetch_add(1, Ordering::SeqCst);
                axum::Json(serde_json::json!({
                    "token": "minted-installation-token",
                    "expires_at": "2035-01-01T00:00:00Z"
                }))
            }),
        )
        .route(
            "/repos/{org}/{repo}",
            get(
                |State(forge): State<Forge>, headers: HeaderMap| async move {
                    forge.repo_auth.lock().unwrap().push(
                        headers
                            .get("authorization")
                            .map(|auth| auth.to_str().unwrap().to_owned()),
                    );
                    axum::Json(serde_json::json!({ "private": forge.private }))
                },
            ),
        )
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    (url, state)
}

fn client() -> reqwest::Client {
    crate::tls::install_crypto_provider();
    reqwest::Client::new()
}

fn namespace() -> Namespace {
    Namespace::new("FerrLabs", "Blastlands").unwrap()
}

// The security case the App must not soften: an installation token is admitted
// to every private repository the App covers, so a 200 as the App proves
// nothing about the public, and only the repository saying it is public may
// grant the anonymous caller read.
#[tokio::test]
async fn a_private_repository_stays_refused_even_where_the_app_is_installed() {
    let root = tempfile::tempdir().unwrap();
    let (url, forge) = forge(true, true).await;
    let app = app_under_test(&root);

    let answer = github::public(&client(), &url, Some(&app), &namespace()).await;

    assert!(matches!(answer, Err(Error::Unauthenticated)));
    assert_eq!(
        forge.repo_auth.lock().unwrap().as_slice(),
        [Some("Bearer minted-installation-token".to_owned())],
        "the question was asked as the App, and the visibility field refused it"
    );
}

#[tokio::test]
async fn a_public_repository_is_granted_read_as_the_app() {
    let root = tempfile::tempdir().unwrap();
    let (url, _forge) = forge(true, false).await;
    let app = app_under_test(&root);

    let answer = github::public(&client(), &url, Some(&app), &namespace()).await;

    assert!(matches!(answer, Ok(Permission::Read)));
}

// The App not being installed on a repository is not a refusal: the question
// falls back to the plain anonymous ask, credentials absent, exactly as if no
// App were configured.
#[tokio::test]
async fn no_installation_falls_back_to_the_anonymous_question() {
    let root = tempfile::tempdir().unwrap();
    let (url, forge) = forge(false, false).await;
    let app = app_under_test(&root);

    let answer = github::public(&client(), &url, Some(&app), &namespace()).await;

    assert!(matches!(answer, Ok(Permission::Read)));
    assert_eq!(
        forge.repo_auth.lock().unwrap().as_slice(),
        [None],
        "with no installation the lookup must carry no credentials at all"
    );
}

#[tokio::test]
async fn the_installation_token_is_minted_once_and_cached() {
    let root = tempfile::tempdir().unwrap();
    let (url, forge) = forge(true, false).await;
    let app = app_under_test(&root);

    for _ in 0..3 {
        github::public(&client(), &url, Some(&app), &namespace())
            .await
            .unwrap();
    }

    assert_eq!(
        forge.minted.load(Ordering::SeqCst),
        1,
        "a busy server exchanges once until expiry, not once a request"
    );
}
