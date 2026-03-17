use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use k8s_openapi::api::core::v1::ConfigMap;
use kube::{
    api::{Api, ListParams, ObjectMeta, Patch, PatchParams},
    runtime::controller::Action,
    Client, ResourceExt,
};
use serde_json::json;
use tracing::{info, warn};

use crate::controllers::provider::Context;
use crate::crd::llm_workload::{ProviderSelector, RoutingStrategy};
use crate::crd::{LLMProvider, LLMWorkload, LLMWorkloadStatus};
use crate::routing::{is_ready, select_provider};

/// Error type for the workload reconciler.
#[derive(Debug, thiserror::Error)]
pub enum WorkloadError {
    #[error("Kubernetes API error: {0}")]
    Kube(#[from] kube::Error),
    #[error("No ready providers available for workload {0}")]
    NoReadyProviders(String),
}

/// Main reconcile function for LLMWorkload resources.
///
/// Flow:
///   1. List candidate LLMProviders via providerSelector.
///   2. Filter to only Ready providers.
///   3. Check budget — if exhausted, write no-op ConfigMap and requeue.
///   4. Apply routing strategy to select active provider.
///   5. Write resolved config to target ConfigMap.
///   6. Patch workload status.
pub async fn reconcile(
    workload: Arc<LLMWorkload>,
    ctx: Arc<Context>,
) -> Result<Action, WorkloadError> {
    let name = workload.name_any();
    let namespace = workload.namespace().unwrap_or_default();
    let client = ctx.client.clone();

    // ── 1. Gather candidate providers ────────────────────────────────────
    let all_candidates =
        list_candidate_providers(&client, &workload.spec.provider_selector).await?;

    // ── 2. Filter to ready only ───────────────────────────────────────────
    let ready: Vec<&LLMProvider> = all_candidates.iter().filter(|p| is_ready(p)).collect();

    if ready.is_empty() {
        warn!(workload = %name, "no ready providers; writing no-op ConfigMap");
        write_noop_configmap(&client, &namespace, &workload.spec.target_config_map).await?;
        patch_workload_status(&client, &namespace, &name, None).await?;
        return Ok(Action::requeue(Duration::from_secs(30)));
    }

    // ── 3. Budget enforcement ─────────────────────────────────────────────
    let budget_used = workload
        .status
        .as_ref()
        .and_then(|s| s.tokens_budget_used)
        .unwrap_or(0.0);

    let budget_remaining = (workload.spec.budget_per_hour - budget_used).max(0.0);

    if budget_remaining <= 0.0 {
        warn!(workload = %name, "budget exhausted (used ${budget_used:.4}/hr); writing no-op ConfigMap");
        write_noop_configmap(&client, &namespace, &workload.spec.target_config_map).await?;
        patch_workload_status(&client, &namespace, &name, None).await?;
        return Ok(Action::requeue(Duration::from_secs(60)));
    }

    // ── 4. Route ──────────────────────────────────────────────────────────
    let counter = std::sync::atomic::AtomicUsize::new(0);
    let chosen = select_provider(&ready, &workload.spec.routing_strategy, &counter)
        .ok_or_else(|| WorkloadError::NoReadyProviders(name.clone()))?;

    let chosen_name = chosen.name_any();
    info!(
        workload = %name,
        provider = %chosen_name,
        strategy = ?workload.spec.routing_strategy,
        "routing decision made"
    );

    ctx.metrics
        .inc_request(&chosen_name, &name, &format!("{:?}", workload.spec.routing_strategy));

    // ── 5. Write ConfigMap ────────────────────────────────────────────────
    write_configmap(
        &client,
        &namespace,
        &workload.spec.target_config_map,
        &workload.name_any(),
        chosen,
        &workload.spec.routing_strategy,
        budget_remaining,
        workload.spec.budget_per_hour,
    )
    .await?;

    // ── 6. Patch workload status ──────────────────────────────────────────
    patch_workload_status(&client, &namespace, &name, Some(&chosen_name)).await?;

    Ok(Action::requeue(Duration::from_secs(30)))
}

/// List LLMProviders matching the workload's providerSelector.
/// Supports both `matchLabels` and explicit `names`.
async fn list_candidate_providers(
    client: &Client,
    selector: &ProviderSelector,
) -> Result<Vec<LLMProvider>, WorkloadError> {
    let api: Api<LLMProvider> = Api::all(client.clone());

    if let Some(names) = &selector.names {
        let mut providers = Vec::new();
        for n in names {
            match api.get(n).await {
                Ok(p) => providers.push(p),
                Err(kube::Error::Api(e)) if e.code == 404 => {
                    warn!(provider = %n, "named provider not found; skipping");
                }
                Err(e) => return Err(WorkloadError::Kube(e)),
            }
        }
        Ok(providers)
    } else if let Some(labels) = &selector.match_labels {
        let label_selector = labels
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(",");
        let lp = ListParams::default().labels(&label_selector);
        Ok(api.list(&lp).await?.items)
    } else {
        Ok(vec![])
    }
}

/// Write the resolved provider config to the target ConfigMap.
/// Uses server-side apply — creates the ConfigMap if absent, patches if present.
async fn write_configmap(
    client: &Client,
    namespace: &str,
    cm_name: &str,
    workload_name: &str,
    provider: &LLMProvider,
    strategy: &RoutingStrategy,
    budget_remaining: f64,
    budget_limit: f64,
) -> Result<(), WorkloadError> {
    let api: Api<ConfigMap> = Api::namespaced(client.clone(), namespace);

    let auth_secret_name = provider
        .spec
        .auth_secret_ref
        .as_ref()
        .map(|s| s.name.as_str())
        .unwrap_or("");
    let auth_secret_key = provider
        .spec
        .auth_secret_ref
        .as_ref()
        .map(|s| s.key.as_str())
        .unwrap_or("");

    let mut data = BTreeMap::new();
    data.insert("PROVIDER_ENDPOINT".into(), provider.spec.endpoint.clone());
    data.insert("PROVIDER_NAME".into(), provider.name_any());
    data.insert("MODEL_NAME".into(), provider.spec.model.clone());
    data.insert("AUTH_SECRET_NAME".into(), auth_secret_name.to_string());
    data.insert("AUTH_SECRET_KEY".into(), auth_secret_key.to_string());
    data.insert("BUDGET_REMAINING_USD".into(), format!("{budget_remaining:.4}"));
    data.insert("BUDGET_LIMIT_USD".into(), format!("{budget_limit:.4}"));
    data.insert("ROUTING_STRATEGY".into(), format!("{strategy:?}"));
    data.insert(
        "OPERATOR_RECONCILED_AT".into(),
        chrono::Utc::now().to_rfc3339(),
    );

    let cm = ConfigMap {
        metadata: ObjectMeta {
            name: Some(cm_name.to_string()),
            namespace: Some(namespace.to_string()),
            labels: Some(BTreeMap::from([
                ("managed-by".into(), "llm-operator".into()),
                ("llmworkload".into(), workload_name.to_string()),
            ])),
            ..Default::default()
        },
        data: Some(data),
        ..Default::default()
    };

    api.patch(
        cm_name,
        &PatchParams::apply("llm-operator").force(),
        &Patch::Apply(&cm),
    )
    .await?;

    info!(configmap = %cm_name, provider = %provider.name_any(), "ConfigMap written");
    Ok(())
}

/// Write a no-op ConfigMap when no providers are available or budget is exhausted.
async fn write_noop_configmap(
    client: &Client,
    namespace: &str,
    cm_name: &str,
) -> Result<(), WorkloadError> {
    let api: Api<ConfigMap> = Api::namespaced(client.clone(), namespace);

    let mut data = BTreeMap::new();
    data.insert("PROVIDER_ENDPOINT".into(), String::new());
    data.insert("PROVIDER_NAME".into(), String::new());
    data.insert("MODEL_NAME".into(), String::new());
    data.insert("AUTH_SECRET_NAME".into(), String::new());
    data.insert("AUTH_SECRET_KEY".into(), String::new());
    data.insert("BUDGET_REMAINING_USD".into(), "0.0000".into());
    data.insert("BUDGET_LIMIT_USD".into(), String::new());
    data.insert("ROUTING_STRATEGY".into(), String::new());
    data.insert(
        "OPERATOR_RECONCILED_AT".into(),
        chrono::Utc::now().to_rfc3339(),
    );

    let cm = ConfigMap {
        metadata: ObjectMeta {
            name: Some(cm_name.to_string()),
            namespace: Some(namespace.to_string()),
            ..Default::default()
        },
        data: Some(data),
        ..Default::default()
    };

    api.patch(
        cm_name,
        &PatchParams::apply("llm-operator").force(),
        &Patch::Apply(&cm),
    )
    .await?;

    Ok(())
}

/// Patch the workload's status subresource.
async fn patch_workload_status(
    client: &Client,
    namespace: &str,
    name: &str,
    active_provider: Option<&str>,
) -> Result<(), WorkloadError> {
    let api: Api<LLMWorkload> = Api::namespaced(client.clone(), namespace);

    let status = LLMWorkloadStatus {
        active_provider: active_provider.map(String::from),
        tokens_budget_used: None, // updated by token-tracking pipeline in future
        conditions: None,
    };

    let patch = json!({ "status": status });
    api.patch_status(name, &PatchParams::apply("llm-operator"), &Patch::Merge(&patch))
        .await?;

    Ok(())
}

/// Called by the kube-rs Controller on reconcile errors.
pub fn error_policy(
    _workload: Arc<LLMWorkload>,
    error: &WorkloadError,
    _ctx: Arc<Context>,
) -> Action {
    warn!(err = %error, "workload reconcile error; retrying in 30s");
    Action::requeue(Duration::from_secs(30))
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    /// Budget enforcement: if used >= limit, remaining should be 0.
    #[test]
    fn budget_remaining_clamps_to_zero() {
        let limit = 5.0_f64;
        let used = 6.0_f64; // over budget
        let remaining = (limit - used).max(0.0);
        assert_eq!(remaining, 0.0);
    }

    /// Budget enforcement: under budget, remaining is correct.
    #[test]
    fn budget_remaining_under_limit() {
        let limit = 5.0_f64;
        let used = 2.5_f64;
        let remaining = (limit - used).max(0.0);
        assert!((remaining - 2.5).abs() < f64::EPSILON);
    }

    /// Budget remaining is formatted to 4 decimal places in ConfigMap.
    #[test]
    fn budget_remaining_format() {
        let remaining = 3.141592_f64;
        let formatted = format!("{remaining:.4}");
        assert_eq!(formatted, "3.1416");
    }
}
