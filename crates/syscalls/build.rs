//! Build script for compiling the raw syscall assembly library.
//!
//! This script is executed by Cargo when building the crate. It:
//! 1. Checks for the presence of `nasm` and `ar` in the system PATH.
//! 2. Compiles `asm/syscall.asm` into a 64‑bit ELF object file.
//! 3. Archives the object file into a static library `libsyscall.a`.
//! 4. Instructs Cargo to link against that static library.
//!
//! The script is automatically re‑run whenever `asm/syscall.asm` changes,
//! ensuring the assembly is rebuilt when needed.

use std::{env, path::PathBuf, process::Command};

fn main() {
    // -------------------------------------------------------------------------
    // Rerun trigger: if the assembly source changes, rebuild.
    // -------------------------------------------------------------------------
    let syscall_src_file = "./asm/syscall.asm";
    // Tell Cargo to rerun this build script if the assembly file is modified.
    println!("cargo:rerun-if-changed={}", syscall_src_file);

    // -------------------------------------------------------------------------
    // Dependency checks: ensure NASM (assembler) and ar (archiver) are installed.
    // -------------------------------------------------------------------------
    // Check for NASM by attempting to run `nasm --version`.
    // If the command fails or is not found, panic with a helpful error message.
    let nasm = match Command::new("nasm").arg("--version").output() {
        Ok(output) if output.status.success() => "nasm",
        _ => panic!("NASM not found. Install nasm (apt install nasm)."),
    };

    // Similarly, check for `ar` (part of GNU binutils).
    let ar = match Command::new("ar").arg("--version").output() {
        Ok(output) if output.status.success() => "ar",
        _ => panic!("ar not found. Install binutils (apt install binutils)."),
    };

    // -------------------------------------------------------------------------
    // Prepare output paths inside Cargo's OUT_DIR.
    // -------------------------------------------------------------------------
    // OUT_DIR is a temporary directory unique to each build, ensuring clean builds.
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let obj_file = out_dir.join("syscall.o"); // Temporary object file
    let lib_file = out_dir.join("libsyscall.a"); // Final static library

    // -------------------------------------------------------------------------
    // Step 1: Assemble the .asm file into an ELF64 object file.
    // -------------------------------------------------------------------------
    let nasm_status = Command::new(nasm)
        .args(&["-f", "elf64", syscall_src_file, "-o"])
        .arg(&obj_file)
        .status()
        .expect("nasm execution failed"); // Panic if the command cannot be spawned.
    if !nasm_status.success() {
        panic!("NASM compilation failed");
    }

    // -------------------------------------------------------------------------
    // Step 2: Create a static library archive containing the object file.
    // -------------------------------------------------------------------------
    // `ar crs` creates an archive (c), replacing existing members (r), and
    // generating an index (s) for faster linking.
    let ar_status = Command::new(ar)
        .args(&[
            "crs",
            &lib_file.to_string_lossy(), // Convert Path to string slice
            &obj_file.to_string_lossy(),
        ])
        .status()
        .expect("ar execution failed");
    if !ar_status.success() {
        panic!("ar static linking failed");
    }

    // -------------------------------------------------------------------------
    // Step 3: Instruct Cargo to link against the produced static library.
    // -------------------------------------------------------------------------
    // Tell the linker to search for libraries in `out_dir`.
    println!("cargo:rustc-link-search=native={}", out_dir.display());
    // Link `libsyscall.a` (the `static=syscall` part adds `-l syscall`).
    println!("cargo:rustc-link-lib=static=syscall");

    // Emit a warning (visible in build output) to confirm the library location.
    // This is optional but helpful for debugging.
    println!(
        "cargo:warning=Linked libsyscall.a from {}",
        out_dir.display()
    );

    // Note: The generated static library exports the raw_syscall* symbols
    // defined in the assembly. Rust code can then declare them as `extern`
    // functions and call them directly.
}
