use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

const TTL: Duration = Duration::from_secs(30);

async fn counted(cache: &Cached<u32>, loads: &AtomicUsize) -> u32 {
    *cache
        .get(|| async {
            loads.fetch_add(1, Ordering::SeqCst);
            Ok(7)
        })
        .await
        .unwrap()
}

#[tokio::test(start_paused = true)]
async fn many_reads_inside_the_window_load_once() {
    let cache = Cached::new(TTL);
    let loads = AtomicUsize::new(0);

    for _ in 0..50 {
        assert_eq!(counted(&cache, &loads).await, 7);
    }

    assert_eq!(loads.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn a_read_after_the_window_loads_again() {
    let cache = Cached::new(TTL);
    let loads = AtomicUsize::new(0);
    counted(&cache, &loads).await;

    tokio::time::advance(TTL).await;
    counted(&cache, &loads).await;

    assert_eq!(loads.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn invalidating_forces_the_next_read_to_load() {
    let cache = Cached::new(TTL);
    let loads = AtomicUsize::new(0);
    counted(&cache, &loads).await;

    cache.invalidate().await;
    counted(&cache, &loads).await;

    assert_eq!(loads.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn a_failed_load_is_not_kept() {
    let cache: Cached<u32> = Cached::new(TTL);

    assert!(
        cache
            .get(|| async { Err(Error::Unauthenticated) })
            .await
            .is_err()
    );

    let loads = AtomicUsize::new(0);
    assert_eq!(counted(&cache, &loads).await, 7);
}
