//! Raw Linux syscall interface for x86-64.
//!
//! This module provides the lowest‑level bindings to the Linux kernel syscall interface.
//! It links against a static library `libsyscall.a` (built from assembly sources) which
//! contains the actual `syscall` instruction wrappers. The functions `raw_syscall0` through
//! `raw_syscall6` are imported from that library and are `unsafe extern "C"` functions.
//!
//! The public API of this module consists of the `syscall0`…`syscall6` wrappers, which
//! invoke the raw syscalls and convert the kernel's convention (negative return values
//! indicate errors) into a `Result<i64, SyscallError>`. This allows idiomatic error
//! handling in Rust.
//!
//! Additionally, this module re‑exports everything from the `safe` submodule, which
//! contains type‑safe wrappers for specific system calls (e.g., `open`, `read`, `mmap`).
//!
//! # Safety
//!
//! All functions in this module are `unsafe` because they directly invoke the kernel
//! and require the caller to uphold the usual invariants for syscalls:
//! - Pointers must be valid, correctly aligned, and point to owned or appropriately
//!   borrowed memory.
//! - File descriptors must be valid and open.
//! - Arguments must obey Linux kernel semantics for the given syscall number.
//! - The syscall number must be correct for the operation.
//!
//! Incorrect usage can lead to undefined behavior, memory corruption, or security
//! vulnerabilities.

mod safe;

pub use safe::*;

// -----------------------------------------------------------------------------
// External assembly functions (from libsyscall.a)
// -----------------------------------------------------------------------------

// Link against the static library `libsyscall.a` which implements the raw syscall stubs.
// The library is built by the crate's build script from `asm/syscall.asm`.
#[link(name = "syscall", kind = "static")]
unsafe extern "C" {
    /// Invoke a syscall with 0 arguments.
    ///
    /// # Arguments
    /// * `num` - Syscall number.
    ///
    /// # Returns
    /// The raw return value from the kernel. Negative values indicate errors
    /// (errno negated), non‑negative values indicate success.
    fn raw_syscall0(num: i64) -> i64;

    /// Invoke a syscall with 1 argument.
    ///
    /// # Arguments
    /// * `num` - Syscall number.
    /// * `a1`  - First argument (passed in `rdi` to the kernel).
    fn raw_syscall1(num: i64, a1: i64) -> i64;

    /// Invoke a syscall with 2 arguments.
    ///
    /// # Arguments
    /// * `num` - Syscall number.
    /// * `a1`  - First argument (kernel `rdi`).
    /// * `a2`  - Second argument (kernel `rsi`).
    fn raw_syscall2(num: i64, a1: i64, a2: i64) -> i64;

    /// Invoke a syscall with 3 arguments.
    ///
    /// # Arguments
    /// * `num` - Syscall number.
    /// * `a1`  - First argument (kernel `rdi`).
    /// * `a2`  - Second argument (kernel `rsi`).
    /// * `a3`  - Third argument (kernel `rdx`).
    fn raw_syscall3(num: i64, a1: i64, a2: i64, a3: i64) -> i64;

    /// Invoke a syscall with 4 arguments.
    ///
    /// # Arguments
    /// * `num` - Syscall number.
    /// * `a1`  - First argument (kernel `rdi`).
    /// * `a2`  - Second argument (kernel `rsi`).
    /// * `a3`  - Third argument (kernel `rdx`).
    /// * `a4`  - Fourth argument (kernel `r10`).
    fn raw_syscall4(num: i64, a1: i64, a2: i64, a3: i64, a4: i64) -> i64;

    /// Invoke a syscall with 5 arguments.
    ///
    /// # Arguments
    /// * `num` - Syscall number.
    /// * `a1`  - First argument (kernel `rdi`).
    /// * `a2`  - Second argument (kernel `rsi`).
    /// * `a3`  - Third argument (kernel `rdx`).
    /// * `a4`  - Fourth argument (kernel `r10`).
    /// * `a5`  - Fifth argument (kernel `r8`).
    fn raw_syscall5(num: i64, a1: i64, a2: i64, a3: i64, a4: i64, a5: i64) -> i64;

    /// Invoke a syscall with 6 arguments.
    ///
    /// # Arguments
    /// * `num` - Syscall number.
    /// * `a1`  - First argument (kernel `rdi`).
    /// * `a2`  - Second argument (kernel `rsi`).
    /// * `a3`  - Third argument (kernel `rdx`).
    /// * `a4`  - Fourth argument (kernel `r10`).
    /// * `a5`  - Fifth argument (kernel `r8`).
    /// * `a6`  - Sixth argument (kernel `r9`, passed on the stack in the assembly stub).
    fn raw_syscall6(num: i64, a1: i64, a2: i64, a3: i64, a4: i64, a5: i64, a6: i64) -> i64;
}

