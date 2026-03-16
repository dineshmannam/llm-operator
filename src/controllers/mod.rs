use std::sync::Arc;

use futures::StreamExt;
use kube::{api::Api, runtime::Controller, Client};
use tracing::info;

use crate::crd::LLMProvider;
use crate::metrics::MetricsRegistry;

pub mod provider;
pub mod workload;

pub use provider::Context;

/// Start all controllers and run them until the process exits.
///
/// Currently starts only the LLMProvider controller.
/// The LLMWorkload controller is wired up in Weekend 2 (Task 5 → workload.rs).
pub async fn run(client: Client, metrics: Arc<MetricsRegistry>) -> anyhow::Result<()> {
    let ctx = Arc::new(Context {
        client: client.clone(),
        metrics,
    });

    let provider_api: Api<LLMProvider> = Api::all(client);

    info!("starting LLMProvider controller");

    Controller::new(provider_api, Default::default())
        .run(provider::reconcile, provider::error_policy, ctx)
        .for_each(|result| async move {
            match result {
                Ok(obj) => tracing::debug!(?obj, "reconciled"),
                Err(e) => tracing::warn!(err = %e, "reconcile error"),
            }
        })
        .await;

    Ok(())
}
