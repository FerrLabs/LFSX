use super::*;

fn response(headers: &[(&str, &str)]) -> reqwest::Response {
    let mut builder = axum::http::Response::builder().status(403);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }

    reqwest::Response::from(builder.body(String::new()).unwrap())
}

#[test]
fn a_retry_after_is_taken_as_the_duration_it_is() {
    assert_eq!(retry_after(&response(&[("retry-after", "120")])), 120);
}

// A forge that says zero is saying "not yet", and a client told to wait no time
// at all comes straight back into the same limit.
#[test]
fn a_zero_wait_still_waits() {
    assert_eq!(retry_after(&response(&[("retry-after", "0")])), 1);
}

#[test]
fn an_absolute_reset_becomes_a_duration() {
    let in_two_minutes = time::OffsetDateTime::now_utc().unix_timestamp() + 120;
    let seconds = retry_after(&response(&[
        ("x-ratelimit-remaining", "0"),
        ("x-ratelimit-reset", &in_two_minutes.to_string()),
    ]));

    assert!(
        (115..=120).contains(&seconds),
        "the primary limit sends when it resets, not how long to wait: {seconds}"
    );
}

// A reset already past, or a clock that disagrees with the forge's, must not
// turn into "come back immediately".
#[test]
fn a_reset_in_the_past_falls_back_to_the_default() {
    let a_minute_ago = time::OffsetDateTime::now_utc().unix_timestamp() - 60;

    assert_eq!(
        retry_after(&response(&[
            ("x-ratelimit-remaining", "0"),
            ("x-ratelimit-reset", &a_minute_ago.to_string()),
        ])),
        DEFAULT
    );
}

#[test]
fn a_limit_that_says_nothing_still_gets_a_wait() {
    assert_eq!(retry_after(&response(&[])), DEFAULT);
}

// The half that matters for GitHub: a 403 is both how a repository is refused
// and how a limit is reported, and only the headers separate them.
#[test]
fn a_plain_refusal_is_not_mistaken_for_a_limit() {
    assert!(rate_limited(&response(&[("x-ratelimit-remaining", "4999")])).is_none());
    assert!(rate_limited(&response(&[])).is_none());
}

#[test]
fn an_exhausted_quota_is_recognised_as_one() {
    assert_eq!(
        rate_limited(&response(&[("x-ratelimit-remaining", "0")])),
        Some(DEFAULT)
    );
    assert_eq!(rate_limited(&response(&[("retry-after", "30")])), Some(30));
}
