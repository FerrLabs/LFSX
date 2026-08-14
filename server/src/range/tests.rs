use super::*;

const SIZE: u64 = 1000;

#[test]
fn a_closed_range_is_taken_as_written() {
    assert_eq!(
        Range::parse(Some("bytes=0-499"), SIZE),
        Range::Slice { start: 0, end: 499 }
    );
    assert_eq!(
        Range::parse(Some("bytes=500-999"), SIZE),
        Range::Slice {
            start: 500,
            end: 999
        }
    );
}

#[test]
fn an_open_range_runs_to_the_end() {
    assert_eq!(
        Range::parse(Some("bytes=900-"), SIZE),
        Range::Slice {
            start: 900,
            end: 999
        },
        "this is the shape a client uses to resume an interrupted download"
    );
}

#[test]
fn a_suffix_range_counts_back_from_the_end() {
    assert_eq!(
        Range::parse(Some("bytes=-100"), SIZE),
        Range::Slice {
            start: 900,
            end: 999
        }
    );
    assert_eq!(
        Range::parse(Some("bytes=-5000"), SIZE),
        Range::Slice { start: 0, end: 999 },
        "asking for more trailing bytes than exist yields the whole object, not an error"
    );
}

#[test]
fn an_end_past_the_object_is_clamped_rather_than_refused() {
    assert_eq!(
        Range::parse(Some("bytes=500-100000"), SIZE),
        Range::Slice {
            start: 500,
            end: 999
        }
    );
}

#[test]
fn a_range_starting_past_the_object_is_unsatisfiable() {
    assert_eq!(
        Range::parse(Some("bytes=1000-"), SIZE),
        Range::Unsatisfiable
    );
    assert_eq!(
        Range::parse(Some("bytes=2000-3000"), SIZE),
        Range::Unsatisfiable
    );
    assert_eq!(Range::parse(Some("bytes=-0"), SIZE), Range::Unsatisfiable);
    assert_eq!(
        Range::parse(Some("bytes=0-"), 0),
        Range::Unsatisfiable,
        "an empty object satisfies no range at all"
    );
}

#[test]
fn a_backwards_range_is_unsatisfiable() {
    assert_eq!(
        Range::parse(Some("bytes=500-100"), SIZE),
        Range::Unsatisfiable
    );
}

#[test]
fn anything_unparseable_serves_the_whole_object() {
    for header in [
        "bytes=abc-def",
        "items=0-100",
        "bytes=0-499, 900-999",
        "bytes",
        "",
    ] {
        assert_eq!(
            Range::parse(Some(header), SIZE),
            Range::Full,
            "ignoring a range we do not understand is allowed, refusing the transfer is not ({header})"
        );
    }
    assert_eq!(Range::parse(None, SIZE), Range::Full);
}

#[test]
fn the_length_is_what_goes_in_content_length() {
    assert_eq!(Range::Slice { start: 0, end: 499 }.length(SIZE), 500);
    assert_eq!(
        Range::Slice {
            start: 999,
            end: 999
        }
        .length(SIZE),
        1
    );
    assert_eq!(Range::Full.length(SIZE), SIZE);
}