// -----------------------------------------------------------------------------
// Safe‑ish wrappers that convert negative returns into `Result`.
// -----------------------------------------------------------------------------

/// Invoke a syscall with 0 arguments, returning a `Result`.
///
/// # Safety
///
/// Same as `raw_syscall0`; caller must ensure the syscall number is valid and
/// that the operation is safe in the current context.
#[inline]
pub unsafe fn syscall0(num: i64) -> Result<i64, SyscallError> {
    let ret = unsafe { raw_syscall0(num) };
    if ret < 0 {
        Err(SyscallError::from_raw(ret))
    } else {
        Ok(ret)
    }
}

/// Invoke a syscall with 1 argument, returning a `Result`.
///
/// # Safety
///
/// Same as `raw_syscall1`; caller must ensure the syscall number is correct
/// and the argument is valid for that syscall.
#[inline]
pub unsafe fn syscall1(num: i64, a1: i64) -> Result<i64, SyscallError> {
    let ret = unsafe { raw_syscall1(num, a1) };
    if ret < 0 {
        Err(SyscallError::from_raw(ret))
    } else {
        Ok(ret)
    }
}

/// Invoke a syscall with 2 arguments, returning a `Result`.
///
/// # Safety
///
/// Same as `raw_syscall2`; caller must ensure the syscall number is correct
/// and all arguments are valid.
#[inline]
pub unsafe fn syscall2(num: i64, a1: i64, a2: i64) -> Result<i64, SyscallError> {
    let ret = unsafe { raw_syscall2(num, a1, a2) };
    if ret < 0 {
        Err(SyscallError::from_raw(ret))
    } else {
        Ok(ret)
    }
}

/// Invoke a syscall with 3 arguments, returning a `Result`.
///
/// # Safety
///
/// Same as `raw_syscall3`; caller must ensure the syscall number is correct
/// and all arguments are valid.
#[inline]
pub unsafe fn syscall3(num: i64, a1: i64, a2: i64, a3: i64) -> Result<i64, SyscallError> {
    let ret = unsafe { raw_syscall3(num, a1, a2, a3) };
    if ret < 0 {
        Err(SyscallError::from_raw(ret))
    } else {
        Ok(ret)
    }
}

/// Invoke a syscall with 4 arguments, returning a `Result`.
///
/// # Safety
///
/// Same as `raw_syscall4`; caller must ensure the syscall number is correct
/// and all arguments are valid.
#[inline]
pub unsafe fn syscall4(num: i64, a1: i64, a2: i64, a3: i64, a4: i64) -> Result<i64, SyscallError> {
    let ret = unsafe { raw_syscall4(num, a1, a2, a3, a4) };
    if ret < 0 {
        Err(SyscallError::from_raw(ret))
    } else {
        Ok(ret)
    }
}

/// Invoke a syscall with 5 arguments, returning a `Result`.
///
/// # Safety
///
/// Same as `raw_syscall5`; caller must ensure the syscall number is correct
/// and all arguments are valid.
#[inline]
pub unsafe fn syscall5(
    num: i64,
    a1: i64,
    a2: i64,
    a3: i64,
    a4: i64,
    a5: i64,
) -> Result<i64, SyscallError> {
    let ret = unsafe { raw_syscall5(num, a1, a2, a3, a4, a5) };
    if ret < 0 {
        Err(SyscallError::from_raw(ret))
    } else {
        Ok(ret)
    }
}

/// Invoke a syscall with 6 arguments, returning a `Result`.
///
/// # Safety
///
/// Same as `raw_syscall6`; caller must ensure the syscall number is correct
/// and all arguments are valid.
#[inline]
pub unsafe fn syscall6(
    num: i64,
    a1: i64,
    a2: i64,
    a3: i64,
    a4: i64,
    a5: i64,
    a6: i64,
) -> Result<i64, SyscallError> {
    let ret = unsafe { raw_syscall6(num, a1, a2, a3, a4, a5, a6) };
    if ret < 0 {
        Err(SyscallError::from_raw(ret))
    } else {
        Ok(ret)
    }
}
