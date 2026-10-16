use sha2::{Digest, Sha256};

use crate::matcher::literal::CompiledLiterals;
use crate::matcher::CandidateBudget;
use crate::model::{RuleSet, Scene};
use crate::normalize::table_by_id;

#[test]
fn differential_10k_frozen_literal_alias_projection() {
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/oracle/manifest.json")).unwrap();
    let corpus = include_bytes!("../fixtures/oracle/cases-10k.jsonl");
    assert_eq!(
        format!("{:x}", Sha256::digest(corpus)),
        manifest["cases_sha256"].as_str().unwrap()
    );
    assert_eq!(manifest["seed"], 20261016);
    let rules: RuleSet = serde_json::from_str(include_str!("oracle-ruleset.json")).unwrap();
    let engine =
        CompiledLiterals::compile(&rules, table_by_id(&rules.t2s_table_id).unwrap()).unwrap();
    let mut cases = 0;
    let mut deferred_gap_hits = 0;
    let mut checked_hits = 0;
    for line in std::str::from_utf8(corpus).unwrap().lines() {
        let case: serde_json::Value = serde_json::from_str(line).unwrap();
        let scene: Scene = serde_json::from_value(case["scene"].clone()).unwrap();
        let expected: Vec<_> = case["expected"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|hit| {
                let deferred = hit["methods"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|method| method == "gap" || method == "regex");
                if deferred {
                    deferred_gap_hits += 1;
                }
                !deferred
            })
            .cloned()
            .collect();
        let actual = engine
            .scan_text(
                case["text"].as_str().unwrap(),
                scene,
                case["t2s"].as_bool().unwrap(),
                &mut CandidateBudget::default(),
                &|| false,
            )
            .unwrap();
        let actual:Vec<_>=actual.into_iter().map(|hit|serde_json::json!({"rule_id":hit.rule_id,"variant_id":hit.variant_id,"action":hit.action,"start":hit.start,"end":hit.end,"methods":hit.methods})).collect();
        assert_eq!(actual, expected, "frozen oracle case {}", case["id"]);
        checked_hits += actual.len();
        cases += 1;
    }
    assert_eq!(cases, 10_000);
    assert_eq!(manifest["case_count"], cases);
    println!("T08_DIFFERENTIAL cases={cases} literal_alias_hits={checked_hits} deferred_gap_hits={deferred_gap_hits} seed=20261016 scope=literal-alias-only");
}
