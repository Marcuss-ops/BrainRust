//! Cross-language contract for the catalog quality policy.
//!
//! The fixture is owned by the Go catalog package and read verbatim here. A
//! policy change that only updates one language makes this test fail. The
//! fixture lives in the PipelineGen checkout, so this test skips only when a
//! standalone BrainRust checkout cannot see it.

use std::path::PathBuf;

use imagecatalog::{candidate_quality_problem, CatalogCandidate};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Contract {
    schema: String,
    cases: Vec<ContractCase>,
}

#[derive(Debug, Deserialize)]
struct ContractCase {
    name: String,
    want_eligible: bool,
    candidate: CatalogCandidate,
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../refactored/internal/capabilities/images/entitycatalog/testdata/catalog_policy_contract.json",
    )
}

#[test]
fn catalog_policy_matches_the_shared_go_fixture() {
    let path = fixture_path();
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            eprintln!(
                "skip: Go policy fixture not found at {} (run from the workspace checkout)",
                path.display()
            );
            return;
        }
        Err(err) => panic!("read shared policy fixture {}: {err}", path.display()),
    };
    let contract: Contract = serde_json::from_str(&raw).expect("shared policy fixture schema");
    assert_eq!(contract.schema, "entity_image_catalog_policy.v1");
    assert!(contract.cases.len() >= 7);
    for case in contract.cases {
        let problem = candidate_quality_problem(&case.candidate, None);
        let eligible = problem.is_none();
        assert_eq!(
            eligible, case.want_eligible,
            "{}: problem = {:?}, want eligible = {}",
            case.name, problem, case.want_eligible
        );
    }
}
