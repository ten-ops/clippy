//! Main daemon entry point and core runtime logic.
//!
//! This module orchestrates the entire clipboard monitoring daemon:
//! - Parses configuration from environment variables (via [`DaemonConfig`]).
//! - Optionally daemonizes (forks, detaches, redirects stdio).
//! - Installs signal handlers for graceful shutdown.
//! - Starts a metrics reporter thread and a systemd watchdog thread (if configured).
//! - Enters the main event loop, connecting to the X server and processing clipboard events.
//! - Handles reconnection with exponential backoff on errors.
//! - Supports systemd notify protocol for readiness and watchdog keep-alive.
//!
//! The daemon is designed to be robust, logging all significant events and
//! exiting only on fatal errors or explicit signals.

use monitor::ClipboardBackend;
use monitor::backend::x11::X11Connection;
use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use syscalls::{SockAddrUn, SyscallError, close, connect, open, socket, write};
use telemetry::sink::PeriodicSink;

mod config;
mod daemonize;
mod signals;

pub use config::{DaemonConfig, LogDest, LogLevel};
pub use daemonize::daemonize;
pub use signals::{RUNNING, install_handlers};

// The following syscall constants are kept for reference but not used directly
// in this module (they are used by the daemonize module). They are marked as
// allowed dead code to avoid warnings when not used.
#[allow(dead_code)]
const SYS_FORK: i64 = 57;
#[allow(dead_code)]
const SYS_SETSID: i64 = 112;
#[allow(dead_code)]
const SYS_DUP2: i64 = 33;
#[allow(dead_code)]
const SYS_OPEN: i64 = 2;

// Socket constants for AF_UNIX.
const AF_UNIX: i32 = 1;
#[allow(dead_code)]
const SOCK_STREAM: i32 = 1;
const SOCK_DGRAM: i32 = 2;

// File open flags (Linux O_* constants).
const O_WRONLY: i32 = 1;
const O_CREAT: i32 = 64;
const O_APPEND: i32 = 1024;
const O_CLOEXEC: i32 = 0x80000;

// Default file mode for created log files (rw-r--r--).
const MODE: i32 = 0o644;

/// Macro for debug printing that is disabled in test builds.
///
/// This is a simple wrapper around `println!` that only expands when
/// `cfg!(not(test))` is true. It allows developers to leave debug prints
/// in the code without polluting test output.
macro_rules! dbg_println {
    ($($arg:tt)*) => {
        if !cfg!(test) {
            println!($($arg)*);
        }
    };
}

/// Opens a log file with write‑only, create, append, and close‑on‑exec flags.
///
/// # Arguments
/// * `path` – Filesystem path to the log file.
///
/// # Returns
/// * `Ok(fd)` – A raw file descriptor open for writing.
/// * `Err(SyscallError)` – If the `open` syscall fails (e.g., permission denied).
///
/// # Safety
/// This function calls the unsafe `syscalls::open` function. The caller must
/// ensure that the provided path is a valid null‑terminated C string. Here,
/// `CString::new` guarantees that, and any conversion error is mapped to an
/// artificial `SyscallError` (with raw value -1) to maintain a consistent
/// error type.
fn open_log_file(path: &str) -> Result<i32, SyscallError> {
    let c_path = CString::new(path).map_err(|_| SyscallError::from_raw(-1))?;
    let flags = O_WRONLY | O_CREAT | O_APPEND | O_CLOEXEC;
    unsafe { open(c_path.as_ptr(), flags, MODE) }
}

