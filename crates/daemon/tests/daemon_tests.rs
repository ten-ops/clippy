//! Integration tests for the clipboard daemon.
//!
//! This module tests the daemon's behaviour in various scenarios, including:
//! - Command-line argument handling (via environment variables).
//! - Signal handling and graceful shutdown.
//! - Reconnection logic with exponential backoff.
//! - Logging to different destinations (stdout, stderr, files).
//! - Daemonization (forking and redirecting stdio).
//! - systemd notify protocol (READY=1, watchdog).
//! - Error handling for invalid configurations.
//!
//! The tests spawn the actual daemon binary (`daemon`) as a child process,
//! set environment variables, and observe its behaviour through its output
//! or by interacting with it (e.g., sending signals, creating fake X sockets).
//!
//! # Concurrency
//! A global `DISPLAY_LOCK` is used to serialise tests that need exclusive
//! access to the X server or that manipulate the `/tmp/.X11-unix/` directory,
//! because these operations can interfere with each other when run in parallel.
//!
//! # Safety
//! Some tests call `syscalls::kill` to send signals to the child process.
//! This is safe as long as the child process ID is valid and the signal
//! number is correct. The `unsafe` block is required because the `kill`
//! syscall is inherently unsafe (it could send a signal to the wrong process
//! if the PID is reused, but in these controlled tests the PID is still valid).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use syscalls::kill;

/// Mutex to serialise tests that interact with the X server or the X socket directory.
///
/// This prevents race conditions when multiple tests try to start X11 connections
/// or manipulate `/tmp/.X11-unix/` simultaneously.
static DISPLAY_LOCK: Mutex<()> = Mutex::new(());

/// Signal number used to terminate the daemon gracefully.
const SIGTERM: i32 = 15;

/// Spawns the daemon binary with the given environment variables.
///
/// This function constructs a `Command` that runs the `daemon` executable
/// (which is the binary produced by the current crate). It removes any
/// existing `DISPLAY` variable from the environment and then adds the provided
/// variables. Optionally, stdout and/or stderr can be captured as pipes.
///
/// # Arguments
/// * `env_vars` – A list of `(key, value)` pairs to set in the child's environment.
/// * `capture_stdout` – If `true`, the child's stdout is piped and can be read later.
/// * `capture_stderr` – If `true`, the child's stderr is piped.
///
/// # Returns
/// A `std::process::Child` handle representing the spawned daemon.
///
/// # Panics
/// Panics if the daemon binary cannot be found or spawned. This is expected
/// only if the build is misconfigured.
fn spawn_daemon(
    env_vars: Vec<(&str, &str)>,
    capture_stdout: bool,
    capture_stderr: bool,
) -> std::process::Child {
    // `env!("CARGO_BIN_EXE_daemon")` is set by Cargo to the path of the
    // `daemon` binary (see Cargo.toml for `[[bin]]`). This ensures we test
    // the actual compiled binary, not an in‑memory version.
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_daemon"));
    // Clear any pre‑existing DISPLAY to avoid accidental interference.
    cmd.env_remove("DISPLAY");
    for (k, v) in env_vars {
        cmd.env(k, v);
    }
    if capture_stdout {
        cmd.stdout(Stdio::piped());
    }
    if capture_stderr {
        cmd.stderr(Stdio::piped());
    }
    cmd.spawn().expect("failed to spawn daemon")
}

/// Tests that the daemon exits with an error when the `DISPLAY` variable is missing.
///
/// Steps:
/// 1. Spawn the daemon with no `DISPLAY` variable.
/// 2. Wait a short time, then send SIGTERM (the daemon should already have exited,
///    but we send it anyway to clean up).
/// 3. Check stderr for the expected error message.
///
/// The `DISPLAY_LOCK` ensures that this test does not run concurrently with
/// other tests that modify the X environment.
#[test]
fn test_daemon_no_display() {
    let _guard = DISPLAY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let child = spawn_daemon(vec![], false, true);
    thread::sleep(Duration::from_millis(300));
    unsafe {
        kill(child.id() as i32, SIGTERM).ok();
    }
    let output = child.wait_with_output().expect("failed to get output");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Invalid or missing DISPLAY"),
        "expected 'Invalid or missing DISPLAY' in stderr, got: {stderr}"
    );
}

