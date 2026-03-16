//! Routing engine — selects an LLMProvider from a candidate list.
//!
//! The routing decision is pure (no I/O): it takes a slice of `&LLMProvider`
//! and a `RoutingStrategy` enum and returns the chosen provider (or `None` if
//! the list is empty).
//!
//! Keeping routing logic separate from the controller means it is trivially
//! unit-testable without a live cluster.

pub mod strategy;

pub use strategy::{is_ready, select_provider, RoutingStrategy};
