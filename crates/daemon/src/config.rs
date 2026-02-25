//! Configuration management for a daemon application.
//!
//! This module defines the logging level and destination enums, and the main
//! daemon configuration struct which is populated from environment variables.
//! It provides parsing and validation logic for reconnection delays, backoff
//! multiplier, log settings, and metrics collection intervals.
//!
//! # Environment variables
//!
//! - `CLIPPER_RECONNECT_DELAY` – initial reconnect delay in seconds (default: 2)
//! - `CLIPPER_RECONNECT_MAX_DELAY` – maximum reconnect delay in seconds (default: 30)
//! - `CLIPPER_RECONNECT_BACKOFF_MULTIPLIER` – exponential backoff factor (default: 2.0)
//! - `CLIPPER_DAEMONIZE` – if "1", "true", or "yes" (case‑insensitive) enables daemon mode
//! - `CLIPPER_LOG_FILE` – optional path to log file (if not set, logs go to configured destination)
//! - `CLIPPER_METRICS_INTERVAL_SECONDS` – seconds between metrics emissions (default: 60)
//! - `CLIPPER_LOG_LEVEL` – one of "error", "warn"/"warning", "info" (default: info)
//! - `CLIPPER_LOG_DEST` – one of "stdout" or "stderr" (default: stdout)
//!
//! Invalid numeric values cause an immediate exit with an error message.
//! Invalid log level/destination fall back to defaults after printing a warning.

use std::str::FromStr;
use std::time::Duration;

/// Log verbosity level.
///
/// Ordered from least to most verbose (Info > Warn > Error). Derives common
/// traits for comparison, cloning, and debugging.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    /// Informational messages (normal operation).
    Info,
    /// Warning messages (potential issues).
    Warn,
    /// Error messages (critical failures).
    Error,
}

impl Default for LogLevel {
    /// Returns the default log level, which is [`LogLevel::Info`].
    fn default() -> Self {
        LogLevel::Info
    }
}

impl FromStr for LogLevel {
    type Err = &'static str;

    /// Parses a string into a [`LogLevel`].
    ///
    /// # Arguments
    /// * `s` – The string to parse (case‑insensitive).
    ///
    /// # Returns
    /// * `Ok(LogLevel)` – on successful parse.
    /// * `Err(&'static str)` – if the string does not match any known level.
    ///
    /// # Examples
    /// ```
    /// use std::str::FromStr;
    /// use daemon::LogLevel;
    ///
    /// assert_eq!(LogLevel::from_str("warn"), Ok(LogLevel::Warn));
    /// assert_eq!(LogLevel::from_str("WARNING"), Ok(LogLevel::Warn));
    /// assert_eq!(LogLevel::from_str("debug"), Err("invalid log level, expected error/warn/info"));
    /// ```
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "error" => Ok(LogLevel::Error),
            "warn" | "warning" => Ok(LogLevel::Warn),
            "info" => Ok(LogLevel::Info),
            _ => Err("invalid log level, expected error/warn/info"),
        }
    }
}

/// Log output destination.
///
/// Determines whether log messages are written to standard output or standard error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogDest {
    /// Write logs to stdout.
    Stdout,
    /// Write logs to stderr.
    Stderr,
}

impl Default for LogDest {
    /// Returns the default destination, [`LogDest::Stdout`].
    fn default() -> Self {
        LogDest::Stdout
    }
}

impl FromStr for LogDest {
    type Err = &'static str;

    /// Parses a string into a [`LogDest`].
    ///
    /// # Arguments
    /// * `s` – The string to parse (case‑insensitive).
    ///
    /// # Returns
    /// * `Ok(LogDest)` – on successful parse.
    /// * `Err(&'static str)` – if the string is neither "stdout" nor "stderr".
    ///
    /// # Examples
    /// ```
    /// use std::str::FromStr;
    /// use daemon::LogDest;
    ///
    /// assert_eq!(LogDest::from_str("stdout"), Ok(LogDest::Stdout));
    /// assert_eq!(LogDest::from_str("STDERR"), Ok(LogDest::Stderr));
    /// assert_eq!(LogDest::from_str("file"), Err("invalid log destination, expected stdout/stderr"));
    /// ```
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "stdout" => Ok(LogDest::Stdout),
            "stderr" => Ok(LogDest::Stderr),
            _ => Err("invalid log destination, expected stdout/stderr"),
        }
    }
}

/// Complete daemon configuration, typically loaded from environment variables.
///
/// All fields are public to allow direct inspection or overriding after creation.
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    /// Initial delay before the first reconnection attempt.
    pub reconnect_delay: Duration,
    /// Maximum allowed delay between reconnection attempts (exponential backoff will not exceed this).
    pub reconnect_max_delay: Duration,
    /// Multiplier applied to the current delay after each failed attempt.
    /// Must be >= 1.0; if a lower value is provided, it is clamped to 1.0 and a warning is emitted.
    pub reconnect_backoff_multiplier: f64,
    /// Logging verbosity level.
    pub log_level: LogLevel,
    /// Where to write log messages (stdout or stderr). Ignored if `log_file` is set.
    pub log_dest: LogDest,
    /// Whether the application should daemonize (fork into background).
    pub daemonize: bool,
    /// Optional path to a log file. If `Some`, `log_dest` is ignored and logs are written to this file.
    pub log_file: Option<String>,
    /// Interval at which metrics are collected and reported.
    pub metrics_interval: Duration,
}

