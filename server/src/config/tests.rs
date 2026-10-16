use std::path::PathBuf;
use std::time::Duration;

use super::auth::{
    LOOKUP_BUDGET, allowed, anonymous_read, api_url, is_set, lookup_budget, provider,
};
use super::dashboard::{DASHBOARD_DIR, dashboard};
use super::storage::{azure_credential, encryption_key, gcs_credential};
use super::*;
use crate::auth::Namespaces;

// The default is the whole point of this function: an operator who sets nothing
// gets a server that asks for a credential. Everything else here guards against
// a value that looks like consent without being it.
#[test]
fn anonymous_read_is_closed_unless_it_is_asked_for() {
    assert!(!anonymous_read(None), "unset must not open it");

    for refused in ["false", "", "1", "yes", "True", "TRUE", " true", "true "] {
        assert!(
            !anonymous_read(Some(refused)),
            "{refused:?} is not the string that opens it"
        );
    }

    assert!(anonymous_read(Some("true")), "the one value that opens it");
}

#[test]
fn forgejo_and_gitea_are_the_same_forge() {
    assert_eq!(provider(Some("gitea")).unwrap(), Provider::Gitea);
    assert_eq!(
        provider(Some("forgejo")).unwrap(),
        Provider::Gitea,
        "Forgejo is a fork of Gitea and answers the same API, so naming either has to work"
    );
    assert_eq!(provider(Some("gitlab")).unwrap(), Provider::Gitlab);

    for github in [None, Some(""), Some("github")] {
        assert_eq!(provider(github).unwrap(), Provider::Github, "{github:?}");
    }
}

#[test]
fn an_unknown_auth_provider_is_refused_by_name() {
    for refused in ["GitLab", "none", "gitlb", "nonsense"] {
        let message = provider(Some(refused)).unwrap_err().to_string();
        assert!(message.contains("LFSX_AUTH"), "{message}");
        assert!(message.contains(refused), "{message}");
    }
}

#[test]
fn an_unparseable_size_is_refused_instead_of_becoming_unlimited() {
    for refused in ["5Gi", "5.36870912e+10", "-1", "lots"] {
        let message = bytes("LFSX_REPO_QUOTA", Some(refused))
            .unwrap_err()
            .to_string();
        assert!(message.contains("LFSX_REPO_QUOTA"), "{message}");
        assert!(message.contains(refused), "{message}");
    }

    assert_eq!(bytes("LFSX_REPO_QUOTA", None).unwrap(), None);
    assert_eq!(bytes("LFSX_REPO_QUOTA", Some("")).unwrap(), None);
    assert_eq!(bytes("LFSX_REPO_QUOTA", Some("0")).unwrap(), None);
    assert_eq!(
        bytes("LFSX_REPO_QUOTA", Some(" 1024 ")).unwrap(),
        Some(1024)
    );
}

#[test]
fn an_unparseable_duration_is_refused_instead_of_taking_the_default() {
    for refused in ["7d", "1.5", "-3", "soon"] {
        let message = seconds("LFSX_LOCK_MAX_AGE", Some(refused))
            .unwrap_err()
            .to_string();
        assert!(message.contains("LFSX_LOCK_MAX_AGE"), "{message}");
        assert!(message.contains(refused), "{message}");
    }

    assert_eq!(seconds("LFSX_LOCK_MAX_AGE", None).unwrap(), None);
    assert_eq!(
        seconds("LFSX_LOCK_MAX_AGE", Some("90")).unwrap(),
        Some(Duration::from_secs(90))
    );
}

#[test]
fn an_unparseable_bind_address_is_refused() {
    let message = error::parse::<SocketAddr>("LFSX_BIND", Some("localhost"), "an address")
        .unwrap_err()
        .to_string();
    assert!(message.contains("LFSX_BIND"), "{message}");
    assert!(message.contains("localhost"), "{message}");

    assert_eq!(
        error::parse::<SocketAddr>("LFSX_BIND", Some("127.0.0.1:9000"), "an address").unwrap(),
        Some(SocketAddr::from(([127, 0, 0, 1], 9000)))
    );
}

