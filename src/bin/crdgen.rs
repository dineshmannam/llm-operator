//! Utility binary: prints CRD YAML for all custom resources to stdout.
//!
//! Usage:
//!   cargo run --bin crdgen > config/crd/llmprovider.yaml
//!
//! Run this whenever the CRD structs change and commit the updated YAML.

use kube::CustomResourceExt;
use llm_operator::crd::{LLMProvider, LLMWorkload};

fn main() {
    for yaml in [
        serde_yaml::to_string(&LLMProvider::crd()).unwrap(),
        serde_yaml::to_string(&LLMWorkload::crd()).unwrap(),
    ] {
        println!("---");
        print!("{yaml}");
    }
}
