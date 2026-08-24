use std::sync::Mutex;
use std::time::Instant;

// A ceiling on how often this server will ask the forge anything.
//
// The permission cache already stops one bad token hammering the API, which is
// what #29 built, but it is keyed by the token. A caller that sends a different
// one every request misses every entry and costs a lookup each time, and
// inventing a token that does not exist is free.
//
// What runs out is not this server. GitHub counts a failed authentication
// against the address that made it, so the budget being drained is the one every
// real lookup shares, and legitimate pushes start being refused because somebody
// else spent it.
//
// So the server caps its own rate first. Under a flood a caller is refused here
// rather than by the forge, which is the same answer at the client and a very
// different afternoon behind it: this refusal lifts the moment the flood stops,
// where an exhausted forge quota lasts the rest of the hour and takes every
// other repository on the server with it.
//
// It is a ceiling on this server, not fairness between callers. Telling two
// callers apart needs the address a request came from, and behind a reverse
// proxy that is the proxy: per-client limiting belongs there, where the real
// address is known. What a proxy cannot know is which requests cost a forge
// lookup, and that is exactly what this counts.
pub struct Budget {
    per_minute: Option<u32>,
    state: Mutex<State>,
}

struct State {
    allowance: f64,
    refilled_at: Instant,
}

impl Budget {
    // Refused rather than asked. There is no per-request log: under the flood
    // this exists for, that is thousands of identical lines a second, and the
    // thing an operator watches is `lfsx_rejections_total{cause="lookup_budget_spent"}`,
    // which counts them without drowning everything else.
    pub fn afford(&self) -> Result<(), crate::error::Error> {
        match self.spend() {
            None => Ok(()),
            Some(retry_after) => Err(crate::error::Error::LookupBudgetSpent { retry_after }),
        }
    }

    pub fn new(per_minute: Option<u32>) -> Self {
        match per_minute {
            Some(per_minute) => tracing::info!(
                per_minute,
                "this server will not ask the forge more often than this, counting only lookups \
                 the permission cache could not answer"
            ),
            None => tracing::warn!(
                "LFSX_AUTH_LOOKUP_BUDGET=0, so there is no ceiling on forge lookups: a caller \
                 sending a different token every request can spend this server's standing with \
                 the forge, and every repository shares it"
            ),
        }

        Self {
            per_minute,
            state: Mutex::new(State {
                allowance: per_minute.unwrap_or_default().into(),
                refilled_at: Instant::now(),
            }),
        }
    }

    // None when there was room. Some(seconds) when there was not, which is how
    // long until there is: a client is told when to come back rather than left
    // to guess, the same way a throttled forge tells this server.
    //
    // A bucket rather than a count per minute, because a fixed window lets a
    // caller spend a whole minute's worth in the last instant of one and the
    // same again in the first instant of the next.
    pub fn spend(&self) -> Option<u64> {
        let capacity = f64::from(self.per_minute?);
        let mut state = self.state.lock().expect("forge lookup budget");

        let now = Instant::now();
        let earned = now.duration_since(state.refilled_at).as_secs_f64() * capacity / 60.0;
        state.allowance = (state.allowance + earned).min(capacity);
        state.refilled_at = now;

        if state.allowance >= 1.0 {
            state.allowance -= 1.0;
            return None;
        }

        // Rounded up and never zero. Told to come back in no time at all, a
        // client comes back in no time at all, and the flood this exists to
        // damp is made of exactly that.
        let wait = (1.0 - state.allowance) * 60.0 / capacity;

        Some((wait.ceil() as u64).max(1))
    }
}

#[cfg(test)]
mod tests;
