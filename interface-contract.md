# Interface Contract: llm-operator → RAG App

Defines exactly what the operator writes and what the application consumes.
This is the boundary between the Rust infrastructure layer and the Python application layer.

---

## Overview

```
┌─────────────────────┐        writes        ┌──────────────────┐
│   llm-operator      │ ──────────────────►  │   ConfigMap      │
│   (Rust, infra)     │                       │   (K8s resource) │
└─────────────────────┘                       └────────┬─────────┘
                                                       │ reads
                                              ┌────────▼─────────┐
                                              │   RAG App        │
                                              │   (Python)       │
                                              └──────────────────┘

Auth key is NEVER copied — app reads it directly from the original Secret.
Operator only writes the reference (secret name + key) into the ConfigMap.
```

---

## What the Operator Writes

### ConfigMap: `spec.targetConfigMap` (same namespace as LLMWorkload)

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: <LLMWorkload.spec.targetConfigMap>   # e.g. "summarizer-llm-config"
  namespace: <same as LLMWorkload>
  labels:
    managed-by: llm-operator
    llmworkload: <LLMWorkload name>
data:
  # ── Routing resolution ─────────────────────────────────────────
  PROVIDER_ENDPOINT: "https://openrouter.ai/api/v1"
  PROVIDER_NAME: "openrouter"
  MODEL_NAME: "openai/gpt-4o"

  # ── Auth reference (not the key value — just where to find it) ─
  AUTH_SECRET_NAME: "openrouter-key"
  AUTH_SECRET_KEY: "apiKey"

  # ── Budget signal ──────────────────────────────────────────────
  BUDGET_REMAINING_USD: "4.28"        # updated each reconcile cycle
  BUDGET_LIMIT_USD: "5.00"

  # ── Metadata (useful for logging / tracing in the app) ─────────
  ROUTING_STRATEGY: "CostAware"
  OPERATOR_RECONCILED_AT: "2026-03-16T14:32:00Z"
```

### What the Operator Does NOT Write
- The API key value (stays in the original Secret, operator only passes the reference)
- Per-request routing decisions (operator is config-layer, not proxy-layer — see note below)
- Model parameters (temperature, max_tokens) — app owns those

---

## What the RAG App Reads

### Startup / reload
```python
import os
from kubernetes import client, config

config.load_incluster_config()
v1 = client.CoreV1Api()

cm = v1.read_namespaced_config_map(
    name=os.environ["LLM_CONFIG_MAP"],      # injected via Deployment env
    namespace=os.environ["POD_NAMESPACE"],
)
data = cm.data

PROVIDER_ENDPOINT = data["PROVIDER_ENDPOINT"]
MODEL_NAME        = data["MODEL_NAME"]

# Resolve auth key from the referenced Secret (never from ConfigMap)
secret = v1.read_namespaced_secret(
    name=data["AUTH_SECRET_NAME"],
    namespace=os.environ["POD_NAMESPACE"],
)
import base64
API_KEY = base64.b64decode(
    secret.data[data["AUTH_SECRET_KEY"]]
).decode()
```

### Making an LLM call (OpenAI-compatible)
```python
from openai import OpenAI

llm = OpenAI(base_url=PROVIDER_ENDPOINT, api_key=API_KEY)

response = llm.chat.completions.create(
    model=MODEL_NAME,
    messages=[{"role": "user", "content": prompt}],
)
```

This works for OpenRouter, Ollama (v1 mode), and any OpenAI-compatible endpoint
— the app doesn't need to know which backend is active.

---

## CRD Change Required

`LLMProviderSpec` needs a `model` field — it doesn't have one yet.

```rust
// Add to LLMProviderSpec in src/crd/llm_provider.rs:

/// Default model identifier to use with this provider.
/// Format is provider-specific:
///   OpenRouter: "openai/gpt-4o", "anthropic/claude-3-5-sonnet"
///   Ollama:     "llama3", "mistral", "phi3"
///   OpenAI:     "gpt-4o", "gpt-4o-mini"
pub model: String,
```

Update the example YAMLs accordingly:
```yaml
# openrouter-provider.yaml
spec:
  model: "openai/gpt-4o-mini"   # cheaper default; workload can override later

# ollama-provider.yaml
spec:
  model: "llama3"
```

---

## Contract Guarantees (operator promises to the app)

| Guarantee | Detail |
|---|---|
| ConfigMap always exists | Operator creates it on first reconcile, never deletes it |
| Fields always present | All keys in the schema above are always written, never partial |
| AUTH_SECRET_NAME resolves | Operator only selects providers whose secret exists and is readable |
| BUDGET_REMAINING_USD ≥ 0 | If budget is exhausted, operator switches provider (or writes "0.00") before the app sees it |
| OPERATOR_RECONCILED_AT is fresh | If the timestamp is stale (> 2× probe interval), app should treat config as potentially outdated |

---

## What This Contract Deliberately Does NOT Cover

- **Per-request routing** — the operator picks a provider at reconcile time, not per-request. All requests within a reconcile window go to the same provider. In production you'd add a proxy layer for per-request decisions.
- **Token counting** — the app is responsible for tracking its own token usage and reporting back (Weekend 2: workload reconciler will read a metric to update `tokensBudgetUsed`).
- **Failover mid-request** — if the provider goes down between reconcile cycles, the app sees a failed HTTP call. The operator will detect it and update the ConfigMap on the next probe, but there's no real-time signal. A proxy would handle this inline.

These are known tradeoffs for a demo/portfolio scope. See README "Production Considerations" for the full picture.

---

## Dependency Direction

```
RAG App  →  reads  →  ConfigMap  ←  writes  ←  llm-operator
RAG App  →  reads  →  Secret     (operator only passes the reference)
```

The app has zero dependency on the operator binary or its APIs.
The operator has zero knowledge of the app.
The ConfigMap is the only coupling point — intentional.
