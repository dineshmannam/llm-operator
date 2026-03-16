//! Admission webhook handler for `LLMWorkload` resources.
//!
//! Validates incoming LLMWorkload CREATE/UPDATE requests:
//!
//!   1. **Budget cap check** — rejects if `spec.budgetPerHour` exceeds a
//!      cluster-wide maximum (configurable via operator flags).
//!   2. **Provider existence check** — rejects if none of the named providers
//!      in `spec.providerSelector` exist in the cluster yet.
//!   3. **Compliance label check** — if `spec.compliance.dataResidency` is set,
//!      validates that all selected providers carry the matching region label.
//!   4. **No-training-data enforcement** — if `spec.compliance.noTrainingData`
//!      is true, rejects providers whose LLMProvider spec doesn't declare
//!      `trainingOptOut: true`.
//!
//! Returns a Kubernetes `AdmissionResponse` with `allowed: true/false` and a
//! human-readable `status.message` on rejection.

// TODO Weekend 2: define handler fn validate_llm_workload(Json(review): Json<AdmissionReview>) -> Json<AdmissionResponse>
// TODO Weekend 2: implement each validation check as a private fn returning Result<(), String>
// TODO Weekend 2: add unit tests with synthetic AdmissionReview fixtures
