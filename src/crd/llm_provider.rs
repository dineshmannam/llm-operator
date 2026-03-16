use std::collections::BTreeMap;

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Reference to a Kubernetes Secret containing an API key.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecretRef {
    /// Name of the Secret in the same namespace (or operator namespace for cluster-scoped providers).
    pub name: String,
    /// Key within the Secret's `data` map.
    pub key: String,
}

/// HTTP probe configuration used by the provider health controller.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HealthCheckConfig {
    /// HTTP path to GET (e.g. `/api/v1/models`). Must return 2xx for the probe to pass.
    pub path: String,
    /// How often to probe, in seconds.
    pub interval_seconds: u32,
    /// Probe timeout in seconds. If the provider doesn't respond within this window
    /// the probe is counted as failed.
    pub timeout_seconds: u32,
}

/// Lightweight Condition entry (mirrors `meta/v1 Condition`).
/// Defined here because `k8s_openapi` types don't implement `JsonSchema`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Condition {
    /// e.g. "Ready", "Degraded"
    #[serde(rename = "type")]
    pub type_: String,
    /// "True", "False", or "Unknown"
    pub status: String,
    /// Short machine-readable reason token (PascalCase).
    pub reason: String,
    /// Human-readable description.
    pub message: String,
    /// RFC3339 timestamp of the last transition.
    pub last_transition_time: String,
}

/// Desired state of an LLMProvider.
#[derive(CustomResource, Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[kube(
    group = "llm.platform.io",
    version = "v1alpha1",
    kind = "LLMProvider",
    plural = "llmproviders",
    shortname = "llmp",
    status = "LLMProviderStatus",
    printcolumn = r#"{"name":"Ready","type":"string","jsonPath":".status.ready"}"#,
    printcolumn = r#"{"name":"Endpoint","type":"string","jsonPath":".spec.endpoint"}"#,
    printcolumn = r#"{"name":"Cost/1KTok","type":"number","jsonPath":".spec.costPerToken"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#,
)]
pub struct LLMProviderSpec {
    /// Base URL of the inference API (e.g. `https://openrouter.ai/api/v1`).
    pub endpoint: String,

    /// Default model identifier for this provider.
    /// Format is provider-specific:
    ///   - OpenRouter: `"openai/gpt-4o-mini"`, `"anthropic/claude-3-5-sonnet"`
    ///   - Ollama:     `"llama3"`, `"mistral"`, `"phi3"`
    ///   - OpenAI:     `"gpt-4o"`, `"gpt-4o-mini"`
    /// Written to the target ConfigMap as `MODEL_NAME` for the app to consume.
    pub model: String,

    /// Reference to a Kubernetes Secret containing the API key.
    /// Omit for unauthenticated providers (e.g. local Ollama).
    pub auth_secret_ref: Option<SecretRef>,

    /// Cost in USD per 1,000 tokens (blended prompt + completion estimate).
    /// Used by the CostAware routing strategy to pick the cheapest ready provider.
    pub cost_per_token: f64,

    /// Maximum requests per minute this provider can handle.
    /// The routing engine avoids providers that are at capacity.
    pub rate_limit: Option<u32>,

    /// HTTP health probe configuration.
    pub health_check: Option<HealthCheckConfig>,

    /// Arbitrary key/value labels for routing affinity
    /// (e.g. `tier: primary`, `region: us-east-1`).
    pub tags: Option<BTreeMap<String, String>>,
}

/// Observed state of an LLMProvider, written by the provider controller.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LLMProviderStatus {
    /// True when the most recent health probe succeeded.
    pub ready: bool,

    /// RFC3339 timestamp of the most recent health probe attempt.
    pub last_probe_time: Option<String>,

    /// Standard Kubernetes condition list.
    pub conditions: Option<Vec<Condition>>,
}
