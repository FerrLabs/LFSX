use super::*;

#[test]
fn the_oid_is_the_first_column_of_ls_files() {
    let output = "\
5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03 * Assets/Art/Hero.psd
b1946ac92492d2347c6235b4d2611184d4b1b5a9c8a7f7e6d5c4b3a2918070f1 - Assets/Scenes/Arena.unity
";

    assert_eq!(
        parse_ls_files(output),
        vec![
            "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03",
            "b1946ac92492d2347c6235b4d2611184d4b1b5a9c8a7f7e6d5c4b3a2918070f1"
        ]
    );
}

#[test]
fn anything_that_is_not_a_digest_is_dropped() {
    let output = "\
short * Assets/Art/Hero.psd
zzz1b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03 * Assets/Art/Villain.psd

";

    assert!(
        parse_ls_files(output).is_empty(),
        "a malformed line must not be sent as a retained oid, since anything not retained is swept"
    );
}
