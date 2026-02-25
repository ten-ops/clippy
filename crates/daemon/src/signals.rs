//! Raw signal handling using direct Linux system calls (no libc).
//!
//! This module sets up signal handlers for `SIGINT` and `SIGTERM` using the
//! `rt_sigaction` syscall. It defines a global atomic flag `RUNNING` that is
//! set to `false` when either signal is received, allowing the main loop to
//! shut down gracefully.
//!
//! # Why raw syscalls?
//!
//! The code avoids the libc wrapper to have precise control over the signal
//! handler installation, particularly the use of a custom signal restorer
//! written in assembly. This is necessary because the kernel, when returning
//! from a signal handler, expects a specific restorer function that performs
//! the `rt_sigreturn` syscall. On modern Linux, the `SA_RESTORER` flag must
//! be provided, and the restorer address must point to a function that invokes
//! the `rt_sigreturn` syscall (number 15 on x86_64).
//!
//! # Safety
//!
//! Signal handlers are extremely restricted: they can only call async‑signal‑safe
//! functions. This handler only writes to an atomic variable, which is safe
//! because `AtomicBool` operations are lock‑free and signal‑safe on all platforms
//! that Rust supports.
//!
//! The assembly restorer is also safe because it consists of a single `syscall`
//! instruction and does not touch any memory beyond what the kernel expects.
//!
//! # Platform
//!
//! This code is specific to Linux on x86_64. It will not compile or work on
//! other architectures or operating systems.

use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use syscalls::{SyscallError, syscall4};

// Signal numbers for the signals we want to catch.
const SIGINT: i32 = 2;
const SIGTERM: i32 = 15;

// System call number for rt_sigaction on x86_64.
const SYS_RT_SIGACTION: i64 = 13;

// Flag indicating that we are providing a custom restorer function.
const SA_RESTORER: u64 = 0x04000000;

/// Global flag indicating whether the daemon should keep running.
///
/// Initially `true`. When a signal (`SIGINT` or `SIGTERM`) is received, the
/// signal handler sets this to `false`. The main loop should periodically
/// check this flag and exit cleanly when it becomes `false`.
///
/// # Ordering
/// The signal handler uses `SeqCst` ordering to ensure that the store is
/// visible to all threads as soon as possible. The main loop may use a weaker
/// ordering (e.g., `Relaxed` or `Acquire`) because the exact moment of
/// visibility is not critical – the loop will see the change eventually.
pub static RUNNING: AtomicBool = AtomicBool::new(true);

/// Signal handler for `SIGINT` and `SIGTERM`.
///
/// This function is called by the kernel when the signal is delivered.
/// It simply sets `RUNNING` to `false`.
///
/// # Safety
/// This function must only perform async‑signal‑safe operations.
/// Writing to an `AtomicBool` meets that requirement.
extern "C" fn handle_signal(_: i32) {
    RUNNING.store(false, Ordering::SeqCst);
}

core::arch::global_asm!(
    ".global _clipper_sigreturn_restorer",
    "_clipper_sigreturn_restorer:",
    "mov rax, 15",
    "syscall",
);

// Declare the external symbol so we can take its address.
unsafe extern "C" {
    fn _clipper_sigreturn_restorer();
}

/// Structure matching the kernel's `sigaction` layout for the `rt_sigaction` syscall.
///
/// This struct must exactly match what the Linux kernel expects on x86_64.
/// Fields:
/// - `sa_handler`: address of the signal handler function.
/// - `sa_flags`: flags modifying the behaviour (e.g., `SA_RESTORER`).
/// - `sa_restorer`: address of the restorer function (used only if `SA_RESTORER` is set).
/// - `sa_mask`: signal mask to block during handler execution (here, empty).
///
/// The struct is `repr(C)` to prevent Rust from reordering fields.
#[repr(C)]
struct SigAction {
    sa_handler: usize,
    sa_flags: u64,
    sa_restorer: usize,
    sa_mask: u64,
}

/// Installs signal handlers for `SIGINT` and `SIGTERM`.
///
/// This function uses the `rt_sigaction` syscall directly. It sets up a
/// `SigAction` with the custom handler, the `SA_RESTORER` flag, and the
/// assembly restorer. The signal mask is empty.
///
/// # Returns
/// * `Ok(())` on success.
/// * `Err(SyscallError)` if the syscall fails (e.g., invalid arguments).
///
/// # Safety
/// This function is unsafe because it performs raw system calls and relies on
/// the correctness of the assembly restorer. However, the implementation is
/// carefully crafted to be safe under the assumption that the syscall numbers
/// and struct layouts are correct for the target platform.
///
/// # Panics
/// This function does not panic, but it will return an error if the syscall fails.
///
/// # Example
/// ```
/// use daemon::install_handlers;
///
/// if let Err(e) = install_handlers() {
///     eprintln!("Failed to install signal handlers");
///     std::process::exit(1);
/// }
/// // ... main loop, checking RUNNING periodically ...
/// ```
pub fn install_handlers() -> Result<(), SyscallError> {
    let act = SigAction {
        sa_handler: handle_signal as usize,
        sa_flags: SA_RESTORER,
        sa_restorer: _clipper_sigreturn_restorer as usize,
        sa_mask: 0,
    };

    // Install the same handler for both SIGINT and SIGTERM.
    for &signum in &[SIGINT, SIGTERM] {
        // The fourth argument to rt_sigaction is the size of the signal set (8 bytes on x86_64).
        let res = unsafe {
            syscall4(
                SYS_RT_SIGACTION,
                signum as i64,
                &act as *const _ as i64,
                ptr::null::<SigAction>() as i64,
                8,
            )?
        };
        if res < 0 {
            return Err(SyscallError::from_raw(res));
        }
    }
    Ok(())
}
