use super::*;

const OID: &str = "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03";

fn namespace() -> Namespace {
    Namespace::new("FerrLabs", "Blastlands").unwrap()
}

#[test]
fn a_key_carries_the_number_back_out_of_itself() {
    for size in [0, 1, 4096, u64::MAX] {
        assert_eq!(
            read(&key(&namespace(), OID, size)),
            Some((OID.to_owned(), size)),
            "{size}"
        );
    }
}

// Everything that walks a repository's prefix meets both, and reading one of
// these as a marker is a claim on an object whose name ends in a number.
#[test]
fn an_index_key_is_never_mistaken_for_a_marker() {
    assert!(is_one(&key(&namespace(), OID, 12)));
    assert!(!is_one(&format!("FerrLabs/Blastlands/58/91/{OID}")));
}

// A marker read as an index entry would be an object of no size at all, which is
// a repository reported as holding nothing.
#[test]
fn a_marker_carries_no_size() {
    assert_eq!(read(&format!("FerrLabs/Blastlands/58/91/{OID}")), None);
}

#[test]
fn nothing_else_is_read_as_a_size() {
    for nonsense in [
        // The oid is not one.
        "FerrLabs/Blastlands/.sizes/short.12",
        // The size is not a number.
        &format!("FerrLabs/Blastlands/.sizes/{OID}.enormous"),
        // A size that would not fit the counter it feeds.
        &format!("FerrLabs/Blastlands/.sizes/{OID}.99999999999999999999"),
        "FerrLabs/Blastlands/.sizes/",
    ] {
        assert_eq!(read(nonsense), None, "{nonsense}");
    }
}
