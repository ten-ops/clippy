//! Unit tests for the daemon configuration parsing logic.
//!
//! This module tests that [`DaemonConfig::from_env`] correctly reads environment
//! variables and falls back to defaults when variables are missing. It also
//! verifies that the parsed values are converted into the appropriate Rust types
//! (`Duration`, `f64`, `bool`, enums).
//!
//! # Important
//! - These tests modify environment variables using `std::env::set_var` and
//!   `std::env::remove_var`. Modifying environment variables is **unsafe** in
//!   a multithreaded context because environment changes are global and can
//!   affect other threads. However, Rust's test harness runs each test in a
//!   single thread by default, and these tests are not run concurrently with
//!   others that rely on specific environment values. The `unsafe` blocks are
//!   used to acknowledge that we are deliberately bypassing Rust's safe
//!   guarantees for environment manipulation.
//! - Cases where `DaemonConfig::from_env` would call `std::process::exit`
//!   (e.g., invalid numeric values) **cannot** be tested in a unit test
//!   because they would terminate the test runner. Those scenarios are covered
//!   by integration tests (see `tests/daemon.rs`).

use daemon::DaemonConfig;
use std::time::Duration;

#[test]
fn test_display_parsing() {
    // --- Test successful parsing of all environment variables ---
    // Set a complete set of environment variables to their expected values.
    unsafe {
        std::env::set_var("DISPLAY", ":0");
        std::env::set_var("CLIPPER_RECONNECT_DELAY", "5");
        std::env::set_var("CLIPPER_RECONNECT_MAX_DELAY", "60");
        std::env::set_var("CLIPPER_RECONNECT_BACKOFF_MULTIPLIER", "2.5");
        std::env::set_var("CLIPPER_METRICS_INTERVAL_SECONDS", "30");
        std::env::set_var("CLIPPER_LOG_LEVEL", "warn");
        std::env::set_var("CLIPPER_LOG_DEST", "stderr");
        std::env::set_var("CLIPPER_DAEMONIZE", "1");
        std::env::set_var("CLIPPER_LOG_FILE", "/tmp/clipper.log");
    }

    let config = DaemonConfig::from_env();

    // Verify that each field matches the expected value.
    assert_eq!(config.reconnect_delay, Duration::from_secs(5));
    assert_eq!(config.reconnect_max_delay, Duration::from_secs(60));
    assert_eq!(config.reconnect_backoff_multiplier, 2.5);
    assert_eq!(config.metrics_interval, Duration::from_secs(30));
    assert!(matches!(config.log_level, daemon::LogLevel::Warn));
    assert!(matches!(config.log_dest, daemon::LogDest::Stderr));
    assert!(config.daemonize);
    assert_eq!(config.log_file, Some("/tmp/clipper.log".to_string()));

    // --- Test fallback to defaults when variables are missing ---
    // Remove one variable and verify that the default is used.
    unsafe {
        std::env::remove_var("CLIPPER_RECONNECT_DELAY");
    }
    let config = DaemonConfig::from_env();
    assert_eq!(config.reconnect_delay, Duration::from_secs(2)); // default value

    // --- Note on invalid values ---
    // We do not test invalid numeric values here because `DaemonConfig::from_env`
    // calls `std::process::exit` on parse errors, which would abort the test runner.
    // Those cases are covered in integration tests (e.g., `test_daemon_invalid_reconnect_delay`).
}
