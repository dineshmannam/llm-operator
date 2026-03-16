//! Admission webhook server module.
//!
//! Exposes `pub async fn serve(addr: SocketAddr, tls_config: RustlsConfig)`
//! which the main entrypoint calls. Internally mounts:
//!   - POST /validate  — ValidatingAdmissionWebhook handler for LLMWorkload
//!   - GET  /healthz   — liveness probe for the webhook pod
//!
//! TLS is required by Kubernetes for admission webhooks. In-cluster certs are
//! expected to be mounted at /tls/tls.crt and /tls/tls.key (managed by
//! cert-manager or the Helm chart's self-signed CA job).

pub mod admission;

// TODO Weekend 2: implement pub async fn serve(addr, tls_config) using axum::Router
