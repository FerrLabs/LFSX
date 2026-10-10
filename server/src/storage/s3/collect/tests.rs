use super::*;

const OID: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
const OTHER: &str = "486ea46224d1bb4fb680f34f7c9ad96a8f24ec88be73ea8e5a6c65260e9cb8a7";

fn entry(key: &str, size: u64) -> keyspace::Entry {
    keyspace::Entry {
        key: key.to_owned(),
        modified: None,
        size,
    }
}

fn marker(prefix: &str, oid: &str) -> String {
    format!("{prefix}{}/{}/{oid}", &oid[..2], &oid[2..4])
}

fn oid(raw: &str) -> Oid {
    Oid::parse(raw).unwrap()
}

const OURS: &str = "FerrLabs/Blastlands/";

#[test]
fn a_marker_is_ours_or_a_claim_by_another_repository() {
    let survey = Survey::of(
        vec![
            entry(&marker(OURS, OID), 0),
            entry(&marker("FerrLabs/RogueLite/", OTHER), 0),
        ],
        OURS,
    );

    assert_eq!(survey.markers.len(), 2);
    assert_eq!(survey.mine.len(), 1);
    assert_eq!(survey.mine[0].1, oid(OID));
    assert_eq!(survey.claimed_elsewhere, HashSet::from([oid(OTHER)]));
}

#[test]
fn a_repository_on_another_forge_claims_the_same_bytes() {
    let survey = Survey::of(
        vec![
            entry(&marker(OURS, OID), 0),
            entry(&marker("work~FerrLabs/Blastlands/", OID), 0),
        ],
        OURS,
    );

    assert_eq!(survey.mine.len(), 1);
    assert!(
        survey.claimed_elsewhere.contains(&oid(OID)),
        "the same org/repo on a named forge is another holder, so the bytes must stay"
    );
}

#[test]
fn content_keys_give_sizes_and_never_claim() {
    let survey = Survey::of(vec![entry(&format!(".content/2c/f2/{OID}"), 1234)], OURS);

    assert_eq!(survey.content_sizes.get(OID), Some(&1234));
    assert!(survey.markers.is_empty());
    assert!(survey.claimed_elsewhere.is_empty());
}

#[test]
fn bookkeeping_keys_are_never_read_as_claims() {
    let survey = Survey::of(
        vec![
            entry(&format!(".locks/FerrLabs/Blastlands/{OTHER}"), 0),
            entry(&format!(".refs/{OID}/FerrLabs/RogueLite"), 0),
            entry(&format!(".incoming/FerrLabs/RogueLite/2c/f2/{OTHER}"), 0),
            entry(&format!(".probe/{OTHER}"), 0),
        ],
        OURS,
    );

    assert!(survey.markers.is_empty());
    assert!(
        survey.claimed_elsewhere.is_empty(),
        "a lock id or an in-flight upload taken for a claim would keep an object forever"
    );
}

#[test]
fn only_our_size_index_is_kept_and_none_of_it_is_a_marker() {
    let ns = Namespace::new("FerrLabs", "Blastlands").unwrap();
    let theirs = Namespace::new("FerrLabs", "RogueLite").unwrap();
    let ours_sized = sizes::key(&ns, &oid(OID), 42);

    let survey = Survey::of(
        vec![
            entry(&ours_sized, 0),
            entry(&sizes::key(&theirs, &oid(OTHER), 7), 0),
        ],
        OURS,
    );

    assert_eq!(survey.sized.get(&oid(OID)), Some(&ours_sized));
    assert_eq!(survey.sized.len(), 1);
    assert!(survey.markers.is_empty());
    assert!(survey.claimed_elsewhere.is_empty());
}

#[test]
fn a_key_that_does_not_end_in_an_oid_is_ignored() {
    let survey = Survey::of(vec![entry("FerrLabs/Blastlands/README", 0)], OURS);

    assert!(survey.markers.is_empty());
    assert!(survey.mine.is_empty());
}
