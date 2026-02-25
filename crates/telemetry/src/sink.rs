//! # Periodic Metrics Reporting
//!
//! This module provides infrastructure for periodically exporting metrics
//! to various sinks (e.g., stdout, files, monitoring systems). It defines
//! a trait `Sink` for custom report implementations and a `PeriodicSink`
//! that runs a background thread to invoke the sink at regular intervals.
//!
//! ## Design Notes
//! - The reporting thread is detached (no join handle) and runs until a
//!   global stop flag (`AtomicBool`) is set to `false`.
//! - All sinks must implement `Send + Sync` to be safely callable from the
//!   background thread.
//! - Metrics are obtained via the global `Metrics::get()` instance.
//!
//! ## Potential Pitfalls
//! - The stop flag must be `'static` and live for the entire program;
//!   typically it's a `static` variable. The thread will not exit until
//!   the flag is set to `false` and the current sleep completes.
//! - Using `Ordering::Relaxed` for the flag load is sufficient for a simple
//!   stop signal, but if precise memory ordering with other threads is
//!   required, consider `Acquire`/`Release`.
//! - The sink's `report` method runs inside the background thread; any
//!   panics will terminate the thread. Consider adding panic handling if
//!   resilience is needed.
//! - `thread::sleep` may return early due to signals, but the loop will
//!   simply continue, which is acceptable for periodic reporting.

use std::{
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::Metrics;

/// A trait for reporting metrics to a specific destination.
///
/// Implementors must be `Send + Sync` to allow safe usage from a background
/// thread. The `report` method is called periodically by `PeriodicSink`.
pub trait Sink: Send + Sync {
    /// Outputs the current state of the given metrics.
    ///
    /// # Arguments
    /// * `metrics` - Reference to the global metrics instance.
    ///
    /// # Notes
    /// - Implementations should avoid blocking for long periods, as this
    ///   delays the reporting schedule.
    /// - The method is called from a dedicated thread, so panics will
    ///   terminate that thread.
    fn report(&self, metrics: &Metrics);
}

/// A sink that writes metrics to standard output in a human-readable format.
///
/// Each report is a single line prefixed with a UNIX timestamp (seconds since
/// epoch). The timestamp is derived from `SystemTime`, falling back to `0`
/// if the system clock is before the epoch (unlikely).
pub struct StdoutSink;

impl Sink for StdoutSink {
    fn report(&self, metrics: &Metrics) {
        // Obtain current timestamp in seconds since UNIX_EPOCH.
        // If system time is before epoch (should never happen on typical OSes),
        // default to 0 to avoid a panic.
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Load each counter with Relaxed ordering. We only need a consistent
        // snapshot, not synchronization with other memory operations.
        println!(
            "[{}] metrics: clipboard_events={}, connection_retries_count={}, eintr={}, failed_fetches={}",
            timestamp,
            metrics.clipboard_event_count.load(Ordering::Relaxed),
            metrics.connection_retries_count.load(Ordering::Relaxed),
            metrics.eintr_count.load(Ordering::Relaxed),
            metrics.fetch_failed_count.load(Ordering::Relaxed)
        );
    }
}

/// A background thread that periodically reports metrics using a provided sink.
///
/// The thread runs indefinitely until the shared `running` flag becomes `false`.
/// After each report interval, it invokes the sink's `report` method with the
/// global metrics instance.
pub struct PeriodicSink<S: Sink + 'static> {
    sink: S,
    interval: Duration,
    running: &'static AtomicBool,
    metrics: &'static Metrics,
}

impl<S: Sink + 'static> PeriodicSink<S> {
    /// Creates a new `PeriodicSink` instance.
    ///
    /// # Arguments
    /// * `sink` - The sink implementation that will handle each report.
    /// * `interval` - How long to wait between reports.
    /// * `running` - A static atomic boolean that controls the background thread.
    ///               When this flag becomes `false`, the thread will exit after
    ///               completing its current sleep.
    ///
    /// # Notes
    /// - The `running` flag must be `'static` (e.g., a `static` variable) to
    ///   ensure it lives as long as the spawned thread.
    /// - The metrics instance is obtained automatically from `Metrics::get()`.
    pub fn new(sink: S, interval: Duration, running: &'static AtomicBool) -> Self {
        Self {
            sink,
            interval,
            running,
            metrics: Metrics::get(),
        }
    }

    /// Starts the background reporting thread.
    ///
    /// The thread is spawned and immediately returns; no handle is provided
    /// to join or cancel the thread (cancellation is only possible via the
    /// `running` flag).
    ///
    /// # Behavior
    /// - The thread loops while `self.running.load(Ordering::Relaxed)` is `true`.
    /// - On each iteration, it sleeps for `self.interval`, then calls
    ///   `self.sink.report(self.metrics)`.
    /// - If the sink's `report` panics, the thread will terminate.
    ///
    /// # Examples
    /// ```
    /// use std::sync::atomic::AtomicBool;
    /// use std::time::Duration;
    /// use telemetry::{StdoutSink, PeriodicSink};
    ///
    /// static RUNNING: AtomicBool = AtomicBool::new(true);
    ///
    /// let sink = StdoutSink;
    /// let periodic = PeriodicSink::new(sink, Duration::from_secs(5), &RUNNING);
    /// periodic.start(); // background thread now prints metrics every 5 seconds.
    /// ```
    pub fn start(self) {
        thread::spawn(move || {
            // Loop until the running flag is cleared.
            // Using Relaxed ordering is sufficient because we only need to
            // eventually see the update; there is no critical section protected
            // by this flag.
            while self.running.load(Ordering::Relaxed) {
                // Sleep for the configured interval. If the sleep is interrupted,
                // it will return early, but the loop will just continue.
                thread::sleep(self.interval);

                // Invoke the sink to report the current metrics.
                // Any panic here will abort this thread.
                self.sink.report(self.metrics);
            }
        });
        // Note: The thread is detached; no join handle is stored.
        // The thread will continue until the flag becomes false.
    }
}
