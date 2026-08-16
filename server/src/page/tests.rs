use super::*;
use crate::locks::Owner;

fn lock(id: &str) -> Lock {
    Lock {
        id: id.to_owned(),
        path: format!("Assets/{id}.psd"),
        locked_at: "2026-08-15T00:00:00Z".to_owned(),
        owner: Owner {
            name: "writer".to_owned(),
        },
    }
}

fn locks(count: usize) -> Vec<Lock> {
    (0..count).map(|n| lock(&format!("{n:04}"))).collect()
}

fn ids(page: &Page) -> Vec<&str> {
    page.locks.iter().map(|lock| lock.id.as_str()).collect()
}

#[test]
fn a_short_list_comes_back_whole_with_no_cursor() {
    let page = paginate(locks(3), None, None);

    assert_eq!(ids(&page), ["0000", "0001", "0002"]);
    assert!(
        page.next_cursor.is_empty(),
        "an empty cursor is what tells the client it has the whole list"
    );
}

#[test]
fn a_limit_is_honoured_and_says_where_to_resume() {
    let page = paginate(locks(10), None, Some(4));

    assert_eq!(ids(&page), ["0000", "0001", "0002", "0003"]);
    assert_eq!(page.next_cursor, "0003");
}

#[test]
fn the_cursor_resumes_after_the_lock_it_names() {
    let first = paginate(locks(10), None, Some(4));
    let second = paginate(locks(10), Some(&first.next_cursor), Some(4));

    assert_eq!(ids(&second), ["0004", "0005", "0006", "0007"]);
    assert_eq!(second.next_cursor, "0007");
}

#[test]
fn the_last_page_closes_the_walk() {
    let page = paginate(locks(10), Some("0007"), Some(4));

    assert_eq!(ids(&page), ["0008", "0009"]);
    assert!(
        page.next_cursor.is_empty(),
        "a cursor on the last page sends the client round again for nothing"
    );
}

#[test]
fn a_lock_released_between_pages_does_not_lose_the_ones_after_it() {
    let mut remaining = locks(10);
    remaining.retain(|lock| lock.id != "0004");

    let page = paginate(remaining, Some("0003"), Some(4));

    assert_eq!(
        ids(&page),
        ["0005", "0006", "0007", "0008"],
        "the cursor is a position in the ordering, not an index, so releasing a lock mid-walk \
         skips it rather than shifting everything after it out of view"
    );
}

#[test]
fn an_unreasonable_limit_is_brought_back_to_one_this_server_will_serve() {
    assert_eq!(paginate(locks(5), None, Some(0)).locks.len(), 1);
    assert_eq!(paginate(locks(5), None, Some(usize::MAX)).locks.len(), 5);
    assert_eq!(
        paginate(locks(2000), None, Some(usize::MAX)).locks.len(),
        MAX
    );
}

#[test]
fn the_order_does_not_depend_on_how_the_directory_was_read() {
    let mut shuffled = locks(6);
    shuffled.reverse();

    assert_eq!(
        ids(&paginate(shuffled, None, Some(3))),
        ["0000", "0001", "0002"],
        "a cursor is only meaningful against a stable order, and a directory listing is not one"
    );
}