// One left on the end produces `//repos/...`, which is a 404 for a repository
// that is right there, on the forges that do not normalise it.
#[test]
fn an_api_root_keeps_no_trailing_slash() {
    assert_eq!(
        api_url(Provider::Gitea, Some("https://git.example.com/api/v1/")).unwrap(),
        "https://git.example.com/api/v1"
    );
    assert_eq!(
        api_url(Provider::Github, None).unwrap(),
        "https://api.github.com",
        "an unset variable still falls back to the default for a forge that has one"
    );
}

// Guessing gitea.com for an operator running their own instance would resolve
// their namespaces against a stranger's forge, and a public repository there
// sharing a name would hand out anonymous read on objects it has nothing to do
// with. Not starting is the better failure.
#[test]
fn a_self_hosted_forge_refuses_to_start_without_its_api_root() {
    let error = api_url(Provider::Gitea, None).unwrap_err();

    assert_eq!(
        error.to_string(),
        "LFSX_GITEA_API_URL=\"\" is not set: a self-hosted forge has no default API root to fall back on"
    );
}

// Zero is the one value that cannot mean what it says: a ceiling of no lookups
// is a server that refuses everyone it has not already seen, so it is read as
// the operator turning the ceiling off.
#[test]
fn a_lookup_budget_of_zero_turns_the_ceiling_off() {
    assert_eq!(lookup_budget(Some("0")), None);
    assert_eq!(lookup_budget(Some(" 0 ")), None);
}

#[test]
fn an_unset_or_unreadable_lookup_budget_keeps_the_default() {
    for kept in [None, Some("nonsense"), Some(""), Some("-1"), Some("6.5")] {
        assert_eq!(
            lookup_budget(kept),
            Some(LOOKUP_BUDGET),
            "{kept:?} must not take the ceiling away, and must not stop the server either"
        );
    }
}

#[test]
fn a_lookup_budget_is_taken_as_written() {
    assert_eq!(lookup_budget(Some("25")), Some(25));
}

#[test]
fn a_transfer_cap_is_taken_as_written_and_zero_turns_it_off() {
    assert_eq!(transfer_cap(Some("64")), 64);
    assert_eq!(transfer_cap(Some(" 0 ")), 0);
}

#[test]
fn an_unset_or_unreadable_transfer_cap_keeps_the_default() {
    for kept in [None, Some("nonsense"), Some(""), Some("-1"), Some("6.5")] {
        assert_eq!(
            transfer_cap(kept),
            TRANSFER_CAP,
            "{kept:?} must not take the backstop away, and must not stop the server either"
        );
    }
}

#[test]
fn the_idle_limit_is_taken_as_written_and_zero_keeps_the_default() {
    assert_eq!(
        transfer_idle_timeout(Some(Duration::from_secs(15))),
        Duration::from_secs(15)
    );
    assert_eq!(transfer_idle_timeout(None), TRANSFER_IDLE_TIMEOUT);
    assert_eq!(
        transfer_idle_timeout(Some(Duration::ZERO)),
        TRANSFER_IDLE_TIMEOUT,
        "zero would drop every transfer before its first byte"
    );
}

fn asked_from(host: &str, scheme: Option<&str>) -> String {
    let mut headers = HeaderMap::new();
    headers.insert(header::HOST, host.parse().unwrap());
    if let Some(scheme) = scheme {
        headers.insert("x-forwarded-proto", scheme.parse().unwrap());
    }

    Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        metrics_bind: None,
        dashboard: None,
        forges: Vec::new(),
        storage_root: PathBuf::from("."),
        public_url: None,
        action_lifetime: 1800,
        gc_grace: Duration::ZERO,
        staging_max_age: Duration::ZERO,
        lock_max_age: None,
        max_object_size: None,
        max_concurrent_transfers: 128,
        transfer_idle_timeout: std::time::Duration::from_secs(60),
        repo_quota: None,
        compression: None,
        encryption_key: None,
        storage: Storage::Local,
        auth: Auth::Disabled,
    }
    .base_url(&headers)
}

// What the hrefs are for: the client sends the object there, with its credential
// attached. So a `Host` that is not a host has to fall back to something useless
// rather than be pasted into a URL.
//
// `real.example@evil.example` is the one that matters. It resolves to the second
// name with the first read as a username, so a header somebody sent becomes a
// redirect nobody wrote.
#[test]
fn a_host_that_is_not_an_authority_never_reaches_a_url() {
    for forged in [
        "real.example@evil.example",
        "real.example/../evil.example",
        "evil.example/path",
        "real.example evil.example",
        "",
    ] {
        assert_eq!(
            asked_from(forged, None),
            "http://localhost",
            "{forged:?} must not end up in an href"
        );
    }
}

