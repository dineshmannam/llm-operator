//! Reconciler for `LLMWorkload` resources.
//!
//! Reconcile loop responsibilities:
//!   1. List all LLMProviders matching the workload's providerSelector.
//!   2. Filter to only Ready providers.
//!   3. Pass the candidate list to the routing engine to select an active provider.
//!   4. Write the resolved endpoint URL into the target ConfigMap (creating if absent).
//!   5. Patch the workload's `.status.activeProvider` and budget usage.
//!   6. Enforce budget: if tokensBudgetUsed >= budgetPerHour, set provider to a
//!      no-op fallback and emit a Warning event.
//!
//! The ConfigMap approach keeps this operator decoupled from the inference proxy;
//! the proxy just reads `PROVIDER_ENDPOINT` from the mounted ConfigMap.

// TODO Weekend 2: implement pub async fn reconcile(workload: Arc<LLMWorkload>, ctx: Arc<Context>)
// TODO Weekend 2: implement ConfigMap write helper
// TODO Weekend 2: implement budget enforcement logic
