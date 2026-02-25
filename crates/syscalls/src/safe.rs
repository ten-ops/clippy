//! Raw Linux syscall wrappers for x86-64.
//!
//! This module provides safe (but still `unsafe` due to raw pointer handling) interfaces
//! to common Linux system calls. It relies on the low‑level `syscallN` functions
//! (implemented in assembly) to perform the actual syscall invocation.
//!
//! All functions return `Result<T, SyscallError>`, where `SyscallError` wraps the
//! negative error code returned by the kernel. On success, the syscall's return value
//! is converted to the appropriate Rust type.
//!
//! # Safety
//!
//! These functions are `unsafe` because they operate on raw pointers and make assumptions
//! about the validity of memory regions, file descriptors, and process state. Callers
//! must ensure:
//! - Pointers are valid, properly aligned, and point to owned or appropriately borrowed
//!   memory.
//! - File descriptors refer to open, valid file handles.
//! - Syscall arguments obey Linux kernel semantics (e.g., flags, prot, etc.).
//! - The calling context is suitable for the requested operation (e.g., signals are
//!   handled appropriately for `kill`).
//!
//! # Note
//!
//! Syscall numbers are hardcoded for the Linux x86-64 ABI. This module is not portable
//! to other architectures or operating systems.

use crate::{syscall1, syscall2, syscall3, syscall5, syscall6};

/// Error type representing a failed system call.
///
/// Wraps the negated error code returned by the kernel (e.g., -EINVAL becomes `SyscallError(22)`).
/// This type is `Copy`, `Clone`, and comparable.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct SyscallError(pub i32);

// -----------------------------------------------------------------------------
// Linux x86-64 syscall numbers
// -----------------------------------------------------------------------------
const SYS_READ: i64 = 0;
const SYS_WRITE: i64 = 1;
const SYS_OPEN: i64 = 2;
const SYS_CLOSE: i64 = 3;
const SYS_MMAP: i64 = 9;
const SYS_PRCTL: i64 = 157;
const SYS_SOCKET: i64 = 41;
const SYS_CONNECT: i64 = 42;
const SYS_KILL: i64 = 62;

impl SyscallError {
    /// Constructs a `SyscallError` from a raw syscall return value.
    ///
    /// # Panics
    ///
    /// In debug builds, this function will panic if `ret` is not negative.
    /// In release builds, the negative value is simply negated and stored.
    ///
    /// # Arguments
    ///
    /// * `ret` - A negative integer returned by a syscall, representing an error.
    #[inline]
    pub fn from_raw(ret: i64) -> Self {
        debug_assert!(
            ret < 0,
            "SyscallError::from_raw called with a non-negative value"
        );
        Self((-ret) as i32)
    }
}

/// Opens a file.
///
/// Wrapper for the `open` system call (number 2).
///
/// # Arguments
///
/// * `path` - Pointer to a null‑terminated C string containing the path to open.
/// * `flags` - Open flags (e.g., `O_RDONLY`, `O_WRONLY`, `O_CREAT`). See `fcntl.h`.
/// * `mode` - Permission mode bits used when creating a file (ignored otherwise).
///
/// # Returns
///
/// On success, returns the new file descriptor (`i32`). On error, returns `SyscallError`.
///
/// # Safety
///
/// The caller must ensure that `path` points to a valid, null‑terminated string and that
/// the memory remains valid for the duration of the syscall.
#[inline]
pub unsafe fn open(path: *const i8, flags: i32, mode: i32) -> Result<i32, SyscallError> {
    Ok(unsafe { syscall3(SYS_OPEN, path as i64, flags as i64, mode as i64)? as i32 })
}

