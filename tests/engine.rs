//! T11 acceptance tests: engine aggregation, ordering, budget and the
//! single-snapshot guarantee (plan 4.9, AC-046..AC-051).

use imboy_textguard::engine::Snapshot;
use imboy_textguard::model::{
    self, Action, AliasBoundary, AliasRule, AliasType, CheckRequest, ErrorCode, GapConfig,
    MatchMethod, RegexRule, RuleAction, RuleSet, Scene, SceneSemantics, Semantics, Severity,
    TextField, VariantType,
};
use imboy_textguard::normalize::table_by_id;

fn base_ruleset() -> RuleSet {
    serde_json::from_str(include_str!("fixtures/contract-ruleset.json")).unwrap()
}

fn enhanced() -> RuleSet {
    let mut rules = base_ruleset();
    rules.semantics_by_scene = SceneSemantics {
        channel_message: Semantics::EnhancedV1,
        moment_post: Semantics::EnhancedV1,
        profile_field: Semantics::EnhancedV1,
    };
    rules
}

fn snapshot(rules: &RuleSet) -> Snapshot {
    let table = table_by_id(&rules.t2s_table_id).unwrap();
    Snapshot::build(rules, table).unwrap()
}

fn check(
    snapshot: &Snapshot,
    id: &str,
    scene: Scene,
    fields: &[(&str, &str)],
) -> imboy_textguard::model::CheckResponse {
    snapshot
        .check(
            &CheckRequest {
                request_id: id.into(),
                scene,
                fields: fields
                    .iter()
                    .map(|(field_id, text)| TextField {
                        field_id: (*field_id).into(),
                        text: (*text).into(),
                    })
                    .collect(),
            },
            &|| false,
        )
        .unwrap()
}

const NO_CANCEL: fn() -> bool = || false;

#[test]
fn ac046_strongest_action_is_never_downgraded() {
    let mut rules = enhanced();
    rules.literals[0].word = "测试".into();
    rules.literals[0].severity = Severity::High; // canonical -> block
    rules.literals[0].aliases = vec![alias_rule("p1", "test", RuleAction::Review)];
    rules.literals[0].gap = GapConfig {
        enabled: true,
        action: RuleAction::Review,
    };
    let snap = snapshot(&rules);
    let response = check(
        &snap,
        "r1",
        Scene::ChannelMessage,
        &[("body", "x test 测试 y")],
    );
    assert_eq!(response.decision, Action::Block);
    assert!(response
        .hits
        .iter()
        .any(|hit| hit.effective_action == RuleAction::Block));
    // block witnesses sort ahead of the review evidence
    assert_eq!(response.hits[0].effective_action, RuleAction::Block);
}

#[test]
fn ac047_high_rule_alias_evidence_stays_review() {
    let mut rules = enhanced();
    rules.literals[0].word = "软件测试".into();
    rules.literals[0].severity = Severity::High; // canonical action would be block
    rules.literals[0].aliases = vec![alias_rule("p1", "testcase", RuleAction::Review)];
    let snap = snapshot(&rules);
    // only the alias fires; the engine reports the alias effective action
    let response = check(
        &snap,
        "r2",
        Scene::ChannelMessage,
        &[("body", "a testcase b")],
    );
    assert_eq!(response.decision, Action::Review);
    assert_eq!(response.hits.len(), 1);
    assert_eq!(response.hits[0].severity, Severity::High);
    assert_eq!(response.hits[0].effective_action, RuleAction::Review);
    assert_eq!(response.hits[0].variant_type, VariantType::Pinyin);
}

#[test]
fn ac048a_truncation_after_full_scan_keeps_the_block() {
    let mut rules = enhanced();
    rules.literals[0].word = "测试".into();
    rules.literals[0].severity = Severity::Low; // review flood
                                                // a second high rule appears once at the end of the field
    let high = model::LiteralRule {
        id: "1002".into(),
        word: "封禁".into(),
        category: "test".into(),
        severity: Severity::High,
        scenes: vec![Scene::ChannelMessage],
        aliases: vec![],
        gap: GapConfig {
            enabled: false,
            action: RuleAction::Review,
        },
    };
    rules.literals.push(high);
    let snap = snapshot(&rules);
    let text = format!("{}{}", "测试".repeat(120), "封禁");
    let response = check(&snap, "r3", Scene::ChannelMessage, &[("body", &text)]);
    assert_eq!(response.decision, Action::Block);
    assert_eq!(response.hits.len(), 100);
    assert!(response.hits_truncated);
    assert_eq!(response.hits[0].effective_action, RuleAction::Block);
    assert_eq!(response.hits[0].rule_id, "1002");
}

