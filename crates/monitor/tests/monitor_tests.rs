//! Integration tests for the X11 clipboard backend.
//!
//! This module tests the [`X11Connection`] implementation against a real (or mock) X server.
//! Tests are designed to be robust: they gracefully skip when no X server is available,
//! and they isolate environment variable changes using a custom guard pattern to prevent
//! interference between tests.
//!
//! # Environment Handling
//! - The `DISPLAY` environment variable is captured once at module load into `REAL_DISPLAY`.
//! - Each test that modifies `DISPLAY` uses [`DisplayGuard`] to restore the original value
//!   automatically on drop.
//! - A global mutex (`DISPLAY_LOCK`) serializes all tests that modify `DISPLAY`, preventing
//!   race conditions.
//!
//! # Note on Unsafety
//! The code uses `unsafe` blocks to call `env::set_var` and `env::remove_var` because these
//! functions are unsafe when used in a multi-threaded context. The global mutex ensures
//! that only one test at a time modifies environment variables, making the unsafe blocks
//! safe in practice.

use monitor::backend::x11::{X11Connection, X11Error};
use std::env;
use std::sync::{Mutex, OnceLock};

/// Caches the original `DISPLAY` environment variable value at module load.
///
/// This ensures that all tests can refer to the original display, even if other tests
/// temporarily override it.
static REAL_DISPLAY: OnceLock<Option<String>> = OnceLock::new();

/// Returns the original `DISPLAY` value (as captured when the module was first accessed).
fn real_display() -> Option<String> {
    REAL_DISPLAY
        .get_or_init(|| env::var("DISPLAY").ok())
        .clone()
}

/// Global mutex to serialize tests that modify the `DISPLAY` environment variable.
///
/// This prevents concurrent modifications to the environment, which would be unsafe
/// (as `set_var` and `remove_var` are not thread-safe).
static DISPLAY_LOCK: Mutex<()> = Mutex::new(());

/// RAII guard that temporarily overrides the `DISPLAY` environment variable.
///
/// When created, it acquires a global lock and saves the current `DISPLAY` value.
/// It then allows the caller to modify `DISPLAY` arbitrarily. When the guard is dropped,
/// the original `DISPLAY` value is restored.
///
/// # Example
/// ```rust
/// let _guard = DisplayGuard::acquire();
/// unsafe { env::set_var("DISPLAY", ":1"); }
/// // ... test code ...
/// // On drop, DISPLAY reverts to its original value.
/// ```
struct DisplayGuard {
    /// Lock guard held for the duration of the test.
    _lock: std::sync::MutexGuard<'static, ()>,
    /// Saved original `DISPLAY` value to restore on drop.
    saved: Option<String>,
}

impl DisplayGuard {
    /// Acquires a new guard, saving the current `DISPLAY` and allowing temporary changes.
    ///
    /// This method also ensures that `REAL_DISPLAY` is initialized (so that later calls
    /// to `real_display()` return the original value).
    fn acquire() -> Self {
        // Initialize REAL_DISPLAY if not already done.
        let _ = real_display();

        // Acquire the global lock to serialize environment modifications.
        let lock = DISPLAY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = env::var("DISPLAY").ok();
        DisplayGuard { _lock: lock, saved }
    }
}

