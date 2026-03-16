use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use kube::{api::Api, ResourceExt};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::controllers::provider::Context;
use crate::crd::llm_workload::{ComplianceConfig, ProviderSelector};
use crate::crd::{LLMProvider, LLMWorkload};
use crate::metrics::AdmissionLabels;
use crate::routing::is_ready;

// ── AdmissionReview wire types ────────────────────────────────────────────────

/// Incoming AdmissionReview from kube-apiserver.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionReview {
    pub request: Option<AdmissionRequest>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionRequest {
    pub uid: String,
    /// The raw LLMWorkload object being created or updated.
    pub object: serde_json::Value,
}

/// Outgoing AdmissionReview response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionReviewResponse {
    pub api_version: String,
    pub kind: String,
    pub response: AdmissionResponse,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionResponse {
    pub uid: String,
    pub allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<AdmissionStatus>,
}

#[derive(Debug, Serialize)]
pub struct AdmissionStatus {
    pub message: String,
}

// ── Handler ───────────────────────────────────────────────────────────────────

/// POST /validate — ValidatingAdmissionWebhook handler for LLMWorkload.
pub async fn validate_llm_workload(
    State(ctx): State<Arc<Context>>,
    Json(review): Json<AdmissionReview>,
) -> Json<AdmissionReviewResponse> {
    let request = match review.request {
        Some(r) => r,
        None => return deny("", "missing request in AdmissionReview"),
    };
    let uid = request.uid.clone();

    let workload: LLMWorkload = match serde_json::from_value(request.object) {
        Ok(w) => w,
        Err(e) => return deny(&uid, &format!("failed to parse LLMWorkload: {e}")),
    };

    // ── Check 1: budget cap (pure, no K8s API call) ───────────────────────
    if let Err(msg) = check_budget(&workload.spec.budget_per_hour, ctx.max_budget_per_hour) {
        record_admission(&ctx, false);
        return deny(&uid, &msg);
    }

    // ── Checks 2–4: provider-based (require K8s API) ──────────────────────
    let providers = match list_matching_providers(&ctx.client, &workload.spec.provider_selector).await {
        Ok(p) => p,
        Err(e) => return deny(&uid, &format!("failed to list providers: {e}")),
    };

    if let Err(msg) = check_provider_existence(&providers, &workload.name_any()) {
        record_admission(&ctx, false);
        return deny(&uid, &msg);
    }

    if let Some(compliance) = &workload.spec.compliance {
        if let Err(msg) = check_compliance(&providers, compliance) {
            record_admission(&ctx, false);
            return deny(&uid, &msg);
        }
    }

    info!(workload = %workload.name_any(), "admission allowed");
    record_admission(&ctx, true);
    allow(&uid)
}

// ── Validation checks ─────────────────────────────────────────────────────────

/// Check 1: budgetPerHour must not exceed the cluster-wide cap.
pub fn check_budget(budget_per_hour: &f64, max: f64) -> Result<(), String> {
    if *budget_per_hour > max {
        Err(format!(
            "budgetPerHour ${budget_per_hour:.2} exceeds cluster maximum ${max:.2}"
        ))
    } else {
        Ok(())
    }
}

/// Check 2: at least one provider must match the selector.
pub fn check_provider_existence(providers: &[LLMProvider], workload_name: &str) -> Result<(), String> {
    if providers.is_empty() {
        Err(format!(
            "no LLMProviders match the providerSelector for workload '{workload_name}'"
        ))
    } else {
        Ok(())
    }
}

