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
    );

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

#[test]
#[should_panic(expected = "LFSX_FORGE_WORK_AUTH must be github, gitlab, gitea or forgejo")]
fn a_forge_without_a_provider_refuses_to_start() {
    parse("work", &primary(), reading(&[]));
}

#[test]
#[should_panic(expected = "LFSX_FORGE_WORK_API_URL must be set")]
fn a_self_hosted_forge_without_its_api_root_refuses_to_start() {
    parse(
        "work",
        &primary(),
        reading(&[("LFSX_FORGE_WORK_AUTH", "gitea")]),
    );
}

#[test]
#[should_panic(expected = "not api or dashboard")]
fn a_forge_cannot_take_a_name_the_dashboard_uses() {
    parse(
        "api",
        &primary(),
        reading(&[("LFSX_FORGE_API_AUTH", "github")]),
    );
}

#[test]
#[should_panic(expected = "names work twice")]
fn a_forge_named_twice_refuses_to_start() {
    parse(
        "work,work",
        &primary(),
        reading(&[("LFSX_FORGE_WORK_AUTH", "github")]),
    );
}

#[test]
#[should_panic(expected = "LFSX_AUTH=disabled")]
fn named_forges_need_authentication() {
    parse(
        "work",
        &Auth::Disabled,
        reading(&[("LFSX_FORGE_WORK_AUTH", "github")]),
    );
}
