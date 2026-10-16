//! T09 acceptance tests: bounded gap and local exceptions (plan 4.6/4.8).

use std::cell::Cell;

use sha2::{Digest, Sha256};

use crate::matcher::exceptions::CompiledExceptions;
use crate::matcher::gap::CompiledGaps;
use crate::matcher::literal::{CompileIssue, CompiledLiterals};
use crate::matcher::{aggregate, CandidateBudget, RawHit};
use crate::model::{
    ErrorCode, ExceptionRule, MatchMethod, RuleAction, RuleSet, Scene, Semantics, Severity, View,
};
use crate::normalize::table_by_id;

fn rules() -> RuleSet {
    serde_json::from_str(include_str!("../fixtures/contract-ruleset.json")).unwrap()
}

fn enhanced() -> RuleSet {
    let mut rules = rules();
    rules.semantics_by_scene = crate::model::SceneSemantics {
        channel_message: Semantics::EnhancedV1,
        moment_post: Semantics::EnhancedV1,
        profile_field: Semantics::EnhancedV1,
    };
    rules
}

fn compile(rules: &RuleSet) -> (CompiledLiterals, CompiledGaps, CompiledExceptions) {
    let table = table_by_id(&rules.t2s_table_id).unwrap();
    (
        CompiledLiterals::compile(rules, table).unwrap(),
        CompiledGaps::compile(rules, table).unwrap(),
        CompiledExceptions::compile(rules, table).unwrap(),
    )
}

fn gap_hits(rules: &RuleSet, text: &str) -> Vec<RawHit> {
    let (_, gaps, _) = compile(rules);
    gaps.scan_text(
        text,
        Scene::ChannelMessage,
        true,
        &mut CandidateBudget::default(),
        &|| false,
    )
    .unwrap()
}

fn one_word_rules(word: &str, gap_enabled: bool) -> RuleSet {
    let mut rules = enhanced();
    rules.literals.truncate(1);
    rules.literals[0].word = word.into();
    rules.literals[0].severity = Severity::High;
    rules.literals[0].aliases.clear();
    rules.literals[0].gap = crate::model::GapConfig {
        enabled: gap_enabled,
        action: RuleAction::Review,
    };
    rules
}

fn hit_tuple(hit: &RawHit) -> (String, String, u32, u32, Vec<MatchMethod>) {
    (
        hit.rule_id.clone(),
        hit.variant_id.clone(),
        hit.start,
        hit.end,
        hit.methods.clone(),
    )
}

#[test]
fn ac032_adjacent_gap_two_hits_and_three_misses() {
    let rules = one_word_rules("测试词", true);
    let hits = gap_hits(&rules, "前测**试词后");
    assert_eq!(
        hits.iter().map(hit_tuple).collect::<Vec<_>>(),
        vec![(
            "1001".into(),
            "canonical".into(),
            3,
            14,
            vec![MatchMethod::Literal, MatchMethod::Gap]
        )]
    );
    assert!(gap_hits(&rules, "测***试词").is_empty(), "adjacent 3 > 2");
}

#[test]
fn ac033_total_gap_four_hits_and_five_misses() {
    let rules = one_word_rules("测试词条", true);
    let hits = gap_hits(&rules, "x测**试**词条y");
    assert_eq!(hits.len(), 1, "total 4 <= 4 hits");
    assert_eq!((hits[0].start, hits[0].end), (1, 17));
    assert!(
        gap_hits(&rules, "测**试**词*条").is_empty(),
        "total 5 > 4 misses even with adjacency <= 2"
    );
}

#[test]
fn ac034_hard_boundaries_block_gap_stitching() {
    let rules = one_word_rules("测试词", true);
    for text in [
        "测。试词",
        "测.试词",
        "测\n试词",
        "测\r试词",
        "测\u{2028}试词",
        "测\u{2029}试词",
        "测！试词",
        "测；试词",
        "测？试词",
    ] {
        assert!(gap_hits(&rules, text).is_empty(), "boundary in {text:?}");
    }
    // boundaries also stop candidates that would span them
    assert!(gap_hits(&rules, "测试.词").is_empty());
}

