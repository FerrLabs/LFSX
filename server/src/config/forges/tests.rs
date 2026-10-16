use std::collections::HashMap;
use std::time::Duration;

use super::*;

fn primary() -> Auth {
    Auth::Forge {
        provider: Provider::Github,
        api_url: "https://api.github.com".into(),
        cache_ttl: Duration::from_secs(60),
        rejection_ttl: Duration::from_secs(10),
        lookup_budget: Some(600),
        github_app: None,
        anonymous_read: true,
        restricted: Namespaces::parse("LFSX_RESTRICTED", Some("acme/art")),
        allowed: Some(Namespaces::parse("LFSX_ALLOWED", Some("acme/*"))),
    }
}

fn reading(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let values: HashMap<String, String> = pairs
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect();
    move |name| values.get(name).cloned()
}

#[test]
fn each_forge_reads_its_own_variables_and_shares_the_cache_settings() {
    let forges = parse(
        "work, self-hosted",
        &primary(),
        reading(&[
            ("LFSX_FORGE_WORK_AUTH", "gitlab"),
            (
                "LFSX_FORGE_WORK_API_URL",
                "https://gitlab.work.example/api/v4/",
            ),
            ("LFSX_FORGE_WORK_ALLOWED", "platform/*"),
            ("LFSX_FORGE_SELF_HOSTED_AUTH", "forgejo"),
            (
                "LFSX_FORGE_SELF_HOSTED_API_URL",
                "https://git.example.com/api/v1",
            ),
            ("LFSX_FORGE_SELF_HOSTED_ANONYMOUS_READ", "true"),
        ]),
    )
    .unwrap();

    assert_eq!(forges.len(), 2);
    let Auth::Forge {
        provider,
        api_url,
        cache_ttl,
        lookup_budget,
        anonymous_read,
        restricted,
        allowed,
        ..
    } = &forges[0].auth
    else {
        panic!("a named forge is a forge");
    };
    assert_eq!(forges[0].name, "work");
    assert_eq!(*provider, Provider::Gitlab);
    assert_eq!(api_url, "https://gitlab.work.example/api/v4");
    assert_eq!(*cache_ttl, Duration::from_secs(60));
    assert_eq!(*lookup_budget, Some(600));
    assert!(!anonymous_read);
    assert!(restricted.is_empty());
    assert_eq!(
        allowed.as_ref().map(Namespaces::entries),
        Some(vec!["platform/*".into()])
    );

    let Auth::Forge {
        provider,
        anonymous_read,
        allowed,
        ..
    } = &forges[1].auth
    else {
        panic!("a named forge is a forge");
    };
    assert_eq!(forges[1].name, "self-hosted");
    assert_eq!(*provider, Provider::Gitea);
    assert!(anonymous_read);
    assert!(allowed.is_none());
}

fn refused(names: &str, primary: &Auth, pairs: &[(&str, &str)]) -> String {
    parse(names, primary, reading(pairs))
        .unwrap_err()
        .to_string()
}

#[test]
fn a_forge_without_a_provider_refuses_to_start() {
    assert_eq!(
        refused("work", &primary(), &[]),
        "LFSX_FORGE_WORK_AUTH=\"\" is not one of github, gitlab, gitea or forgejo"
    );
}

#[test]
fn a_forge_with_an_unknown_provider_refuses_to_start() {
    assert_eq!(
        refused("work", &primary(), &[("LFSX_FORGE_WORK_AUTH", "bitbucket")]),
        "LFSX_FORGE_WORK_AUTH=\"bitbucket\" is not one of github, gitlab, gitea or forgejo"
    );
}

#[test]
fn a_self_hosted_forge_without_its_api_root_refuses_to_start() {
    assert_eq!(
        refused("work", &primary(), &[("LFSX_FORGE_WORK_AUTH", "gitea")]),
        "LFSX_FORGE_WORK_API_URL=\"\" is not set: a self-hosted forge has no default API root to fall back on"
    );
}

