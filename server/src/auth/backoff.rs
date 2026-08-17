// How long to wait when a forge says it is throttling us.
//
// `Retry-After` is what GitHub sends for a secondary limit and what GitLab sends
// for any of them, and it is already a duration. GitHub's primary limit sends an
// absolute reset instead, so that becomes one. A limit with neither is still a
// limit, and gets a minute rather than an invitation to come straight back.
const DEFAULT: u64 = 60;

pub(super) fn retry_after(response: &reqwest::Response) -> u64 {
    let headers = response.headers();
    let number = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok())
    };

    if let Some(after) = number("retry-after") {
        return after.max(1);
    }

    let now = u64::try_from(time::OffsetDateTime::now_utc().unix_timestamp()).unwrap_or_default();

    number("x-ratelimit-reset")
        .map(|reset| reset.saturating_sub(now))
        .filter(|seconds| *seconds > 0)
        .unwrap_or(DEFAULT)
}

// A 403 from GitHub is ambiguous: it is how it refuses a repository and how it
// reports a limit. Only the headers tell them apart.
pub(super) fn rate_limited(response: &reqwest::Response) -> Option<u64> {
    let headers = response.headers();
    let exhausted = headers.contains_key("retry-after")
        || headers
            .get("x-ratelimit-remaining")
            .is_some_and(|remaining| remaining.as_bytes() == b"0");

    exhausted.then(|| retry_after(response))
}

#[cfg(test)]
mod tests;