#[test]
fn ac035_fields_are_independent_and_extra_noise_is_not_ignorable() {
    let rules = one_word_rules("测试词", true);
    // emoji, letters, digits and non-frozen whitespace are not noise
    for text in [
        "测🙂试词",
        "测x试词",
        "测1试词",
        "测\t试词",
        "测\u{3000}试词",
    ] {
        assert!(gap_hits(&rules, text).is_empty(), "noise in {text:?}");
    }
    // scanning two fields never stitches across them: the scanner takes
    // one field at a time and has no cross-field state
    let (_, gaps, _) = compile(&rules);
    let mut first = gaps
        .scan_text(
            "软件测",
            Scene::ChannelMessage,
            true,
            &mut CandidateBudget::default(),
            &|| false,
        )
        .unwrap();
    let second = gaps
        .scan_text(
            "试词喽",
            Scene::ChannelMessage,
            true,
            &mut CandidateBudget::default(),
            &|| false,
        )
        .unwrap();
    first.extend(second);
    assert!(first.is_empty(), "cross-field stitching must not happen");
}

#[test]
fn ac036_t2s_plus_gap_returns_original_byte_range() {
    let mut rules = one_word_rules("测试词", true);
    rules.literals[0].word = "测试词".into();
    let hits = gap_hits(&rules, "🙂測*試_詞");
    assert_eq!(
        hits.iter().map(hit_tuple).collect::<Vec<_>>(),
        vec![(
            "1001".into(),
            "canonical".into(),
            4,
            15,
            vec![MatchMethod::Literal, MatchMethod::T2s, MatchMethod::Gap]
        )]
    );
}

fn exception_rules(exception: ExceptionRule) -> RuleSet {
    let mut rules = one_word_rules("测试", false);
    rules.literals[0].severity = Severity::Low; // canonical action: review
    rules.exceptions = vec![exception];
    rules
}

fn exception(
    id: &str,
    phrase: &str,
    scenes: &[Scene],
    rule_id: &str,
    variant_ids: &[&str],
    views: &[View],
) -> ExceptionRule {
    ExceptionRule {
        id: id.into(),
        phrase: phrase.into(),
        scenes: scenes.to_vec(),
        rule_id: rule_id.into(),
        variant_ids: variant_ids.iter().map(|v| v.to_string()).collect(),
        views: views.to_vec(),
    }
}

/// Full pipeline in the T11 order: raw literal scans (unaggregated, so the
/// B/T view identity of every candidate survives) -> exception filter ->
/// gap merge -> single aggregation. Aggregating before the filter would let
/// a B-view hit absorb its T-view twin and then wrongly exempt both.
fn pipeline(rules: &RuleSet, text: &str, scene: Scene, t2s: bool) -> Vec<RawHit> {
    let (literals, gaps, exceptions) = compile(rules);
    let mut budget = CandidateBudget::default();
    let cancel = || false;
    let semantics = match rules.semantics_by_scene.get(scene) {
        Semantics::CompatV1 => crate::normalize::Semantics::CompatV1,
        Semantics::EnhancedV1 => crate::normalize::Semantics::EnhancedV1,
    };
    let base_units = crate::normalize::normalize_str(text, semantics, &cancel).unwrap();
    let table = table_by_id(&rules.t2s_table_id).unwrap();
    let t2s_units = if t2s && semantics == crate::normalize::Semantics::EnhancedV1 {
        Some(crate::normalize::t2s_units(&base_units, table))
    } else {
        None
    };
    let mut literal = literals
        .scan_base(&base_units, scene, &mut budget, &cancel)
        .unwrap();
    if let Some(translated) = t2s_units.as_deref() {
        literal.extend(
            literals
                .scan_t2s(translated, scene, &mut budget, &cancel)
                .unwrap(),
        );
    }
    let filtered = exceptions
        .filter_literal_hits(
            &base_units,
            t2s_units.as_deref(),
            scene,
            literal,
            &mut budget,
            &cancel,
        )
        .unwrap();
    let mut all = filtered;
    all.extend(
        gaps.scan_text(text, scene, t2s, &mut budget, &cancel)
            .unwrap(),
    );
    aggregate(all)
}

