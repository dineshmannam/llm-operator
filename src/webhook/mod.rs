use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use axum::{
    routing::{get, post},
    Router,
};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder;
use hyper_util::service::TowerToHyperService;
use tokio::net::TcpListener;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;
use tracing::{info, warn};

use crate::controllers::provider::Context;

pub mod admission;

/// Default paths — match the Helm volumeMount (`/tls` from `webhook.tlsSecretName` secret).
const DEFAULT_CERT_PATH: &str = "/tls/tls.crt";
const DEFAULT_KEY_PATH: &str = "/tls/tls.key";

/// Start the admission webhook server.
///
/// TLS is auto-detected at startup:
///   - If `WEBHOOK_TLS_CERT` / `WEBHOOK_TLS_KEY` env vars point to valid PEM files
///     (or the defaults `/tls/tls.crt` / `/tls/tls.key` exist), the server starts
///     with TLS using the provided certificate.
///   - Otherwise it falls back to plain HTTP and logs a warning. This mode is safe
///     for demo clusters where the ValidatingWebhookConfiguration sets
///     `webhooks[].clientConfig.insecureSkipTLSVerify: true`.
///
/// Mounts:
///   POST /validate — ValidatingAdmissionWebhook handler for LLMWorkload
///   GET  /healthz  — liveness / readiness probe target
pub async fn serve(addr: SocketAddr, ctx: Arc<Context>) {
    let app = Router::new()
        .route("/validate", post(admission::validate_llm_workload))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(ctx);

    let cert_path = std::env::var("WEBHOOK_TLS_CERT").unwrap_or_else(|_| DEFAULT_CERT_PATH.into());
    let key_path = std::env::var("WEBHOOK_TLS_KEY").unwrap_or_else(|_| DEFAULT_KEY_PATH.into());

    let listener = TcpListener::bind(addr).await.unwrap();

    if Path::new(&cert_path).exists() && Path::new(&key_path).exists() {
        info!(%addr, cert = %cert_path, key = %key_path, "webhook server listening (TLS)");
        let tls_acceptor = build_tls_acceptor(&cert_path, &key_path)
            .expect("failed to load TLS cert/key — check cert and key PEM files");
        serve_tls(app, listener, tls_acceptor).await;
    } else {
        warn!(
            %addr,
            cert = %cert_path,
            "TLS cert not found; webhook running in plain HTTP mode. \
             Set insecureSkipTLSVerify: true in ValidatingWebhookConfiguration for local clusters."
        );
        axum::serve(listener, app).await.unwrap();
    }
}

/// Accept TLS connections in a loop and drive each one with hyper's HTTP/1.1 builder.
async fn serve_tls(app: Router, listener: TcpListener, tls_acceptor: TlsAcceptor) {
    loop {
        let (tcp_stream, remote_addr) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                warn!(err = %e, "TCP accept error");
                continue;
            }
        };

        let tls_acceptor = tls_acceptor.clone();
        let app = app.clone();

        tokio::spawn(async move {
            let tls_stream = match tls_acceptor.accept(tcp_stream).await {
                Ok(s) => s,
                Err(e) => {
                    warn!(%remote_addr, err = %e, "TLS handshake failed");
                    return;
                }
            };

            let io = TokioIo::new(tls_stream);
            let svc = TowerToHyperService::new(app);

            if let Err(e) = Builder::new(TokioExecutor::new())
                .serve_connection(io, svc)
                .await
            {
                // "connection reset" is normal — don't log as a warning
                let msg = e.to_string();
                if !msg.contains("connection closed") && !msg.contains("reset") {
                    warn!(%remote_addr, err = %e, "connection error");
                }
            }
        });
    }
}

/// Build a `TlsAcceptor` from PEM-encoded cert and key files.
///
/// Supports PKCS#8, PKCS#1 (RSA), and SEC1 (EC) private key formats —
/// whatever cert-manager or openssl produces.
fn build_tls_acceptor(cert_path: &str, key_path: &str) -> anyhow::Result<TlsAcceptor> {
    let cert_pem = std::fs::read(cert_path)?;
    let key_pem = std::fs::read(key_path)?;

    let certs = rustls_pemfile::certs(&mut cert_pem.as_slice()).collect::<Result<Vec<_>, _>>()?;

    let key = rustls_pemfile::private_key(&mut key_pem.as_slice())?
        .ok_or_else(|| anyhow::anyhow!("no private key found in {key_path}"))?;

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;

    Ok(TlsAcceptor::from(Arc::new(config)))
}
