//! Integration tests for raw Linux syscall wrappers.
//!
//! This module contains a suite of tests that exercise the syscall wrappers
//! defined in the parent module. The tests verify correct behavior for
//! success cases, expected error conditions, and edge cases (e.g., invalid
//! file descriptors, non‑existent paths).
//!
//! All tests are marked `unsafe` because they invoke the raw syscall wrappers,
//! which require the caller to uphold the usual kernel invariants. In the
//! context of a controlled test environment, these operations are safe as
//! long as the assumptions (e.g., existence of `/dev/zero`, `/dev/null`) hold.
//!
//! The tests rely on the following syscalls: `open`, `read`, `write`, `close`,
//! `mmap`, `prctl`, `socket`, `connect`. They use the safe wrapper functions
//! from the `syscalls` module.

use std::{ffi::CString, ptr};

use syscalls::{SockAddrUn, close, connect, mmap, open, prctl, read, socket, write};

// -----------------------------------------------------------------------------
// Common error codes (from errno.h) used in assertions
// -----------------------------------------------------------------------------
const EBADF: i32 = 9; // Bad file descriptor
const EINVAL: i32 = 22; // Invalid argument
const ENOENT: i32 = 2; // No such file or directory

// Socket constants (from sys/socket.h)
const AF_UNIX: i32 = 1; // Unix domain sockets
const SOCK_STREAM: i32 = 1; // Stream socket (TCP‑like)

/// Tests that opening `/dev/zero`, reading from it, and then closing works correctly.
///
/// # Expected behavior
/// 1. `open("/dev/zero")` returns a valid file descriptor (≥ 0).
/// 2. `read()` from that descriptor fills a buffer with zeros and returns the full length.
/// 3. `close()` succeeds.
/// 4. After closing, a subsequent `read()` on the same descriptor fails with `EBADF`.
///
/// # Safety
/// Uses raw syscalls; assumes `/dev/zero` exists and is readable.
#[test]
fn test_open_read_close() {
    let path = CString::new("/dev/zero").unwrap();
    unsafe {
        let fd = open(path.as_ptr(), 0, 0).expect("/dev/zero failed to open");
        assert!(fd >= 0);

        let mut buf = [0u8; 16];
        let n = read(fd, &mut buf).unwrap();
        assert_eq!(n, 16);
        assert_eq!(buf, [0u8; 16]);

        close(fd).expect("failed to close fd");
        let err = read(fd, &mut buf).unwrap_err();
        assert_eq!(err.0, EBADF);
    }
}

/// Tests that writing to `/dev/null` consumes all data without error.
///
/// # Expected behavior
/// 1. `open("/dev/null")` returns a valid file descriptor.
/// 2. `write()` of a byte slice returns the full length.
/// 3. `close()` succeeds.
///
/// # Safety
/// Assumes `/dev/null` exists and is writable.
#[test]
fn test_write_to_dev_null() {
    let path = CString::new("/dev/null").unwrap();
    unsafe {
        let fd = open(path.as_ptr(), 1, 0).unwrap();
        assert!(fd >= 0);

        let data = b"hello from syscall";
        let n = write(fd, data).unwrap();
        assert_eq!(n, data.len());

        close(fd).expect("close failed");
    }
}

/// Tests that opening a non‑existent path returns the expected error (`ENOENT`).
///
/// # Expected behavior
/// `open("/not/a/valid/path")` fails with error code `ENOENT`.
///
/// # Safety
/// The path pointer is valid, and the syscall is invoked correctly.
#[test]
fn test_open_non_existing() {
    let path = CString::new("/not/a/valid/path").unwrap();
    unsafe {
        let err = open(path.as_ptr(), 0, 0).unwrap_err();
        assert_eq!(err.0, ENOENT);
    }
}

/// Tests that `close()` on an invalid file descriptor returns `EBADF`.
///
/// # Expected behavior
/// `close(999_999)` fails with `EBADF`.
///
/// # Safety
/// The file descriptor is obviously invalid, but calling `close` on it is safe
/// (it will just return an error).
#[test]
fn test_close_invalid_fd() {
    unsafe {
        let err = close(999_999).unwrap_err();
        assert_eq!(err.0, EBADF);
    }
}

/// Tests that `mmap()` with obviously invalid arguments returns `EINVAL`.
///
/// # Expected behavior
/// `mmap(NULL, 0, 0, 32, -1, 0)` fails with `EINVAL`.
/// (Flags value 32 is `MAP_ANONYMOUS`; but length 0 and invalid fd trigger error.)
///
/// # Safety
/// The arguments are invalid, but the call is safe (no pointers are dereferenced).
#[test]
fn test_mmap_invalid() {
    unsafe {
        let err = mmap(ptr::null_mut(), 0, 0, 32, -1, 0).unwrap_err();
        assert_eq!(err.0, EINVAL);
    }
}

/// Tests that `prctl()` with an invalid option returns `EINVAL`.
///
/// # Expected behavior
/// `prctl(-1, ...)` fails with `EINVAL`.
///
/// # Safety
/// The option is invalid, but the call does not cause undefined behavior.
#[test]
fn test_prctl() {
    unsafe {
        let err = prctl(-1, 0, 0, 0, 0).unwrap_err();
        assert_eq!(err.0, EINVAL);
    }
}

/// Tests basic creation of a Unix domain socket.
///
/// # Expected behavior
/// 1. `socket(AF_UNIX, SOCK_STREAM, 0)` returns a valid file descriptor.
/// 2. The descriptor can be closed successfully.
///
/// # Safety
/// Assumes the kernel supports Unix domain sockets (always true on Linux).
#[test]
fn test_socket_creation() {
    unsafe {
        let fd = socket(AF_UNIX, SOCK_STREAM, 0).expect("failed to create socket");
        assert!(fd >= 0);
        close(fd).expect("failed to close socket");
    }
}

/// Tests that attempting to connect a Unix socket to a non‑existent path fails with `ENOENT`.
///
/// # Steps
/// 1. Create a Unix stream socket.
/// 2. Prepare a `SockAddrUn` structure with a path that does not exist.
/// 3. Call `connect()` - it should fail with `ENOENT`.
/// 4. Close the socket.
///
/// # Safety
/// - The socket descriptor is valid.
/// - The `SockAddrUn` structure is properly zeroed and the path is correctly copied
///   (null‑terminated and within bounds).
/// - The call to `connect` is safe because the address structure is correctly initialized,
///   even though the path does not exist.
#[test]
fn test_invalid_socket_connection() {
    unsafe {
        let fd = socket(AF_UNIX, SOCK_STREAM, 0).expect("socket fd creation failed");
        let mut socket: SockAddrUn = std::mem::zeroed();
        socket.sun_family = AF_UNIX as u16;

        // Copy the path "/invalid/socket/path\0" into sun_path.
        // The path is null‑terminated; if it exceeds 108 bytes, truncation would occur,
        // but this path is well within the limit.
        let path = b"/invalid/socket/path\0";
        for (i, &b) in path.iter().enumerate() {
            if i < socket.sun_path.len() {
                socket.sun_path[i] = b as i8;
            }
        }

        let len = std::mem::size_of::<SockAddrUn>() as u32;
        let err = connect(fd, &socket as *const _, len).unwrap_err();

        assert_eq!(err.0, ENOENT);

        close(fd).expect("failed to close socket fd");
    }
}