#[test]
fn ac037_exception_exempts_only_the_covered_hit() {
    let rules = exception_rules(exception(
        "exc-01",
        "软件测试",
        &[Scene::ChannelMessage, Scene::MomentPost],
        "1001",
        &["canonical"],
        &[View::B],
    ));
    let hits = pipeline(&rules, "软件测试后还有测试", Scene::ChannelMessage, false);
    assert_eq!(hits.len(), 1, "only the later hit survives");
    assert_eq!((hits[0].start, hits[0].end), (21, 27));
    // the exempted range itself is still fully inside the phrase
    let inside = pipeline(&rules, "软件测试", Scene::ChannelMessage, false);
    assert!(inside.is_empty());
}

#[test]
fn ac038_wrong_targets_are_never_exempted() {
    let base = || {
        exception(
            "exc-01",
            "软件测试",
            &[Scene::MomentPost],
            "1001",
            &["canonical"],
            &[View::B],
        )
    };
    // different scene: channel_message has no exemption
    let wrong_scene = exception_rules(base());
    assert_eq!(
        pipeline(&wrong_scene, "软件测试", Scene::ChannelMessage, false).len(),
        1
    );
    // different rule id: same position, other rule not exempted
    let mut rules = exception_rules(base());
    let mut twin = rules.literals[0].clone();
    twin.id = "1002".into();
    rules.literals.push(twin);
    let hits = pipeline(&rules, "软件测试", Scene::MomentPost, false);
    assert_eq!(
        hits.len(),
        1,
        "rule 1002 hit is not covered by the exception"
    );
    assert_eq!(hits[0].rule_id, "1002");
    // different variant: alias variant not exempted by a canonical exception
    let mut rules = exception_rules(base());
    rules.literals[0].aliases = vec![crate::model::AliasRule {
        variant_id: "pinyin-01".into(),
        variant_type: crate::model::AliasType::Pinyin,
        pattern: "软件测试".into(),
        action: RuleAction::Review,
        boundary: crate::model::AliasBoundary::None,
    }];
    let hits = pipeline(&rules, "软件测试", Scene::MomentPost, false);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].variant_id, "pinyin-01");
    // partial coverage never exempts
    let partial = exception(
        "exc-02",
        "软件测",
        &[Scene::MomentPost],
        "1001",
        &["canonical"],
        &[View::B],
    );
    let rules = exception_rules(partial);
    assert_eq!(
        pipeline(&rules, "软件测试", Scene::MomentPost, false).len(),
        1,
        "partial span coverage must not exempt"
    );
}

#[test]
fn ac039_unsupported_exception_targets_are_rejected_at_publish() {
    let table = |rules: &RuleSet| table_by_id(&rules.t2s_table_id).unwrap();
    // dangling rule id
    let mut rules = exception_rules(exception(
        "exc-x",
        "软件测试",
        &[Scene::ChannelMessage],
        "9999",
        &["canonical"],
        &[View::B],
    ));
    let error = CompiledExceptions::compile(&rules, table(&rules))
        .err()
        .unwrap();
    assert_eq!(error.issue, CompileIssue::InvalidVariantId);
    // regex rule id (add one and point the exception at it)
    rules.literals[0].gap.enabled = false;
    rules.regex_rules = vec![crate::model::RegexRule {
        id: "5001".into(),
        expression: "测试+".into(),
        display_word: "测试".into(),
        category: "test".into(),
        severity: Severity::Low,
        action: RuleAction::Review,
        scenes: vec![Scene::ChannelMessage],
    }];
    rules.exceptions[0].rule_id = "5001".into();
    let error = CompiledExceptions::compile(&rules, table(&rules))
        .err()
        .unwrap();
    assert_eq!(error.issue, CompileIssue::InvalidVariantId);
    // dangling variant id
    rules.exceptions[0] = exception(
        "exc-x",
        "软件测试",
        &[Scene::ChannelMessage],
        "1001",
        &["no-such-variant"],
        &[View::B],
    );
    let error = CompiledExceptions::compile(&rules, table(&rules))
        .err()
        .unwrap();
    assert_eq!(error.issue, CompileIssue::InvalidVariantId);
    // more than 32 variants per exception
    rules.exceptions[0] = exception(
        "exc-x",
        "软件测试",
        &[Scene::ChannelMessage],
        "1001",
        &(0..33).map(|_| "canonical").collect::<Vec<&str>>(),
        &[View::B],
    );
    let error = CompiledExceptions::compile(&rules, table(&rules))
        .err()
        .unwrap();
    assert_eq!(error.issue, CompileIssue::InvalidMetadata);
    // empty views
    rules.exceptions[0] = exception(
        "exc-x",
        "软件测试",
        &[Scene::ChannelMessage],
        "1001",
        &["canonical"],
        &[],
    );
    let error = CompiledExceptions::compile(&rules, table(&rules))
        .err()
        .unwrap();
    assert_eq!(error.issue, CompileIssue::InvalidMetadata);
    // duplicate exception ids
    rules.exceptions = vec![
        exception(
            "exc-dup",
            "软件测试",
            &[Scene::ChannelMessage],
            "1001",
            &["canonical"],
            &[View::B],
        ),
        exception(
            "exc-dup",
            "另外测试",
            &[Scene::ChannelMessage],
            "1001",
            &["canonical"],
            &[View::B],
        ),
    ];
    let error = CompiledExceptions::compile(&rules, table(&rules))
        .err()
        .unwrap();
    assert_eq!(error.issue, CompileIssue::InvalidMetadata);
    // more than 1000 exceptions
    rules.exceptions = (0..1001)
        .map(|index| {
            exception(
                &format!("exc-{index}"),
                "软件测试",
                &[Scene::ChannelMessage],
                "1001",
                &["canonical"],
                &[View::B],
            )
        })
        .collect();
    let error = CompiledExceptions::compile(&rules, table(&rules))
        .err()
        .unwrap();
    assert_eq!(error.issue, CompileIssue::InvalidMetadata);
    assert_eq!(ErrorCode::from(error), ErrorCode::InvalidRuleset);
}

