use std::cell::Cell;

use crate::matcher::literal::{CompileIssue, CompiledLiterals};
use crate::matcher::{aggregate, CandidateBudget, RawHit, MAX_REQUEST_CANDIDATES};
use crate::model::{
    AliasBoundary, AliasRule, AliasType, ErrorCode, MatchMethod, RuleAction, RuleSet, Scene,
    Semantics, Severity,
};
use crate::normalize::{normalize_str, table_by_id, Semantics as NormalizationSemantics};

fn rules() -> RuleSet {
    serde_json::from_str(include_str!("../fixtures/contract-ruleset.json")).unwrap()
}

fn compile(rules: &RuleSet) -> CompiledLiterals {
    CompiledLiterals::compile(rules, table_by_id(&rules.t2s_table_id).unwrap()).unwrap()
}

fn scan(engine: &CompiledLiterals, text: &str) -> Vec<RawHit> {
    engine
        .scan_text(
            text,
            Scene::ChannelMessage,
            true,
            &mut CandidateBudget::default(),
            &|| false,
        )
        .unwrap()
}

fn enhanced() -> RuleSet {
    let mut rules = rules();
    rules.semantics_by_scene.channel_message = Semantics::EnhancedV1;
    rules
}

fn alias(id: &str, pattern: &str, kind: AliasType, boundary: AliasBoundary) -> AliasRule {
    AliasRule {
        variant_id: id.into(),
        pattern: pattern.into(),
        variant_type: kind,
        action: RuleAction::Review,
        boundary,
    }
}

fn rejected(rules: &RuleSet, issue: CompileIssue) {
    let error = CompiledLiterals::compile(rules, table_by_id(&rules.t2s_table_id).unwrap())
        .err()
        .unwrap();
    assert_eq!(error.issue, issue);
    assert_eq!(ErrorCode::from(error), ErrorCode::InvalidRuleset);
    assert!(!error.to_string().contains(&rules.literals[0].word));
}

#[test]
fn overlapping_prefix_suffix_and_repeated_occurrences_keep_original_ranges() {
    let mut rules = enhanced();
    rules.literals[0].word = "测试".into();
    rules.literals[0].severity = Severity::Medium;
    let mut longer = rules.literals[0].clone();
    longer.id = "1002".into();
    longer.word = "测试词".into();
    longer.severity = Severity::High;
    rules.literals.push(longer);
    let hits = scan(&compile(&rules), "🙂测试词测试词");
    let actual: Vec<_> = hits
        .iter()
        .map(|hit| (hit.rule_id.as_str(), hit.start, hit.end, hit.action))
        .collect();
    assert_eq!(
        actual,
        vec![
            ("1001", 4, 10, RuleAction::Review),
            ("1001", 13, 19, RuleAction::Review),
            ("1002", 4, 13, RuleAction::Block),
            ("1002", 13, 22, RuleAction::Block)
        ]
    );
    rules.literals.truncate(1);
    rules.literals[0].word = "aba".into();
    let hits = scan(&compile(&rules), "ababa");
    assert_eq!(
        hits.iter()
            .map(|hit| (hit.start, hit.end))
            .collect::<Vec<_>>(),
        vec![(0, 3), (2, 5)]
    );
}

#[test]
fn rule_input_permutations_preserve_hits_and_strongest_decision() {
    let mut rules = enhanced();
    for (id, word) in [("1002", "测试"), ("1003", "词"), ("1004", "测")] {
        let mut rule = rules.literals[0].clone();
        rule.id = id.into();
        rule.word = word.into();
        rule.severity = Severity::Medium;
        rules.literals.push(rule);
    }
    let expected = scan(&compile(&rules), "测试词测试词");
    for shift in 0..rules.literals.len() {
        rules.literals.rotate_left(shift);
        assert_eq!(scan(&compile(&rules), "测试词测试词"), expected);
    }
    rules.literals.reverse();
    assert_eq!(scan(&compile(&rules), "测试词测试词"), expected);
    assert_eq!(
        expected.iter().map(|hit| hit.action).max(),
        Some(RuleAction::Block)
    );
}