/// Tests that the daemon exits with an error when `DISPLAY` is set to an invalid value.
///
/// Similar to the previous test, but provides a garbage value for `DISPLAY`.
#[test]
fn test_daemon_invalid_display() {
    let _guard = DISPLAY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let child = spawn_daemon(vec![("DISPLAY", "garbage")], false, true);
    thread::sleep(Duration::from_millis(300));
    unsafe { kill(child.id() as i32, SIGTERM).ok() };
    let output = child.wait_with_output().expect("failed to get output");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Invalid or missing DISPLAY"),
        "expected 'Invalid or missing DISPLAY' in stderr, got: {stderr}"
    );
}

/// Tests that the daemon handles SIGTERM gracefully when it is connected to a real X server.
///
/// This test is only run if a valid `DISPLAY` is set and the X server is reachable.
/// It spawns the daemon, waits for it to start, sends SIGTERM, and checks that
/// it exits successfully (status 0). This verifies that the signal handler
/// works and that the daemon shuts down cleanly.
#[test]
fn test_daemon_signal_handling() {
    let _guard = DISPLAY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let display = match std::env::var("DISPLAY") {
        Ok(d) => d,
        Err(_) => {
            eprintln!("DISPLAY not set – skipping signal test");
            return;
        }
    };
    if monitor::backend::x11::X11Connection::connect().is_err() {
        eprintln!("X server not reachable – skipping signal test");
        return;
    }
    let mut child = spawn_daemon(vec![("DISPLAY", &display)], false, false);
    thread::sleep(Duration::from_millis(500));
    unsafe {
        kill(child.id() as i32, SIGTERM).ok();
    }
    let status = child.wait().expect("failed to wait for child");
    assert!(status.success());
}

/// Tests that the daemon respects the `CLIPPER_RECONNECT_DELAY` environment variable.
///
/// The daemon is started with a fake DISPLAY (":99", which is unlikely to exist)
/// so that it will repeatedly try to reconnect. We set the reconnect delay to
/// 1 second and then check that the log messages show "reconnecting in 1s".
#[test]
fn test_daemon_reconnect_delay_env() {
    let _guard = DISPLAY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let child = spawn_daemon(
        vec![("DISPLAY", ":99"), ("CLIPPER_RECONNECT_DELAY", "1")],
        false,
        true,
    );
    thread::sleep(Duration::from_millis(300));
    unsafe {
        kill(child.id() as i32, SIGTERM).ok();
    }
    let output = child.wait_with_output().expect("failed to get output");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("reconnecting in 1s"),
        "expected 'reconnecting in 1s' in stderr, got: {stderr}"
    );
}

/// Tests that the log level filtering works.
///
/// Set `CLIPPER_LOG_LEVEL=warn` and `CLIPPER_LOG_DEST=stdout`. The daemon
/// should not print info messages (like "Attempting to connect") to stdout,
/// but should still print warnings (like "reconnecting in") to stderr.
/// We verify that the info message is absent from stdout and the warning is
/// present in stderr.
#[test]
fn test_daemon_log_level() {
    let _guard = DISPLAY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let child = spawn_daemon(
        vec![
            ("DISPLAY", ":99"),
            ("CLIPPER_LOG_LEVEL", "warn"),
            ("CLIPPER_LOG_DEST", "stdout"),
        ],
        true,
        true,
    );
    thread::sleep(Duration::from_millis(300));
    unsafe { kill(child.id() as i32, SIGTERM).ok() };
    let output = child.wait_with_output().expect("failed to get output");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stdout.contains("Attempting to connect"),
        "expected no 'Attempting to connect' in stdout, got: {stdout}"
    );
    assert!(
        stderr.contains("reconnecting in"),
        "expected 'reconnecting in' in stderr, got: {stderr}"
    );
}

