# llm-operator

[![CI](https://github.com/dineshmannam/llm-operator/actions/workflows/ci.yml/badge.svg)](https://github.com/dineshmannam/llm-operator/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/dineshmannam/llm-operator)](https://github.com/dineshmannam/llm-operator/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org)
[![k8s: 1.27+](https://img.shields.io/badge/k8s-1.27%2B-326CE5?logo=kubernetes&logoColor=white)](https://kubernetes.io)

A production-grade Kubernetes operator written in Rust ([kube-rs](https://github.com/kube-rs/kube)) that manages LLM provider lifecycles, enforces cost budgets, and routes inference workloads across heterogeneous backends.

Built as a portfolio piece demonstrating: operator patterns, cost-aware distributed routing, admission control, and Prometheus observability — all on top of the Kubernetes control loop model.

---

## Demo

> Video walkthrough coming soon — will cover the demo scenes below end-to-end on an AKS cluster.

Key scenes covered in the demo:

| Scene | What it shows |
|---|---|
| Provider health transitions | `kubectl describe llmprovider` shows Ready ↔ Unhealthy K8s Events |
| Cost-aware routing | Workload reconciler picks cheapest ready provider; ConfigMap updated live |
| Fallback routing | Primary provider goes down; operator reroutes to fallback within one reconcile loop |
| Budget cap enforcement | Webhook rejects `budgetPerHour: 999` at admission time — no pod scheduled |
| Compliance rejection | `noTrainingData: true` workload rejected when no provider carries the opt-out label |
| Prometheus metrics | `curl /metrics` shows token counters, routing latency histogram, budget gauges |

---

## Architecture

```
┌─────────────────────────────────────────────────────┐
│                   Kubernetes API                    │
│                                                     │
│  LLMProvider CRD          LLMWorkload CRD           │
│  (OpenRouter, Ollama…)    (routing policy, budget)  │
└────────┬──────────────────────────┬─────────────────┘
         │ watch/reconcile          │ watch/reconcile
         ▼                          ▼
┌─────────────────┐      ┌──────────────────────────┐
│ ProviderCtrl    │      │  WorkloadCtrl             │
│                 │      │                           │
│ • health probe  │      │ • resolve provider chain  │
│ • status patch  │      │ • apply routing strategy  │
│ • emit events   │      │ • write ConfigMap target  │
└─────────────────┘      └──────────────────────────┘
                                    │
                         ┌──────────▼──────────┐
                         │   Routing Engine     │
                         │                     │
                         │ cost-aware │ latency │
                         │ round-robin│ fallback│
                         └─────────────────────┘

┌──────────────────────────┐   ┌──────────────────┐
│ Admission Webhook (axum) │   │ Metrics (/metrics)│
│                          │   │                  │
│ • validate budget caps   │   │ • token counters  │
│ • reject bad providers   │   │ • routing latency │
│ • compliance labels      │   │ • budget gauges   │
└──────────────────────────┘   └──────────────────┘
```

## Design Decisions

| Decision | Rationale |
|---|---|
| Rust + kube-rs | Memory safety + zero-cost async; no JVM GC pauses in the hot reconcile path |
| Two separate CRDs | `LLMProvider` is cluster-scoped (ops concern); `LLMWorkload` is namespace-scoped (dev concern) |
| ConfigMap as routing target | Decouples operator from inference proxy; any sidecar can read the resolved endpoint |
| Admission webhook for budget | Enforces cost policy at admission time, before pods are scheduled |
| Distroless final image | Smallest possible attack surface; no shell in prod |

## Consuming the Operator (App Integration)

The operator exposes a stable interface via Kubernetes primitives — no SDK, no direct dependency on the operator binary. See **[interface-contract.md](./interface-contract.md)** for:

- Exactly what the operator writes to the ConfigMap
- How the app resolves the auth secret reference
- Contract guarantees (what the operator promises)
- Known tradeoffs vs. a full proxy-in-the-path architecture

## Quick Start

### Local / CI cluster

```bash
# 1. Install CRDs
kubectl apply -f config/crd/

# 2. Deploy operator via Helm
helm install llm-operator helm/llm-operator/ \
  --set image.tag=v0.1.0 \
  --set webhook.insecureSkipTLSVerify=true   # for clusters without cert-manager

# 3. Create a provider
kubectl apply -f examples/openrouter-provider.yaml

# 4. Create a workload with cost-aware routing
kubectl apply -f examples/summarization-workload.yaml

# 5. Check status
kubectl get llmproviders,llmworkloads -A
```

### AKS (demo cluster)

```bash
# Provision a minimal cluster (1 system node + 2 user nodes)
az aks create \
  --resource-group llm-operator-demo \
  --name llm-operator-aks \
  --node-count 1 \
  --node-vm-size Standard_D2s_v3 \
  --nodepool-name system

az aks nodepool add \
  --resource-group llm-operator-demo \
  --cluster-name llm-operator-aks \
  --name workloads \
  --node-count 2 \
  --node-vm-size Standard_D2s_v3

az aks get-credentials --resource-group llm-operator-demo --name llm-operator-aks

# Install cert-manager for TLS (optional — skip for demo with insecureSkipTLSVerify)
kubectl apply -f https://github.com/cert-manager/cert-manager/releases/latest/download/cert-manager.yaml
kubectl apply -f config/tls/issuer.yaml
kubectl apply -f config/tls/certificate.yaml

# Deploy
kubectl apply -f config/crd/
helm install llm-operator helm/llm-operator/ --set image.tag=v0.1.0
```

## Production Considerations

**This is a config-layer operator, not a proxy.** The operator picks a provider at reconcile time (every 60s by default) and writes the result to a ConfigMap. Per-request routing decisions — load shedding, circuit breaking, streaming — belong in a proxy sidecar that reads from that ConfigMap. This is a deliberate tradeoff documented in [interface-contract.md](./interface-contract.md).

| Concern | Current state | Path forward |
|---|---|---|
| Per-request routing | Not supported — provider selected at reconcile time | Add an Envoy/NGINX sidecar that reads the ConfigMap |
| Token tracking | Budget gauge is static; actual spend not metered | Wire a token-counting middleware into the proxy layer |
| TLS | Auto-detected at startup; falls back to HTTP if no cert found | Use cert-manager with `config/tls/` manifests in production |
| HA | Single replica by default | Set `replicaCount > 1`; leader election is already enabled |

## Project Status

| Weekend | Focus | Status |
|---|---|---|
| 1 | CRD structs + provider health controller | [x] done |
| 2 | Workload reconciler + admission webhook | [x] done |
| 3 | Helm chart + CI + README polish | [x] done |

## License

MIT