#[test]
fn normalization_collisions_retain_distinct_rules_and_alias_actions() {
    let mut rules = enhanced();
    rules.literals[0].word = "ABC".into();
    let mut rule = rules.literals[0].clone();
    rule.id = "1002".into();
    rule.word = "abc".into();
    rule.severity = Severity::Low;
    rules.literals.push(rule);
    rules.literals[0].aliases.push(alias(
        "same",
        "abc",
        AliasType::Lookalike,
        AliasBoundary::None,
    ));
    let hits = scan(&compile(&rules), "ａｂｃ");
    assert_eq!(hits.len(), 3);
    assert!(hits.iter().any(|hit| hit.rule_id == "1001"
        && hit.variant_id == "canonical"
        && hit.action == RuleAction::Block));
    assert!(hits
        .iter()
        .any(|hit| hit.rule_id == "1002" && hit.action == RuleAction::Review));
    assert!(hits.iter().any(|hit| hit.variant_id == "same"
        && hit.action == RuleAction::Review
        && hit.methods == vec![MatchMethod::Alias]));
}

#[test]
fn fanout_exact_128_allowed_and_129_rejected_in_base_and_t2s() {
    let mut rules = enhanced();
    let rule = rules.literals[0].clone();
    rules.literals = (1..=128)
        .map(|id| {
            let mut value = rule.clone();
            value.id = id.to_string();
            value
        })
        .collect();
    assert_eq!(scan(&compile(&rules), "测试词").len(), 128);
    let mut extra = rule.clone();
    extra.id = "129".into();
    rules.literals.push(extra);
    rejected(&rules, CompileIssue::MetadataFanout);
    rules.literals = (1..=128)
        .map(|id| {
            let mut value = rule.clone();
            value.id = id.to_string();
            value.word = "测".into();
            value
        })
        .collect();
    let mut extra = rule;
    extra.id = "129".into();
    extra.word = "測".into();
    rules.literals.push(extra);
    rejected(&rules, CompileIssue::MetadataFanout);
}

#[test]
fn empty_and_zero_width_patterns_reject_without_panics() {
    for word in ["", "\u{200b}\u{200c}\u{200d}\u{feff}"] {
        let mut rules = enhanced();
        rules.literals[0].word = word.into();
        let error = CompiledLiterals::compile(&rules, table_by_id(&rules.t2s_table_id).unwrap())
            .err()
            .unwrap();
        assert_eq!(error.issue, CompileIssue::EmptyPattern);
    }
    for pattern in ["", "\u{200b}"] {
        let mut rules = enhanced();
        rules.literals[0].aliases.push(alias(
            "bad",
            pattern,
            AliasType::Homophone,
            AliasBoundary::None,
        ));
        assert_eq!(
            CompiledLiterals::compile(&rules, table_by_id(&rules.t2s_table_id).unwrap())
                .err()
                .unwrap()
                .issue,
            CompileIssue::EmptyPattern
        );
    }
}

#[test]
fn ids_variants_and_scene_metadata_are_validated_atomically() {
    for id in [
        "",
        "0",
        "01",
        "-1",
        "re:word",
        "词",
        "123456789012345678901",
    ] {
        let mut rules = enhanced();
        rules.literals[0].id = id.into();
        rejected(&rules, CompileIssue::InvalidRuleId);
    }
    let mut rules = enhanced();
    rules.literals.push(rules.literals[0].clone());
    rejected(&rules, CompileIssue::DuplicateRuleId);
    for id in ["", "canonical", "regex", "has space", "词"] {
        let mut rules = enhanced();
        rules.literals[0].aliases.push(alias(
            id,
            "alias",
            AliasType::Pinyin,
            AliasBoundary::AsciiWord,
        ));
        rejected(&rules, CompileIssue::InvalidVariantId);
    }
    let mut rules = enhanced();
    let item = alias("same", "alias", AliasType::Pinyin, AliasBoundary::AsciiWord);
    rules.literals[0].aliases = vec![item.clone(), item];
    rejected(&rules, CompileIssue::DuplicateVariantId);
    for scenes in [vec![], vec![Scene::ChannelMessage, Scene::ChannelMessage]] {
        let mut rules = enhanced();
        rules.literals[0].scenes = scenes;
        rejected(&rules, CompileIssue::InvalidMetadata);
    }
    for category in [String::new(), "x".repeat(65)] {
        let mut rules = enhanced();
        rules.literals[0].category = category;
        rejected(&rules, CompileIssue::InvalidMetadata);
    }
}

