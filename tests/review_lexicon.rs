use imboy_textguard::engine::Snapshot;
use imboy_textguard::model::{Action, CheckRequest, RuleSet, Scene, Severity, TextField};
use imboy_textguard::normalize::t2s;
use imboy_textguard::rules::RulesState;
use std::path::PathBuf;

const DATA: &str = "data/dictionaries/review_sources_20261020";

fn package(category: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(DATA)
            .join("rulesets")
            .join(format!("{category}.ruleset.json")),
    )
    .expect("checked-in ruleset must exist; no downloading during tests")
}

fn request(text: &str, scene: Scene) -> CheckRequest {
    CheckRequest {
        request_id: "lexicon-verification".into(),
        scene,
        fields: vec![TextField {
            field_id: "nickname".into(),
            text: text.into(),
        }],
    }
}

#[test]
fn all_five_packages_compile_and_every_term_is_review_only_in_profile() {
    let mut total = 0;
    for category in [
        "sexual",
        "drugs",
        "terrorism",
        "political_abuse",
        "all_categories",
    ] {
        let rules: RuleSet = serde_json::from_slice(&package(category)).unwrap();
        assert!(!rules.literals.is_empty());
        assert!(rules.regex_rules.is_empty());
        assert!(rules.exceptions.is_empty());
        let table = t2s::table_by_id(&rules.t2s_table_id).unwrap();
        let snapshot = Snapshot::build(&rules, table).unwrap();
        for rule in &rules.literals {
            assert_eq!(rule.severity, Severity::Medium);
            assert_eq!(rule.scenes, vec![Scene::ProfileField]);
            assert!(rule.aliases.is_empty());
            assert!(!rule.gap.enabled);
            let result = snapshot
                .check(&request(&rule.word, Scene::ProfileField), &|| false)
                .unwrap();
            assert_eq!(result.decision, Action::Review, "rule {}", rule.id);
            assert!(result.scan_complete);
            assert!(result.hits.iter().any(|hit| hit.rule_id == rule.id));
        }
        if category != "all_categories" {
            total += rules.literals.len();
        } else {
            assert_eq!(total, rules.literals.len());
            for scene in [Scene::ChannelMessage, Scene::MomentPost] {
                let result = snapshot
                    .check(&request(&rules.literals[0].word, scene), &|| false)
                    .unwrap();
                assert_eq!(result.decision, Action::Allow);
                assert!(result.hits.is_empty());
            }
            for text in [
                "李小明",
                "张三",
                "清风明月",
                "中文学习小组",
                "药物安全教育",
                "新闻阅读",
                "普通政治讨论",
                "政府工作报告",
            ] {
                let result = snapshot
                    .check(&request(text, Scene::ProfileField), &|| false)
                    .unwrap();
                assert_eq!(result.decision, Action::Allow);
            }
        }
    }
}

#[test]
fn full_ruleset_publishes_and_recovers_without_changing_bootstrap() {
    let dir = std::env::temp_dir().join(format!(
        "textguard-lexicon-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let bytes = package("all_categories");
    let expected: RuleSet = serde_json::from_slice(&bytes).unwrap();
    let table = t2s::table_by_id(&expected.t2s_table_id).unwrap();
    let state = RulesState::load(&dir, table).unwrap();
    let (_, etag) = state.active_state().unwrap();
    let receipt = state.publish(&bytes, Some(&etag)).unwrap();
    assert_eq!(receipt.generation, 1);
    assert_eq!(receipt.scene_counts, [0, 0, expected.literals.len()]);
    drop(state);
    let restored = RulesState::load(&dir, table).unwrap();
    assert!(restored.ready());
    assert_eq!(restored.current().unwrap().revision(), receipt.revision);
    assert_eq!(
        restored.current().unwrap().counts().canonical,
        expected.literals.len()
    );
    drop(restored);
    std::fs::remove_dir_all(dir).unwrap();
    let seed: RuleSet = serde_json::from_slice(include_bytes!(
        "../data/dictionaries/default_review_seed.json"
    ))
    .unwrap();
    assert_eq!(
        seed.literals.len(),
        52,
        "this PR must not silently replace the bootstrap"
    );
}
