use super::*;

const OID: &str = "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03";

fn keyring(keys: &[String]) -> Keyring {
    Keyring::parse(&keys.join("\n")).unwrap()
}

fn key(byte: u8) -> String {
    hex::encode([byte; KEY])
}

#[test]
fn what_goes_in_comes_back_out() {
    let ring = keyring(&[key(1)]);
    let object = ring.writing();
    let plain = b"a mesh nobody else is entitled to read".repeat(64);

    let sealed = object.seal(0, true, OID, &plain).unwrap();

    assert_ne!(sealed, plain, "the whole point");
    assert_eq!(sealed.len() as u64, plain.len() as u64 + TAG);
    assert_eq!(object.open(0, true, OID, &sealed).unwrap(), plain);
}

#[test]
fn a_reader_rebuilds_the_same_key_from_the_id_and_salt_on_the_object() {
    let ring = keyring(&[key(1), key(2)]);
    let object = ring.writing();
    let sealed = object.seal(0, true, OID, b"an asset").unwrap();

    let reopened = ring.reading(object.id(), object.salt()).unwrap();

    assert_eq!(reopened.open(0, true, OID, &sealed).unwrap(), b"an asset");
}

#[test]
fn a_key_the_ring_does_not_hold_is_named_rather_than_guessed_at() {
    let written = keyring(&[key(1)]);
    let object = written.writing();

    // Only the error is formatted: neither a Keyring nor an ObjectKey derives
    // Debug, because the one place key material must never reach is a log line.
    let outcome = keyring(&[key(9)]).reading(object.id(), object.salt()).err();

    assert!(
        matches!(outcome, Some(Error::UnknownKey)),
        "an operator who rotated a key out and still has objects written under it needs to be \
         told that, not handed a decryption failure that reads like corruption: {outcome:?}"
    );
}

#[test]
fn an_older_key_still_reads_what_it_wrote_after_a_rotation() {
    let before = keyring(&[key(1)]);
    let object = before.writing();
    let sealed = object
        .seal(0, true, OID, b"pushed before the rotation")
        .unwrap();

    // The new key goes on the front: writes use it, and the old one stays for
    // everything already on disk.
    let after = keyring(&[key(2), key(1)]);

    assert_eq!(
        after
            .reading(object.id(), object.salt())
            .unwrap()
            .open(0, true, OID, &sealed)
            .unwrap(),
        b"pushed before the rotation"
    );
    assert_eq!(
        after.writing().id(),
        keyring(&[key(2)]).writing().id(),
        "and the first key is the one new objects are written under"
    );
}

#[test]
fn two_objects_under_one_key_never_share_a_keystream() {
    let ring = keyring(&[key(1)]);
    let first = ring.writing();
    let second = ring.writing();

    assert_ne!(
        first.salt(),
        second.salt(),
        "the nonce is only a frame counter, so two objects sharing a key would encrypt their \
         first frame with the same keystream — the salt is what makes that impossible rather \
         than unlikely"
    );

    let plain = b"identical bytes pushed twice";
    assert_ne!(
        first.seal(0, true, OID, plain).unwrap(),
        second.seal(0, true, OID, plain).unwrap()
    );
}

// Each of these is a rearrangement someone with write access to the disk could
// perform without touching a single byte inside a frame. The AEAD refuses them
// because what a frame is bound to is part of what it authenticates.
#[test]
fn a_frame_is_only_valid_where_it_was_written() {
    let ring = keyring(&[key(1)]);
    let object = ring.writing();
    let sealed = object
        .seal(3, false, OID, b"the fourth frame of an object")
        .unwrap();

    let elsewhere = "0000000000000000000000000000000000000000000000000000000000000000";

    assert!(object.open(4, false, OID, &sealed).is_err(), "moved frame");
    assert!(
        object.open(3, true, OID, &sealed).is_err(),
        "truncated after"
    );
    assert!(
        object.open(3, false, elsewhere, &sealed).is_err(),
        "moved object"
    );
    assert!(object.open(3, false, OID, &sealed).is_ok());
}

#[test]
fn a_flipped_bit_is_refused_rather_than_decrypted_into_something_else() {
    let ring = keyring(&[key(1)]);
    let object = ring.writing();
    let mut sealed = object
        .seal(0, true, OID, b"an asset worth its integrity")
        .unwrap();
    sealed[4] ^= 0x01;

    assert!(matches!(
        object.open(0, true, OID, &sealed),
        Err(Error::Tampered)
    ));
}

#[test]
fn a_key_file_that_is_not_one_is_refused_at_load_rather_than_at_the_first_push() {
    for contents in ["", "# only a comment\n", "not hex", &key(1)[..40]] {
        assert!(
            Keyring::parse(contents).is_err(),
            "accepted {contents:?}, which would have failed on the first upload instead"
        );
    }
}

#[test]
fn comments_and_blank_lines_are_not_keys() {
    let ring = Keyring::parse(&format!(
        "# the current key\n{}\n\n# retired\n{}\n",
        key(1),
        key(2)
    ))
    .unwrap();

    assert_eq!(ring.keys.len(), 2);
    assert_eq!(ring.writing().id(), keyring(&[key(1)]).writing().id());
}

#[test]
fn the_same_key_listed_twice_is_a_typo_and_is_said_so() {
    let outcome = Keyring::parse(&format!("{}\n{}\n", key(1), key(1))).err();

    assert!(
        matches!(outcome, Some(Error::Misconfigured(_))),
        "{outcome:?}"
    );
}
