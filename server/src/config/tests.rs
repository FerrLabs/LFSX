use super::*;

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
    assert_eq!(provider(Some("gitea")), Provider::Gitea);
    assert_eq!(
        provider(Some("forgejo")),
        Provider::Gitea,
        "Forgejo is a fork of Gitea and answers the same API, so naming either has to work"
    );
    assert_eq!(provider(Some("gitlab")), Provider::Gitlab);

    for github in [None, Some("github"), Some("Gitea"), Some("nonsense")] {
        assert_eq!(provider(github), Provider::Github, "{github:?}");
    }
}

// One left on the end produces `//repos/...`, which is a 404 for a repository
// that is right there, on the forges that do not normalise it.
#[test]
fn an_api_root_keeps_no_trailing_slash() {
    assert_eq!(
        api_url(Provider::Gitea, Some("https://git.example.com/api/v1/")),
        "https://git.example.com/api/v1"
    );
    assert_eq!(
        api_url(Provider::Github, None),
        "https://api.github.com",
        "an unset variable still falls back to the default for a forge that has one"
    );
}

// Guessing gitea.com for an operator running their own instance would resolve
// their namespaces against a stranger's forge, and a public repository there
// sharing a name would hand out anonymous read on objects it has nothing to do
// with. Not starting is the better failure.
#[test]
#[should_panic(expected = "LFSX_GITEA_API_URL must be set")]
fn a_self_hosted_forge_refuses_to_start_without_its_api_root() {
    api_url(Provider::Gitea, None);
}