#[test]
fn unknown_enum_values_fail_deserialization() {
    for (field, value) in [
        ("severity", serde_json::json!("critical")),
        ("scenes", serde_json::json!(["private_message"])),
    ] {
        let mut value_rules = serde_json::to_value(enhanced()).unwrap();
        value_rules["literals"][0][field] = value;
        assert!(serde_json::from_value::<RuleSet>(value_rules).is_err());
    }
}

#[test]
fn pinyin_ascii_fence_uses_normalized_neighbors_and_allows_chinese() {
    let mut rules = enhanced();
    rules.literals[0].aliases.push(alias(
        "pinyin",
        "ceshici",
        AliasType::Pinyin,
        AliasBoundary::AsciiWord,
    ));
    let engine = compile(&rules);
    for text in [
        "ceshici",
        "中文ceshici中文",
        "中文ＣＥＳＨＩＣＩ中文",
        "🙂ceshici🙂",
    ] {
        assert_eq!(scan(&engine, text).len(), 1, "{text}");
    }
    for text in [
        "myceshicitool",
        "0ceshici",
        "ceshici_",
        "_ceshici",
        "ｘceshici",
        "ceshici９",
    ] {
        assert!(scan(&engine, text).is_empty(), "{text}");
    }
}

#[test]
fn spaced_pinyin_initials_and_four_alias_categories_are_explicit_only() {
    let mut rules = enhanced();
    rules.literals[0].word = "森林".into();
    rules.literals[0].aliases.push(alias(
        "full",
        "senlin",
        AliasType::Pinyin,
        AliasBoundary::AsciiWord,
    ));
    for text in ["sen lin", "sl", "森木木", "森淋", "森杶"] {
        assert!(scan(&compile(&rules), text).is_empty());
    }
    for (id, pattern, kind, boundary) in [
        (
            "space",
            "sen lin",
            AliasType::Pinyin,
            AliasBoundary::AsciiWord,
        ),
        (
            "initials",
            "sl",
            AliasType::Pinyin,
            AliasBoundary::AsciiWord,
        ),
        (
            "decomposed",
            "森木木",
            AliasType::Decomposed,
            AliasBoundary::None,
        ),
        (
            "homophone",
            "森淋",
            AliasType::Homophone,
            AliasBoundary::None,
        ),
        (
            "lookalike",
            "森杶",
            AliasType::Lookalike,
            AliasBoundary::None,
        ),
    ] {
        rules.literals[0]
            .aliases
            .push(alias(id, pattern, kind, boundary));
    }
    let engine = compile(&rules);
    for text in ["sen lin", "sl", "森木木", "森淋", "森杶"] {
        let hits = scan(&engine, text);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].action, RuleAction::Review);
    }
    for text in ["木木", "森淋淋木木", "林林"] {
        let hits = scan(&engine, text);
        assert!(!hits.iter().any(|hit| hit.variant_id == "canonical"));
    }
}

#[test]
fn aliases_never_inherit_severity_t2s_gap_or_recursive_alias_expansion() {
    let mut rules = enhanced();
    rules.literals[0].aliases = vec![
        alias(
            "homophone",
            "测試",
            AliasType::Homophone,
            AliasBoundary::None,
        ),
        alias(
            "explicit-block",
            "blockalias",
            AliasType::Lookalike,
            AliasBoundary::None,
        ),
    ];
    rules.literals[0].aliases[1].action = RuleAction::Block;
    let engine = compile(&rules);
    let hits = scan(&engine, "测試");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].action, RuleAction::Review);
    assert_eq!(hits[0].methods, vec![MatchMethod::Alias]);
    assert!(scan(&engine, "测试").is_empty());
    assert!(scan(&engine, "测*試").is_empty());
    assert_eq!(scan(&engine, "blockalias")[0].action, RuleAction::Block);
}

