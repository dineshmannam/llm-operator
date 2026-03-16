use std::collections::BTreeMap;

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::crd::llm_provider::Condition;

/// How the workload reconciler selects an active provider from candidates.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub enum RoutingStrategy {
    /// Pick the provider with the lowest `costPerToken`. Falls back to the
    /// next cheapest when the primary is unhealthy or rate-limited.
    CostAware,
    /// Pick the provider with the lowest observed p50 probe latency.
    LatencyFirst,
    /// Distribute requests evenly across all ready providers.
    RoundRobin,
    /// Stick to the first (lexicographic) provider; only fail over when unhealthy.
    Failover,
}

/// Selects candidate LLMProviders for this workload.
/// Either `matchLabels` OR `names` must be set (not both).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSelector {
    /// Label selector — all LLMProviders matching these labels are candidates.
    pub match_labels: Option<BTreeMap<String, String>>,
    /// Explicit list of LLMProvider names (cluster-scoped, so no namespace).
    pub names: Option<Vec<String>>,
}

/// Compliance constraints that filter which providers are eligible.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ComplianceConfig {
    /// If set, only providers labeled `provider.llm.platform.io/region: <value>`
    /// matching this prefix are eligible (e.g. `"us"` matches `us-east-1`).
    pub data_residency: Option<String>,
    /// If true, only providers labeled `provider.llm.platform.io/training-opt-out: "true"`
    /// are eligible. The admission webhook enforces this at admission time.
    pub no_training_data: Option<bool>,
}

/// Optional scale hint based on a queue-depth metric.
/// When the named metric exceeds the threshold, the operator emits an annotation
/// that an HPA or KEDA ScaledObject can act on.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct QueueDepthScaler {
    /// Prometheus metric name to watch (must be available in the cluster's metrics pipeline).
    pub metric_name: String,
    /// Value above which the scale-out annotation is written.
    pub threshold: u32,
}

/// Desired state of an LLMWorkload.
#[derive(CustomResource, Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[kube(
    group = "llm.platform.io",
    version = "v1alpha1",
    kind = "LLMWorkload",
    plural = "llmworkloads",
    shortname = "llmw",
    namespaced,
    status = "LLMWorkloadStatus",
    printcolumn = r#"{"name":"Strategy","type":"string","jsonPath":".spec.routingStrategy"}"#,
    printcolumn = r#"{"name":"ActiveProvider","type":"string","jsonPath":".status.activeProvider"}"#,
    printcolumn = r#"{"name":"BudgetUsed","type":"number","jsonPath":".status.tokensBudgetUsed"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#,
)]
pub struct LLMWorkloadSpec {
    /// Which algorithm to use when selecting among candidate providers.
    pub routing_strategy: RoutingStrategy,

    /// Selects the pool of LLMProviders this workload may route to.
    pub provider_selector: ProviderSelector,

    /// Maximum spend in USD per rolling hour window.
    /// The admission webhook rejects workloads that set this above the
    /// operator's cluster-wide `maxBudgetPerHour` flag.
    pub budget_per_hour: f64,

    /// Compliance filtering constraints.
    pub compliance: Option<ComplianceConfig>,

    /// Name of the ConfigMap (in the same namespace) that the workload
    /// reconciler writes the resolved provider endpoint into.
    /// Key written: `PROVIDER_ENDPOINT`.
    pub target_config_map: String,

    /// Optional: emit a scale-out annotation when a queue-depth metric exceeds a threshold.
    pub scale_on_queue_depth: Option<QueueDepthScaler>,
}

/// Observed state of an LLMWorkload, written by the workload controller.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LLMWorkloadStatus {
    /// Name of the currently active LLMProvider.
    pub active_provider: Option<String>,

    /// Rolling hourly spend in USD based on token usage.
    pub tokens_budget_used: Option<f64>,

    /// Standard Kubernetes condition list.
    pub conditions: Option<Vec<Condition>>,
}
