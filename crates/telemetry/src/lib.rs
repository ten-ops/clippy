//! # Metrics Collection and Reporting Library
//!
//! This crate provides a thread-safe metrics collection system with
//! atomic counters and a periodic reporting mechanism that can send
//! metrics to various sinks (e.g., stdout). The main components are
//! the `Metrics` struct for tracking events and the `PeriodicSink`
//! for background reporting.
//!
//! ## Modules
//! - `metrics`: Defines the metrics container and atomic counters.
//! - `sink`: Defines the reporting trait and implementations.
//!
//! ## Re-exports
//! For convenience, the key types are re-exported at the crate root:
//! - `Metrics`: Global metrics container.
//! - `Sink`: Trait for custom report destinations.
//! - `StdoutSink`: Sink that prints to standard output.
//! - `PeriodicSink`: Background thread that periodically reports metrics.

pub mod metrics;
pub mod sink;

pub use metrics::Metrics;
pub use sink::{PeriodicSink, Sink, StdoutSink};
