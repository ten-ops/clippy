# Makefile for clippy - X11 clipboard monitoring daemon

# Default target
.PHONY: all
all: build

# Build debug version
.PHONY: build
build:
	cargo build

# Build release version
.PHONY: release
release:
	cargo build --release

# Clean build artifacts
.PHONY: clean
clean:
	cargo clean

# Run tests (all crates)
.PHONY: test
test:
	cargo test --workspace

# Run tests with output
.PHONY: test-verbose
test-verbose:
	cargo test --workspace -- --nocapture

# Run the daemon (debug build) with default DISPLAY=:0
.PHONY: run
run:
	@DISPLAY=:1 cargo run

# Run the daemon in release mode
.PHONY: run-release
run-release:
	DISPLAY=:1 cargo run --release

# Run benchmarks (if any)
.PHONY: bench
bench:
	cargo bench

# Generate documentation
.PHONY: doc
doc:
	cargo doc --workspace --document-private-items --open

# Check code (quick)
.PHONY: check
check:
	cargo check --workspace

# Format code
.PHONY: fmt
fmt:
	cargo fmt --all

# Run clippy lints
.PHONY: clippy
clippy:
	cargo clippy --workspace -- -D warnings

# Install daemon binary to ~/.cargo/bin
.PHONY: install
install:
	cargo install --path crates/daemon

# Uninstall daemon binary
.PHONY: uninstall
uninstall:
	cargo uninstall daemon

# Help
.PHONY: help
help:
	@echo "Available targets:"
	@echo "  all          - Build debug version (default)"
	@echo "  build        - Build debug version"
	@echo "  release      - Build release version"
	@echo "  clean        - Clean build artifacts"
	@echo "  test         - Run all tests"
	@echo "  test-verbose - Run tests with output"
	@echo "  run          - Run daemon (debug) with DISPLAY=:0"
	@echo "  run-release  - Run daemon (release) with DISPLAY=:0"
	@echo "  bench        - Run benchmarks"
	@echo "  doc          - Generate and open documentation"
	@echo "  check        - Cargo check"
	@echo "  fmt          - Format code"
	@echo "  clippy       - Run clippy lints"
	@echo "  install      - Install daemon binary"
	@echo "  uninstall    - Uninstall daemon binary"
	@echo "  help         - Show this help"