impl DaemonConfig {
    /// Constructs a [`DaemonConfig`] by reading and validating environment variables.
    ///
    /// # Panics / Exit behaviour
    /// If a variable that expects a numeric value contains an unparsable string,
    /// the function prints an error message to stderr and calls `std::process::exit(1)`.
    /// This is intentional for a daemon’s startup phase – invalid configuration
    /// is considered fatal and the process must not continue.
    ///
    /// # Warnings
    /// - If `CLIPPER_RECONNECT_BACKOFF_MULTIPLIER` is less than 1.0, it is set to 1.0
    ///   and a warning is printed.
    /// - If `CLIPPER_LOG_LEVEL` contains an invalid value, the default (`Info`) is used
    ///   and a warning is printed.
    /// - If `CLIPPER_LOG_DEST` contains an invalid value, the default (`Stdout`) is used
    ///   and a warning is printed.
    ///
    /// # Returns
    /// A fully populated `DaemonConfig` with values derived from the environment
    /// or built‑in defaults.
    ///
    /// # Note
    /// This function is intended to be called once at application startup.
    /// It reads environment variables directly; no caching is performed.
    pub fn from_env() -> Self {
        /// Helper: parse an environment variable as `u64`. Exits on parse error.
        fn parse_env_u64(key: &str, default: u64) -> u64 {
            match std::env::var(key) {
                Ok(s) => match s.parse::<u64>() {
                    Ok(v) => v,
                    Err(_) => {
                        // Invalid number → fatal error (configuration is unusable)
                        eprintln!("ERROR: {}='{}' is not a valid number, aborting", key, s);
                        std::process::exit(1);
                    }
                },
                Err(_) => default,
            }
        }

        /// Helper: parse an environment variable as `f64`. Exits on parse error.
        fn parse_env_f64(key: &str, default: f64) -> f64 {
            match std::env::var(key) {
                Ok(s) => match s.parse::<f64>() {
                    Ok(v) => v,
                    Err(_) => {
                        eprintln!("ERROR: {}='{}' is not a valid number, aborting", key, s);
                        std::process::exit(1);
                    }
                },
                Err(_) => default,
            }
        }

        /// Helper: determine if an environment variable is set to a truthy value.
        /// Returns `true` if the variable exists and its lowercased value is
        /// exactly `"1"`, `"true"`, or `"yes"`. Otherwise returns `false`.
        fn env_var_is_true(key: &str) -> bool {
            std::env::var(key)
                .ok()
                .map(|s| {
                    let s = s.to_lowercase();
                    s == "1" || s == "true" || s == "yes"
                })
                .unwrap_or(false)
        }

        // --- Parse core numeric values with defaults ---
        let reconnect_secs = parse_env_u64("CLIPPER_RECONNECT_DELAY", 2);
        let reconnect_max_secs = parse_env_u64("CLIPPER_RECONNECT_MAX_DELAY", 30);
        let backoff_multiplier = parse_env_f64("CLIPPER_RECONNECT_BACKOFF_MULTIPLIER", 2.0);
        let daemonize = env_var_is_true("CLIPPER_DAEMONIZE");
        let log_file = std::env::var("CLIPPER_LOG_FILE").ok();

        let metrics_interval_secs = parse_env_u64("CLIPPER_METRICS_INTERVAL_SECONDS", 60);
        let metrics_interval = Duration::from_secs(metrics_interval_secs);

        // --- Build Durations from parsed seconds ---
        let reconnect_delay = Duration::from_secs(reconnect_secs);
        // Ensure max delay is at least the initial delay (defensive programming)
        let reconnect_max_delay = Duration::from_secs(reconnect_max_secs.max(reconnect_secs));
        // Clamp backoff multiplier to >= 1.0; warn if it was below.
        let reconnect_backoff_multiplier = if backoff_multiplier < 1.0 {
            eprintln!(
                "WARN: CLIPPER_RECONNECT_BACKOFF_MULTIPLIER={} is <1, using 1.0",
                backoff_multiplier
            );
            1.0
        } else {
            backoff_multiplier
        };

        // --- Parse optional log level, fall back to default on error ---
        let log_level = match std::env::var("CLIPPER_LOG_LEVEL") {
            Ok(s) => s.parse().unwrap_or_else(|e| {
                eprintln!(
                    "ERROR: CLIPPER_LOG_LEVEL='{}' invalid: {}, using default Info",
                    s, e
                );
                LogLevel::default()
            }),
            Err(_) => LogLevel::default(),
        };

        // --- Parse optional log destination, fall back to default on error ---
        let log_dest = match std::env::var("CLIPPER_LOG_DEST") {
            Ok(s) => s.parse().unwrap_or_else(|e| {
                eprintln!(
                    "ERROR: CLIPPER_LOG_DEST='{}' invalid: {}, using default Stdout",
                    s, e
                );
                LogDest::default()
            }),
            Err(_) => LogDest::default(),
        };

        // Assemble the final configuration
        Self {
            reconnect_delay,
            reconnect_max_delay,
            reconnect_backoff_multiplier,
            log_level,
            log_dest,
            daemonize,
            log_file,
            metrics_interval,
        }
    }
}
