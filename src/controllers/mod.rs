use std::sync::Arc;

use futures::StreamExt;
use kube::{api::Api, runtime::Controller, Client};
use tracing::info;

use crate::crd::{LLMProvider, LLMWorkload};
use crate::metrics::MetricsRegistry;

pub mod provider;
pub mod workload;

pub use provider::Context;

/// Start all controllers and run them until the process exits.
pub async fn run(
    client: Client,
    metrics: Arc<MetricsRegistry>,
    max_budget_per_hour: f64,
) -> anyhow::Result<()> {
    let ctx = Arc::new(Context {
        client: client.clone(),
        metrics,
        max_budget_per_hour,
    });

    let provider_api: Api<LLMProvider> = Api::all(client.clone());
    let workload_api: Api<LLMWorkload> = Api::all(client);

    info!("starting LLMProvider and LLMWorkload controllers");

    let provider_ctrl = Controller::new(provider_api, Default::default())
        .run(provider::reconcile, provider::error_policy, ctx.clone())
        .for_each(|result| async move {
            match result {
                Ok(obj) => tracing::debug!(?obj, "provider reconciled"),
                Err(e) => tracing::warn!(err = %e, "provider reconcile error"),
            }
        });

    let workload_ctrl = Controller::new(workload_api, Default::default())
        .run(workload::reconcile, workload::error_policy, ctx)
        .for_each(|result| async move {
            match result {
                Ok(obj) => tracing::debug!(?obj, "workload reconciled"),
                Err(e) => tracing::warn!(err = %e, "workload reconcile error"),
            }
        });

    futures::join!(provider_ctrl, workload_ctrl);

    Ok(())
}