#[test]
fn an_ordinary_host_still_works() {
    assert_eq!(
        asked_from("lfs.example.com", None),
        "http://lfs.example.com"
    );
    assert_eq!(
        asked_from("lfs.example.com:8443", Some("https")),
        "https://lfs.example.com:8443"
    );
    assert_eq!(asked_from("[::1]:8080", None), "http://[::1]:8080");
}

// A scheme is one of two words. Anything else is not a proxy telling this server
// how it was reached, and `javascript` or `file` pasted in front of `://` is not
// a URL anybody should hand a client.
#[test]
fn only_the_two_schemes_a_client_can_use_are_believed() {
    for invented in ["javascript", "file", "HTTPS ", "gopher", ""] {
        assert_eq!(
            asked_from("lfs.example.com", Some(invented)),
            "http://lfs.example.com",
            "{invented:?}"
        );
    }
}

// And a configured public URL is never second-guessed, whatever the caller sent.
#[test]
fn a_configured_public_url_wins() {
    let mut headers = HeaderMap::new();
    headers.insert(header::HOST, "evil.example".parse().unwrap());

    let config = Config {
        public_url: Some("https://lfs.example.com".into()),
        bind: "127.0.0.1:0".parse().unwrap(),
        metrics_bind: None,
        dashboard: None,
        forges: Vec::new(),
        storage_root: PathBuf::from("."),
        action_lifetime: 1800,
        gc_grace: Duration::ZERO,
        staging_max_age: Duration::ZERO,
        lock_max_age: None,
        max_object_size: None,
        max_concurrent_transfers: 128,
        transfer_idle_timeout: std::time::Duration::from_secs(60),
        repo_quota: None,
        compression: None,
        encryption_key: None,
        storage: Storage::Local,
        auth: Auth::Disabled,
    };

    assert_eq!(config.base_url(&headers), "https://lfs.example.com");
}

#[test]
fn an_encryption_key_comes_from_one_source() {
    assert_eq!(encryption_key(None, None).unwrap(), None);
    assert_eq!(encryption_key(Some(""), Some("")).unwrap(), None);
    assert_eq!(
        encryption_key(Some("/etc/lfsx/keys/key"), None).unwrap(),
        Some(KeySource::File("/etc/lfsx/keys/key".into()))
    );
    assert_eq!(
        encryption_key(None, Some("vault kv get -field=key lfsx")).unwrap(),
        Some(KeySource::Command("vault kv get -field=key lfsx".into()))
    );
}

#[test]
fn two_key_sources_refuse_to_start() {
    let error = encryption_key(Some("/etc/keys"), Some("vault kv get")).unwrap_err();

    assert_eq!(
        error.to_string(),
        "LFSX_ENCRYPTION_KEY_FILE=\"/etc/keys\" is not allowed alongside LFSX_ENCRYPTION_KEY_COMMAND: pick one source for the keys"
    );
}

#[test]
fn an_azure_credential_comes_from_one_source_or_from_the_identity() {
    assert_eq!(azure_credential(None, None), AzureCredential::Identity);
    assert_eq!(
        azure_credential(Some(""), Some("")),
        AzureCredential::Identity
    );
    assert_eq!(
        azure_credential(Some("a2V5"), None),
        AzureCredential::AccountKey("a2V5".into())
    );
    assert_eq!(
        azure_credential(None, Some("sv=2022-11-02&sig=x")),
        AzureCredential::Sas("sv=2022-11-02&sig=x".into())
    );
}

#[test]
#[should_panic(expected = "both set")]
fn an_account_key_and_a_sas_token_refuse_to_start() {
    azure_credential(Some("a2V5"), Some("sv=2022-11-02&sig=x"));
}

#[test]
fn gcs_asks_the_metadata_server_unless_given_a_key_file_or_told_none() {
    assert_eq!(gcs_credential(None), GcsCredential::Metadata);
    assert_eq!(gcs_credential(Some("")), GcsCredential::Metadata);
    assert_eq!(gcs_credential(Some("none")), GcsCredential::Anonymous);
    assert_eq!(
        gcs_credential(Some("/var/run/secrets/gcs/key.json")),
        GcsCredential::ServiceAccount("/var/run/secrets/gcs/key.json".into())
    );
}

