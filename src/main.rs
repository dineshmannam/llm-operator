use std::sync::Arc;

use axum::{routing::get, Router};
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use llm_operator::{controllers, metrics::MetricsRegistry, webhook};

/// Cluster-wide max budget cap in USD/hr. Configurable via env var LLM_MAX_BUDGET.
const DEFAULT_MAX_BUDGET_PER_HOUR: f64 = 100.0;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── Tracing ───────────────────────────────────────────────────────────
    tracing_subscriber::registry()
        .with(fmt::layer().json())
        .with(EnvFilter::from_default_env().add_directive("llm_operator=info".parse()?))
        .init();

    info!("llm-operator starting");

    // ── Kubernetes client ─────────────────────────────────────────────────
    let client = kube::Client::try_default().await?;
    info!("connected to Kubernetes API");

    // ── Shared metrics registry ───────────────────────────────────────────
    let metrics = Arc::new(MetricsRegistry::new());

    // ── Axum server (metrics + liveness) ─────────────────────────────────
    let metrics_clone = metrics.clone();
    let http_server = async move {
        let app = Router::new()
            .route("/healthz", get(|| async { "ok" }))
            .route(
                "/metrics",
                get(move || {
                    let m = metrics_clone.clone();
                    async move { render_metrics(&m) }
                }),
            );

        let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
        info!("HTTP server listening on :8080");
        axum::serve(listener, app).await.unwrap();
    };

    // ── Run controller + HTTP server concurrently ─────────────────────────
    let max_budget = std::env::var("LLM_MAX_BUDGET")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MAX_BUDGET_PER_HOUR);

    // Shared context used by controllers and the webhook
    let ctx = std::sync::Arc::new(llm_operator::controllers::provider::Context {
        client: client.clone(),
        metrics: metrics.clone(),
        max_budget_per_hour: max_budget,
    });

    let webhook_addr = "0.0.0.0:8443".parse().unwrap();

    tokio::join!(
        controllers::run(client, metrics, max_budget),
        webhook::serve(webhook_addr, ctx),
        http_server,
    )
    .0?;

    Ok(())
}

/// Render all registered metrics in OpenMetrics text format.
fn render_metrics(registry: &MetricsRegistry) -> String {
    let mut buf = String::new();
    prometheus_client::encoding::text::encode(&mut buf, &registry.registry).unwrap();
    buf
}
