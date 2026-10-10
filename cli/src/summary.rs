use std::fmt::Write;

use serde_json::Value;

const GIB: f64 = 1_073_741_824.0;

fn count(report: &Value, field: &str) -> u64 {
    report[field].as_u64().unwrap_or_default()
}

fn gib(bytes: u64) -> f64 {
    bytes as f64 / GIB
}

pub fn gc(report: &Value, dry_run: bool) -> String {
    let kept = count(report, "within_grace");

    format!(
        "{} {} objects, {:.1} GiB{}",
        if dry_run { "would free" } else { "freed" },
        count(report, "swept"),
        gib(count(report, "bytes")),
        if kept > 0 {
            format!(", {kept} left alone inside the grace period")
        } else {
            String::new()
        }
    )
}

pub fn dedupe(report: &Value, dry_run: bool) -> String {
    let refused = count(report, "refused");

    format!(
        "{} {} objects into the shared store, linked {}, {} {:.2} GiB{}",
        if dry_run { "would move" } else { "moved" },
        count(report, "adopted"),
        count(report, "linked"),
        if dry_run { "freeing" } else { "freed" },
        gib(count(report, "reclaimed")),
        if refused > 0 {
            format!(", {refused} refused, see the server log")
        } else {
            String::new()
        }
    )
}

pub fn compress(report: &Value, dry_run: bool) -> String {
    let before = count(report, "before");
    let after = count(report, "after");
    let kept = after.saturating_mul(100).checked_div(before).unwrap_or(100);

    format!(
        "{} {} objects: {:.2} GiB -> {:.2} GiB ({}% smaller), {} already compressed, {} left as \
         they were",
        if dry_run {
            "would compress"
        } else {
            "compressed"
        },
        count(report, "compressed"),
        gib(before),
        gib(after),
        100u64.saturating_sub(kept),
        count(report, "already"),
        count(report, "left_alone"),
    )
}

pub struct Audit {
    pub text: String,
    pub clean: bool,
}

pub fn verify(report: &Value) -> Audit {
    let listed = |field: &str| report[field].as_array().cloned().unwrap_or_default();
    let corrupt = listed("corrupt");
    let unreadable = listed("unreadable");
    let incomplete = report["incomplete"].as_bool().unwrap_or_default();

    let mut text = format!(
        "read {} objects, {:.2} GiB\n",
        count(report, "checked"),
        gib(count(report, "bytes"))
    );

    for (label, oids) in [("corrupt", &corrupt), ("unreadable", &unreadable)] {
        if oids.is_empty() {
            continue;
        }

        let _ = writeln!(text, "{label}:");
        for oid in oids {
            let _ = writeln!(text, "  {}", oid.as_str().unwrap_or_default());
        }
    }

    if incomplete {
        text.push_str(
            "part of the repository could not be listed: this audit is not a clean bill\n",
        );
    }

    Audit {
        text,
        clean: !incomplete && corrupt.is_empty() && unreadable.is_empty(),
    }
}

#[cfg(test)]
mod tests;
