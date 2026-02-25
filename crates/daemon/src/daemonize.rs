//! Low-level Unix daemonization using raw system calls (Linux‑specific).
//!
//! This module implements the classic “double‑fork” procedure to detach a process
//! from its terminal and run it as a daemon. All operations are performed via
//! direct `syscall` invocations to avoid libc dependencies and to have precise
//! control over error handling.
//!
//! # Daemonization steps
//!
//! 1. **First `fork()`** – The parent exits, the child continues. This ensures
//!    that the daemon is not a session leader, which is a prerequisite for
//!    `setsid()`.
//! 2. **`setsid()`** – The child creates a new session, becoming its leader and
//!    losing its controlling terminal.
//! 3. **Second `fork()`** – The session leader exits, the grandchild continues.
//!    This guarantees that the daemon cannot re‑acquire a controlling terminal
//!    (because it is not a session leader).
//! 4. **`chdir("/")`** – Change working directory to root to avoid blocking
//!    mount points.
//! 5. **Open `/dev/null`** – Obtain a file descriptor for the null device.
//! 6. **`dup2()`** – Redirect stdin, stdout, and stderr to `/dev/null`.
//! 7. **Close the original `/dev/null` fd** – Keep only the duplicated ones.
//!
//! # Safety
//!
//! This module uses raw system calls (`syscall0`, `syscall1`, `syscall3`) that
//! are unsafe because they bypass Rust’s safety guarantees and directly invoke
//! the kernel. The caller must ensure that:
//!
//! * The syscall numbers correspond to the correct functions on the target
//!   architecture (here: x86_64 Linux).
//! * File descriptors are valid and not closed unexpectedly.
//! * The process does not hold any locks or resources that would be affected
//!   by `fork()` (e.g., threads, mutexes – but note that a daemon should be
//!   single‑threaded at this stage).
//!
//! # Error handling
//!
//! Each syscall is checked for a negative return value, which indicates an
//! error. In such a case, a corresponding `DaemonError` variant is returned.
//! Because the process is in a critical startup phase, most errors are fatal.
//!
//! # Platform
//!
//! This code is Linux‑specific (syscall numbers are for x86_64). It will not
//! compile or work on other Unix‑like systems without adjustment.

use syscalls::{SyscallError, syscall0, syscall1, syscall3};

// Syscall numbers for Linux x86_64.
// See: /usr/include/asm/unistd_64.h
const SYS_FORK: i64 = 57;
const SYS_SETSID: i64 = 112;
const SYS_DUP2: i64 = 33;
const SYS_CHDIR: i64 = 80;
const SYS_CLOSE: i64 = 3;

/// Errors that can occur during daemonization.
///
/// Each variant corresponds to a specific syscall failure.
/// The `Syscall` variant wraps the underlying `SyscallError` for cases where
/// more detailed information is available.
#[derive(Debug)]
pub enum DaemonError {
    /// The `fork()` syscall returned an error (e.g., insufficient memory).
    ForkFailed,
    /// The `setsid()` syscall failed (e.g., the process was already a session leader).
    SetsidFailed,
    /// `dup2()` failed (e.g., invalid file descriptor).
    Dup2Failed,
    /// `close()` failed (e.g., invalid file descriptor).
    CloseFailed,
    /// `chdir()` failed (e.g., permission denied or path does not exist).
    ChdirFailed,
    /// A raw syscall returned an error that contains additional information.
    #[allow(dead_code)]
    Syscall(SyscallError),
}

impl From<SyscallError> for DaemonError {
    /// Converts a low‑level `SyscallError` into a `DaemonError::Syscall`.
    ///
    /// This allows the use of the `?` operator on raw syscall results.
    fn from(err: SyscallError) -> Self {
        DaemonError::Syscall(err)
    }
}

/// Invokes the `fork` system call.
///
/// # Returns
/// - `Ok(0)` in the child process.
/// - `Ok(pid)` in the parent process (the PID of the child).
/// - `Err(DaemonError::ForkFailed)` if the syscall indicated an error.
///
/// # Safety
/// This function is unsafe because it calls `syscall0`. The caller must ensure
/// that the process state is fork‑safe (e.g., no threads holding locks).
fn fork() -> Result<i32, DaemonError> {
    let ret = unsafe { syscall0(SYS_FORK) }?;
    if ret < 0 {
        // Negative return means error; no further details are captured here.
        Err(DaemonError::ForkFailed)
    } else {
        Ok(ret as i32)
    }
}

/// Invokes the `setsid` system call.
///
/// Creates a new session if the calling process is not a process group leader.
///
/// # Returns
/// - `Ok(session_id)` on success.
/// - `Err(DaemonError::SetsidFailed)` on error.
///
/// # Safety
/// Unsafe because it calls `syscall0`.
fn setsid() -> Result<i32, DaemonError> {
    let ret = unsafe { syscall0(SYS_SETSID) }?;
    if ret < 0 {
        Err(DaemonError::SetsidFailed)
    } else {
        Ok(ret as i32)
    }
}