#[test]
fn ac048b_regex_first_witnesses_overflow_the_response_bound() {
    let mut rules = enhanced();
    rules.literals.clear();
    rules.allow_empty = true;
    for index in 1..=32 {
        rules.regex_rules.push(RegexRule {
            id: format!("re:r{index:02}"),
            expression: format!("zz{index}\\d+"),
            display_word: format!("正则{index}"),
            category: "test".into(),
            severity: Severity::Low,
            action: RuleAction::Review,
            scenes: vec![Scene::ChannelMessage],
        });
    }
    let snap = snapshot(&rules);
    let fields: Vec<(String, String)> = (0..8)
        .map(|field| {
            (
                format!("f{field}"),
                (1..=32)
                    .map(|index| format!("zz{index}7 "))
                    .collect::<String>(),
            )
        })
        .collect();
    let refs: Vec<(&str, &str)> = fields
        .iter()
        .map(|(id, text)| (id.as_str(), text.as_str()))
        .collect();
    let response = check(&snap, "r4", Scene::ChannelMessage, &refs);
    // 32 rules x 8 fields = 256 first witnesses; only 100 are returned
    assert_eq!(response.hits.len(), 100);
    assert!(response.hits_truncated);
    assert!(response.hits.iter().all(|hit| hit.variant_id == "regex"));
    assert!(response.scan_complete);
}

#[test]
fn ac049_request_budget_spans_all_fields_and_never_partially_succeeds() {
    let mut rules = enhanced();
    rules.literals.clear();
    // five rules share the same one-character word: every occurrence fans
    // out to five candidates
    for id in ["1001", "1002", "1003", "1004", "1005"] {
        rules.literals.push(model::LiteralRule {
            id: id.into(),
            word: "测".into(),
            category: "test".into(),
            severity: Severity::Low,
            scenes: vec![Scene::ChannelMessage],
            aliases: vec![],
            gap: GapConfig {
                enabled: false,
                action: RuleAction::Review,
            },
        });
    }
    let snap = snapshot(&rules);
    let request = CheckRequest {
        request_id: "r5".into(),
        scene: Scene::ChannelMessage,
        fields: vec![TextField {
            field_id: "body".into(),
            text: "测".repeat(4400), // 13200 bytes, 4400 chars x 5 rules = 22000
        }],
    };
    let outcome = snap.check(&request, &NO_CANCEL);
    assert_eq!(outcome.unwrap_err(), ErrorCode::MatchBudgetExceeded);
}

#[test]
fn ac050_every_field_reads_the_same_snapshot() {
    let mut rules = enhanced();
    rules.literals[0].word = "测试".into();
    rules.literals[0].severity = Severity::High;
    let first = snapshot(&rules);
    let before = check(
        &first,
        "r6a",
        Scene::ChannelMessage,
        &[("a", "测试"), ("b", "测试")],
    );
    assert_eq!(before.decision, Action::Block);
    assert_eq!(before.fields_checked, 2);
    let revision = before.ruleset_revision.clone();

    // build a different snapshot; the old one keeps serving its own rules
    let mut other = rules.clone();
    other.literals[0].word = "无关".into();
    let second = snapshot(&other);
    let after_old = check(
        &first,
        "r6b",
        Scene::ChannelMessage,
        &[("a", "测试"), ("b", "测试")],
    );
    let after_new = check(
        &second,
        "r6c",
        Scene::ChannelMessage,
        &[("a", "测试"), ("b", "测试")],
    );
    assert_eq!(after_old.decision, Action::Block);
    assert_eq!(after_old.ruleset_revision, revision);
    assert_eq!(after_new.decision, Action::Allow);
    assert_ne!(after_new.ruleset_revision, revision);
    // one request reports exactly one revision/generation pair everywhere
    assert_eq!(after_old.generation, before.generation);
    assert_eq!(after_old.base_rules_digest, before.base_rules_digest);
}

#[test]
fn ac051_dedup_keeps_strongest_simplest_and_never_merges_positions() {
    let mut rules = enhanced();
    rules.literals[0].word = "测试".into();
    rules.literals[0].severity = Severity::High;
    let snap = snapshot(&rules);
    // B and T views both contain the simplified word at the same place;
    // aggregation keeps one strongest-simplest [literal] witness
    let response = check(
        &snap,
        "r7",
        Scene::ChannelMessage,
        &[("body", "测试 再次 测试")],
    );
    assert_eq!(response.hits.len(), 2, "two positions stay separate");
    assert!(response
        .hits
        .iter()
        .all(|hit| hit.match_methods == vec![MatchMethod::Literal]));

    // traditional text: only the T view fires, keeping the t2s evidence
    let response = check(&snap, "r7b", Scene::ChannelMessage, &[("body", "測試")]);
    assert_eq!(response.hits.len(), 1);
    assert_eq!(
        response.hits[0].match_methods,
        vec![MatchMethod::Literal, MatchMethod::T2s]
    );
}

