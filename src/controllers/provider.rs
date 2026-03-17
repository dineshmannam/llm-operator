use std::sync::Arc;
use std::time::Duration;

use kube::{
    api::{Api, Patch, PatchParams},
    runtime::{
        controller::Action,
        events::{Event, EventType, Recorder, Reporter},
        reflector::ObjectRef,
    },
    Client, ResourceExt,
};
use serde_json::json;
use tracing::{info, warn};

use crate::crd::{
    llm_provider::{Condition, HealthCheckConfig},
    LLMProvider, LLMProviderStatus,
};
use crate::metrics::MetricsRegistry;

/// Shared state passed to every reconcile call and the admission webhook.
pub struct Context {
    pub client: Client,
    pub metrics: Arc<MetricsRegistry>,
    /// Cluster-wide maximum allowed `budgetPerHour` (USD).
    /// The admission webhook rejects workloads that exceed this value.
    pub max_budget_per_hour: f64,
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
    let was_ready = provider.status.as_ref().map(|s| s.ready).unwrap_or(false);

    if probe_ok != was_ready {
        emit_health_event(client.clone(), &provider, probe_ok, &probe_err_msg).await;
    }

    // ── 3. Patch status ───────────────────────────────────────────────────
    ctx.metrics.set_provider_health(&name, probe_ok);

    let now = chrono::Utc::now().to_rfc3339();
    let reason = if probe_ok {
        "ProbeSucceeded"
    } else {
        "ProbeFailed"
    };
    let message = if probe_ok {
        format!(
            "Health probe to {} succeeded in {:.0}ms",
            provider.spec.endpoint,
            elapsed * 1000.0
        )
    } else {
        probe_err_msg
            .clone()
            .unwrap_or_else(|| "Probe failed".into())
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

/// Emit a Kubernetes Event on health state transitions so operators can see
/// Ready ↔ Unhealthy changes in `kubectl describe llmprovider <name>`.
async fn emit_health_event(
    client: Client,
    provider: &LLMProvider,
    now_ready: bool,
    err_msg: &Option<String>,
) {
    let obj_ref = ObjectRef::from_obj(provider).erase();

    let recorder = Recorder::new(
        client,
        Reporter {
            controller: "llm-operator".into(),
            instance: None,
        },
        obj_ref.into(),
    );

    let (event_type, reason, note) = if now_ready {
        (
            EventType::Normal,
            "ProviderReady",
            "Health probe succeeded; provider marked Ready.".to_string(),
        )
    } else {
        (
            EventType::Warning,
            "ProviderUnhealthy",
            format!(
                "Health probe failed: {}",
                err_msg.as_deref().unwrap_or("unknown error")
            ),
        )
    };

    if let Err(e) = recorder
        .publish(Event {
            type_: event_type,
            reason: reason.into(),
            note: Some(note),
            action: "HealthProbe".into(),
            secondary: None,
        })
        .await
    {
        warn!(provider = %provider.name_any(), err = %e, "failed to publish K8s event");
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
