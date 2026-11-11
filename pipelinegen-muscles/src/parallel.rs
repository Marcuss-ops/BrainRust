//! Bounded fan-out for independent blocking operations.
//!
//! Media probes are independent but each one spawns an `ffprobe` process. A
//! thread (and process) per input makes a long timeline spike the whole host,
//! so the caller supplies a small concurrency limit and this module preserves
//! input order while never running more than `limit` operations at once.

use std::thread;

/// Upper bound for independent media probes. Eight keeps a normal timeline
/// saturated without letting a 200-segment assembly spawn 200 ffprobe
/// processes at the same time.
pub(crate) const MAX_PARALLEL_IO: usize = 8;

/// Runs `f` over `items`, at most `limit` at a time, preserving input order.
/// A panicking worker is converted into an `Err` for that item instead of
/// unwinding through the persistent media worker.
pub(crate) fn bounded_map<T, U, F>(items: &[T], limit: usize, f: F) -> Vec<Result<U, String>>
where
    T: Sync,
    U: Send,
    F: Fn(&T) -> Result<U, String> + Sync,
{
    let limit = limit.max(1);
    let mut results = Vec::with_capacity(items.len());
    for chunk in items.chunks(limit) {
        let chunk_results = thread::scope(|scope| {
            let f = &f;
            let handles: Vec<_> = chunk
                .iter()
                .map(|item| scope.spawn(move || f(item)))
                .collect();
            handles
                .into_iter()
                .map(|handle| {
                    handle
                        .join()
                        .unwrap_or_else(|_| Err("parallel worker panicked".to_string()))
                })
                .collect::<Vec<_>>()
        });
        results.extend(chunk_results);
    }
    results
}

#[cfg(test)]
mod tests {
    use super::bounded_map;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[test]
    fn bounded_map_caps_concurrency_and_preserves_order() {
        let items: Vec<usize> = (0..20).collect();
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let results = bounded_map(&items, 4, |item| {
            let now = active.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(5));
            active.fetch_sub(1, Ordering::SeqCst);
            Ok(*item * 2)
        });
        assert_eq!(results.len(), 20);
        for (index, result) in results.into_iter().enumerate() {
            assert_eq!(result.expect("bounded map item"), index * 2);
        }
        assert!(peak.load(Ordering::SeqCst) <= 4);
    }

    #[test]
    fn bounded_map_reports_a_panicking_worker() {
        let items = [0usize, 1];
        let results = bounded_map(&items, 2, |item| {
            if *item == 0 {
                panic!("synthetic bounded-map panic");
            }
            Ok(*item)
        });
        assert!(results[0].is_err());
        assert_eq!(results[1].as_ref().expect("second item"), &1);
    }
}
