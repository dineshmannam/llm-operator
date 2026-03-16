use std::sync::Arc;
use std::time::Duration;

use kube::{
    api::{Api, Patch, PatchParams},
    runtime::controller::Action,
    Client, ResourceExt,
};
use serde_json::json;
use tracing::{info, warn};

use crate::crd::{
    llm_provider::{Condition, HealthCheckConfig},
    LLMProvider, LLMProviderStatus,
};
use crate::metrics::MetricsRegistry;

/// Shared state passed to every reconcile call.
pub struct Context {
    pub client: Client,
    pub metrics: Arc<MetricsRegistry>,
}

/// Error type for the provider reconciler.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("Kubernetes API error: {0}")]
    Kube(#[from] kube::Error),
    #[error("HTTP probe failed: {0}")]
    Probe(String),
    #[error("Auth secret {name}/{key} not found")]
    MissingSecret { name: String, key: String },
}

/// Main reconcile function — called by the kube-rs Controller on every relevant event.
///
/// Flow:
///   1. Issue an HTTP health probe to the provider's endpoint.
///   2. Patch `.status.ready` and `.status.lastProbeTime`.
///   3. Emit a Kubernetes Event on health state transitions.
///   4. Return a requeue duration based on `spec.healthCheck.intervalSeconds`.
pub async fn reconcile(
    provider: Arc<LLMProvider>,
    ctx: Arc<Context>,
) -> Result<Action, ProviderError> {
    let name = provider.name_any();
    let client = ctx.client.clone();

    // Cluster-scoped resource — no namespace needed.
    let api: Api<LLMProvider> = Api::all(client.clone());

    let start = std::time::Instant::now();

    // ── 1. Run health probe ───────────────────────────────────────────────
    let (probe_ok, probe_err_msg) = run_health_probe(&provider).await;

    let elapsed = start.elapsed().as_secs_f64();
    ctx.metrics.observe_probe_duration(&name, elapsed);

    // ── 2. Determine if health state changed ─────────────────────────────
    let was_ready = provider
        .status
        .as_ref()
        .map(|s| s.ready)
        .unwrap_or(false);

    if probe_ok != was_ready {
        // TODO Weekend 2: replace with proper K8s Event via kube::runtime::events::Recorder
        if probe_ok {
            info!(provider = %name, "health state transition: Unhealthy → Ready");
        } else {
            warn!(provider = %name, err = ?probe_err_msg, "health state transition: Ready → Unhealthy");
        }
    }

    // ── 3. Patch status ───────────────────────────────────────────────────
    ctx.metrics.set_provider_health(&name, probe_ok);

    let now = chrono::Utc::now().to_rfc3339();
    let reason = if probe_ok { "ProbeSucceeded" } else { "ProbeFailed" };
    let message = if probe_ok {
        format!("Health probe to {} succeeded in {:.0}ms", provider.spec.endpoint, elapsed * 1000.0)
    } else {
        probe_err_msg.clone().unwrap_or_else(|| "Probe failed".into())
    };

    let new_status = LLMProviderStatus {
        ready: probe_ok,
        last_probe_time: Some(now.clone()),
        conditions: Some(vec![Condition {
            type_: "Ready".into(),
            status: if probe_ok { "True" } else { "False" }.into(),
            reason: reason.into(),
            message,
            last_transition_time: now,
        }]),
    };

    let patch = json!({ "status": new_status });
    api.patch_status(
        &name,
        &PatchParams::apply("llm-operator"),
        &Patch::Merge(&patch),
    )
    .await?;

    if probe_ok {
        info!(provider = %name, "probe succeeded");
    } else {
        warn!(provider = %name, err = ?probe_err_msg, "probe failed");
    }

    // ── 4. Requeue after configured interval (default 60s) ───────────────
    let interval = provider
        .spec
        .health_check
        .as_ref()
        .map(|h: &HealthCheckConfig| h.interval_seconds)
        .unwrap_or(60);

    Ok(Action::requeue(Duration::from_secs(interval as u64)))
}

/// Issue an HTTP GET to `spec.endpoint + spec.healthCheck.path`.
/// Returns `(true, None)` on 2xx, `(false, Some(reason))` otherwise.
async fn run_health_probe(provider: &LLMProvider) -> (bool, Option<String>) {
    let path = provider
        .spec
        .health_check
        .as_ref()
        .map(|h| h.path.as_str())
        .unwrap_or("/");
    let timeout_secs = provider
        .spec
        .health_check
        .as_ref()
        .map(|h| h.timeout_seconds)
        .unwrap_or(5);

    let url = format!("{}{}", provider.spec.endpoint.trim_end_matches('/'), path);

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs as u64))
        .build()
    {
        Ok(c) => c,
        Err(e) => return (false, Some(format!("failed to build HTTP client: {e}"))),
    };

    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => (true, None),
        Ok(resp) => (
            false,
            Some(format!("HTTP {} from {url}", resp.status().as_u16())),
        ),
        Err(e) => (false, Some(format!("request error: {e}"))),
    }
}

/// Called by the kube-rs Controller when reconcile returns an error.
/// Determines how long to wait before the next retry.
pub fn error_policy(
    _provider: Arc<LLMProvider>,
    error: &ProviderError,
    _ctx: Arc<Context>,
) -> Action {
    warn!(err = %error, "reconcile error; retrying in 30s");
    Action::requeue(Duration::from_secs(30))
}