#[test]
fn gap_patterns_with_ignorable_or_boundary_characters_are_rejected() {
    for word in ["测 试", "测*试词", "测.试词", "测。试词", "测-试", "测_试"] {
        let rules = one_word_rules(word, true);
        let table = table_by_id(&rules.t2s_table_id).unwrap();
        let error = CompiledGaps::compile(&rules, table).err().unwrap();
        assert_eq!(
            error.issue,
            CompileIssue::InvalidMetadata,
            "word {word:?} must be rejected"
        );
    }
}

#[test]
fn compat_scenes_keep_legacy_behavior_without_gap_or_exceptions() {
    let mut rules = exception_rules(exception(
        "exc-01",
        "软件测试",
        &[
            Scene::ChannelMessage,
            Scene::MomentPost,
            Scene::ProfileField,
        ],
        "1001",
        &["canonical"],
        &[View::B, View::T],
    ));
    rules.literals[0].gap = crate::model::GapConfig {
        enabled: true,
        action: RuleAction::Review,
    };
    rules.semantics_by_scene.profile_field = Semantics::CompatV1;
    let hits = pipeline(&rules, "软件测*试", Scene::ProfileField, false);
    // compat keeps only plain canonical literal hits; "测*试" never hits
    // the plain literal, gap is off, and no exemption is applied anyway
    assert!(hits.is_empty(), "compat must not run enhanced capabilities");
    let plain = pipeline(&rules, "软件测试", Scene::ProfileField, false);
    assert_eq!(plain.len(), 1, "plain literal still works in compat");
    assert_eq!(plain[0].methods, vec![MatchMethod::Literal]);
}

#[test]
fn gap_and_regex_hits_are_never_exempted_by_exceptions() {
    let mut rules = one_word_rules("测试词", true);
    rules.exceptions = vec![exception(
        "exc-01",
        "软件测*试_词",
        &[Scene::ChannelMessage],
        "1001",
        &["canonical"],
        &[View::B, View::T],
    )];
    // the exception phrase covers exactly the gap candidate range, but gap
    // candidates are out of scope for exceptions (plan 4.8)
    let hits = pipeline(&rules, "软件测*试_词", Scene::ChannelMessage, false);
    assert_eq!(hits.len(), 1, "gap evidence survives the exception");
    assert_eq!(
        hits[0].methods,
        vec![MatchMethod::Literal, MatchMethod::Gap]
    );
}

