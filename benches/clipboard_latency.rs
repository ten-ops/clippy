//! Benchmark for clipboard fetch performance using the X11 backend.
//!
//! This module uses the Criterion benchmarking framework to measure the time
//! taken to retrieve the current clipboard contents via the X11 clipboard
//! monitor backend. It connects to the X server, performs a few warm‑up
//! fetches, and then benchmarks the `get_clipboard()` method.
//!
//! # Prerequisites
//! - A running X server (the `DISPLAY` environment variable must be set).
//! - The X11 connection must be successful; otherwise the benchmark is skipped.
//!
//! # Note
//! The `ClipboardOwner` struct is present but unused in the benchmark. It
//! appears to be a leftover from a different test or example; it is kept
//! as dead code (allowed) to avoid removal warnings. It demonstrates how
//! one might set up a persistent clipboard owner, but is not invoked.

use criterion::{Criterion, criterion_group, criterion_main};
use monitor::backend::x11::X11Connection;
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

/// A struct that would represent an application owning the clipboard.
///
/// This is a stub intended to show how a clipboard owner could be implemented.
/// It is not used in the actual benchmark, but is kept for reference or future
/// expansion. The `#[allow(dead_code)]` attribute suppresses warnings about
/// unused fields and methods.
#[allow(dead_code)]
struct ClipboardOwner {
    /// The X11 connection.
    conn: X11Connection,
    /// Flag indicating whether the owner should keep running.
    running: AtomicBool,
}

impl ClipboardOwner {
    /// Attempts to create a new clipboard owner.
    ///
    /// # Returns
    /// `Some(ClipboardOwner)` if the connection succeeds and required atoms
    /// can be interned; otherwise `None`.
    fn _new() -> Option<Self> {
        let mut conn = X11Connection::connect().ok()?;

        // Intern the atoms we would need to claim ownership.
        let _clipboard = conn.intern_atom("CLIPBOARD", false).ok()?;
        let _utf8_string = conn.intern_atom("UTF8_STRING", false).ok()?;

        // The owner's window (provided by the connection) would be used.
        let _owner = conn.our_window;

        Some(ClipboardOwner {
            conn,
            running: AtomicBool::new(true),
        })
    }

    /// Runs the owner loop (does nothing but sleep).
    fn _run(&self) {
        while self.running.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for ClipboardOwner {
    fn drop(&mut self) {
        // Signal the loop to exit.
        self.running.store(false, Ordering::Relaxed);
    }
}

/// Benchmarks the time to fetch the clipboard contents.
///
/// This function is the main benchmark entry point. It:
/// 1. Checks for a valid `DISPLAY` environment variable.
/// 2. Attempts to connect to the X server; skips if connection fails.
/// 3. Performs three warm‑up clipboard fetches to ensure any one‑time
///    initialisation (e.g., atom caching) does not skew the results.
/// 4. Runs the Criterion benchmark, calling `get_clipboard()` repeatedly
///    and passing the result to `black_box` to prevent optimisation.
///
/// The benchmark is named "clipboard_fetch".
fn bench_clipboard_fetch(c: &mut Criterion) {
    // Skip benchmark if no DISPLAY is set (common in headless CI environments).
    if std::env::var("DISPLAY").is_err() {
        eprintln!("DISPLAY not set – skipping benchmark");
        return;
    }

    // Establish a connection that will be used for the benchmark.
    let mut bench_conn = match X11Connection::connect() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to connect to X server: {:?}", e);
            return;
        }
    };

    // Warm‑up: perform a few fetches to stabilise the measurement.
    // This helps ensure that any lazy initialisation is completed before
    // the benchmark begins.
    for _ in 0..3 {
        let _ = bench_conn.get_clipboard();
    }

    // Define the benchmark.
    c.bench_function("clipboard_fetch", |b| {
        b.iter(|| {
            // Fetch the clipboard contents. If the fetch fails, the benchmark
            // will panic – this is acceptable because a failure indicates a
            // problem with the environment or the implementation.
            let data = bench_conn.get_clipboard().expect("clipboard fetch failed");
            // Use black_box to prevent the compiler from optimising away the
            // result or the function call.
            black_box(data);
        })
    });
}

// Boilerplate for Criterion: define a group and run it.
criterion_group!(benches, bench_clipboard_fetch);
criterion_main!(benches);
