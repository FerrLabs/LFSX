use super::*;

#[test]
fn the_health_routes_are_found_above_a_named_forge() {
    assert_eq!(
        service_root("https://lfs.example.com/-/work"),
        "https://lfs.example.com"
    );
    assert_eq!(
        service_root("https://example.com/lfs/-/work"),
        "https://example.com/lfs"
    );
}

#[test]
fn a_base_without_a_forge_is_its_own_root() {
    assert_eq!(
        service_root("https://lfs.example.com"),
        "https://lfs.example.com"
    );
    assert_eq!(
        service_root("https://example.com/lfs"),
        "https://example.com/lfs"
    );
    assert_eq!(
        service_root("https://lfs.example.com/-/"),
        "https://lfs.example.com/-/"
    );
}