#[test]
fn gap_candidates_share_the_request_budget() {
    let rules = one_word_rules("测试词", true);
    let (_, gaps, _) = compile(&rules);
    let text = "测*试词".repeat(200);
    let calls = Cell::new(0u32);
    let cancel = || {
        calls.set(calls.get() + 1);
        calls.get() > 5000
    };
    let mut budget = CandidateBudget::default();
    let outcome = gaps.scan_text(&text, Scene::ChannelMessage, true, &mut budget, &cancel);
    match outcome {
        Err(ErrorCode::CheckDeadline) | Err(ErrorCode::MatchBudgetExceeded) => {}
        Err(other) => panic!("unexpected error {other:?}"),
        Ok(hits) => panic!("expected budget enforcement, got {} hits", hits.len()),
    }
}

#[test]
fn exception_matching_consumes_the_shared_budget() {
    let mut rules = exception_rules(exception(
        "exc-01",
        "软件测试",
        &[Scene::ChannelMessage],
        "1001",
        &["canonical"],
        &[View::B],
    ));
    rules.literals[0].id = "1001".into();
    let table = table_by_id(&rules.t2s_table_id).unwrap();
    let exceptions = CompiledExceptions::compile(&rules, table).unwrap();
    let text = "软件测试".repeat(100);
    let units =
        crate::normalize::normalize_str(&text, crate::normalize::Semantics::EnhancedV1, &|| false)
            .unwrap();
    let hits: Vec<_> = (0..20)
        .map(|_| RawHit {
            rule_id: "1001".into(),
            variant_id: "canonical".into(),
            action: RuleAction::Review,
            start: 6,
            end: 12,
            methods: vec![MatchMethod::Literal],
        })
        .collect();
    let mut budget = CandidateBudget::default();
    // simulate a nearly exhausted request budget
    for _ in 0..19_950 {
        budget.consume(&|| false).unwrap();
    }
    let outcome = exceptions.filter_literal_hits(
        &units,
        None,
        Scene::ChannelMessage,
        hits,
        &mut budget,
        &|| false,
    );
    assert!(
        matches!(
            outcome,
            Err(ErrorCode::MatchBudgetExceeded) | Err(ErrorCode::CheckDeadline)
        ),
        "exception scanning must respect the shared budget"
    );
}

#[test]
fn t_view_exception_exempts_t_view_literal_evidence() {
    let mut rules = one_word_rules("测试词", false);
    rules.exceptions = vec![exception(
        "exc-t",
        "软件測試词",
        &[Scene::ChannelMessage],
        "1001",
        &["canonical"],
        &[View::T],
    )];
    // T view hit "测试词" (from 測試词) is covered by the T exception phrase
    let hits = pipeline(&rules, "软件測試词", Scene::ChannelMessage, true);
    assert!(hits.is_empty(), "T evidence must be exempted");
    // B view is not covered by a T-only exception (plain simplified text)
    let hits = pipeline(&rules, "软件测试词", Scene::ChannelMessage, true);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].methods, vec![MatchMethod::Literal]);
}

#[test]
fn differential_10k_with_gap_witnesses() {
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/oracle/manifest.json")).unwrap();
    let corpus = include_bytes!("../fixtures/oracle/cases-10k.jsonl");
    assert_eq!(
        format!("{:x}", Sha256::digest(corpus)),
        manifest["cases_sha256"].as_str().unwrap()
    );
    let rules: RuleSet = serde_json::from_str(include_str!("oracle-ruleset.json")).unwrap();
    let table = table_by_id(&rules.t2s_table_id).unwrap();
    let literals = CompiledLiterals::compile(&rules, table).unwrap();
    let gaps = CompiledGaps::compile(&rules, table).unwrap();
    let cancel = || false;
    let mut cases = 0;
    let mut gap_hits = 0;
    for line in std::str::from_utf8(corpus).unwrap().lines() {
        let case: serde_json::Value = serde_json::from_str(line).unwrap();
        let scene: Scene = serde_json::from_value(case["scene"].clone()).unwrap();
        let t2s = case["t2s"].as_bool().unwrap();
        let mut budget = CandidateBudget::default();
        let mut all = literals
            .scan_text(
                case["text"].as_str().unwrap(),
                scene,
                t2s,
                &mut budget,
                &cancel,
            )
            .unwrap();
        if case
            .get("gap")
            .and_then(|flag| flag.as_bool())
            .unwrap_or(false)
        {
            all.extend(
                gaps.scan_text(
                    case["text"].as_str().unwrap(),
                    scene,
                    t2s,
                    &mut budget,
                    &cancel,
                )
                .unwrap(),
            );
        }
        let actual: Vec<_> = aggregate(all)
            .into_iter()
            .map(|hit| {
                gap_hits += usize::from(
                    hit.methods
                        .iter()
                        .any(|method| matches!(method, MatchMethod::Gap)),
                );
                serde_json::json!({
                    "rule_id": hit.rule_id,
                    "variant_id": hit.variant_id,
                    "action": hit.action,
                    "start": hit.start,
                    "end": hit.end,
                    "methods": hit.methods,
                })
            })
            .collect();
        let expected = case["expected"].as_array().unwrap();
        assert_eq!(actual, *expected, "frozen oracle case {}", case["id"]);
        cases += 1;
    }
    assert_eq!(cases, 10_000);
    println!(
        "T09_DIFFERENTIAL_GAP cases={cases} gap_hits={gap_hits} seed=20261016 scope=gap-added"
    );
}