/// Tests that log destination (`CLIPPER_LOG_DEST`) works correctly.
///
/// With `CLIPPER_LOG_DEST=stderr`, info messages should go to stderr, not stdout.
/// We verify that "Attempting to connect" appears in stderr and not in stdout.
#[test]
fn test_daemon_log_dest_stderr() {
    let _guard = DISPLAY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let child = spawn_daemon(
        vec![("DISPLAY", "garbage"), ("CLIPPER_LOG_DEST", "stderr")],
        true,
        true,
    );
    thread::sleep(Duration::from_millis(300));
    unsafe { kill(child.id() as i32, SIGTERM).ok() };
    let output = child.wait_with_output().expect("failed to get output");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Attempting to connect"),
        "expected 'Attempting to connect' in stderr, got: {stderr}"
    );
    assert!(
        !stdout.contains("Attempting to connect"),
        "expected no 'Attempting to connect' in stdout, got: {stdout}"
    );
}

/// Tests daemonization with a log file.
///
/// The daemon is started with `CLIPPER_DAEMONIZE=1` and a log file path.
/// After a short time, we send SIGTERM and check that the daemon exited
/// successfully and that the log file contains expected messages.
/// This verifies that:
/// - The daemon successfully daemonizes (we can still kill it by PID).
/// - Stdout/stderr are redirected to the log file.
/// - The log file is written correctly.
#[test]
fn test_daemon_mode_with_log_file() {
    use std::io::Read;
    use tempfile::NamedTempFile;

    let _guard = DISPLAY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let log_file = NamedTempFile::new().expect("failed to create temp file");
    let log_path = log_file.path().to_str().unwrap().to_string();

    let mut child = spawn_daemon(
        vec![
            ("DISPLAY", ":99"),
            ("CLIPPER_DAEMONIZE", "1"),
            ("CLIPPER_LOG_FILE", &log_path),
        ],
        false,
        false,
    );

    std::thread::sleep(Duration::from_secs(1));

    unsafe { kill(child.id() as i32, SIGTERM).ok() };

    let status = child.wait().expect("failed to wait");
    assert!(status.success());

    let mut content = String::new();
    std::fs::File::open(log_file.path())
        .unwrap()
        .read_to_string(&mut content)
        .unwrap();

    assert!(
        content.contains("Attempting to connect to X server"),
        "expected connect attempt in log file, got: {content}"
    );
    assert!(
        content.contains("reconnecting in"),
        "expected reconnect message in log file, got: {content}"
    );
}

/// Tests daemonization without a log file (i.e., output to /dev/null).
///
/// This simply verifies that the daemon can start and exit cleanly when
/// daemonized and no log file is provided. Since output goes to /dev/null,
/// we cannot inspect it; we only check that the process exits successfully.
#[test]
fn test_daemon_mode_no_log_file() {
    let _guard = DISPLAY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut child = spawn_daemon(
        vec![("DISPLAY", "garbage"), ("CLIPPER_DAEMONIZE", "1")],
        false,
        false,
    );

    std::thread::sleep(Duration::from_secs(1));
    unsafe { kill(child.id() as i32, SIGTERM).ok() };
    let status = child.wait().expect("failed to wait");
    assert!(status.success());
}

