# 📋 Clippy. A Zero-Dependency Linux Clipboard Monitor Daemon

> **A blazing-fast, zero-libc X11 clipboard monitoring daemon written in Rust - with raw syscalls, hand-rolled assembly, and no external X11 libraries.**

---

![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg) [![Rust Version](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org/) [![Platform](https://img.shields.io/badge/platform-Linux%20x86__64-lightgrey.svg)](https://kernel.org/) ![PRs Welcome](https://img.shields.io/badge/PRs-welcome-brightgreen.svg) [![GitHub Stars](https://img.shields.io/github/stars/ten-ops/clippy?style=social)](https://github.com/ten-ops/clippy/stargazers)

---

## 🔍 What Is Clippy?

**Clippy** is a lightweight, production-grade Linux clipboard monitoring daemon that watches the **X11 CLIPBOARD selection** for changes and fires a user-defined handler for every new clipboard event - with **zero dependency on libX11, libxcb, or any C runtime library**.

Instead of pulling in heavyweight display libraries, Clippy speaks the **raw X11 wire protocol** directly over a Unix domain socket, authenticates via **MIT-MAGIC-COOKIE-1** from `~/.Xauthority`, uses the **XFixes extension** (`XFixesSelectSelectionInput`) for efficient change detection, and drops to a proper POSIX background daemon via a **double-fork** with all I/O redirected to `/dev/null`.

Every system call - `fork`, `setsid`, `dup2`, `read`, `write`, `socket`, `connect`, `mmap`, `prctl`, `rt_sigaction` - is dispatched through a **custom NASM assembly stub layer** compiled into a static library and linked at build time. No glibc wrapper overhead. No hidden allocations. No surprises.

---

## ✨ Key Features

- **Zero external X11 dependencies** - raw X11 wire protocol over Unix sockets
- **XFixes-based clipboard monitoring** - event-driven, not polling
- **UTF-8 and STRING fallback** - gracefully handles both modern and legacy clipboard owners
- **MIT-MAGIC-COOKIE-1 authentication** - reads `~/.Xauthority` automatically
- **Hand-rolled syscall layer** - NASM assembly stubs for all Linux syscalls, statically linked
- **Double-fork daemonization** - proper POSIX daemon with `setsid`, `chdir("/")`, and `/dev/null` I/O
- **Raw signal handling** - custom `rt_sigaction` with `SA_RESTORER` and a hand-written `rt_sigreturn` trampoline
- **Exponential backoff reconnection** - configurable delay, max delay, and multiplier
- **Telemetry & metrics** - atomic counters for clipboard events, retries, EINTR, and failed fetches
- **Pluggable `ClipboardBackend` trait** - swap backends without touching daemon logic
- **Criterion benchmarks** - latency measurement for clipboard fetch round-trips
- **Fully environment-variable-driven configuration** - no config files needed
- **`make`-driven workflow** - one Makefile to build, test, lint, bench, install, and document

---

## 🏗️ Architecture

Clippy is a **Cargo workspace** composed of four focused crates:

```
clippy/
├── benches/
├── crates/
│   ├── daemon/               # Binary: main loop, signal handling, daemonization, reconnect logic
│   ├── monitor/              # Library: ClipboardBackend trait + X11 backend implementation
│   ├── syscalls/             # Library: NASM assembly stubs + safe Rust wrappers
│   └── telemetry/            # Library: atomic metrics counters + pluggable sinks
├── asm/
│   └── syscall.asm           # Raw x86-64 syscall stubs (NASM)
├── benches/
│   └── clipboard_latency.rs  # Criterion latency benchmark
├── src/
├── tests/
└── Makefile                  # All common development tasks
```

### Data Flow

```
X Server (XFixes SelectionNotify)
        │
        ▼
  X11Connection::run_clipboard_monitor()
        │  raw X11 wire protocol over /tmp/.X11-unix/X{N}
        ▼
  get_clipboard()
        │  ConvertSelection → SelectionNotify → GetProperty
        ▼
  handler(Vec<u8>)
        │
        ▼
  Metrics::inc_clipboard_event()
```

---

## 🚀 Getting Started

### Prerequisites

|Requirement|Version|Notes|
|---|---|---|
|Rust toolchain|stable|`rustup update stable`|
|NASM assembler|≥ 2.14|`apt install nasm`|
|GNU binutils (`ar`)|any|usually pre-installed|
|Linux kernel|x86-64|other architectures are not supported|
|X11 display server|any|XFixes extension must be present|

> **Check XFixes availability:** `xdpyinfo | grep xfixes`

### Quick Start

```bash
git clone https://github.com/ten-ops/clippy.git
cd clippy

# Debug build
make

# Release build
make release

# Run immediately (debug, DISPLAY=:0)
make run

# Run in release mode
make run-release
```

The `build.rs` script automatically compiles `asm/syscall.asm` via NASM, archives it into `libsyscall.a`, and links it statically into the final binary. If NASM is not found, the build fails loudly with a clear error message.

### Install System-Wide

```bash
# Installs the daemon binary to ~/.cargo/bin
make install

# Uninstall
make uninstall
```

---

## 🛠️ Makefile Reference

All common development tasks are available via `make`. Run `make help` for a quick summary at any time.

|Target|Command|Description|
|---|---|---|
|`all` / `build`|`make`|Build debug version (default target)|
|`release`|`make release`|Build optimised release binary|
|`run`|`make run`|Run daemon in debug mode (`DISPLAY=:0`)|
|`run-release`|`make run-release`|Run daemon in release mode (`DISPLAY=:0`)|
|`test`|`make test`|Run all tests across the entire workspace|
|`test-verbose`|`make test-verbose`|Run tests with captured stdout/stderr output|
|`bench`|`make bench`|Run Criterion benchmarks|
|`check`|`make check`|Fast workspace check (no codegen)|
|`fmt`|`make fmt`|Format all code with `rustfmt`|
|`lint`|`make lint`|Run `clippy` with `-D warnings`|
|`doc`|`make doc`|Generate and open rustdoc in browser|
|`install`|`make install`|Install daemon binary to `~/.cargo/bin`|
|`uninstall`|`make uninstall`|Remove installed daemon binary|
|`clean`|`make clean`|Remove all build artefacts|
|`help`|`make help`|Print available targets|

> **Note:** `DISPLAY` defaults to `:1` in the Makefile's `run` and `run-release` targets. Override inline: `DISPLAY=:0 make run`.

---

## ⚙️ Configuration

All configuration is via **environment variables**. There are no config files. This makes Clippy trivially integrable with `systemd`, Docker, or any process supervisor.

|Variable|Default|Description|
|---|---|---|
|`CLIPPER_DAEMONIZE`|`false`|Set to `1`, `true`, or `yes` to fork to background|
|`CLIPPER_RECONNECT_DELAY`|`2`|Base reconnection delay in seconds|
|`CLIPPER_RECONNECT_MAX_DELAY`|`30`|Maximum reconnection delay (exponential backoff cap)|
|`CLIPPER_RECONNECT_BACKOFF_MULTIPLIER`|`2.0`|Backoff growth multiplier per retry (must be ≥ 1.0)|
|`CLIPPER_LOG_LEVEL`|`info`|Log verbosity: `info`, `warn`, or `error`|
|`CLIPPER_LOG_DEST`|`stdout`|Log destination: `stdout` or `stderr`|
|`CLIPPER_LOG_FILE`|_(unset)_|Path to log file (daemon mode only)|
|`CLIPPER_METRICS_INTERVAL_SECONDS`|`60`|How often the periodic metrics sink reports|
|`DISPLAY`|_(required)_|X11 display string, e.g. `:0` or `unix:0`|
|`XAUTHORITY`|`~/.Xauthority`|Path to X authority file (auto-detected)|

### Example: systemd User Service

```ini
[Unit]
Description=Clippy Clipboard Monitor Daemon
After=graphical-session.target

[Service]
Type=simple
ExecStart=%h/.cargo/bin/daemon
Environment=DISPLAY=:0
Environment=CLIPPER_LOG_DEST=stderr
Environment=CLIPPER_METRICS_INTERVAL_SECONDS=30
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
```

---

## 🧪 Testing

### Run the Test Suite

```bash
# All crates
make test

# With captured output (useful for debugging failures)
make test-verbose

# Target a specific crate directly
cargo test -p syscalls
cargo test -p monitor
DISPLAY=:1 cargo test -p daemon
```

The **integration test suite** (`daemon_tests.rs`) covers daemon exits cleanly when `DISPLAY` is unset or malformed, SIGTERM and SIGINT handling with graceful shutdown, reconnection behaviour under simulated transient failures, and metrics counters incrementing correctly across events.

The **syscall test suite** (`syscall_tests.rs`) covers `open`/`read`/`close` on `/dev/zero` and `/dev/null`, error paths for `ENOENT`, `EBADF`, and `EINVAL`, `mmap` with invalid arguments, `prctl` with unknown options, and Unix domain socket creation and failed connect.

### Benchmarks

> **Prerequisite:** The clipboard must contain content before running (copy any text in any application first). The benchmark measures only the success path.

```bash
# Ensure DISPLAY is set and clipboard has content, then:
make bench
```

This runs the `clipboard_fetch` Criterion benchmark - measuring the full round-trip latency of a single `ConvertSelection → SelectionNotify → GetProperty` cycle.

---

### Signal Handling

Signal handlers use `rt_sigaction` with `SA_RESTORER`. The restorer is a **naked global assembly function** (`_clippy_sigreturn_restorer`) that calls `sys_rt_sigreturn` directly - required on Linux x86-64 to correctly unwind signal stack frames without glibc's `__restore_rt`. Without it, signal delivery corrupts the stack. With it, it just works.

---

## 🏛️ X11 Backend: Implementation Notes

The X11 backend (`monitor/src/backend/x11.rs`) implements the complete clipboard acquisition protocol without libX11:

1. **Connection** - Opens a Unix domain socket to `/tmp/.X11-unix/X{N}` and performs the full X11 handshake with MIT-MAGIC-COOKIE-1 authentication parsed directly from `~/.Xauthority`.
2. **Window creation** - Creates an `InputOnly` window used as the selection requestor and `GetProperty` target.
3. **XFixes negotiation** - Uses `QueryExtension` to discover the XFixes opcode and event base, then calls `XFixesSelectSelectionInput` to subscribe to `SetSelectionOwnerNotify` events on the `CLIPBOARD` atom.
4. **Clipboard fetch** - Sends `ConvertSelection` targeting `UTF8_STRING` first, falls back to `STRING`. Waits for `SelectionNotify`, reads the result via `GetProperty` using the correct `value_length × (format/8)` byte count - not the padded 4-byte-unit total, which would silently corrupt non-ASCII text.
5. **Error classification** - `is_fatal_error` distinguishes permanent errors (no display, unsupported TCP transport, XFixes unavailable) from transient ones (connection resets, EINTR) to drive the daemon's reconnection loop correctly.

---

## 📊 Metrics & Telemetry

The `telemetry` crate exposes a global `Metrics` singleton backed by `AtomicU64` counters - no locks, no allocations, safe to increment from any thread:

|Counter|Description|
|---|---|
|`clipboard_event_count`|Successfully processed clipboard change events|
|`connection_retries`|Transient reconnection attempts (backoff loop iterations)|
|`eintr_count`|Syscalls interrupted by signals (EINTR)|
|`failed_fetches`|`ConvertSelection` failures (owner refused, timeout, or malformed reply)|

`PeriodicSink` spawns a background thread that calls `report()` on a configured `Sink` every `CLIPPER_METRICS_INTERVAL_SECONDS`. The built-in `StdoutSink` emits a single timestamped line per interval. Implement the `Sink` trait to ship metrics to Prometheus, statsd, or any other observability backend.

---

## 🗺️ Roadmap - What's Coming

> **Clippy might be under active development.** The current release is a solid, working foundation - but there could be more in the pipeline. If I'm less busy.

Here's what's on the horizon when the schedule clears:

### 🔜 Near-Term

- **`PRIMARY` selection support** - monitor the middle-click selection in addition to `CLIPBOARD`, with a unified event stream
- **INCR protocol** - handle large clipboard payloads delivered in incremental X11 chunks, which the current implementation does not yet support
- **File-based structured logging** - JSON log sink with configurable rotation for proper daemon-mode observability

### 🔭 Longer-Term

- **Wayland backend** - a `WaylandBackend` implementing `ClipboardBackend` via the `wl-data-control` protocol (wlroots/KDE), making Clippy display-server-agnostic
- **Prometheus metrics exporter** - expose telemetry counters via a scrape endpoint
- **`aarch64` / `riscv64` support** - additional syscall ABI stubs in NASM, expanded build matrix in CI
- **A proper man page** - `clippy(1)`, because documentation you can read offline.

**Watch or star the repository** to be notified when new releases drop. If there's a specific feature you need sooner rather than later, open an issue - well-reasoned feature requests get attended to.

---

## 🤝 Contributing

Contributions are **warmly welcomed** - see the below on how to contribute.

### How to Contribute

1. **Fork** the repository and clone your fork
2. **Create a branch**: `git checkout -b feat/your-feature-name`
3. **Write tests** for any new behaviour
4. **Lint and format**: `make fmt && make lint`
5. **Ensure tests pass**: `make test`
6. **Open a Pull Request** with a clear description of what changed and why

### Code Standards

Run `make fmt` and `make lint` before opening any PR. All public items must carry doc comments. Every `unsafe` block must have a `// SAFETY:` comment articulating the invariants that make it sound. No bare `unwrap()` in non-test code without an inline justification.

### Reporting Issues

Please open a GitHub issue with your Linux distribution and kernel version (`uname -a`), X server version (`Xorg -version`), whether XFixes is present (`xdpyinfo | grep xfixes`), and the full error output captured with `CLIPPER_LOG_LEVEL=info CLIPPER_LOG_DEST=stderr make run`.

---

## ⭐ Star This Project

**[⭐ Star Clippy on GitHub →](https://github.com/ten-ops/clippy)**

---

## 🔑 Keywords

`rust clipboard daemon` · `linux clipboard monitor` · `x11 clipboard rust` · `zero dependency x11` · `raw syscall rust` · `xfixes clipboard` · `clipboard watcher linux` · `x11 wire protocol rust` · `linux daemon rust` · `clipboard manager daemon` · `rust x11 no libx11` · `clipboard selection monitor` · `nasm syscall rust ffi` · `daemonize rust linux` · `x11 unix socket rust` · `rust systems programming` · `clipboard monitor x11 wayland` · `rust no libc linux`

---

## 📄 License

Clippy is licensed under the MIT License. You are free to use, copy, modify, merge, publish, distribute, sublicense, and sell copies of the software - provided the copyright notice is retained.

---

## 🙏 Acknowledgements

Clippy was built by reading the [X Window System Protocol specification](https://www.x.org/releases/X11R7.7/doc/xproto/x11protocol.html), the [XFixes Extension specification](https://www.x.org/releases/X11R7.7/doc/fixesproto/fixesproto.txt), and the Linux `rt_sigaction(2)` man page a numerous amount of times.

---

## Contact me

If you have questions or suggestions, you can reach me on **Session**: `05113397ab0111e2ec2615d8a0d71499d8eaa5b5a92ebf5e2f2d79cbd858c73830`

---

_Built with Love_
