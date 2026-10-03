use super::*;

#[test]
fn ordinary_names_are_accepted() {
    let ns = Namespace::new("FerrLabs", "LFSX").unwrap();

    assert_eq!(ns.org(), "FerrLabs");
    assert_eq!(ns.repo(), "LFSX");
}

#[test]
fn traversal_segments_are_rejected() {
    for (org, repo) in [
        ("..", "LFSX"),
        ("FerrLabs", ".."),
        ("FerrLabs", "."),
        ("FerrLabs", "sub/dir"),
        ("FerrLabs", "..%2Fetc"),
        ("", "LFSX"),
        ("FerrLabs", ""),
    ] {
        assert!(
            Namespace::new(org, repo).is_err(),
            "{org}/{repo} escaped validation"
        );
    }
}

#[test]
fn absurdly_long_names_are_rejected() {
    assert!(Namespace::new("FerrLabs", "a".repeat(101)).is_err());
    assert!(Namespace::new("FerrLabs", "a".repeat(100)).is_ok());
}

#[test]
fn the_default_forge_keeps_the_layout_it_always_had() {
    let ns = Namespace::new("FerrLabs", "LFSX").unwrap();

    assert_eq!(ns.forge(), None);
    assert_eq!(ns.stored_org(), "FerrLabs");
    assert_eq!(ns.to_string(), "FerrLabs/LFSX");
    assert_eq!(ns.upstream(), "FerrLabs/LFSX");
    assert_eq!(ns.url_path(), "FerrLabs/LFSX");
}

#[test]
fn a_named_forge_is_stored_apart_and_asked_about_as_itself() {
    let ns = Namespace::on("work", "FerrLabs", "LFSX").unwrap();

    assert_eq!(ns.forge(), Some("work"));
    assert_eq!(ns.org(), "FerrLabs");
    assert_eq!(ns.stored_org(), "work~FerrLabs");
    assert_eq!(ns.to_string(), "work~FerrLabs/LFSX");
    assert_eq!(ns.upstream(), "FerrLabs/LFSX");
    assert_eq!(ns.url_path(), "-/work/FerrLabs/LFSX");
    assert_ne!(ns, Namespace::new("FerrLabs", "LFSX").unwrap());
}

#[test]
fn no_org_can_be_mistaken_for_one_on_a_named_forge() {
    assert!(Namespace::new("work~FerrLabs", "LFSX").is_err());
}

#[test]
fn forge_names_cannot_shadow_the_dashboard_or_escape_a_path() {
    for forge in [
        "",
        "api",
        "dashboard",
        "Work",
        "-work",
        "a/b",
        "..",
        "a~b",
        &"a".repeat(33),
    ] {
        assert!(
            Namespace::on(forge, "FerrLabs", "LFSX").is_err(),
            "{forge} was accepted"
        );
    }
    for forge in ["work", "gitea-2", "1"] {
        assert!(Namespace::on(forge, "FerrLabs", "LFSX").is_ok(), "{forge}");
    }
}
