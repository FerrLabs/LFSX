use serde_json::json;

use super::*;

#[test]
fn compression_that_saved_space_reports_the_share_saved() {
    let line = compress(
        &json!({ "compressed": 3, "before": 1000, "after": 250, "already": 1, "left_alone": 2 }),
        false,
    );

    assert!(line.starts_with("compressed 3 objects"), "{line}");
    assert!(line.contains("(75% smaller)"), "{line}");
    assert!(
        line.contains("1 already compressed, 2 left as they were"),
        "{line}"
    );
}

#[test]
fn compression_that_grew_the_objects_reports_nothing_saved_instead_of_overflowing() {
    let line = compress(&json!({ "before": 100, "after": 120 }), true);

    assert!(line.starts_with("would compress"), "{line}");
    assert!(line.contains("(0% smaller)"), "{line}");
}

#[test]
fn compression_of_nothing_reports_nothing_saved() {
    let line = compress(&json!({}), false);

    assert!(line.contains("(0% smaller)"), "{line}");
}

#[test]
fn collection_mentions_what_the_grace_period_kept_only_when_it_kept_something() {
    let quiet = gc(&json!({ "swept": 4, "bytes": 1_073_741_824u64 }), false);
    let kept = gc(&json!({ "swept": 4, "within_grace": 2 }), true);

    assert_eq!(quiet, "freed 4 objects, 1.0 GiB");
    assert!(kept.starts_with("would free 4 objects"), "{kept}");
    assert!(
        kept.ends_with(", 2 left alone inside the grace period"),
        "{kept}"
    );
}

#[test]
fn deduplication_points_at_the_log_when_objects_were_refused() {
    let line = dedupe(&json!({ "adopted": 5, "linked": 3, "refused": 1 }), false);

    assert!(
        line.starts_with("moved 5 objects into the shared store, linked 3, freed"),
        "{line}"
    );
    assert!(line.ends_with(", 1 refused, see the server log"), "{line}");
}

#[test]
fn an_audit_is_clean_only_when_everything_was_read_and_nothing_was_wrong() {
    assert!(verify(&json!({ "checked": 10 })).clean);

    for report in [
        json!({ "corrupt": ["abc"] }),
        json!({ "unreadable": ["abc"] }),
        json!({ "incomplete": true }),
    ] {
        assert!(!verify(&report).clean, "{report}");
    }
}

#[test]
fn an_audit_lists_each_bad_object_under_its_kind() {
    let audit = verify(&json!({ "checked": 2, "corrupt": ["aaa"], "unreadable": ["bbb"] }));

    assert!(audit.text.contains("corrupt:\n  aaa\n"), "{}", audit.text);
    assert!(
        audit.text.contains("unreadable:\n  bbb\n"),
        "{}",
        audit.text
    );
}
