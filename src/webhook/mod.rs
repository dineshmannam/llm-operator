use std::net::SocketAddr;
use std::sync::Arc;

use axum::{routing::{get, post}, Router};
use tracing::info;

use crate::controllers::provider::Context;

pub mod admission;

/// Start the admission webhook HTTP server.
///
/// Mounts:
///   POST /validate — ValidatingAdmissionWebhook handler for LLMWorkload
///   GET  /healthz  — liveness probe
///
/// # TLS note
/// Kubernetes requires HTTPS for admission webhooks in production.
/// For the demo cluster, configure the ValidatingWebhookConfiguration with
/// `insecureSkipTLSVerify: true` or use cert-manager to provision certs and
/// terminate TLS at the ingress layer. Full in-process TLS via rustls is
/// planned for the Weekend 3 Helm/CI pass.
pub async fn serve(addr: SocketAddr, ctx: Arc<Context>) {
    let app = Router::new()
        .route("/validate", post(admission::validate_llm_workload))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(ctx);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    info!(%addr, "webhook server listening");
    axum::serve(listener, app).await.unwrap();
}
