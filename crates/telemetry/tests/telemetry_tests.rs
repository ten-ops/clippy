//! # Unit Tests for Telemetry Metrics and Reporting
//!
//! This module contains integration-style tests for the `telemetry` crate,
//! verifying the behavior of metrics counters and the periodic reporting
//! infrastructure.
//!
//! ## Test Design Notes
//! - Tests run sequentially by default (Rust test harness does not parallelize
//!   tests by default unless `--test-threads=1` is used, but it's safe to assume
//!   they can run in parallel as they only read/increment atomic counters).
//! - The `Metrics::get()` singleton is shared across tests; increments are
//!   cumulative. Tests rely on relative changes (before/after) to avoid
//!   interdependency.
//! - `Box::leak` is used in `test_periodic_sink` to obtain a `'static` reference
//!   to an `AtomicBool` – this leaks memory but is acceptable in a test context
//!   where the process exits immediately afterward.
//!
//! ## Potential Pitfalls
//! - If tests are run with `--test-threads=1`, there is no risk of interference.
//!   With parallel threads, counters might be incremented by other tests
//!   concurrently, but the before/after pattern still holds because each test
//!   operates on its own delta.
//! - The `test_periodic_sink` timing is approximate; it uses `thread::sleep`
//!   which may be imprecise. The assertion checks for at least 2 reports,
//!   allowing for scheduling variability.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use telemetry::{Metrics, PeriodicSink, Sink, StdoutSink};

/// Verifies that `inc_clipboard_event_count` and `inc_connection_retries_count`
/// correctly increment their respective counters by exactly one.
///
/// This test captures the counter value before the increment, performs the
/// increment, and asserts that the new value is exactly one greater.
#[test]
fn test_metric_increments() {
    let metrics = Metrics::get();

    // Test clipboard_event_count
    let before = metrics.clipboard_event_count.load(Ordering::Relaxed);
    metrics.inc_clipboard_event_count();
    let after = metrics.clipboard_event_count.load(Ordering::Relaxed);
    assert_eq!(after, before + 1);

    // Test connection_retries_count
    let before = metrics.connection_retries_count.load(Ordering::Relaxed);
    metrics.inc_connection_retries_count();
    let after = metrics.connection_retries_count.load(Ordering::Relaxed);
    assert_eq!(after, before + 1);
}

/// Verifies that `inc_eintr_count` increments the `eintr_count` counter by one.
#[test]
fn test_eintr_increments() {
    let metrics = Metrics::get();
    let before = metrics.eintr_count.load(Ordering::Relaxed);
    metrics.inc_eintr_count();
    let after = metrics.eintr_count.load(Ordering::Relaxed);
    assert_eq!(after, before + 1);
}

/// Verifies that `inc_fetch_failed_count` increments the `fetch_failed_count`
/// counter by one.
#[test]
fn test_fetches_failed_count() {
    let metrics = Metrics::get();
    let before = metrics.fetch_failed_count.load(Ordering::Relaxed);
    metrics.inc_fetch_failed_count();
    let after = metrics.fetch_failed_count.load(Ordering::Relaxed);
    assert_eq!(after, before + 1);
}

/// Ensures that `StdoutSink::report` does not panic when called.
///
/// This is a smoke test: it only verifies that the method executes without
/// panicking; output is printed to stdout but not captured/asserted.
#[test]
fn test_stdout_sink_does_not_panic() {
    let sink = StdoutSink;
    sink.report(Metrics::get());
}

/// Tests the `PeriodicSink` background reporting thread.
///
/// This test uses a mock sink that counts how many times `report` was called.
/// It starts a periodic sink with a 50ms interval, lets it run for ~120ms,
/// then stops it via the `running` flag and verifies that at least two reports
/// were generated (accounting for thread scheduling and sleep inaccuracies).
///
/// # Implementation Details
/// - A `MockSink` wraps an `Arc<AtomicUsize>` to count invocations.
/// - The `running` flag is created as an `Arc<AtomicBool>`, then leaked to
///   obtain a `'static` reference, as required by `PeriodicSink`.
/// - After stopping the flag, a short sleep ensures the background thread has
///   time to exit.
#[test]
fn test_periodic_sink() {
    // Mock sink that simply increments a counter on each report.
    struct MockSink {
        call_count: Arc<AtomicUsize>,
    }

    impl Sink for MockSink {
        fn report(&self, _: &Metrics) {
            self.call_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    let call_count = Arc::new(AtomicUsize::new(0));
    let sink = MockSink {
        call_count: call_count.clone(),
    };

    // Create a running flag that we can later set to false.
    // We need a 'static reference, so we leak a boxed Arc (acceptable in tests).
    let running = Arc::new(AtomicBool::new(true));
    let leaked_running: &'static AtomicBool = &*Box::leak(Box::new(running.clone()));

    // Start periodic reporting with a 50ms interval.
    let periodic = PeriodicSink::new(sink, Duration::from_millis(50), leaked_running);
    periodic.start();

    // Let it run for ~120ms (should produce at least 2 reports, possibly 3).
    thread::sleep(Duration::from_millis(120));

    // Signal the thread to stop.
    running.store(false, Ordering::Relaxed);

    // Give the thread time to exit (it may be sleeping when flag is set).
    thread::sleep(Duration::from_millis(100));

    let count = call_count.load(Ordering::Relaxed);
    // Expect at least 2 reports; timing variance could cause 2 or 3.
    assert!(count >= 2, "expected at least 2 reports, but got {}", count);
}
