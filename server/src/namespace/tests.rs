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
    assert!(Namespace::new("FerrLabs", &"a".repeat(101)).is_err());
    assert!(Namespace::new("FerrLabs", &"a".repeat(100)).is_ok());
}
