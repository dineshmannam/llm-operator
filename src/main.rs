use std::sync::Arc;

use axum::{routing::get, Router};
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use llm_operator::{controllers, metrics::MetricsRegistry};

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
    tokio::join!(
        controllers::run(client, metrics),
        http_server,
    ).0?;

    Ok(())
}

/// Render all registered metrics in OpenMetrics text format.
fn render_metrics(registry: &MetricsRegistry) -> String {
    let mut buf = String::new();
    prometheus_client::encoding::text::encode(&mut buf, &registry.registry).unwrap();
    buf
}