#[test]
fn capability_matrix_compat_runs_canonical_literal_only() {
    let mut rules = enhanced();
    rules.literals[0].word = "测试词".into();
    rules.literals[0].severity = Severity::High;
    rules.literals[0].aliases = vec![alias_rule("p1", "ceshici", RuleAction::Review)];
    rules.literals[0].gap = GapConfig {
        enabled: true,
        action: RuleAction::Review,
    };
    rules.regex_rules.push(RegexRule {
        id: "re:zz".into(),
        expression: "zz\\d+".into(),
        display_word: "正则词".into(),
        category: "test".into(),
        severity: Severity::Low,
        action: RuleAction::Review,
        scenes: vec![Scene::ProfileField, Scene::ChannelMessage],
    });
    rules.semantics_by_scene.profile_field = Semantics::CompatV1;
    rules.exceptions = vec![model::ExceptionRule {
        id: "exc".into(),
        phrase: "软件测试词".into(),
        scenes: vec![Scene::ProfileField],
        rule_id: "1001".into(),
        variant_ids: vec!["canonical".into()],
        views: vec![model::View::B],
    }];
    let snap = snapshot(&rules);
    let compat = check(
        &snap,
        "r8a",
        Scene::ProfileField,
        &[("body", "软件测试词 測試詞 ceshici zz9")],
    );
    // compat: only plain canonical literals on the B(compat) view; no gap,
    // no alias, no regex, no t2s, and exceptions never apply
    assert_eq!(compat.decision, Action::Block);
    assert_eq!(compat.hits.len(), 1);
    assert_eq!(compat.hits[0].match_methods, vec![MatchMethod::Literal]);
    assert_eq!(compat.semantics_id, "compat-v1");

    let enhanced_response = check(
        &snap,
        "r8b",
        Scene::ChannelMessage,
        &[("body", "软件测*试词 測試詞 ceshici zz9")],
    );
    // enhanced: gap (测*试词), t2s (測試詞), alias (ceshici) and regex (zz9)
    let methods: Vec<Vec<MatchMethod>> = enhanced_response
        .hits
        .iter()
        .map(|hit| hit.match_methods.clone())
        .collect();
    assert!(methods.contains(&vec![MatchMethod::Literal, MatchMethod::Gap]));
    assert!(methods.contains(&vec![MatchMethod::Literal, MatchMethod::T2s]));
    assert!(methods.contains(&vec![MatchMethod::Alias]));
    assert!(methods.contains(&vec![MatchMethod::Regex]));
    assert_eq!(enhanced_response.semantics_id, "enhanced-v1");
    assert!(enhanced_response.scan_complete);
    assert!(!enhanced_response.hits_truncated);
    // the plain literal inside the exception phrase is exempted in enhanced
    assert!(!enhanced_response
        .hits
        .iter()
        .any(|hit| hit.match_methods == vec![MatchMethod::Literal]));
}

#[test]
fn cancellation_returns_error_not_allow() {
    let rules = enhanced();
    let snap = snapshot(&rules);
    let request = CheckRequest {
        request_id: "r9".into(),
        scene: Scene::ChannelMessage,
        fields: vec![TextField {
            field_id: "body".into(),
            text: "测试词".into(),
        }],
    };
    let outcome = snap.check(&request, &|| true);
    assert_eq!(outcome.unwrap_err(), ErrorCode::CheckDeadline);
}

#[test]
fn invalid_requests_are_rejected_before_any_scanning() {
    let snap = snapshot(&enhanced());
    for request in [
        CheckRequest {
            request_id: "bad id!".into(),
            scene: Scene::ChannelMessage,
            fields: vec![TextField {
                field_id: "a".into(),
                text: "x".into(),
            }],
        },
        CheckRequest {
            request_id: "ok".into(),
            scene: Scene::ChannelMessage,
            fields: vec![],
        },
        CheckRequest {
            request_id: "ok".into(),
            scene: Scene::ChannelMessage,
            fields: vec![
                TextField {
                    field_id: "a".into(),
                    text: "x".into(),
                },
                TextField {
                    field_id: "a".into(),
                    text: "y".into(),
                },
            ],
        },
    ] {
        assert!(matches!(
            snap.check(&request, &NO_CANCEL),
            Err(ErrorCode::InvalidArgument)
        ));
    }
}

fn alias_rule(id: &str, pattern: &str, action: RuleAction) -> AliasRule {
    AliasRule {
        variant_id: id.into(),
        variant_type: AliasType::Pinyin,
        pattern: pattern.into(),
        action,
        boundary: AliasBoundary::AsciiWord,
    }
}
