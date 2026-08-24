use super::*;

#[test]
fn a_budget_spends_down_to_its_ceiling_and_then_refuses() {
    let budget = Budget::new(Some(60));

    for spent in 0..60 {
        assert_eq!(budget.spend(), None, "lookup {spent} was within the minute");
    }

    let refused = budget.spend();

    assert!(refused.is_some(), "the sixty-first is over the ceiling");
}

// Never zero. A client told to come back immediately does, and a flood is made
// of exactly that.
#[test]
fn a_refusal_always_says_when_to_come_back() {
    let budget = Budget::new(Some(60));

    for _ in 0..60 {
        budget.spend();
    }

    for _ in 0..20 {
        assert!(budget.spend().is_some_and(|wait| wait >= 1));
    }
}

// A small ceiling has to say a longer wait than a large one, because that is the
// only thing the number means: how long until this server will ask again.
#[test]
fn a_tighter_ceiling_asks_for_a_longer_wait() {
    let tight = Budget::new(Some(6));
    let loose = Budget::new(Some(600));

    for _ in 0..6 {
        tight.spend();
    }
    for _ in 0..600 {
        loose.spend();
    }

    assert!(tight.spend().unwrap() > loose.spend().unwrap());
}

// Unset is unlimited, which is what a server nobody is flooding wants and what
// every deployment had before this existed.
#[test]
fn no_ceiling_never_refuses() {
    let budget = Budget::new(None);

    for _ in 0..10_000 {
        assert_eq!(budget.spend(), None);
    }
}

// The bucket refills, so a server that was throttled a moment ago is not
// throttled for good. Asserted by handing it a starting point in the past rather
// than by sleeping, which would make the suite wait for a clock.
#[test]
fn a_bucket_refills_over_time() {
    let budget = Budget::new(Some(60));

    for _ in 0..60 {
        budget.spend();
    }
    assert!(budget.spend().is_some());

    budget.state.lock().unwrap().refilled_at -= std::time::Duration::from_secs(30);

    for spent in 0..30 {
        assert_eq!(
            budget.spend(),
            None,
            "half a minute earns half a minute's worth, and {spent} is inside it"
        );
    }
}
