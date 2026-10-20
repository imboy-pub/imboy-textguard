use imboy_textguard::engine::Snapshot;
use imboy_textguard::model::{Action, CheckRequest, RuleSet, Scene, TextField};
use imboy_textguard::normalize::t2s;

#[test]
fn bundled_review_seed_is_valid_and_categorized() {
    let bytes = include_bytes!("../data/dictionaries/default_review_seed.json");
    let rules: RuleSet = serde_json::from_slice(bytes).expect("valid JSON ruleset");
    assert_eq!(rules.literals.len(), 52);
    let mut counts = std::collections::BTreeMap::new();
    for rule in &rules.literals {
        *counts.entry(rule.category.as_str()).or_insert(0usize) += 1;
        assert!(rule.aliases.is_empty());
        assert!(!rule.gap.enabled);
    }
    assert_eq!(counts.get("sexual"), Some(&14));
    assert_eq!(counts.get("drugs"), Some(&14));
    assert_eq!(counts.get("terrorism"), Some(&12));
    assert_eq!(counts.get("political_abuse"), Some(&12));
    let table = t2s::table_by_id(&rules.t2s_table_id).expect("supported mapping");
    let snapshot = Snapshot::build(&rules, table).expect("compilable seed");
    let request = CheckRequest {
        request_id: "seed-smoke".into(),
        scene: Scene::ProfileField,
        fields: vec![TextField {
            field_id: "nickname".into(),
            text: "招嫖服务".into(),
        }],
    };
    let result = snapshot.check(&request, &|| false).expect("check succeeds");
    assert_eq!(result.decision, Action::Review);
    assert!(result.scan_complete);
}
