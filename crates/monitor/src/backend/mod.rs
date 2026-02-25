//! X11 backend implementation for clipboard monitoring.
//!
//! This module provides the concrete [`X11Connection`] type and associated error
//! handling for interacting with the X11 clipboard. It implements the
//! [`ClipboardBackend`] trait, allowing it to be used polymorphically where a
//! backend is required.
//!
//! # Re-exports
//! All public items from the [`x11`] submodule are re-exported here for convenience,
//! so that users can access the X11 backend via `monitor::backend::x11::*` or
//! directly through this module.

pub mod x11;
pub use x11::*;