#[test]
fn a_forge_cannot_take_a_name_the_dashboard_uses() {
    assert_eq!(
        refused("api", &primary(), &[("LFSX_FORGE_API_AUTH", "github")]),
        "LFSX_FORGES=\"api\" is not a forge name: 1 to 32 lowercase letters, digits or dashes, and not api or dashboard"
    );
}

#[test]
fn a_forge_name_with_capitals_refuses_to_start() {
    assert_eq!(
        refused("Work", &primary(), &[("LFSX_FORGE_WORK_AUTH", "github")]),
        "LFSX_FORGES=\"Work\" is not a forge name: 1 to 32 lowercase letters, digits or dashes, and not api or dashboard"
    );
}

#[test]
fn a_forge_named_twice_refuses_to_start() {
    assert_eq!(
        refused(
            "work,work",
            &primary(),
            &[("LFSX_FORGE_WORK_AUTH", "github")]
        ),
        "LFSX_FORGES=\"work\" is not named only once"
    );
}

#[test]
fn named_forges_need_authentication() {
    assert_eq!(
        refused(
            "work",
            &Auth::Disabled,
            &[("LFSX_FORGE_WORK_AUTH", "github")]
        ),
        "LFSX_FORGES=\"work\" is not usable with LFSX_AUTH=disabled: with authentication off there is nothing to ask the other forges about"
    );
}

#[test]
fn a_named_github_forge_can_have_its_own_app() {
    let forges = parse(
        "enterprise",
        &primary(),
        reading(&[
            ("LFSX_FORGE_ENTERPRISE_AUTH", "github"),
            (
                "LFSX_FORGE_ENTERPRISE_API_URL",
                "https://github.example.com/api/v3",
            ),
            ("LFSX_FORGE_ENTERPRISE_GITHUB_APP_ID", "1234"),
            (
                "LFSX_FORGE_ENTERPRISE_GITHUB_APP_KEY_FILE",
                "/secrets/enterprise.pem",
            ),
        ]),
    )
    .unwrap();

    let Auth::Forge { github_app, .. } = &forges[0].auth else {
        panic!("a named forge is a forge");
    };
    assert_eq!(
        github_app
            .as_ref()
            .map(|app| (app.app_id.as_str(), app.key_file.clone())),
        Some(("1234", std::path::PathBuf::from("/secrets/enterprise.pem")))
    );
}

#[test]
fn an_app_on_a_forge_that_is_not_github_does_nothing() {
    let forges = parse(
        "work",
        &primary(),
        reading(&[
            ("LFSX_FORGE_WORK_AUTH", "gitlab"),
            ("LFSX_FORGE_WORK_GITHUB_APP_ID", "1234"),
            ("LFSX_FORGE_WORK_GITHUB_APP_KEY_FILE", "/secrets/work.pem"),
        ]),
    )
    .unwrap();

    let Auth::Forge { github_app, .. } = &forges[0].auth else {
        panic!("a named forge is a forge");
    };
    assert!(github_app.is_none());
}

#[test]
fn half_an_app_on_a_named_forge_refuses_to_start() {
    assert_eq!(
        refused(
            "enterprise",
            &primary(),
            &[
                ("LFSX_FORGE_ENTERPRISE_AUTH", "github"),
                ("LFSX_FORGE_ENTERPRISE_GITHUB_APP_ID", "1234"),
            ]
        ),
        "LFSX_FORGE_ENTERPRISE_GITHUB_APP_ID=\"1234\" is not set without its key file: an App needs both an id and a key file"
    );
    assert_eq!(
        refused(
            "enterprise",
            &primary(),
            &[
                ("LFSX_FORGE_ENTERPRISE_AUTH", "github"),
                (
                    "LFSX_FORGE_ENTERPRISE_GITHUB_APP_KEY_FILE",
                    "/secrets/a.pem"
                ),
            ]
        ),
        "LFSX_FORGE_ENTERPRISE_GITHUB_APP_KEY_FILE=\"/secrets/a.pem\" is not set without its app id: an App needs both an id and a key file"
    );
}