#[test]
fn compat_and_enhanced_compile_their_actual_sigma_flavors() {
    let mut rules = enhanced();
    rules.literals[0].word = "ΟΣ".into();
    let engine = compile(&rules);
    assert_eq!(scan(&engine, "ος").len(), 1);
    assert!(scan(&engine, "οσ").is_empty());
    let mut budget = CandidateBudget::default();
    assert_eq!(
        engine
            .scan_text("οσ", Scene::ProfileField, true, &mut budget, &|| false)
            .unwrap()
            .len(),
        1
    );
    assert!(engine
        .scan_text("ος", Scene::ProfileField, true, &mut budget, &|| false)
        .unwrap()
        .is_empty());
}

#[test]
fn compat_scenes_ignore_aliases_and_t2s_and_direct_evidence_wins() {
    let mut rules = enhanced();
    rules.literals[0].aliases.push(alias(
        "pinyin",
        "ceshici",
        AliasType::Pinyin,
        AliasBoundary::AsciiWord,
    ));
    let engine = compile(&rules);
    let mut budget = CandidateBudget::default();
    for text in ["測試詞", "ceshici"] {
        assert!(engine
            .scan_text(text, Scene::ProfileField, true, &mut budget, &|| false)
            .unwrap()
            .is_empty());
    }
    assert_eq!(
        scan(&engine, "測試詞")[0].methods,
        vec![MatchMethod::Literal, MatchMethod::T2s]
    );
    assert_eq!(
        scan(&engine, "测试词測")[0].methods,
        vec![MatchMethod::Literal]
    );
}

#[test]
fn provenance_covers_deleted_zero_width_and_expanded_original_characters() {
    let engine = compile(&enhanced());
    let hits = scan(&engine, "🙂测\u{200b}试词");
    assert_eq!((hits[0].start, hits[0].end), (4, 16));
    let mut rules = enhanced();
    rules.literals[0].word = "i".into();
    let hits = scan(&compile(&rules), "🙂İ");
    assert_eq!((hits[0].start, hits[0].end), (4, 6));
}

#[test]
fn no_trimming_and_empty_input_remains_legal() {
    let mut rules = enhanced();
    rules.literals[0].word = " abc ".into();
    let engine = compile(&rules);
    assert!(scan(&engine, "abc").is_empty());
    assert_eq!(scan(&engine, " abc ").len(), 1);
    assert!(scan(&engine, "").is_empty());
}

#[test]
fn raw_pattern_scalar_and_utf8_limits_apply_to_canonical_and_alias() {
    for word in ["a".repeat(65), "🙂".repeat(49)] {
        let mut rules = enhanced();
        rules.literals[0].word = word.clone();
        rejected(&rules, CompileIssue::PatternLength);
        let mut rules = enhanced();
        rules.literals[0].aliases.push(alias(
            "long",
            &word,
            AliasType::Lookalike,
            AliasBoundary::None,
        ));
        rejected(&rules, CompileIssue::PatternLength);
    }
    for word in ["a".repeat(64), "测".repeat(64)] {
        let mut rules = enhanced();
        rules.literals[0].word = word.clone();
        assert_eq!(scan(&compile(&rules), &word).len(), 1);
    }
}

