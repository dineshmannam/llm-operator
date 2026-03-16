use prometheus_client::{
    metrics::{counter::Counter, family::Family, gauge::Gauge, histogram::Histogram},
    registry::Registry,
};

/// Label set for per-provider metrics.
#[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelSet)]
pub struct ProviderLabels {
    pub provider: String,
}

/// Label set for per-workload metrics.
#[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelSet)]
pub struct WorkloadLabels {
    pub workload: String,
}

/// Label set for routing decision metrics.
#[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelSet)]
pub struct RoutingLabels {
    pub provider: String,
    pub workload: String,
    pub strategy: String,
}

/// Label set for admission webhook decisions.
#[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelSet)]
pub struct AdmissionLabels {
    /// "allowed" or "denied"
    pub result: String,
}

/// Label set for reconcile loop duration.
#[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelSet)]
pub struct ControllerLabels {
    /// "provider" or "workload"
    pub controller: String,
}

/// All Prometheus metrics owned by the operator.
///
/// Construct once via `MetricsRegistry::new()` and pass an `Arc` to controllers.
pub struct MetricsRegistry {
    /// Prometheus registry — used to render /metrics output.
    pub registry: Registry,

    // ── Counters ──────────────────────────────────────────────────────────
    /// Total routing decisions made (incremented each reconcile that writes a ConfigMap).
    pub requests_total: Family<RoutingLabels, Counter>,

    /// Admission webhook decisions.
    pub admission_decisions_total: Family<AdmissionLabels, Counter>,

    // ── Gauges ────────────────────────────────────────────────────────────
    /// 1.0 if provider's last probe succeeded, 0.0 otherwise.
    pub provider_healthy: Family<ProviderLabels, Gauge<f64, std::sync::atomic::AtomicU64>>,

    /// Rolling hourly spend in USD for each workload.
    pub budget_used_usd: Family<WorkloadLabels, Gauge<f64, std::sync::atomic::AtomicU64>>,

    /// Configured budget cap in USD for each workload (for % utilization alerts).
    pub budget_limit_usd: Family<WorkloadLabels, Gauge<f64, std::sync::atomic::AtomicU64>>,

    // ── Histograms ────────────────────────────────────────────────────────
    /// Health probe round-trip latency per provider (used by LatencyFirst strategy).
    pub provider_probe_duration_seconds: Family<ProviderLabels, Histogram>,

    /// Reconcile loop wall-clock time per controller.
    pub reconcile_duration_seconds: Family<ControllerLabels, Histogram>,
}

/// Standard latency buckets in seconds: 5ms … 10s.
const LATENCY_BUCKETS: &[f64] = &[0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0];

fn make_latency_histogram() -> Histogram {
    Histogram::new(LATENCY_BUCKETS.iter().copied())
}

impl MetricsRegistry {
    /// Create and register all metrics. Call once at startup.
    pub fn new() -> Self {
        let mut registry = Registry::default();

        let requests_total = Family::<RoutingLabels, Counter>::default();
        registry.register(
            "llm_operator_requests",
            "Total LLM routing decisions made",
            requests_total.clone(),
        );

        let admission_decisions_total = Family::<AdmissionLabels, Counter>::default();
        registry.register(
            "llm_operator_admission_decisions",
            "Admission webhook decisions (allowed/denied)",
            admission_decisions_total.clone(),
        );

        let provider_healthy =
            Family::<ProviderLabels, Gauge<f64, std::sync::atomic::AtomicU64>>::default();
        registry.register(
            "llm_operator_provider_healthy",
            "1.0 if the provider's last health probe succeeded, 0.0 otherwise",
            provider_healthy.clone(),
        );

        let budget_used_usd =
            Family::<WorkloadLabels, Gauge<f64, std::sync::atomic::AtomicU64>>::default();
        registry.register(
            "llm_operator_budget_used_usd",
            "Rolling hourly spend in USD for each workload",
            budget_used_usd.clone(),
        );

        let budget_limit_usd =
            Family::<WorkloadLabels, Gauge<f64, std::sync::atomic::AtomicU64>>::default();
        registry.register(
            "llm_operator_budget_limit_usd",
            "Configured budget cap in USD per workload",
            budget_limit_usd.clone(),
        );

        let provider_probe_duration_seconds =
            Family::<ProviderLabels, Histogram>::new_with_constructor(make_latency_histogram);
        registry.register(
            "llm_operator_provider_probe_duration_seconds",
            "Health probe round-trip latency per provider",
            provider_probe_duration_seconds.clone(),
        );

        let reconcile_duration_seconds =
            Family::<ControllerLabels, Histogram>::new_with_constructor(make_latency_histogram);
        registry.register(
            "llm_operator_reconcile_duration_seconds",
            "Reconcile loop wall-clock time per controller",
            reconcile_duration_seconds.clone(),
        );

        Self {
            registry,
            requests_total,
            admission_decisions_total,
            provider_healthy,
            budget_used_usd,
            budget_limit_usd,
            provider_probe_duration_seconds,
            reconcile_duration_seconds,
        }
    }

    /// Record a provider health state change.
    pub fn set_provider_health(&self, provider: &str, healthy: bool) {
        self.provider_healthy
            .get_or_create(&ProviderLabels {
                provider: provider.to_string(),
            })
            .set(if healthy { 1.0 } else { 0.0 });
    }

    /// Record a routing decision.
    pub fn inc_request(&self, provider: &str, workload: &str, strategy: &str) {
        self.requests_total
            .get_or_create(&RoutingLabels {
                provider: provider.to_string(),
                workload: workload.to_string(),
                strategy: strategy.to_string(),
            })
            .inc();
    }

    /// Observe a probe round-trip duration (seconds).
    pub fn observe_probe_duration(&self, provider: &str, duration_secs: f64) {
        self.provider_probe_duration_seconds
            .get_or_create(&ProviderLabels {
                provider: provider.to_string(),
            })
            .observe(duration_secs);
    }
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self::new()
    }
}