/// Closes a file descriptor.
///
/// Wrapper for the `close` system call (number 3).
///
/// # Arguments
///
/// * `fd` - File descriptor to close.
///
/// # Returns
///
/// `Ok(())` on success, `Err(SyscallError)` on failure.
///
/// # Safety
///
/// The caller must ensure that `fd` is a valid file descriptor (i.e., was returned by a
/// successful `open`, `socket`, etc.) and that it is not used concurrently in a way that
/// would cause a race condition (e.g., closing while another thread is using it).
#[inline]
pub unsafe fn close(fd: i32) -> Result<(), SyscallError> {
    let _ = unsafe { syscall1(SYS_CLOSE, fd as i64)? };
    Ok(())
}

/// Sends a signal to a process.
///
/// Wrapper for the `kill` system call (number 62).
///
/// # Arguments
///
/// * `child_id` - Process ID to signal.
/// * `sigterm` - Signal number to send (e.g., `SIGTERM`, `SIGKILL`).
///
/// # Returns
///
/// `Ok(())` on success, `Err(SyscallError)` on failure (e.g., invalid PID or permission denied).
///
/// # Safety
///
/// The caller must ensure that sending the signal is appropriate in the current context.
/// Improper use can cause undefined behavior in the target process.
#[inline]
pub unsafe fn kill(child_id: i32, sigterm: i32) -> Result<(), SyscallError> {
    let _ = unsafe { syscall2(SYS_KILL, child_id as i64, sigterm as i64)? };
    Ok(())
}

/// Reads data from a file descriptor into a buffer.
///
/// Wrapper for the `read` system call (number 0).
///
/// # Arguments
///
/// * `fd` - File descriptor to read from.
/// * `buf` - Mutable slice of bytes that will be filled with the read data.
///
/// # Returns
///
/// The number of bytes read (possibly zero on EOF). On error, returns `SyscallError`.
///
/// # Safety
///
/// The caller must ensure that `fd` is open for reading and that the buffer is valid
/// and writable for the requested length. This function may block.
#[inline]
pub unsafe fn read(fd: i32, buf: &mut [u8]) -> Result<usize, SyscallError> {
    let ret = unsafe {
        syscall3(
            SYS_READ,
            fd as i64,
            buf.as_mut_ptr() as i64,
            buf.len() as i64,
        )?
    };
    Ok(ret as usize)
}

/// Writes data from a buffer to a file descriptor.
///
/// Wrapper for the `write` system call (number 1).
///
/// # Arguments
///
/// * `fd` - File descriptor to write to.
/// * `buf` - Slice of bytes to write.
///
/// # Returns
///
/// The number of bytes written. On error, returns `SyscallError`.
///
/// # Safety
///
/// The caller must ensure that `fd` is open for writing and that the buffer contains
/// valid data for the entire length. Partial writes are possible; the caller should
/// handle retrying if necessary.
#[inline]
pub unsafe fn write(fd: i32, buf: &[u8]) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall3(SYS_WRITE, fd as i64, buf.as_ptr() as i64, buf.len() as i64)? };
    Ok(ret as usize)
}

/// Maps files or devices into memory.
///
/// Wrapper for the `mmap` system call (number 9).
///
/// # Arguments
///
/// * `address` - Preferred starting address for the mapping, or `0` to let the kernel choose.
/// * `length`  - Number of bytes to map.
/// * `prot`    - Memory protection flags (e.g., `PROT_READ | PROT_WRITE`).
/// * `flags`   - Mapping flags (e.g., `MAP_PRIVATE`, `MAP_ANONYMOUS`).
/// * `fd`      - File descriptor to map (ignored for anonymous mappings).
/// * `offset`  - Offset within the file where mapping begins (must be page‑aligned).
///
/// # Returns
///
/// On success, returns a pointer to the mapped memory. On error, returns `SyscallError`.
///
/// # Safety
///
/// This function is extremely unsafe. The caller must ensure:
/// - The returned pointer is properly dereferenceable within the mapped range.
/// - All invariants of `mmap` (alignment, offset, fd validity) are upheld.
/// - The mapping is eventually unmapped with `munmap` (not provided here) to avoid leaks.
/// - Concurrent access adheres to the specified protection and sharing flags.
#[inline]
pub unsafe fn mmap(
    address: *mut u8,
    length: usize,
    prot: i32,
    flags: i32,
    fd: i32,
    offset: i64,
) -> Result<*mut u8, SyscallError> {
    let ret = unsafe {
        syscall6(
            SYS_MMAP,
            address as i64,
            length as i64,
            prot as i64,
            flags as i64,
            fd as i64,
            offset,
        )?
    };
    Ok(ret as *mut u8)
}