#[test]
fn an_allow_list_is_only_in_force_when_something_is_listed() {
    assert_eq!(allowed("LFSX_ALLOWED", None), None);
    assert_eq!(allowed("LFSX_ALLOWED", Some("  ")), None);
    assert!(
        allowed("LFSX_ALLOWED", Some("not-a-namespace")).is_some_and(|allowed| allowed.is_empty()),
        "a list that parsed to nothing serves nothing, rather than falling open to every repository"
    );
}

fn github() -> Auth {
    Auth::Forge {
        provider: Provider::Github,
        api_url: "https://api.github.com".into(),
        cache_ttl: Duration::from_secs(60),
        rejection_ttl: Duration::from_secs(10),
        lookup_budget: None,
        anonymous_read: false,
        restricted: Namespaces::parse("LFSX_RESTRICTED", None),
        allowed: None,
        github_app: None,
    }
}

#[test]
fn the_dashboard_is_off_unless_it_is_asked_for() {
    assert!(
        dashboard(None, None, Some("FerrLabs/Infra"), None, &github(), &[])
            .unwrap()
            .is_none()
    );
    assert!(
        dashboard(
            Some("yes"),
            None,
            Some("FerrLabs/Infra"),
            None,
            &github(),
            &[]
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn the_dashboard_is_shown_to_the_admins_of_the_named_repository() {
    let dashboard = dashboard(
        Some("true"),
        Some(""),
        Some("FerrLabs/Infra"),
        None,
        &github(),
        &[],
    )
    .unwrap()
    .unwrap();

    let admins = dashboard.admins.unwrap();
    assert_eq!((admins.org(), admins.repo()), ("FerrLabs", "Infra"));
    assert_eq!(dashboard.dir, PathBuf::from(DASHBOARD_DIR));
}

#[test]
fn a_dashboard_behind_a_forge_refuses_to_start_without_a_repository() {
    let error = dashboard(Some("true"), None, None, None, &github(), &[]).unwrap_err();

    assert_eq!(
        error.to_string(),
        "LFSX_DASHBOARD_REPO=\"\" is not set: the dashboard is shown to the admins of that repository, and nobody else"
    );
}

#[test]
fn a_dashboard_repository_that_is_not_org_repo_refuses_to_start() {
    let error = dashboard(Some("true"), None, Some("Infra"), None, &github(), &[]).unwrap_err();

    assert_eq!(
        error.to_string(),
        "LFSX_DASHBOARD_REPO=\"Infra\" is not org/repo"
    );
}

#[test]
fn without_authentication_the_dashboard_needs_no_repository() {
    let dashboard = dashboard(
        Some("true"),
        Some("/srv/dashboard"),
        None,
        None,
        &Auth::Disabled,
        &[],
    )
    .unwrap()
    .unwrap();

    assert!(dashboard.admins.is_none());
    assert_eq!(dashboard.dir, PathBuf::from("/srv/dashboard"));
}

#[test]
fn a_variable_counts_as_set_whatever_its_entries_parse_to() {
    assert!(is_set(Some("not a namespace")));
    assert!(is_set(Some("acme/*")));
    assert!(!is_set(Some(" ")));
    assert!(!is_set(None));
}

fn named(name: &str) -> Forge {
    Forge {
        name: name.to_owned(),
        auth: github(),
    }
}

#[test]
fn the_dashboard_can_take_its_admins_from_a_named_forge() {
    let dashboard = dashboard(
        Some("true"),
        None,
        Some("FerrLabs/Infra"),
        Some("work"),
        &github(),
        &[named("work")],
    )
    .unwrap()
    .unwrap();

    let admins = dashboard.admins.unwrap();
    assert_eq!(admins.forge(), Some("work"));
    assert_eq!((admins.org(), admins.repo()), ("FerrLabs", "Infra"));
}

#[test]
fn a_dashboard_forge_nobody_configured_refuses_to_start() {
    let error = dashboard(
        Some("true"),
        None,
        Some("FerrLabs/Infra"),
        Some("elsewhere"),
        &github(),
        &[named("work")],
    )
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        "LFSX_DASHBOARD_FORGE=\"elsewhere\" is not a forge listed in LFSX_FORGES"
    );
}