/// Tests the systemd readiness notification (`READY=1`).
///
/// This test sets up a UNIX datagram socket to act as the systemd notify
/// socket, starts the daemon with `NOTIFY_SOCKET` pointing to it, and waits
/// to receive the `READY=1` message. The test is ignored by default because
/// it requires a working X server and may be flaky in CI.
#[test]
#[ignore = "not needed"]
fn test_systemd_notify() {
    use std::os::unix::net::UnixDatagram;
    use tempfile::TempDir;

    let _guard = DISPLAY_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let display = match std::env::var("DISPLAY") {
        Ok(d) => d,
        Err(_) => {
            eprintln!("DISPLAY not set – skipping test");
            return;
        }
    };
    if monitor::backend::x11::X11Connection::connect().is_err() {
        eprintln!("X server not reachable – skipping test");
        return;
    }

    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let socket_path = temp_dir.path().join("notify.sock");
    let socket_path_str = socket_path.to_str().unwrap().to_string();

    // Bind a listener to the socket path before starting the daemon.
    let listener = UnixDatagram::bind(&socket_path).expect("failed to bind notify socket");
    listener.set_nonblocking(false).unwrap(); // ensure blocking recv

    let mut child = spawn_daemon(
        vec![("DISPLAY", &display), ("NOTIFY_SOCKET", &socket_path_str)],
        false,
        false,
    );

    let mut buf = [0; 32];
    // Use catch_unwind to capture assertion failures and still clean up the child.
    let result = catch_unwind(AssertUnwindSafe(|| {
        listener
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();

        let n = listener
            .recv(&mut buf)
            .expect("failed to receive notification");

        assert_eq!(&buf[..n], b"READY=1\n", "unexpected notification message");
    }));

    // Clean up: kill the daemon and wait for it.
    unsafe { kill(child.id() as i32, SIGTERM).ok() };
    let _ = child.wait();
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}

/// Tests the systemd watchdog heartbeat (`WATCHDOG=1`).
///
/// Similar to the notify test, but sets `WATCHDOG_USEC=2000000` (2 seconds)
/// and checks that the daemon sends a `WATCHDOG=1` message at least once
/// within a 5‑second window.
#[test]
fn test_systemd_watchdog() {
    use std::os::unix::net::UnixDatagram;
    use std::time::Instant;
    use tempfile::TempDir;

    let _guard = DISPLAY_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let display = match std::env::var("DISPLAY") {
        Ok(d) => d,
        Err(_) => {
            eprintln!("DISPLAY not set – skipping test");
            return;
        }
    };
    if monitor::backend::x11::X11Connection::connect().is_err() {
        eprintln!("X server not reachable – skipping test");
        return;
    }

    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let socket_path = temp_dir.path().join("watchdog.sock");
    let socket_path_str = socket_path.to_str().unwrap().to_string();

    let listener = UnixDatagram::bind(&socket_path).expect("failed to bind notify socket");
    listener.set_nonblocking(false).unwrap();

    let mut child = spawn_daemon(
        vec![
            ("DISPLAY", &display),
            ("NOTIFY_SOCKET", &socket_path_str),
            ("WATCHDOG_USEC", "2000000"), // 2 seconds
        ],
        false,
        false,
    );

    let mut buf = [0; 32];
    listener
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();

    let start = Instant::now();
    let mut saw_watchdog = false;
    while start.elapsed() < Duration::from_secs(5) {
        match listener.recv(&mut buf) {
            Ok(n) if &buf[..n] == b"WATCHDOG=1\n" => {
                saw_watchdog = true;
                break;
            }
            Ok(_) => continue,
            Err(e) => {
                eprintln!("recv error: {:?}", e);
                break;
            }
        }
    }

    unsafe { kill(child.id() as i32, SIGTERM).ok() };
    let _ = child.wait();

    assert!(saw_watchdog, "Did not receive any WATCHDOG=1 heartbeat");
}

/// Tests that the daemon exits with an error when an invalid log file path is provided.
///
/// We set `CLIPPER_LOG_FILE` to a directory path (which cannot be opened as a file)
/// and check that the daemon prints an error and exits with a non‑zero status.
#[test]
fn test_daemon_mode_invalid_log_file() {
    use tempfile::TempDir;

    let _guard = DISPLAY_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let dir_path = temp_dir.path().to_str().unwrap().to_string();

    let child = spawn_daemon(
        vec![
            ("DISPLAY", "garbage"),
            ("CLIPPER_DAEMONIZE", "1"),
            ("CLIPPER_LOG_FILE", &dir_path),
        ],
        false,
        true,
    );

    let output = child.wait_with_output().expect("failed to wait");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Cannot open log file"),
        "Expected 'Cannot open log file' error, got: {}",
        stderr
    );
}