impl Drop for DisplayGuard {
    /// Restores the original `DISPLAY` value (or removes it if it was not set).
    fn drop(&mut self) {
        unsafe {
            match &self.saved {
                Some(v) => env::set_var("DISPLAY", v),
                None => env::remove_var("DISPLAY"),
            }
        }
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

/// Tests that [`X11Connection::connect`] can establish a connection to the X server.
///
/// This test is non‑fatal: if no X server is running, it simply prints a message and
/// passes. It ensures that the connection does not panic and that `close` works.
#[test]
fn test_x11_connect() {
    let _guard = DisplayGuard::acquire();
    match X11Connection::connect() {
        Ok(conn) => {
            let _ = conn.close();
        }
        Err(e) => {
            println!(
                "X11 connection failed (expected if X is not running): {:?}",
                e
            );
        }
    }
}

/// Tests that [`X11Connection::connect`] works with a valid explicit `DISPLAY` value.
///
/// It sets `DISPLAY` to the original value (or `:0` if none) and attempts to connect.
/// Parsing errors (e.g., `NoDisplay`, `InvalidDisplay`) are considered test failures.
#[test]
fn test_x11_connect_valid_display() {
    let _guard = DisplayGuard::acquire();
    let display = real_display().unwrap_or_else(|| ":0".to_string());
    unsafe {
        env::set_var("DISPLAY", &display);
    }

    match X11Connection::connect() {
        Ok(conn) => {
            let _ = conn.close();
        }
        Err(e) => {
            println!(
                "X11 connection failed (expected if X is not running): {:?}",
                e
            );
            match e {
                X11Error::NoDisplay | X11Error::InvalidDisplay(_) => {
                    panic!("Unexpected parsing error: {:?}", e);
                }
                _ => {}
            }
        }
    }
}

/// Tests that [`X11Connection::connect`] returns `NoDisplay` when `DISPLAY` is not set.
#[test]
fn test_x11_connect_missing_display() {
    let _guard = DisplayGuard::acquire();
    unsafe {
        env::remove_var("DISPLAY");
    }
    let err = X11Connection::connect().unwrap_err();
    assert!(matches!(err, X11Error::NoDisplay));
}

/// Tests that [`X11Connection::connect`] returns `InvalidDisplay` for malformed values.
///
/// Cases tested:
/// - `"garbage"` (no colon)
/// - `":abc"` (non‑numeric display number)
/// - `"0"` (missing colon)
#[test]
fn test_x11_connect_malformed_display() {
    let _guard = DisplayGuard::acquire();

    unsafe {
        env::set_var("DISPLAY", "garbage");
    }
    let err = X11Connection::connect().unwrap_err();
    assert!(matches!(err, X11Error::InvalidDisplay(_)));

    unsafe {
        env::set_var("DISPLAY", ":abc");
    }
    let err = X11Connection::connect().unwrap_err();
    assert!(matches!(err, X11Error::InvalidDisplay(_)));

    unsafe {
        env::set_var("DISPLAY", "0");
    }
    let err = X11Connection::connect().unwrap_err();
    assert!(matches!(err, X11Error::InvalidDisplay(_)));
}

/// Tests that [`X11Connection::connect`] rejects TCP host specifications.
///
/// The X11 backend only supports local UNIX sockets. Any host part other than
/// empty or `"unix"` should result in `TcpNotSupported`.
#[test]
fn test_x11_connect_tcp_not_supported() {
    let _guard = DisplayGuard::acquire();

    unsafe {
        env::set_var("DISPLAY", "localhost:0");
    }
    let err = X11Connection::connect().unwrap_err();
    assert!(matches!(err, X11Error::TcpNotSupported));

    unsafe {
        env::set_var("DISPLAY", "myhost:1");
    }
    let err = X11Connection::connect().unwrap_err();
    assert!(matches!(err, X11Error::TcpNotSupported));
}

/// Tests retrieving the current clipboard contents.
///
/// This test is opportunistic: it only runs if a real X server is available.
/// It does not assume that any clipboard owner exists; a failure to retrieve
/// data (e.g., `SelectionConversionFailed`) is acceptable and logged.
#[test]
fn test_x11_get_clipboard() {
    let _guard = DisplayGuard::acquire();

    let display = match real_display() {
        Some(d) => d,
        None => {
            println!("Skipping clipboard test: DISPLAY not set");
            return;
        }
    };
    unsafe {
        env::set_var("DISPLAY", &display);
    }

    let mut conn = match X11Connection::connect() {
        Ok(c) => c,
        Err(e) => {
            println!(
                "Skipping clipboard test: cannot connect to X server: {:?}",
                e
            );
            return;
        }
    };

    match conn.get_clipboard() {
        Ok(data) => println!("Got clipboard data ({} bytes)", data.len()),
        Err(e) => println!(
            "get_clipboard returned error (acceptable if no owner): {:?}",
            e
        ),
    }
}

/// Tests that the clipboard event subscription can be set up.
///
/// It attempts to call `select_clipboard_events` and expects either success
/// or a `XFixesNotAvailable` error (if the X server lacks the XFIXES extension).
#[test]
fn test_x11_monitor_starts() {
    let _guard = DisplayGuard::acquire();

    let display = match real_display() {
        Some(d) => d,
        None => {
            println!("Skipping monitor setup test: DISPLAY not set");
            return;
        }
    };
    unsafe {
        env::set_var("DISPLAY", &display);
    }

    let mut conn = match X11Connection::connect() {
        Ok(c) => c,
        Err(e) => {
            println!("Skipping monitor setup test: cannot connect: {:?}", e);
            return;
        }
    };

    let result = conn.select_clipboard_events();
    assert!(
        result.is_ok() || matches!(result, Err(X11Error::XFixesNotAvailable)),
        "select_clipboard_events failed unexpectedly: {:?}",
        result
    );
}

/// Tests that reading from a closed Unix stream returns EOF correctly.
///
/// This is a low‑level utility test to verify the behavior of `read` on a closed
/// connection, which is relevant for the socket I/O in the X11 backend.
#[test]
fn test_next_event_connection_closed() {
    use std::io::Read;
    use std::os::unix::net::UnixStream;

    let (server, mut client) = UnixStream::pair().expect("socketpair failed");
    drop(server);

    let mut buf = [0u8; 4];
    match client.read(&mut buf) {
        Ok(0) => {} // Expected EOF
        Ok(n) => panic!("expected EOF, got {} bytes", n),
        Err(_) => {} // Some platforms may return an error instead of EOF
    }
}

/// Tests that dropping an `X11Connection` does not panic and properly closes the socket.
#[test]
fn test_x11_connection_drop() {
    let _guard = DisplayGuard::acquire();

    let display = real_display().unwrap_or_else(|| ":0".to_string());
    unsafe {
        env::set_var("DISPLAY", &display);
    }

    if let Ok(conn) = X11Connection::connect() {
        drop(conn);
    } else {
        println!("Skipping drop test: cannot connect to X server");
    }
}

/// Tests that the `fetch_failed_count` metric increments when clipboard retrieval fails.
///
/// This test requires an active X server and assumes the [`telemetry::Metrics`] singleton
/// is available. It checks that a failed `get_clipboard` call increases the failure counter,
/// while a successful call does not.
#[test]
fn test_clipboard_fetch_failure_increments_metric() {
    use std::sync::atomic::Ordering;
    use telemetry::Metrics;

    let _guard = DisplayGuard::acquire();
    let display = real_display().unwrap_or_else(|| ":0".to_string());
    unsafe {
        env::set_var("DISPLAY", &display);
    }

    let mut conn = match X11Connection::connect() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Cannot connect to X server: {:?}", e);
            return;
        }
    };

    let metrics = Metrics::get();
    let before = metrics.fetch_failed_count.load(Ordering::Relaxed);

    let result = conn.get_clipboard();

    if result.is_err() {
        let after = metrics.fetch_failed_count.load(Ordering::Relaxed);
        assert_eq!(
            after,
            before + 1,
            "failed_fetches should increment on fetch failure"
        );
    } else {
        eprintln!("Clipboard fetch succeeded (there is a clipboard owner) – skipping metric check");
        let after = metrics.fetch_failed_count.load(Ordering::Relaxed);
        assert_eq!(
            after, before,
            "failed_fetches should not increment on success"
        );
    }
}
