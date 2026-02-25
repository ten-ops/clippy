//! Root module for the clipboard monitoring library.
//!
//! This crate provides a platform-agnostic interface for monitoring and
//! interacting with the system clipboard. It defines the core [`ClipboardBackend`]
//! trait and exposes concrete implementations for supported platforms.
//!
//! # Structure
//! - [`backend`] – Contains platform-specific backend implementations (e.g., X11).
//! - [`traits`] – Defines the core trait that all backends must implement.
//!
//! # Re-exports
//! The main trait [`ClipboardBackend`] is re-exported at the crate root for
//! convenient access.

pub mod backend;
pub mod traits;

pub use traits::ClipboardBackend;
