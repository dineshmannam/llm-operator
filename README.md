# llm-operator

A production-grade Kubernetes operator written in Rust ([kube-rs](https://github.com/kube-rs/kube)) that manages LLM provider lifecycles, enforces cost budgets, and routes inference workloads across heterogeneous backends.

Built as a portfolio piece demonstrating: operator patterns, cost-aware distributed routing, admission control, and Prometheus observability — all on top of the Kubernetes control loop model.

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

```bash
# 1. Install CRDs
kubectl apply -f config/crd/

# 2. Deploy operator via Helm (replace <version> with a release tag, e.g. v0.1.0)
helm install llm-operator helm/llm-operator/ \
  --set image.tag=v0.1.0 \
  --set webhook.enabled=true

# 3. Create a provider
kubectl apply -f examples/openrouter-provider.yaml

# 4. Create a workload with cost-aware routing
kubectl apply -f examples/summarization-workload.yaml

# 5. Check status
kubectl get llmproviders,llmworkloads -A
```

## Project Status

| Weekend | Focus | Status |
|---|---|---|
| 1 | CRD structs + provider health controller | [x] done |
| 2 | Workload reconciler + admission webhook | [ ] planned |
| 3 | Helm chart + CI + README polish | [ ] planned |

## License

MIT