/// Logs a message according to the daemon's logging configuration.
///
/// This function respects the configured log level: messages with a level lower
/// than `config.log_level` are silently dropped. It also routes the output to
/// the appropriate destination (`stdout` or `stderr`) based on the message level
/// and the configured `LogDest`. Error and warning messages always go to stderr,
/// regardless of the configured destination, to ensure they are visible in
/// system logs when daemonized.
///
/// # Arguments
/// * `level` – Severity of the log message.
/// * `msg`   – The message text.
/// * `config` – Current daemon configuration (used for level filtering and destination).
///
/// # Notes
/// - Timestamps are seconds since the Unix epoch.
/// - This function does not perform any I/O synchronization; logs may be buffered.
fn log(level: LogLevel, msg: &str, config: &DaemonConfig) {
    // Skip messages that are below the configured verbosity.
    if level < config.log_level {
        return;
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let level_str = match level {
        LogLevel::Error => "ERROR",
        LogLevel::Warn => "WARN",
        LogLevel::Info => "INFO",
    };
    let formatted = format!("[{}] {}: {}", timestamp, level_str, msg);
    // Routing: Info messages go to the configured destination; warnings and errors
    // always go to stderr (so they appear in the systemd journal even if stdout is
    // redirected to /dev/null).
    match (level, config.log_dest) {
        (LogLevel::Info, LogDest::Stdout) => println!("{}", formatted),
        (LogLevel::Info, LogDest::Stderr) => eprintln!("{}", formatted),
        (LogLevel::Warn | LogLevel::Error, _) => eprintln!("{}", formatted),
    }
}

/// Sends a notification message over a UNIX datagram socket (systemd notify protocol).
///
/// This function constructs a `SockAddrUn` from the given socket path, connects
/// a datagram socket, and writes the message. It handles both filesystem‑bound
/// sockets and abstract namespace sockets (if the path starts with '@').
///
/// # Arguments
/// * `socket_path` – The path to the notify socket (as set in `NOTIFY_SOCKET`).
/// * `msg`         – The message payload (e.g., b"READY=1\n").
/// * `config`      – Daemon configuration, used for logging warnings.
///
/// # Returns
/// `true` if the notification was successfully sent, `false` otherwise.
///
/// # Safety
/// This function uses unsafe syscalls (`socket`, `connect`, `write`, `close`).
/// It ensures that all pointers passed are valid (via `CString`) and that
/// addresses are properly zeroed before filling.
fn send_notify(socket_path: &str, msg: &[u8], config: &DaemonConfig) -> bool {
    // Determine if the socket is in the abstract namespace (Linux‑specific).
    // Abstract sockets are denoted by a leading '@', which we replace with a null byte.
    let (addr_path, is_abstract) = if socket_path.starts_with('@') {
        (format!("\0{}", &socket_path[1..]), true)
    } else {
        (socket_path.to_string(), false)
    };

    let c_path = match std::ffi::CString::new(addr_path) {
        Ok(p) => p,
        Err(_) => {
            log(LogLevel::Warn, "Invalid NOTIFY_SOCKET path", config);
            return false;
        }
    };

    // Create a datagram socket (SOCK_DGRAM) in the AF_UNIX domain.
    let fd = match unsafe { socket(AF_UNIX, SOCK_DGRAM, 0) } {
        Ok(f) => f,
        Err(e) => {
            log(
                LogLevel::Warn,
                &format!("Failed to create notify socket: {:?}", e),
                config,
            );
            return false;
        }
    };

    // Prepare the socket address structure.
    // We zero the entire struct to ensure padding and unused fields are clean.
    let mut addr: SockAddrUn = unsafe { std::mem::zeroed() };
    addr.sun_family = AF_UNIX as u16;
    let bytes = c_path.as_bytes_with_nul();
    let max_path = addr.sun_path.len();
    let len_to_copy = bytes.len().min(max_path);
    unsafe {
        std::ptr::copy_nonoverlapping(
            bytes.as_ptr() as *const i8,
            addr.sun_path.as_mut_ptr(),
            len_to_copy,
        );
    }

    // Compute the address length according to the socket type:
    // - For abstract sockets, the length is the size of the family (u16) plus
    //   the length of the name (excluding the leading null? Actually we include it)
    //   plus 1 for the initial null byte (since we replaced '@' with '\0').
    //   The sun_path for abstract sockets includes the leading null as part of
    //   the path; the kernel treats the first byte as the null indicating abstract.
    // - For filesystem sockets, it's the size of family plus the full path length
    //   including the terminating null.
    let addr_len = if is_abstract {
        // The name length includes the initial null we added, plus the rest of the string.
        // socket_path was '@' + rest, so after replacing, we have '\0' + rest.
        // The total length is: sizeof(sa_family_t) + 1 (the leading null) + (rest length)
        // But rest length = socket_path.len() - 1.
        let name_len = socket_path.len() - 1; // length of the part after '@'
        std::mem::size_of::<u16>() + 1 + name_len
    } else {
        let path_len = socket_path.len();
        std::mem::size_of::<u16>() + path_len + 1 // +1 for the terminating null
    };

    // Connect the socket to the target address.
    if let Err(e) = unsafe { connect(fd, &addr as *const _, addr_len as u32) } {
        log(
            LogLevel::Warn,
            &format!("Failed to connect notify socket: {:?}", e),
            config,
        );
        let _ = unsafe { close(fd) };
        return false;
    }

    // Write the message (the entire byte slice).
    let write_result = unsafe { write(fd, msg) };
    // Always close the file descriptor, regardless of write outcome.
    let _ = unsafe { close(fd) };

    if let Err(e) = write_result {
        log(
            LogLevel::Warn,
            &format!("Failed to write to notify socket: {:?}", e),
            config,
        );
        false
    } else {
        true
    }
}

use telemetry::{Metrics, StdoutSink};

/// Core event loop for a clipboard backend.
///
/// This function takes ownership of the process (returns `!`) and runs
/// indefinitely, attempting to connect to the backend and processing clipboard
/// events. On connection failure, it waits with an exponential backoff and
/// retries. If the backend indicates a fatal error, the function exits the
/// process.
///
/// # Type Parameters
/// * `B` – A type implementing the `ClipboardBackend` trait (e.g., `X11Connection`).
///
/// # Arguments
/// * `config` – The daemon configuration (used for delays, logging, etc.).
/// * `notify_socket` – Optional systemd notify socket path. If provided, a
///                     `READY=1` notification is sent after the first successful
///                     clipboard event.
///
/// # Returns
/// This function never returns normally; it exits the process via `std::process::exit`.
/// It may exit with status 0 on clean shutdown (signal received) or status 1 on fatal errors.
///
/// # Panics
/// This function does not panic, but it may call `std::process::exit`.
fn run_backend<B>(config: &DaemonConfig, notify_socket: Option<String>) -> !
where
    B: ClipboardBackend,
    B::Error: 'static + std::fmt::Debug,
{
    // Current reconnection delay, starting from the configured base.
    let mut current_delay = config.reconnect_delay;
    let notify_socket_ref = notify_socket.as_ref();
    let metrics = Metrics::get();

    while RUNNING.load(Ordering::Relaxed) {
        log(LogLevel::Info, "Attempting to connect to X server", config);

        // Validate that the DISPLAY environment variable is set and looks plausible.
        // This check is performed before each connection attempt because the
        // environment could theoretically change, but mainly it provides a clear
        // error message if the variable is missing or malformed.
        let display_ok = match std::env::var("DISPLAY") {
            Ok(ref d) if d.contains(':') => true,
            _ => false,
        };

        if !display_ok {
            log(
                LogLevel::Error,
                "Invalid or missing DISPLAY environment variable – exiting",
                config,
            );
            std::process::exit(1);
        }

        // Try to establish a connection to the backend.
        let mut backend = match B::connect() {
            Ok(b) => {
                log(LogLevel::Info, "Successfully connected to X server", config);
                b
            }
            Err(e) => {
                // If the error is considered fatal by the backend, we exit immediately.
                if B::is_fatal_error(&e) {
                    log(
                        LogLevel::Error,
                        &format!("Fatal monitor error: {:?} – exiting", e),
                        config,
                    );
                    std::process::exit(1);
                }
                metrics.inc_connection_retries_count();
                log(
                    LogLevel::Warn,
                    &format!(
                        "Monitor error: {:?} – reconnecting in {:?}",
                        e, current_delay
                    ),
                    config,
                );
                thread::sleep(current_delay);
                current_delay = apply_backoff(current_delay, config);
                continue;
            }
        };

        // Reset the delay after a successful connection.
        current_delay = config.reconnect_delay;

        // Flag to ensure we send "READY=1" only once, after the first clipboard event.
        let mut ready_sent = false;

        // Enter the backend's event loop. The closure is called for each clipboard update.
        match backend.run(|data| {
            if !ready_sent {
                if let Some(socket) = notify_socket_ref {
                    send_notify(socket, b"READY=1\n", config);
                }
                ready_sent = true;
            }

            metrics.inc_clipboard_event_count();

            // Log the clipboard content; try to interpret as UTF-8, otherwise log as binary.
            if let Ok(s) = std::str::from_utf8(&data) {
                log(LogLevel::Info, &format!("Clipboard: {}", s), config);
            } else {
                log(
                    LogLevel::Info,
                    &format!("Clipboard (binary): {:?}", data),
                    config,
                );
            }
            // Check if a termination signal has been received; if so, exit cleanly.
            if !RUNNING.load(Ordering::SeqCst) {
                log(LogLevel::Info, "Signal received, exiting cleanly", config);
                std::process::exit(0);
            }
        }) {
            Ok(()) => {
                // Backend.run() returned Ok(()) – this typically means the event loop
                // was interrupted by a signal (e.g., EINTR) and we should shut down.
                metrics.inc_eintr_count();
                log(
                    LogLevel::Info,
                    "Monitor returned cleanly (signal received), exiting",
                    config,
                );
                std::process::exit(0);
            }
            Err(e) => {
                // An error occurred while running; we'll retry after a backoff.
                metrics.inc_connection_retries_count();
                log(
                    LogLevel::Warn,
                    &format!(
                        "Monitor error: {:?} – reconnecting in {:?}",
                        e, current_delay
                    ),
                    config,
                );
                thread::sleep(current_delay);
                current_delay = apply_backoff(current_delay, config);
            }
        }
    }

    // If the RUNNING flag is cleared without entering the backend loop, exit.
    log(
        LogLevel::Info,
        "Daemon exiting (RUNNING flag cleared)",
        config,
    );
    std::process::exit(0);
}

/// Applies exponential backoff to a reconnection delay, capping at the configured maximum.
///
/// # Arguments
/// * `d`      – Current delay.
/// * `config` – Daemon configuration containing multiplier and max delay.
///
/// # Returns
/// The new delay after multiplying and capping.
fn apply_backoff(d: Duration, config: &DaemonConfig) -> Duration {
    let secs = d.as_secs_f64() * config.reconnect_backoff_multiplier;
    let max_secs = config.reconnect_max_delay.as_secs_f64();
    let capped = secs.min(max_secs);
    Duration::from_secs_f64(capped)
}

/// Starts a background thread that periodically sends watchdog notifications to systemd.
///
/// If the environment variable `WATCHDOG_USEC` is set to a positive integer,
/// this function spawns a thread that sends `WATCHDOG=1` to the notify socket
/// at half the watchdog interval (as recommended by systemd). The thread runs
/// until the global `RUNNING` flag becomes false.
///
/// # Arguments
/// * `notify_socket` – The socket path from `NOTIFY_SOCKET`. If missing, the
///                     watchdog thread cannot start and a warning is logged.
/// * `config`        – Daemon configuration (for logging).
/// * `running`       – Global atomic flag indicating whether the daemon is still running.
fn start_watchdog(
    notify_socket: Option<String>,
    config: DaemonConfig,
    running: &'static AtomicBool,
) {
    // Parse the watchdog interval from the environment.
    let watchdog_usec = match std::env::var("WATCHDOG_USEC")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
    {
        Some(usec) if usec > 0 => usec,
        _ => return, // Not configured or invalid → no watchdog.
    };
    let socket_path = match notify_socket {
        Some(p) => p,
        None => {
            log(
                LogLevel::Warn,
                "WATCHDOG_USEC set but NOTIFY_SOCKET missing",
                &config,
            );
            return;
        }
    };
    let interval = Duration::from_micros(watchdog_usec);
    // systemd recommends sending keep-alive at half the interval to account for
    // potential scheduling delays.
    let half = interval / 2;

    thread::spawn(move || {
        while running.load(Ordering::Relaxed) {
            thread::sleep(half);
            send_notify(&socket_path, b"WATCHDOG=1\n", &config);
        }
    });
}

/// Main entry point for the daemon.
///
/// This function orchestrates the entire daemon lifecycle:
/// 1. Load configuration from environment.
/// 2. If daemonization is requested, test log file writability (if specified),
///    then daemonize (fork, detach, redirect stdio to the log file).
/// 3. Install signal handlers.
/// 4. Start a metrics reporter thread.
/// 5. Start a watchdog thread (if configured).
/// 6. Log startup information.
/// 7. Enter the main event loop (via `run_backend`).
///
/// # Returns
/// This function never returns; it exits the process with an appropriate status code.
pub fn run() -> ! {
    let config = DaemonConfig::from_env();

    // If we are going to daemonize and a log file is specified, we attempt to open it
    // *before* daemonizing to catch filesystem errors early (while we still have a
    // terminal to print error messages). The file is immediately closed; we will reopen
    // it after daemonization for actual logging.
    if config.daemonize {
        if let Some(log_path) = &config.log_file {
            match open_log_file(log_path) {
                Ok(fd) => {
                    let _ = unsafe { syscalls::close(fd) };
                }
                Err(e) => {
                    eprintln!(
                        "ERROR: Cannot open log file '{}' for writing: {:?}",
                        log_path, e
                    );
                    std::process::exit(1);
                }
            }
        }
    }

    let notify_socket = std::env::var("NOTIFY_SOCKET").ok();

    // Perform the actual daemonization if requested.
    if config.daemonize {
        if let Err(e) = daemonize::daemonize() {
            dbg_println!("Daemonization failed: {:?}", e);
            std::process::exit(1);
        }
        // After daemonization, we are in the daemon process. Redirect stdout and stderr
        // to the log file if one was provided. Note: we do not redirect stdin because
        // it will be pointed to /dev/null by daemonize().
        if let Some(log_path) = &config.log_file {
            match open_log_file(log_path) {
                Ok(fd) => {
                    let _ = daemonize::dup2(fd, 1); // stdout
                    let _ = daemonize::dup2(fd, 2); // stderr
                    let _ = daemonize::close_fd(fd);
                }
                Err(_) => std::process::exit(1),
            }
        }
    }

    // Install signal handlers that set the global RUNNING flag to false on SIGTERM, etc.
    if let Err(e) = install_handlers() {
        dbg_println!("Failed to install signal handlers: {:?}", e);
        std::process::exit(1);
    }

    // Start a background thread that periodically reports metrics to stdout.
    let reporter = PeriodicSink::new(StdoutSink, config.metrics_interval, &RUNNING);
    reporter.start();

    // Start the systemd watchdog thread if applicable.
    start_watchdog(notify_socket.clone(), config.clone(), &RUNNING);

    log(
        LogLevel::Info,
        &format!("Daemon starting. Config: {:?}", config),
        &config,
    );

    // Enter the main clipboard monitoring loop – this call never returns.
    run_backend::<X11Connection>(&config, notify_socket)
}
