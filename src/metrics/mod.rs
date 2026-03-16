//! Prometheus metrics module.
//!
//! Exposes a global `MetricsRegistry` (behind a `once_cell::Lazy`) that all
//! other modules import to record observations. The webhook server exposes
//! these at GET /metrics in the OpenMetrics text format.

pub mod registry;

pub use registry::MetricsRegistry;