/// Checks 3 & 4: compliance constraints must be satisfiable by at least one provider.
///
/// - `dataResidency`: at least one provider must have a region label starting with
///   the specified value (e.g. "us" matches "us-east-1").
/// - `noTrainingData`: if true, at least one provider must carry the
///   `provider.llm.platform.io/training-opt-out: "true"` label.
pub fn check_compliance(providers: &[LLMProvider], compliance: &ComplianceConfig) -> Result<(), String> {
    if let Some(region_prefix) = &compliance.data_residency {
        let ok = providers.iter().any(|p| {
            p.labels()
                .get("provider.llm.platform.io/region")
                .map(|r| r.starts_with(region_prefix.as_str()))
                .unwrap_or(false)
        });
        if !ok {
            return Err(format!(
                "no LLMProvider with region matching '{region_prefix}' found; \
                 set provider.llm.platform.io/region label accordingly"
            ));
        }
    }

    if compliance.no_training_data == Some(true) {
        let ok = providers.iter().any(|p| {
            p.labels()
                .get("provider.llm.platform.io/training-opt-out")
                .map(|v| v == "true")
                .unwrap_or(false)
        });
        if !ok {
            return Err(
                "noTrainingData=true but no LLMProvider has \
                 provider.llm.platform.io/training-opt-out=true label"
                    .to_string(),
            );
        }
    }

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

async fn list_matching_providers(
    client: &kube::Client,
    selector: &ProviderSelector,
) -> Result<Vec<LLMProvider>, kube::Error> {
    use kube::api::ListParams;

    let api: Api<LLMProvider> = Api::all(client.clone());

    if let Some(names) = &selector.names {
        let mut out = Vec::new();
        for n in names {
            if let Ok(p) = api.get(n).await {
                out.push(p);
            }
        }
        Ok(out)
    } else if let Some(labels) = &selector.match_labels {
        let label_selector = labels
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(",");
        Ok(api.list(&ListParams::default().labels(&label_selector)).await?.items)
    } else {
        Ok(vec![])
    }
}

fn allow(uid: &str) -> Json<AdmissionReviewResponse> {
    Json(AdmissionReviewResponse {
        api_version: "admission.k8s.io/v1".into(),
        kind: "AdmissionReview".into(),
        response: AdmissionResponse {
            uid: uid.to_string(),
            allowed: true,
            status: None,
        },
    })
}

fn deny(uid: &str, message: &str) -> Json<AdmissionReviewResponse> {
    Json(AdmissionReviewResponse {
        api_version: "admission.k8s.io/v1".into(),
        kind: "AdmissionReview".into(),
        response: AdmissionResponse {
            uid: uid.to_string(),
            allowed: false,
            status: Some(AdmissionStatus {
                message: message.to_string(),
            }),
        },
    })
}

fn record_admission(ctx: &Context, allowed: bool) {
    ctx.metrics
        .admission_decisions_total
        .get_or_create(&AdmissionLabels {
            result: if allowed { "allowed" } else { "denied" }.into(),
        })
        .inc();
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    use crate::crd::llm_provider::{LLMProviderSpec, LLMProviderStatus};
    use crate::crd::llm_workload::ComplianceConfig;
    use crate::crd::LLMProvider;

    use super::{check_budget, check_compliance, check_provider_existence};

    fn make_provider(name: &str, labels: &[(&str, &str)], ready: bool) -> LLMProvider {
        let mut label_map = BTreeMap::new();
        for (k, v) in labels {
            label_map.insert(k.to_string(), v.to_string());
        }
        LLMProvider {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                labels: Some(label_map),
                ..Default::default()
            },
            spec: LLMProviderSpec {
                endpoint: "https://example.com".into(),
                model: "gpt-4o-mini".into(),
                auth_secret_ref: None,
                cost_per_token: 0.01,
                rate_limit: None,
                health_check: None,
                tags: None,
            },
            status: Some(LLMProviderStatus {
                ready,
                last_probe_time: None,
                conditions: None,
            }),
        }
    }

    // ── Budget cap tests ──────────────────────────────────────────────────

    #[test]
    fn budget_within_cap_is_allowed() {
        assert!(check_budget(&5.0, 100.0).is_ok());
    }

    #[test]
    fn budget_equal_to_cap_is_allowed() {
        assert!(check_budget(&100.0, 100.0).is_ok());
    }

    #[test]
    fn budget_exceeding_cap_is_denied() {
        let err = check_budget(&150.0, 100.0).unwrap_err();
        assert!(err.contains("exceeds cluster maximum"));
    }

    // ── Provider existence tests ──────────────────────────────────────────

    #[test]
    fn empty_provider_list_is_denied() {
        assert!(check_provider_existence(&[], "my-workload").is_err());
    }

    #[test]
    fn non_empty_provider_list_is_allowed() {
        let p = make_provider("p1", &[], true);
        assert!(check_provider_existence(&[p], "my-workload").is_ok());
    }

    // ── Compliance tests ──────────────────────────────────────────────────

    #[test]
    fn data_residency_match_is_allowed() {
        let p = make_provider(
            "p1",
            &[("provider.llm.platform.io/region", "us-east-1")],
            true,
        );
        let compliance = ComplianceConfig {
            data_residency: Some("us".into()),
            no_training_data: None,
        };
        assert!(check_compliance(&[p], &compliance).is_ok());
    }

    #[test]
    fn data_residency_mismatch_is_denied() {
        let p = make_provider(
            "p1",
            &[("provider.llm.platform.io/region", "eu-west-1")],
            true,
        );
        let compliance = ComplianceConfig {
            data_residency: Some("us".into()),
            no_training_data: None,
        };
        let err = check_compliance(&[p], &compliance).unwrap_err();
        assert!(err.contains("region matching 'us'"));
    }

    #[test]
    fn no_training_data_with_opt_out_label_is_allowed() {
        let p = make_provider(
            "p1",
            &[("provider.llm.platform.io/training-opt-out", "true")],
            true,
        );
        let compliance = ComplianceConfig {
            data_residency: None,
            no_training_data: Some(true),
        };
        assert!(check_compliance(&[p], &compliance).is_ok());
    }

    #[test]
    fn no_training_data_missing_label_is_denied() {
        let p = make_provider("p1", &[], true);
        let compliance = ComplianceConfig {
            data_residency: None,
            no_training_data: Some(true),
        };
        let err = check_compliance(&[p], &compliance).unwrap_err();
        assert!(err.contains("training-opt-out"));
    }

    #[test]
    fn no_training_data_false_skips_check() {
        let p = make_provider("p1", &[], true); // no opt-out label
        let compliance = ComplianceConfig {
            data_residency: None,
            no_training_data: Some(false),
        };
        assert!(check_compliance(&[p], &compliance).is_ok());
    }
}
