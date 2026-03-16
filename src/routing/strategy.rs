use std::sync::atomic::{AtomicUsize, Ordering};

use kube::ResourceExt;

use crate::crd::llm_workload::RoutingStrategy;
use crate::crd::{LLMProvider, LLMProviderStatus};

/// Select a provider from `candidates` (all assumed ready) using `strategy`.
///
/// Returns `None` if `candidates` is empty.
///
/// `round_robin_counter` is only consulted for `RoundRobin`; callers may pass
/// `&AtomicUsize::new(0)` for other strategies.
pub fn select_provider<'a>(
    candidates: &'a [&'a LLMProvider],
    strategy: &RoutingStrategy,
    round_robin_counter: &AtomicUsize,
) -> Option<&'a LLMProvider> {
    if candidates.is_empty() {
        return None;
    }

    match strategy {
        RoutingStrategy::CostAware => candidates
            .iter()
            .copied()
            .min_by(|a, b| {
                a.spec
                    .cost_per_token
                    .partial_cmp(&b.spec.cost_per_token)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),

        RoutingStrategy::LatencyFirst => {
            // Latency data lives in metrics; without it we fall back to CostAware.
            // Weekend 2: replace with real histogram lookups from MetricsRegistry.
            select_provider(candidates, &RoutingStrategy::CostAware, round_robin_counter)
        }

        RoutingStrategy::RoundRobin => {
            let idx = round_robin_counter.fetch_add(1, Ordering::Relaxed) % candidates.len();
            Some(candidates[idx])
        }

        RoutingStrategy::Failover => {
            // Stable primary = lexicographically first by name.
            candidates
                .iter()
                .copied()
                .min_by(|a, b| a.name_any().cmp(&b.name_any()))
        }
    }
}

/// Returns true if the provider's status indicates it is ready to serve traffic.
pub fn is_ready(provider: &LLMProvider) -> bool {
    provider
        .status
        .as_ref()
        .map(|s: &LLMProviderStatus| s.ready)
        .unwrap_or(false)
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
    use kube::ResourceExt;

    use crate::crd::{LLMProvider, LLMProviderSpec, LLMProviderStatus};

    use super::{is_ready, select_provider, RoutingStrategy};

    fn make_provider(name: &str, cost: f64, ready: bool) -> LLMProvider {
        LLMProvider {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                ..Default::default()
            },
            spec: LLMProviderSpec {
                endpoint: format!("https://{name}.example.com"),
                model: "gpt-4o-mini".into(),
                auth_secret_ref: None,
                cost_per_token: cost,
                rate_limit: None,
                health_check: None,
                tags: None,
            },
            status: Some(LLMProviderStatus {
                ready,
                last_probe_time: None,
                conditions: None,
            }),
        }
    }

    #[test]
    fn cost_aware_picks_cheapest() {
        let a = make_provider("expensive", 0.05, true);
        let b = make_provider("cheap", 0.001, true);
        let c = make_provider("mid", 0.01, true);
        let candidates = vec![&a, &b, &c];
        let counter = AtomicUsize::new(0);

        let chosen = select_provider(&candidates, &RoutingStrategy::CostAware, &counter).unwrap();
        assert_eq!(chosen.name_any(), "cheap");
    }

    #[test]
    fn cost_aware_empty_returns_none() {
        let counter = AtomicUsize::new(0);
        let result = select_provider(&[], &RoutingStrategy::CostAware, &counter);
        assert!(result.is_none());
    }

    #[test]
    fn round_robin_cycles_through_all() {
        let a = make_provider("a", 0.01, true);
        let b = make_provider("b", 0.01, true);
        let c = make_provider("c", 0.01, true);
        let candidates = vec![&a, &b, &c];
        let counter = AtomicUsize::new(0);

        let first = select_provider(&candidates, &RoutingStrategy::RoundRobin, &counter)
            .unwrap()
            .name_any();
        let second = select_provider(&candidates, &RoutingStrategy::RoundRobin, &counter)
            .unwrap()
            .name_any();
        let third = select_provider(&candidates, &RoutingStrategy::RoundRobin, &counter)
            .unwrap()
            .name_any();
        // All three slots filled (order depends on vec order)
        let visited: std::collections::HashSet<_> = [first, second, third].into_iter().collect();
        assert_eq!(visited.len(), 3);
    }

    #[test]
    fn failover_picks_lexicographically_first() {
        let z = make_provider("zoo", 0.01, true);
        let a = make_provider("alpha", 0.05, true);
        let m = make_provider("mid", 0.01, true);
        let candidates = vec![&z, &m, &a];
        let counter = AtomicUsize::new(0);

        let chosen = select_provider(&candidates, &RoutingStrategy::Failover, &counter).unwrap();
        assert_eq!(chosen.name_any(), "alpha");
    }

    #[test]
    fn is_ready_reflects_status() {
        let ready = make_provider("p", 0.01, true);
        let unready = make_provider("q", 0.01, false);
        // provider with no status → treated as not ready
        let no_status = LLMProvider {
            metadata: ObjectMeta {
                name: Some("r".into()),
                ..Default::default()
            },
            spec: LLMProviderSpec {
                endpoint: "https://r.example.com".into(),
                model: "gpt-4o-mini".into(),
                auth_secret_ref: None,
                cost_per_token: 0.0,
                rate_limit: None,
                health_check: None,
                tags: None,
            },
            status: None,
        };

        assert!(is_ready(&ready));
        assert!(!is_ready(&unready));
        assert!(!is_ready(&no_status));
    }
}