#[test]
fn alias_and_logical_and_canonical_rule_limits_reject_whole_package() {
    let mut rules = enhanced();
    rules.literals[0].aliases = (0..9)
        .map(|index| {
            alias(
                &format!("v{index}"),
                &format!("alias{index}"),
                AliasType::Lookalike,
                AliasBoundary::None,
            )
        })
        .collect();
    rejected(&rules, CompileIssue::AliasLimit);
    rules.literals[0].aliases.clear();
    let template = rules.literals[0].clone();
    rules.literals = vec![template.clone(); 50_001];
    rejected(&rules, CompileIssue::CanonicalRuleLimit);
    rules.literals = (1..=11_112)
        .map(|index| {
            let mut rule = template.clone();
            rule.id = index.to_string();
            rule.word = format!("word{index}");
            rule.aliases = (0..8)
                .map(|variant| {
                    alias(
                        &format!("v{variant}"),
                        &format!("alias{index}v{variant}"),
                        AliasType::Lookalike,
                        AliasBoundary::None,
                    )
                })
                .collect();
            rule
        })
        .collect();
    rejected(&rules, CompileIssue::LogicalPatternLimit);
}

#[test]
fn normalized_bytes_are_total_across_semantics_and_views() {
    let mut rules = enhanced();
    let template = rules.literals[0].clone();
    rules.literals = (1..=31_000)
        .map(|index| {
            let mut rule = template.clone();
            rule.id = index.to_string();
            rule.word = format!("{}{:05}", "测".repeat(59), index);
            rule
        })
        .collect();
    rejected(&rules, CompileIssue::NormalizedByteLimit);
}

#[test]
fn compilation_usage_counts_collisions_and_all_view_entries() {
    let mut rules = enhanced();
    rules.literals[0].word = "abc".into();
    rules.literals[0].aliases.push(alias(
        "same",
        "abc",
        AliasType::Lookalike,
        AliasBoundary::None,
    ));
    let engine = compile(&rules);
    let usage = engine.usage();
    assert_eq!(
        (
            usage.logical_patterns,
            usage.compiled_entries,
            usage.normalized_bytes
        ),
        (2, 4, 12)
    );
}

#[test]
fn request_budget_exact_20000_counts_b_and_t_before_aggregation() {
    let mut rules = enhanced();
    rules.literals[0].word = "a".into();
    let engine = compile(&rules);
    assert_eq!(
        engine.scan_text(
            &"a".repeat(10_001),
            Scene::ChannelMessage,
            true,
            &mut CandidateBudget::default(),
            &|| false
        ),
        Err(ErrorCode::MatchBudgetExceeded)
    );
    let mut budget = CandidateBudget::default();
    assert_eq!(
        engine
            .scan_text(
                &"a".repeat(10_000),
                Scene::ChannelMessage,
                true,
                &mut budget,
                &|| false
            )
            .unwrap()
            .len(),
        10_000
    );
    assert_eq!(budget.consumed(), MAX_REQUEST_CANDIDATES);
    assert_eq!(
        engine.scan_text("a", Scene::ChannelMessage, false, &mut budget, &|| false),
        Err(ErrorCode::MatchBudgetExceeded)
    );
    assert_eq!(
        engine.scan_text("", Scene::ChannelMessage, false, &mut budget, &|| false),
        Err(ErrorCode::MatchBudgetExceeded)
    );
    assert_eq!(budget.consumed(), 20_000);
}

#[test]
fn request_budget_is_shared_across_fields_and_before_scene_filtering() {
    let mut rules = enhanced();
    rules.literals[0].word = "a".into();
    let engine = compile(&rules);
    let mut budget = CandidateBudget::default();
    for _field in 0..8 {
        assert_eq!(
            engine
                .scan_text(
                    &"a".repeat(2500),
                    Scene::ChannelMessage,
                    false,
                    &mut budget,
                    &|| false
                )
                .unwrap()
                .len(),
            2500
        );
    }
    assert_eq!(budget.consumed(), 20_000);
    assert_eq!(
        engine.scan_text("a", Scene::ChannelMessage, false, &mut budget, &|| false),
        Err(ErrorCode::MatchBudgetExceeded)
    );
    rules.literals[0].scenes = vec![Scene::ChannelMessage];
    let template = rules.literals[0].clone();
    rules.literals = (1..=128)
        .map(|index| {
            let mut rule = template.clone();
            rule.id = index.to_string();
            rule
        })
        .collect();
    let engine = compile(&rules);
    let mut budget = CandidateBudget::default();
    assert_eq!(
        engine.scan_text(
            &"a".repeat(157),
            Scene::MomentPost,
            false,
            &mut budget,
            &|| false
        ),
        Err(ErrorCode::MatchBudgetExceeded)
    );
    assert_eq!(budget.consumed(), 20_000);
}