/// Manipulates process‐specific parameters (prctl).
///
/// Wrapper for the `prctl` system call (number 157).
///
/// # Arguments
///
/// * `option` - Which operation to perform (e.g., `PR_SET_NAME`, `PR_GET_DUMPABLE`).
/// * `arg2`..`arg5` - Operation‑specific arguments (often integers or pointers).
///
/// # Returns
///
/// On success, returns the integer result of the `prctl` call (interpretation depends on
/// the option). On error, returns `SyscallError`.
///
/// # Safety
///
/// The meaning of the arguments varies by `option`. The caller must ensure that any
/// pointers passed via `arg2`-`arg5` are valid and correctly aligned. Incorrect use
/// can corrupt process state or leak information.
#[inline]
pub unsafe fn prctl(
    option: i32,
    arg2: i64,
    arg3: i64,
    arg4: i64,
    arg5: i64,
) -> Result<i64, SyscallError> {
    let ret = unsafe { syscall5(SYS_PRCTL, option as i64, arg2, arg3, arg4, arg5)? };
    Ok(ret)
}

/// Unix domain socket address structure.
///
/// Matches the layout of `struct sockaddr_un` in C.
#[repr(C)]
pub struct SockAddrUn {
    /// Address family (always `AF_UNIX` = 1).
    pub sun_family: u16,
    /// Path (null‑terminated string) for the socket.
    pub sun_path: [i8; 108],
}

/// Creates a socket.
///
/// Wrapper for the `socket` system call (number 41).
///
/// # Arguments
///
/// * `domain`   - Protocol family (e.g., `AF_UNIX`, `AF_INET`).
/// * `type_`    - Socket type (e.g., `SOCK_STREAM`, `SOCK_DGRAM`).
/// * `protocol` - Specific protocol (usually 0 for default).
///
/// # Returns
///
/// On success, returns the new socket file descriptor. On error, returns `SyscallError`.
///
/// # Safety
///
/// The caller must ensure that the provided parameters are valid for the kernel version
/// and that the returned descriptor will be closed appropriately.
#[inline]
pub unsafe fn socket(domain: i32, type_: i32, protocol: i32) -> Result<i32, SyscallError> {
    let ret = unsafe { syscall3(SYS_SOCKET, domain as i64, type_ as i64, protocol as i64)? };
    Ok(ret as i32)
}

/// Connects a socket to a peer address.
///
/// Wrapper for the `connect` system call (number 42).
///
/// # Arguments
///
/// * `fd`      - Socket file descriptor.
/// * `socket`  - Pointer to a `SockAddrUn` (or other sockaddr structure) containing the address.
/// * `len`     - Size of the address structure (should be `std::mem::size_of::<SockAddrUn>()`).
///
/// # Returns
///
/// `Ok(())` on success, `Err(SyscallError)` on failure.
///
/// # Safety
///
/// The caller must ensure that `socket` points to a valid, correctly initialized address
/// structure of the appropriate type and that the address length matches the actual
/// structure. For Unix domain sockets, the path must be a null‑terminated string.
#[inline]
pub unsafe fn connect(fd: i32, socket: *const SockAddrUn, len: u32) -> Result<(), SyscallError> {
    let _ = unsafe { syscall3(SYS_CONNECT, fd as i64, socket as i64, len as i64)? };
    Ok(())
}
