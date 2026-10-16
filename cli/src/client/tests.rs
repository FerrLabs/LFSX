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
}

#[test]
fn a_request_timeout_cuts_off_a_server_that_never_answers() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let server = Server::new(
        &format!("http://{}", listener.local_addr().unwrap()),
        None,
        Some(Duration::from_millis(200)),
    )
    .unwrap();

    assert!(server.post("/x", &()).is_err());
}