/// Duplicates a file descriptor using the `dup2` syscall.
///
/// After a successful call, `new_fd` will refer to the same open file description
/// as `old_fd`. If `new_fd` was already open, it is closed beforehand.
///
/// # Arguments
/// * `old_fd` – The file descriptor to duplicate.
/// * `new_fd` – The desired new file descriptor number.
///
/// # Returns
/// `Ok(())` on success, or `Err(DaemonError::Dup2Failed)` if the syscall failed.
///
/// # Safety
/// Unsafe because it calls `syscall3`. The caller must ensure that `old_fd` is
/// a valid open file descriptor and that `new_fd` is within the allowed range
/// and not used in a way that would corrupt process state.
pub fn dup2(old_fd: i32, new_fd: i32) -> Result<(), DaemonError> {
    let ret = unsafe { syscall3(SYS_DUP2, old_fd as i64, new_fd as i64, 0) }?;
    if ret < 0 {
        Err(DaemonError::Dup2Failed)
    } else {
        Ok(())
    }
}

/// Changes the current working directory using the `chdir` syscall.
///
/// # Arguments
/// * `path` – A path to change to (must be a valid null‑terminated string).
///
/// # Returns
/// `Ok(())` on success, or `Err(DaemonError::ChdirFailed)` if the syscall failed
/// or the path contained an interior null byte.
///
/// # Safety
/// Unsafe because it calls `syscall1` with a pointer that must be valid and
/// null‑terminated.
fn chdir(path: &str) -> Result<(), DaemonError> {
    use std::ffi::CString;
    let c_path = CString::new(path).map_err(|_| DaemonError::ChdirFailed)?;
    let ret = unsafe { syscall1(SYS_CHDIR, c_path.as_ptr() as i64) }?;
    if ret < 0 {
        Err(DaemonError::ChdirFailed)
    } else {
        Ok(())
    }
}

/// Opens `/dev/null` with read/write access using a direct `open` syscall.
///
/// This function uses the raw `open` syscall (provided by the `syscalls` crate)
/// because the standard library’s `File::open` would pull in libc dependencies.
///
/// # Returns
/// - `Ok(fd)` – a file descriptor for `/dev/null` on success.
/// - `Err(DaemonError::Syscall(...))` if the open fails.
///
/// # Safety
/// Unsafe because the underlying `open` syscall is unsafe. The path must be a
/// valid null‑terminated string. The flags `2` correspond to `O_RDWR` on Linux.
fn open_devnull() -> Result<i32, DaemonError> {
    use std::ffi::CString;
    let path = CString::new("/dev/null").unwrap(); // Safe: known constant
    let fd = unsafe { syscalls::open(path.as_ptr(), 2, 0) }?; // O_RDWR
    Ok(fd as i32)
}

/// Closes a file descriptor using the `close` syscall.
///
/// # Arguments
/// * `fd` – The file descriptor to close.
///
/// # Returns
/// `Ok(())` on success, or `Err(DaemonError::CloseFailed)` if the syscall failed.
///
/// # Safety
/// Unsafe because it calls `syscall1`. The caller must ensure that `fd` is a
/// valid open descriptor and that closing it does not cause undefined behaviour
/// (e.g., a double close later).
pub fn close_fd(fd: i32) -> Result<(), DaemonError> {
    let ret = unsafe { syscall1(SYS_CLOSE, fd as i64) }?;
    if ret < 0 {
        Err(DaemonError::CloseFailed)
    } else {
        Ok(())
    }
}

/// Turns the calling process into a daemon.
///
/// This function performs the classic double‑fork daemonization sequence,
/// redirects standard file descriptors to `/dev/null`, and changes the working
/// directory to root.
///
/// # Important
/// - This function should be called early in `main()`, before any threads are
///   created and while the process is still single‑threaded. `fork()` in a
///   multi‑threaded program is extremely dangerous (only async‑signal‑safe
///   functions can be called in the child).
/// - The function will terminate the parent processes via `std::process::exit(0)`.
///   Any cleanup or resource release must be done before calling `daemonize()`.
/// - On error, a `DaemonError` is returned. The caller should log it and exit
///   with a non‑zero status, as daemonization is typically a fatal step.
///
/// # Returns
/// `Ok(())` in the final daemon process. The original parent and the
/// intermediate child are terminated inside the function.
///
/// # Errors
/// Returns a `DaemonError` if any system call fails during the process.
///
/// # Example
/// ```no_run
/// use daemon::daemonize;
///
/// fn main() {
///     if let Err(e) = daemonize() {
///         eprintln!("Failed to daemonize: {:?}", e);
///         std::process::exit(1);
///     }
///     // ... rest of daemon logic ...
/// }
/// ```
pub fn daemonize() -> Result<(), DaemonError> {
    // First fork: parent exits, child continues.
    match fork()? {
        0 => {} // Child continues.
        _ => {
            // Parent process exits immediately.
            std::process::exit(0);
        }
    }

    // Create a new session to detach from the terminal.
    setsid()?;

    // Second fork: ensure we cannot re‑acquire a controlling terminal.
    match fork()? {
        0 => {} // Grandchild continues.
        _ => std::process::exit(0),
    }

    // Change to root directory to avoid blocking unmounts.
    chdir("/")?;

    // Open /dev/null for redirecting standard file descriptors.
    let null_fd = open_devnull()?;

    // Redirect stdin, stdout, stderr to /dev/null.
    // If any dup2 fails, the error will propagate and abort daemonization.
    dup2(null_fd, 0)?; // stdin
    dup2(null_fd, 1)?; // stdout
    dup2(null_fd, 2)?; // stderr

    // Close the original fd; the duplicated ones remain open.
    close_fd(null_fd)?;

    Ok(())
}
