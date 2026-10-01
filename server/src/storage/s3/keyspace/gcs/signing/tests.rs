use std::time::Duration;

use super::*;

const EMAIL: &str = "test-iam-credentials@dummy-project-id.iam.gserviceaccount.com";

fn at(timestamp: &str) -> time::OffsetDateTime {
    time::OffsetDateTime::parse(timestamp, &time::format_description::well_known::Rfc3339).unwrap()
}

fn google() -> reqwest::Url {
    reqwest::Url::parse("https://storage.googleapis.com").unwrap()
}

#[test]
fn a_simple_get_canonicalises_as_googles_conformance_suite_says() {
    let canonical = canonical(
        &google(),
        "test-bucket",
        "test-object",
        EMAIL,
        at("2019-02-01T09:00:00Z"),
        Duration::from_secs(10),
    );

    assert_eq!(
        canonical.request,
        "GET\n/test-bucket/test-object\nX-Goog-Algorithm=GOOG4-RSA-SHA256&X-Goog-Credential=test-iam-credentials%40dummy-project-id.iam.gserviceaccount.com%2F20190201%2Fauto%2Fstorage%2Fgoog4_request&X-Goog-Date=20190201T090000Z&X-Goog-Expires=10&X-Goog-SignedHeaders=host\nhost:storage.googleapis.com\n\nhost\nUNSIGNED-PAYLOAD"
    );
    assert_eq!(
        canonical.to_sign(),
        "GOOG4-RSA-SHA256\n20190201T090000Z\n20190201/auto/storage/goog4_request\n00e2fb794ea93d7adb703edaebdd509821fcc7d4f1a79ac5c8d2b394df109320"
    );
}

#[test]
fn the_date_and_the_expiry_move_the_signature_scope() {
    let canonical = canonical(
        &google(),
        "test-bucket",
        "test-object",
        EMAIL,
        at("2019-03-01T09:00:00Z"),
        Duration::from_secs(20),
    );

    assert_eq!(
        canonical.to_sign(),
        "GOOG4-RSA-SHA256\n20190301T090000Z\n20190301/auto/storage/goog4_request\n779f19fdb6fd381390e2d5af04947cf21750277ee3c20e0c97b7e46a1dff8907"
    );
}

#[test]
fn slashes_in_an_object_name_are_kept_and_everything_else_is_escaped() {
    let canonical = canonical(
        &google(),
        "test-bucket",
        "/path/with/slashes/under_score/amper&sand/file.ext",
        EMAIL,
        at("2019-02-01T09:00:00Z"),
        Duration::from_secs(10),
    );

    assert!(
        canonical.request.starts_with(
            "GET\n/test-bucket//path/with/slashes/under_score/amper%26sand/file.ext\n"
        ),
        "{}",
        canonical.request
    );
    assert_eq!(
        canonical.to_sign(),
        "GOOG4-RSA-SHA256\n20190201T090000Z\n20190201/auto/storage/goog4_request\n63c601ecd6ccfec84f1113fc906609cbdf7651395f4300cecd96ddd2c35164f8"
    );
}

#[test]
fn the_url_carries_a_hex_rsa_signature_over_the_string_to_sign() {
    use rsa::pkcs1::{EncodeRsaPrivateKey, EncodeRsaPublicKey};

    let key = rsa::RsaPrivateKey::new(&mut rand_core::OsRng, 2048).expect("a throwaway test key");
    let private = key.to_pkcs1_pem(rsa::pkcs1::LineEnding::LF).unwrap();
    let public = key
        .to_public_key()
        .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
        .unwrap();
    let account = ServiceAccount::from_json(
        &serde_json::json!({
            "client_email": EMAIL,
            "private_key": private.as_str(),
            "token_uri": "https://oauth2.googleapis.com/token",
        })
        .to_string(),
    )
    .unwrap();

    let canonical = canonical(
        &google(),
        "test-bucket",
        "test-object",
        EMAIL,
        at("2019-02-01T09:00:00Z"),
        Duration::from_secs(10),
    );
    let url = signed(&canonical, &account).unwrap();

    let (unsigned, signature) = url.split_once("&X-Goog-Signature=").unwrap();
    assert_eq!(unsigned, canonical.url);
    let signature = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(hex::decode(signature).expect("Google expects the signature hex-encoded"));
    assert!(
        jsonwebtoken::crypto::verify(
            &signature,
            canonical.to_sign().as_bytes(),
            &jsonwebtoken::DecodingKey::from_rsa_pem(public.as_bytes()).unwrap(),
            jsonwebtoken::Algorithm::RS256,
        )
        .unwrap()
    );
}
