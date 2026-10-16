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

fn token() -> Option<String> {
    Some("secret".to_owned())
}

#[test]
fn a_token_is_refused_over_plain_http_to_a_remote_host() {
    for url in [
        "http://lfs.example.com",
        "http://192.168.1.10:8080",
        "ftp://localhost",
    ] {
        let error = Server::new(url, token(), None).err().unwrap();
        assert!(
            error.to_string().contains("refusing to send a token"),
            "{url}"
        );
    }
}

#[test]
fn a_token_is_refused_for_a_url_that_is_not_one() {
    assert!(Server::new("lfs.example.com", token(), None).is_err());
}

#[test]
fn a_token_is_accepted_over_https_and_over_loopback_http() {
    for url in [
        "https://lfs.example.com",
        "http://localhost:8080",
        "http://127.0.0.1:8080",
        "http://[::1]:8080",
    ] {
        assert!(Server::new(url, token(), None).is_ok(), "{url}");
    }
}

#[test]
fn plain_http_is_fine_without_a_token() {
    assert!(Server::new("http://lfs.example.com", None, None).is_ok());
    assert!(Server::new("http://lfs.example.com", Some(String::new()), None).is_ok());
}
