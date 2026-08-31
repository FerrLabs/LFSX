use super::*;
use crate::locks::Owner;

fn overview(locks: Vec<Lock>) -> Overview {
    aged(locks, None)
}

fn aged(locks: Vec<Lock>, lock_max_age: Option<Duration>) -> Overview {
    Overview {
        namespace: Namespace::new("FerrLabs", "Blastlands").unwrap(),
        objects: 1851,
        bytes: 96_468_992,
        locks,
        lock_max_age,
        writable: true,
    }
}

fn lock(path: &str, owner: &str) -> Lock {
    Lock {
        id: "a".repeat(32),
        path: path.to_owned(),
        locked_at: "2026-08-14T15:00:00Z".to_owned(),
        owner: Owner {
            name: owner.to_owned(),
        },
    }
}

#[test]
fn the_page_reports_what_the_store_holds() {
    let page = render(&overview(Vec::new()));

    assert!(page.contains("FerrLabs/Blastlands"));
    assert!(page.contains("1851"));
    assert!(page.contains("92.0 MiB"));
    assert!(page.contains("Nothing is locked"));
}

#[test]
fn locks_are_listed_with_who_holds_them() {
    let page = render(&overview(vec![lock("Assets/Scenes/Arena.unity", "jane")]));

    assert!(page.contains("Assets/Scenes/Arena.unity"));
    assert!(page.contains("jane"));
    assert!(!page.contains("Nothing is locked"));
}

#[test]
fn a_path_cannot_smuggle_markup_into_the_page() {
    let page = render(&overview(vec![lock(
        "Assets/<script>alert(1)</script>.psd",
        "<img src=x onerror=alert(1)>",
    )]));

    assert!(
        !page.contains("<script>alert(1)</script>"),
        "a file name is attacker-controlled: anyone who can push can name a file"
    );
    assert!(!page.contains("<img src=x"));
    assert!(page.contains("&lt;script&gt;"));
}

#[test]
fn sizes_are_readable_rather_than_exact() {
    assert_eq!(human_bytes(512), "512 B");
    assert_eq!(human_bytes(1024), "1.0 KiB");
    assert_eq!(human_bytes(3_221_225_472), "3.0 GiB");
}

#[test]
fn a_stale_lock_says_how_long_it_has_been_and_that_anyone_can_take_it() {
    let week = Duration::from_secs(7 * 24 * 60 * 60);
    let mut abandoned = lock("Assets/Scenes/Arena.unity", "marie");
    abandoned.locked_at = (time::OffsetDateTime::now_utc() - 3 * week)
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap();

    let page = render(&aged(vec![abandoned.clone()], Some(week)));

    assert!(page.contains("untouched for 3 weeks"), "{page}");
    assert!(page.contains("anyone can take it"));
    assert!(
        page.contains("marie"),
        "who had it is the useful half of the answer"
    );
    assert!(page.contains("class=\"stale\""));

    let held = render(&aged(vec![abandoned], None));
    assert!(
        !held.contains("anyone can take it"),
        "with no expiry configured nothing is stale, however old"
    );
}

#[test]
fn an_age_below_a_minute_is_said_in_seconds() {
    assert_eq!(human_age(Duration::from_secs(40)), "40 seconds");
    assert_eq!(human_age(Duration::from_secs(1)), "1 second");
    assert_eq!(human_age(Duration::from_secs(90)), "1 minute");
}

#[test]
fn the_page_shows_a_pages_worth_of_locks_and_counts_the_rest() {
    let many: Vec<Lock> = (0..120)
        .map(|i| lock(&format!("Assets/{i:03}.unity"), "jane"))
        .collect();

    let page = render(&overview(many));

    assert!(page.contains("Assets/049.unity"));
    assert!(
        !page.contains("Assets/050.unity"),
        "the fifty-first lock is summarised, not rendered"
    );
    assert!(page.contains("and 70 more"), "{page}");
}

#[test]
fn a_pages_worth_of_locks_carries_no_summary_row() {
    let exactly: Vec<Lock> = (0..50)
        .map(|i| lock(&format!("Assets/{i:03}.unity"), "jane"))
        .collect();

    let page = render(&overview(exactly));

    assert!(page.contains("Assets/049.unity"));
    assert!(!page.contains("more, list them"), "{page}");
}
