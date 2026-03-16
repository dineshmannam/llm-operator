//! CRD type definitions.
//!
//! Exposes the two custom resources managed by this operator:
//!   - `LLMProvider` — cluster-scoped resource describing a single LLM backend
//!     (endpoint URL, auth secret ref, cost model, health check config).
//!   - `LLMWorkload` — namespace-scoped resource attaching routing policy and
//!     budget constraints to a workload that consumes LLM inference.

pub mod llm_provider;
pub mod llm_workload;

// Re-exports added as types are implemented:
pub use llm_provider::{LLMProvider, LLMProviderSpec, LLMProviderStatus};
pub use llm_workload::{LLMWorkload, LLMWorkloadSpec, LLMWorkloadStatus};