#[test]
fn differential_600_exception_witnesses() {
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/oracle-exceptions/manifest.json")).unwrap();
    let corpus = include_bytes!("../fixtures/oracle-exceptions/cases-600.jsonl");
    assert_eq!(
        format!("{:x}", Sha256::digest(corpus)),
        manifest["cases_sha256"].as_str().unwrap()
    );
    let rules: RuleSet =
        serde_json::from_str(include_str!("../fixtures/oracle-exceptions/ruleset.json")).unwrap();
    let table = table_by_id(&rules.t2s_table_id).unwrap();
    let literals = CompiledLiterals::compile(&rules, table).unwrap();
    let gaps = CompiledGaps::compile(&rules, table).unwrap();
    let exceptions = CompiledExceptions::compile(&rules, table).unwrap();
    let cancel = || false;
    let mut cases = 0;
    let mut exempted = 0usize;
    for line in std::str::from_utf8(corpus).unwrap().lines() {
        let case: serde_json::Value = serde_json::from_str(line).unwrap();
        let scene: Scene = serde_json::from_value(case["scene"].clone()).unwrap();
        let t2s = case["t2s"].as_bool().unwrap();
        let text = case["text"].as_str().unwrap();
        let mut budget = CandidateBudget::default();
        // T11 order: raw unaggregated literal hits so every candidate keeps
        // its B/T view identity through the exception filter.
        let semantics = match rules.semantics_by_scene.get(scene) {
            Semantics::CompatV1 => crate::normalize::Semantics::CompatV1,
            Semantics::EnhancedV1 => crate::normalize::Semantics::EnhancedV1,
        };
        let base_units = crate::normalize::normalize_str(text, semantics, &cancel).unwrap();
        let t2s_units = if t2s && semantics == crate::normalize::Semantics::EnhancedV1 {
            Some(crate::normalize::t2s_units(&base_units, table))
        } else {
            None
        };
        let mut literal = literals
            .scan_base(&base_units, scene, &mut budget, &cancel)
            .unwrap();
        if let Some(translated) = t2s_units.as_deref() {
            literal.extend(
                literals
                    .scan_t2s(translated, scene, &mut budget, &cancel)
                    .unwrap(),
            );
        }
        let before = literal.len();
        let filtered = exceptions
            .filter_literal_hits(
                &base_units,
                t2s_units.as_deref(),
                scene,
                literal,
                &mut budget,
                &cancel,
            )
            .unwrap();
        exempted += before - filtered.len();
        let mut all = filtered;
        all.extend(
            gaps.scan_text(text, scene, t2s, &mut budget, &cancel)
                .unwrap(),
        );
        let actual: Vec<_> = aggregate(all)
            .into_iter()
            .map(|hit| {
                serde_json::json!({
                    "rule_id": hit.rule_id,
                    "variant_id": hit.variant_id,
                    "action": hit.action,
                    "start": hit.start,
                    "end": hit.end,
                    "methods": hit.methods,
                })
            })
            .collect();
        let expected = case["expected"].as_array().unwrap();
        assert_eq!(actual, *expected, "frozen exception case {}", case["id"]);
        cases += 1;
    }
    assert_eq!(cases, 600);
    assert!(exempted > 0, "corpus must exercise real exemptions");
    println!("T09_DIFFERENTIAL_EXCEPTION cases={cases} exempted_hits={exempted} seed=20261017");
}
