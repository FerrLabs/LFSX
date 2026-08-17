use std::time::Duration;

use crate::locks::{self, Lock};
use crate::namespace::Namespace;

pub struct Overview {
    pub namespace: Namespace,
    pub objects: u64,
    pub bytes: u64,
    pub locks: Vec<Lock>,
    pub lock_max_age: Option<Duration>,
    pub writable: bool,
}

pub fn render(overview: &Overview) -> String {
    let Overview {
        namespace,
        objects,
        bytes,
        locks,
        lock_max_age,
        writable,
    } = overview;

    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{namespace} — LFSX</title>
<style>{STYLE}</style>
</head>
<body>
<main>
<h1>{namespace}</h1>
<dl>
<div><dt>Objects</dt><dd>{objects}</dd></div>
<div><dt>On disk</dt><dd>{}</dd></div>
<div><dt>Your access</dt><dd>{}</dd></div>
</dl>
<h2>Locks</h2>
{}
<footer>Read only. Objects are reclaimed with <code>lfsx gc</code>, locks are released with
<code>git lfs unlock</code>.</footer>
</main>
</body>
</html>
"#,
        human_bytes(*bytes),
        if *writable { "read and write" } else { "read" },
        locks_table(locks, *lock_max_age),
    )
}

// This is the only place a person is told a lock has gone stale. `git lfs locks`
// prints the path, the owner and the id, and has no field for anything else, so
// a client cannot be made to show it however the server phrases the JSON.
fn locks_table(locks: &[Lock], max_age: Option<Duration>) -> String {
    if locks.is_empty() {
        return "<p class=\"empty\">Nothing is locked.</p>".to_owned();
    }

    let rows: String = locks
        .iter()
        .map(|lock| match locks::stale_for(lock, max_age) {
            Some(age) => format!(
                "<tr class=\"stale\"><td>{}</td><td>{}</td><td>{} (untouched for {}, anyone can take it)</td></tr>",
                escape(&lock.path),
                escape(&lock.owner.name),
                escape(&lock.locked_at),
                human_age(age)
            ),
            None => format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
                escape(&lock.path),
                escape(&lock.owner.name),
                escape(&lock.locked_at)
            ),
        })
        .collect();

    format!(
        "<table><thead><tr><th>Path</th><th>Held by</th><th>Since</th></tr></thead><tbody>{rows}</tbody></table>"
    )
}

fn human_age(age: Duration) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;

    let seconds = age.as_secs();
    let (count, unit) = match seconds {
        s if s >= 7 * DAY => (s / (7 * DAY), "week"),
        s if s >= DAY => (s / DAY, "day"),
        s if s >= HOUR => (s / HOUR, "hour"),
        s if s >= MINUTE => (s / MINUTE, "minute"),
        // A short ceiling is a strange thing to configure, but "untouched for 0
        // minutes" is a strange thing to print.
        s => (s, "second"),
    };

    format!("{count} {unit}{}", if count == 1 { "" } else { "s" })
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut size = bytes as f64;
    let mut unit = 0;

    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

fn escape(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const STYLE: &str = "\
:root{color-scheme:light dark}\
body{font:16px/1.5 system-ui,sans-serif;margin:0;padding:2rem}\
main{max-width:52rem;margin:0 auto}\
h1{font-size:1.5rem;margin:0 0 1.5rem}\
h2{font-size:1.1rem;margin:2rem 0 .75rem}\
dl{display:grid;grid-template-columns:repeat(auto-fit,minmax(11rem,1fr));gap:1rem;margin:0}\
dt{font-size:.8rem;text-transform:uppercase;letter-spacing:.04em;opacity:.65}\
dd{margin:.25rem 0 0;font-size:1.5rem;font-variant-numeric:tabular-nums}\
table{width:100%;border-collapse:collapse}\
th{text-align:left;font-size:.8rem;text-transform:uppercase;letter-spacing:.04em;opacity:.65;font-weight:400}\
th,td{padding:.5rem 0;border-bottom:1px solid color-mix(in srgb,currentColor 12%,transparent)}\
td{font-variant-numeric:tabular-nums}\
.empty{opacity:.65}\n.stale td{color:#b4690e}\
footer{margin-top:2.5rem;font-size:.85rem;opacity:.65}\
code{font-family:ui-monospace,monospace;font-size:.85em}";

#[cfg(test)]
mod tests;
