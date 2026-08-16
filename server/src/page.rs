use crate::locks::Lock;

// The lock list is answered from a directory of files, so there is no server-side
// cursor to hand out and nothing to keep between requests. The id of the last
// lock returned is enough: the list is ordered by id, so resuming means skipping
// past that one. A lock released in between simply is not there any more, which
// is the same answer a client would get by asking again.
pub const DEFAULT: usize = 100;
pub const MAX: usize = 1000;

pub struct Page {
    pub locks: Vec<Lock>,
    pub next_cursor: String,
}

pub fn paginate(mut locks: Vec<Lock>, cursor: Option<&str>, limit: Option<usize>) -> Page {
    locks.sort_by(|one, other| one.id.cmp(&other.id));

    let from = match cursor {
        Some(cursor) => locks
            .iter()
            .position(|lock| lock.id.as_str() > cursor)
            .unwrap_or(locks.len()),
        None => 0,
    };

    // A limit the client did not ask for is still a limit: without one, a studio
    // that has locked an art directory receives every lock it holds in a single
    // body, and a client honouring the field it sent believes it has seen the
    // whole list.
    let limit = limit.unwrap_or(DEFAULT).clamp(1, MAX);
    let mut page: Vec<Lock> = locks.into_iter().skip(from).take(limit + 1).collect();

    // One more was fetched than asked for: if it is there, there is another page,
    // and the cursor is the last id actually returned.
    let next_cursor = match page.len() > limit {
        true => {
            page.truncate(limit);
            page.last().map(|lock| lock.id.clone()).unwrap_or_default()
        }
        false => String::new(),
    };

    Page {
        locks: page,
        next_cursor,
    }
}

#[cfg(test)]
mod tests;
