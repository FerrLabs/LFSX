use super::*;
use crate::locks::Owner;

fn overview(locks: Vec<Lock>) -> Overview {
    Overview {
        namespace: Namespace::new("FerrLabs", "Blastlands").unwrap(),
        objects: 1851,
        bytes: 96_468_992,
        locks,
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
