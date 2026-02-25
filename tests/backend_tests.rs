//! End-to-end test for the clipboard monitor using the X11 backend.
//!
//! This test verifies that the clipboard monitor correctly detects changes
//! to the X11 clipboard. It uses the `xclip` command-line tool to set the
//! clipboard contents and checks that the monitor's callback is invoked.
//!
//! # Dependencies
//! - A running X server (with a valid `DISPLAY` environment variable).
//! - The `xclip` utility must be installed and accessible in `$PATH`.
//!
//! # Test flow
//! 1. Connect to the X server using `X11Connection`.
//! 2. Record initial metrics (clipboard events and fetch failures).
//! 3. Spawn a background thread that runs the clipboard monitor, incrementing
//!    a counter on each event.
//! 4. Synchronise with the background thread using a barrier to ensure the
//!    monitor is ready.
//! 5. Set the clipboard twice using `xclip`.
//! 6. Verify that the event counter increased appropriately and that metrics
//!    were updated correctly.

use monitor::backend::x11::X11Connection;
use std::process::Command;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;
use telemetry::Metrics;

#[test]
fn test_clipboard_monitor_end_to_end() {
    // Check for the presence of `xclip`; if not found, skip the test.
    // This prevents false failures in environments where the tool is missing.
    if Command::new("xclip").arg("-version").output().is_err() {
        eprintln!("xclip not found – skipping test");
        return;
    }

    // Establish a connection to the X server. This assumes a valid DISPLAY
    // is set. If the connection fails, the test will panic – that's acceptable
    // because we require an X server for meaningful results.
    let mut conn = X11Connection::connect().expect("failed to connect to X server");

    let metrics = Metrics::get();
    let initial_events = metrics.clipboard_event_count.load(Ordering::SeqCst);
    let initial_failures = metrics.fetch_failed_count.load(Ordering::SeqCst);

    // Barrier to synchronise the main thread and the monitor thread.
    // We want the monitor to be running before we start setting the clipboard.
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let barrier_clone = barrier.clone();

    // Atomic counter for clipboard events received by the monitor callback.
    let events_received = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let events_received_clone = events_received.clone();

    // Spawn the background monitor thread.
    let _handle = thread::spawn(move || {
        // Wait for the main thread to signal that it's ready.
        barrier_clone.wait();

        // Run the clipboard monitor. The callback increments the event counter
        // each time a clipboard change is detected.
        conn.run_clipboard_monitor(move |_| {
            events_received_clone.fetch_add(1, Ordering::SeqCst);
        })
    });

    // Wait for the monitor thread to reach the barrier.
    barrier.wait();

    // Give the monitor a moment to fully initialise (e.g., set up event handlers).
    thread::sleep(Duration::from_millis(500));

    // Helper function to set the clipboard contents using `xclip`.
    // It writes the given text to the clipboard via stdin.
    let set_clipboard = |text: &str| {
        let mut child = Command::new("xclip")
            .arg("-selection")
            .arg("clipboard")
            .arg("-i")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .expect("failed to spawn xclip");
        use std::io::Write;
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
        child.wait().expect("xclip failed");
    };

    // First clipboard write - should trigger an event.
    set_clipboard("hello from test");
    thread::sleep(Duration::from_millis(500)); // Allow time for the event to be processed.
    let count1 = events_received.load(Ordering::SeqCst);
    assert!(
        count1 > 0,
        "No event received after first copy (got {})",
        count1
    );

    // Second clipboard write – should trigger another event.
    set_clipboard("world");
    thread::sleep(Duration::from_millis(500));
    let count2 = events_received.load(Ordering::SeqCst);
    assert!(count2 >= 2, "Second event not received (got {})", count2);

    // Verify that the global metrics were updated correctly.
    let final_events = metrics.clipboard_event_count.load(Ordering::SeqCst);
    let final_failures = metrics.fetch_failed_count.load(Ordering::SeqCst);

    assert!(final_events >= initial_events + 2, "events not incremented");
    assert_eq!(
        final_failures, initial_failures,
        "failures should not have increased"
    );

    // Note: The monitor thread is detached; it will be killed when the test
    // process exits. In a long‑running daemon we would need proper shutdown,
    // but for a test this is acceptable.
}