#[test]
fn alias_boundary_rejection_still_consumes_metadata_candidates() {
    let mut rules = enhanced();
    rules.literals[0].aliases = vec![
        alias("one", "a", AliasType::Pinyin, AliasBoundary::AsciiWord),
        alias("two", "a", AliasType::Pinyin, AliasBoundary::AsciiWord),
    ];
    let mut budget = CandidateBudget::default();
    assert_eq!(
        compile(&rules).scan_text(
            &"a".repeat(10_001),
            Scene::ChannelMessage,
            false,
            &mut budget,
            &|| false
        ),
        Err(ErrorCode::MatchBudgetExceeded)
    );
    assert_eq!(budget.consumed(), 20_000);
}

#[test]
fn cancellation_during_candidates_is_terminal_and_shared_budget_supports_future_stages() {
    let mut rules = enhanced();
    rules.literals[0].word = "a".into();
    let engine = compile(&rules);
    let units = normalize_str(
        &"a".repeat(100),
        NormalizationSemantics::EnhancedV1,
        &|| false,
    )
    .unwrap();
    let polls = Cell::new(0);
    let cancel = || {
        polls.set(polls.get() + 1);
        polls.get() > 10
    };
    let mut budget = CandidateBudget::default();
    assert_eq!(
        engine.scan_base(&units, Scene::ChannelMessage, &mut budget, &cancel),
        Err(ErrorCode::CheckDeadline)
    );
    assert!(budget.consumed() > 0 && budget.consumed() < 100);
    assert_eq!(budget.consume(&|| false), Err(ErrorCode::CheckDeadline));
    assert_eq!(
        CompiledLiterals::compile_cancellable(
            &rules,
            table_by_id(&rules.t2s_table_id).unwrap(),
            &|| true
        )
        .err()
        .unwrap()
        .issue,
        CompileIssue::Cancelled
    );
    let mut budget = CandidateBudget::default();
    for _stage_candidate in 0..20_000 {
        budget.consume(&|| false).unwrap();
    }
    assert_eq!(
        engine.scan_base(&units, Scene::ChannelMessage, &mut budget, &|| false),
        Err(ErrorCode::MatchBudgetExceeded)
    );
}

#[test]
fn malformed_provenance_and_oversized_field_fail_closed() {
    let engine = compile(&enhanced());
    let mut budget = CandidateBudget::default();
    assert_eq!(
        engine.scan_base(
            &[crate::normalize::Unit::new('测', 2, 1)],
            Scene::ChannelMessage,
            &mut budget,
            &|| false
        ),
        Err(ErrorCode::NormalizerIncompatible)
    );
    assert_eq!(
        engine.scan_text(
            &"a".repeat(16 * 1024 + 1),
            Scene::ChannelMessage,
            false,
            &mut budget,
            &|| false
        ),
        Err(ErrorCode::TextTooLarge)
    );
}

#[test]
fn aggregation_tie_break_is_independent_of_insertion_order() {
    let canonical = RawHit {
        rule_id: "1".into(),
        variant_id: "canonical".into(),
        action: RuleAction::Block,
        start: 0,
        end: 3,
        methods: vec![MatchMethod::Literal],
    };
    let mut transformed = canonical.clone();
    transformed.methods.push(MatchMethod::T2s);
    assert_eq!(
        aggregate(vec![transformed.clone(), canonical.clone()]),
        vec![canonical.clone()]
    );
    assert_eq!(
        aggregate(vec![canonical.clone(), transformed]),
        vec![canonical]
    );
}
