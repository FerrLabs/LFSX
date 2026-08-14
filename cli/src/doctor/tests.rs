use super::*;

fn batch_with(href: &str) -> Value {
    json!({ "objects": [{ "actions": { "upload": { "href": href } } }] })
}

#[test]
fn the_origin_is_what_gets_compared_not_the_whole_url() {
    let body = batch_with("https://lfs.example.com/FerrLabs/Blastlands/objects/abc");

    assert_eq!(
        advertised_origin(&body).as_deref(),
        Some("https://lfs.example.com")
    );
}

#[test]
fn a_public_url_pointing_somewhere_else_is_visible_in_the_origin() {
    let body = batch_with("http://localhost:8080/FerrLabs/Blastlands/objects/abc");

    assert_ne!(
        advertised_origin(&body),
        origin_of("https://lfs.example.com"),
        "this mismatch is the whole reason doctor exists"
    );
}

#[test]
fn ports_and_schemes_are_part_of_the_origin() {
    assert_eq!(
        origin_of("http://host:8080/a/b").as_deref(),
        Some("http://host:8080")
    );
    assert_ne!(origin_of("https://host"), origin_of("http://host"));
    assert_ne!(origin_of("https://host"), origin_of("https://host:8443"));
}

#[test]
fn a_batch_response_without_an_upload_link_yields_nothing_to_compare() {
    assert_eq!(advertised_origin(&json!({ "objects": [] })), None);
    assert_eq!(origin_of("not-a-url"), None);
}
