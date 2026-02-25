//! # Metrics Collection for Application Monitoring
//!
//! This module provides a thread-safe, globally accessible metrics collector
//! for tracking key operational events in a high-concurrency environment.
//! Metrics are stored as atomic counters to allow lock-free updates from
//! multiple threads.
//!
//! ## Design Notes
//! - Uses `OnceLock` for lazy, one-time initialization of the global singleton.
//! - All counters are `AtomicU64` with `Relaxed` ordering – suitable for
//!   counters where eventual consistency is acceptable and no strict
//!   synchronization with other memory operations is required.
//! - Increment operations are non-blocking and wait-free.
//!
//! ## Potential Pitfalls
//! - `Relaxed` ordering does not provide a happens-before relationship with
//!   other memory accesses. If these counters are used to drive decisions
//!   that require strong synchronization (e.g., fences), consider upgrading
//!   to `Release`/`Acquire` or `SeqCst`.
//! - The counters can wrap around after 2^64-1 increments, but this is
//!   effectively impossible in practice for these event types.

use std::sync::{OnceLock, atomic::AtomicU64};

/// Global metrics singleton, initialized lazily on first access.
static METRICS: OnceLock<Metrics> = OnceLock::new();

/// Container for all application metrics.
///
/// Each metric is an atomic counter that can be safely incremented from
/// multiple threads without locking. The counters are publicly accessible
/// for monitoring and reporting purposes.
#[derive(Debug)]
pub struct Metrics {
    /// Total number of clipboard events processed.
    pub clipboard_event_count: AtomicU64,

    /// Number of connection retry attempts made after transient failures.
    pub connection_retries_count: AtomicU64,

    /// Count of system call interruptions due to `EINTR` (interrupted system call).
    /// This helps detect excessive signal interruptions or the need for restart
    /// logic around blocking syscalls.
    pub eintr_count: AtomicU64,

    /// Number of fetch operations that failed (e.g., network timeouts, server errors).
    pub fetch_failed_count: AtomicU64,
}

impl Metrics {
    /// Creates a new `Metrics` instance with all counters initialized to zero.
    ///
    /// This is private because the global singleton should be obtained via
    /// `Metrics::get()`.
    fn new() -> Self {
        Self {
            clipboard_event_count: AtomicU64::new(0),
            connection_retries_count: AtomicU64::new(0),
            eintr_count: AtomicU64::new(0),
            fetch_failed_count: AtomicU64::new(0),
        }
    }

    /// Returns a reference to the global `Metrics` singleton.
    ///
    /// The first call initializes the metrics; subsequent calls return the
    /// same instance. This function is thread-safe and can be called from
    /// any context.
    ///
    /// # Examples
    /// ```
    /// use telemetry::Metrics;
    ///
    /// let metrics = Metrics::get();
    /// metrics.inc_clipboard_event_count();
    /// ```
    pub fn get() -> &'static Metrics {
        // `OnceLock::get_or_init` ensures the closure runs exactly once,
        // even if multiple threads call this simultaneously.
        METRICS.get_or_init(|| Metrics::new())
    }

    /// Increments the `clipboard_event_count` by one.
    ///
    /// Uses `Ordering::Relaxed` because we only need atomicity, not
    /// synchronization with other memory operations. This is the fastest
    /// option and safe for monotonic counters.
    pub fn inc_clipboard_event_count(&self) {
        self.clipboard_event_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Increments the `connection_retries_count` by one.
    ///
    /// Same relaxed ordering rationale as above.
    pub fn inc_connection_retries_count(&self) {
        self.connection_retries_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Increments the `eintr_count` by one.
    ///
    /// Tracks occurrences of `EINTR` errors in system calls, which may
    /// indicate the need for restart loops or signal handling adjustments.
    pub fn inc_eintr_count(&self) {
        self.eintr_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Increments the `fetch_failed_count` by one.
    ///
    /// Counts failures in fetch operations (e.g., network errors, HTTP 5xx).
    /// This can be used for alerting on high error rates.
    pub fn inc_fetch_failed_count(&self) {
        self.fetch_failed_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}