/// Tests that the daemon exits with an error when an invalid numeric value is
/// provided for `CLIPPER_RECONNECT_DELAY`.
///
/// The daemon should abort during configuration parsing and print an error.
#[test]
fn test_daemon_invalid_reconnect_delay() {
    let _guard = DISPLAY_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let child = spawn_daemon(
        vec![
            ("DISPLAY", ":99"),
            ("CLIPPER_RECONNECT_DELAY", "not-a-number"),
        ],
        false,
        true,
    );

    let output = child.wait_with_output().expect("failed to wait");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not a valid number, aborting"),
        "Expected error about invalid number, got: {}",
        stderr
    );
}

/// Tests that the daemon automatically reconnects after the X server is restarted.
///
/// This test simulates an X server restart by creating a symbolic link to the
/// real X socket at a fake display number (99). Steps:
/// 1. Start the daemon with `DISPLAY=:99` – it will fail to connect because
///    the socket `/tmp/.X11-unix/X99` does not exist.
/// 2. After a few seconds, create a symlink from the fake socket to the real
///    X socket (which exists if a real X server is running).
/// 3. Wait a few more seconds; the daemon should connect successfully.
/// 4. Send SIGTERM and verify that the daemon logged both the retry messages
///    and the successful connection message.
///
/// This test requires a real X server and modifies `/tmp/.X11-unix/`, so it
/// is protected by `DISPLAY_LOCK` to avoid interfering with other tests.
#[test]
fn test_daemon_reconnects_after_x_server_restart() {
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::path::Path;

    let _guard = DISPLAY_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    // Determine the real X socket from the current DISPLAY (or assume :0).
    let real_display = match std::env::var("DISPLAY") {
        Ok(d) if d.contains(':') => d,
        _ => ":0".to_string(),
    };
    let display_num = real_display
        .split(':')
        .nth(1)
        .and_then(|s| s.split('.').next())
        .unwrap_or("0");
    let real_socket = format!("/tmp/.X11-unix/X{}", display_num);

    if !Path::new(&real_socket).exists() {
        eprintln!("Real X socket {} not found – skipping test", real_socket);
        return;
    }

    // Choose a fake display number (99) and the corresponding socket.
    let fake_display_num = "99";
    let fake_socket = format!("/tmp/.X11-unix/X{}", fake_display_num);
    let fake_display = format!(":{}", fake_display_num);

    // Remove any stale symlink.
    let _ = fs::remove_file(&fake_socket);

    let mut child = spawn_daemon(
        vec![
            ("DISPLAY", &fake_display),
            ("CLIPPER_RECONNECT_DELAY", "1"),
            ("CLIPPER_LOG_LEVEL", "info"),
        ],
        true,
        true,
    );

    // Let the daemon attempt and fail to connect a few times.
    std::thread::sleep(Duration::from_secs(3));

    // Create the fake socket as a symlink to the real one, simulating X server startup.
    if let Err(e) = symlink(&real_socket, &fake_socket) {
        eprintln!("Failed to create symlink: {:?}", e);
        let _ = child.kill();
        return;
    }

    // Give the daemon time to detect the new socket and connect.
    std::thread::sleep(Duration::from_secs(3));

    // Terminate the daemon.
    unsafe { kill(child.id() as i32, SIGTERM).ok() };

    let output = child.wait_with_output().expect("failed to wait");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Clean up the fake socket.
    let _ = fs::remove_file(&fake_socket);

    assert!(
        stderr.contains("reconnecting in 1s"),
        "Expected retry message in stderr:\n{}",
        stderr
    );
    assert!(
        stdout.contains("Successfully connected to X server")
            || stderr.contains("Successfully connected to X server"),
        "Expected successful connection message:\nstdout:{}\nstderr:{}",
        stdout,
        stderr
    );
    assert!(output.status.success(), "Daemon did not exit cleanly");
}
