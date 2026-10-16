use crate::model::{valid_identifier, ErrorCode, Hit, MatchMethod, RegexRule, Scene, VariantType};
use crate::normalize::provenance::Unit;
use regex::{Regex, RegexBuilder};
use std::collections::HashSet;

const MAX_RULES: usize = 32;
const MAX_SOURCE_BYTES: usize = 512;
const MAX_NESTING: u32 = 32;
const SIZE_LIMIT: usize = 768 * 1024;
const DFA_SIZE_LIMIT: usize = 256 * 1024;
const TOTAL_CONFIG_LIMIT: usize = 32 * 1024 * 1024;

pub struct CompiledRegex {
    rules: Vec<(RegexRule, Regex)>,
}

fn valid_rule(rule: &RegexRule) -> bool {
    rule.id
        .strip_prefix("re:")
        .is_some_and(|suffix| valid_identifier(suffix) && suffix.len() <= 60)
        && rule.expression.len() <= MAX_SOURCE_BYTES
        && !rule.display_word.is_empty()
        && rule.display_word.len() <= 192
        && rule.display_word.chars().count() <= 64
        && !rule.category.is_empty()
        && rule.category.len() <= 64
        && !rule.scenes.is_empty()
        && rule.scenes.iter().collect::<HashSet<_>>().len() == rule.scenes.len()
}

impl CompiledRegex {
    pub fn compile(rules: &[RegexRule], cancel: &dyn Fn() -> bool) -> Result<Self, ErrorCode> {
        Self::compile_with_limits(rules, SIZE_LIMIT, DFA_SIZE_LIMIT, cancel)
    }

    fn compile_with_limits(
        rules: &[RegexRule],
        size_limit: usize,
        dfa_limit: usize,
        cancel: &dyn Fn() -> bool,
    ) -> Result<Self, ErrorCode> {
        if rules.len() > MAX_RULES
            || size_limit > 1024 * 1024
            || dfa_limit > DFA_SIZE_LIMIT
            || rules
                .len()
                .saturating_mul(size_limit.saturating_add(dfa_limit))
                > TOTAL_CONFIG_LIMIT
        {
            return Err(ErrorCode::InvalidRuleset);
        }
        let mut identities = HashSet::new();
        let mut compiled = Vec::with_capacity(rules.len());
        for rule in rules {
            if cancel() {
                return Err(ErrorCode::CheckDeadline);
            }
            if !valid_rule(rule) || !identities.insert(&rule.id) {
                return Err(ErrorCode::InvalidRuleset);
            }
            let hir = regex_syntax::ParserBuilder::new()
                .nest_limit(MAX_NESTING)
                .build()
                .parse(&rule.expression)
                .map_err(|_| ErrorCode::InvalidRuleset)?;
            if !matches!(hir.properties().minimum_len(), Some(minimum) if minimum > 0) {
                return Err(ErrorCode::InvalidRuleset);
            }
            let expression = RegexBuilder::new(&rule.expression)
                .nest_limit(MAX_NESTING)
                .size_limit(size_limit)
                .dfa_size_limit(dfa_limit)
                .build()
                .map_err(|_| ErrorCode::InvalidRuleset)?;
            compiled.push((rule.clone(), expression));
        }
        Ok(Self { rules: compiled })
    }

    pub fn scan(
        &self,
        scene: Scene,
        field_id: &str,
        units: &[Unit],
        cancel: &dyn Fn() -> bool,
    ) -> Result<Vec<Hit>, ErrorCode> {
        if cancel() {
            return Err(ErrorCode::CheckDeadline);
        }
        let text: String = units.iter().map(|unit| unit.scalar).collect();
        let starts: Vec<usize> = text.char_indices().map(|(offset, _)| offset).collect();
        let mut hits = Vec::new();
        for (rule, expression) in &self.rules {
            if cancel() {
                return Err(ErrorCode::CheckDeadline);
            }
            if !rule.scenes.contains(&scene) {
                continue;
            }
            let witness = expression.find(&text);
            if cancel() {
                return Err(ErrorCode::CheckDeadline);
            }
            if let Some(found) = witness {
                let start = starts.partition_point(|offset| *offset < found.start());
                let end = starts.partition_point(|offset| *offset < found.end());
                hits.push(Hit {
                    field_id: field_id.to_owned(),
                    rule_id: rule.id.clone(),
                    variant_id: "regex".to_owned(),
                    variant_type: VariantType::RegexRule,
                    word: rule.display_word.clone(),
                    category: rule.category.clone(),
                    severity: rule.severity,
                    effective_action: rule.action,
                    start_byte: units[start].start,
                    end_byte: units[end - 1].end,
                    match_methods: vec![MatchMethod::Regex],
                });
            }
        }
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{RuleAction, Severity};
    use crate::normalize::base::{normalize_str, Semantics};
    use std::cell::Cell;

    fn rule(expression: &str) -> RegexRule {
        RegexRule {
            id: "re:r1".into(),
            expression: expression.into(),
            display_word: "synthetic".into(),
            category: "test".into(),
            severity: Severity::High,
            action: RuleAction::Review,
            scenes: vec![Scene::ChannelMessage],
        }
    }

    fn scan(rules: &[RegexRule], text: &str) -> Vec<Hit> {
        let units = normalize_str(text, Semantics::EnhancedV1, &|| false).unwrap();
        CompiledRegex::compile(rules, &|| false)
            .unwrap()
            .scan(Scene::ChannelMessage, "body", &units, &|| false)
            .unwrap()
    }

    #[test]
    fn first_witness_and_later_rules_preserve_actions() {
        let mut second = rule("456");
        second.id = "re:r2".into();
        second.action = RuleAction::Block;
        let hits = scan(&[rule("[0-9]{3}"), second], "123后面456");
        assert_eq!(hits.len(), 2);
        assert_eq!((hits[0].start_byte, hits[0].end_byte), (0, 3));
        assert_eq!(hits[0].effective_action, RuleAction::Review);
        assert_eq!((hits[1].start_byte, hits[1].end_byte), (9, 12));
        assert_eq!(hits[1].effective_action, RuleAction::Block);
    }

    #[test]
    fn unsupported_syntax_rejected_as_one_package() {
        for expression in [r"(a)\1", "(?=a)", "(?<=a)b", "("] {
            assert!(matches!(
                CompiledRegex::compile(&[rule("ok"), rule(expression)], &|| false),
                Err(ErrorCode::InvalidRuleset)
            ));
            assert!(matches!(
                CompiledRegex::compile(&[rule(expression)], &|| false),
                Err(ErrorCode::InvalidRuleset)
            ));
        }
    }

    #[test]
    fn hir_rejects_zero_width_and_impossible_rules() {
        for expression in ["", "a*", "^$", r"\b", r"\B", "a|", "[a&&b]", "(?:x){0}"] {
            assert!(
                matches!(
                    CompiledRegex::compile(&[rule(expression)], &|| false),
                    Err(ErrorCode::InvalidRuleset)
                ),
                "{expression}"
            );
        }
    }

    #[test]
    fn source_nesting_compilation_and_aggregate_budgets() {
        for expression in [
            "x".repeat(513),
            format!("{}x{}", "(".repeat(33), ")".repeat(33)),
            r"\w{100000}".into(),
        ] {
            assert!(matches!(
                CompiledRegex::compile(&[rule(&expression)], &|| false),
                Err(ErrorCode::InvalidRuleset)
            ));
        }
        let rules: Vec<_> = (0..32)
            .map(|index| {
                let mut item = rule("x");
                item.id = format!("re:r{index}");
                item
            })
            .collect();
        assert!(CompiledRegex::compile(&rules, &|| false).is_ok());
        assert!(matches!(
            CompiledRegex::compile(&[rules.clone(), vec![rule("a")]].concat(), &|| false),
            Err(ErrorCode::InvalidRuleset)
        ));
        assert!(matches!(
            CompiledRegex::compile_with_limits(&rules, 1024 * 1024, DFA_SIZE_LIMIT, &|| false),
            Err(ErrorCode::InvalidRuleset)
        ));
        assert!(matches!(
            CompiledRegex::compile_with_limits(&rules[..1], 1024 * 1024 + 1, 0, &|| false),
            Err(ErrorCode::InvalidRuleset)
        ));
        assert!(matches!(
            CompiledRegex::compile_with_limits(
                &rules[..1],
                SIZE_LIMIT,
                DFA_SIZE_LIMIT + 1,
                &|| false
            ),
            Err(ErrorCode::InvalidRuleset)
        ));
    }

    #[test]
    fn expressions_are_not_lowercased_or_width_folded() {
        assert!(scan(&[rule("ABC")], "ABC").is_empty());
        assert_eq!(scan(&[rule(r"\D+")], "ＡＢＣ123").len(), 1);
        assert!(scan(&[rule("ＡＢＣ")], "ＡＢＣ").is_empty());
        let hits = scan(&[rule("abc")], "ＡＢＣ");
        assert_eq!((hits[0].start_byte, hits[0].end_byte), (0, 9));
    }

    #[test]
    fn conservative_original_offsets_cover_removed_and_expanded_scalars() {
        let hits = scan(&[rule("测试词")], "🙂测\u{200b}试词");
        assert_eq!((hits[0].start_byte, hits[0].end_byte), (4, 16));
        let hits = scan(
            &[rule("i"), {
                let mut item = rule("\u{307}");
                item.id = "re:dot".into();
                item
            }],
            "🙂İ",
        );
        assert_eq!(hits.len(), 2);
        assert!(hits
            .iter()
            .all(|hit| (hit.start_byte, hit.end_byte) == (4, 6)));
    }

    #[test]
    fn fields_and_scenes_are_independent_with_first_witness_each() {
        let compiled = CompiledRegex::compile(&[rule("a|ab")], &|| false).unwrap();
        let units = normalize_str("ababa", Semantics::EnhancedV1, &|| false).unwrap();
        for field in ["title", "body"] {
            let hits = compiled
                .scan(Scene::ChannelMessage, field, &units, &|| false)
                .unwrap();
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].field_id, field);
            assert_eq!((hits[0].start_byte, hits[0].end_byte), (0, 1));
        }
        assert!(compiled
            .scan(Scene::ProfileField, "body", &units, &|| false)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn cancellation_at_compile_and_scan_boundaries() {
        assert!(matches!(
            CompiledRegex::compile(&[rule("x")], &|| true),
            Err(ErrorCode::CheckDeadline)
        ));
        let compiled = CompiledRegex::compile(&[rule("x")], &|| false).unwrap();
        let units = normalize_str("x", Semantics::EnhancedV1, &|| false).unwrap();
        for limit in [1, 2, 3] {
            let polls = Cell::new(0);
            let cancel = || {
                polls.set(polls.get() + 1);
                polls.get() >= limit
            };
            assert_eq!(
                compiled.scan(Scene::ChannelMessage, "body", &units, &cancel),
                Err(ErrorCode::CheckDeadline)
            );
        }
    }

    #[test]
    fn identities_and_scene_membership_are_validated() {
        assert!(matches!(
            CompiledRegex::compile(&[rule("a"), rule("b")], &|| false),
            Err(ErrorCode::InvalidRuleset)
        ));
        let mut invalid = rule("a");
        invalid.id.clear();
        assert!(matches!(
            CompiledRegex::compile(&[invalid], &|| false),
            Err(ErrorCode::InvalidRuleset)
        ));
        for scenes in [vec![], vec![Scene::ChannelMessage, Scene::ChannelMessage]] {
            let mut invalid = rule("a");
            invalid.scenes = scenes;
            assert!(matches!(
                CompiledRegex::compile(&[invalid], &|| false),
                Err(ErrorCode::InvalidRuleset)
            ));
        }
    }

    #[test]
    fn schema_dto_and_compiler_agree_on_regex_namespace_boundaries() {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../schemas/ruleset.schema.json")).unwrap();
        let pattern = schema["properties"]["regex_rules"]["items"]["properties"]["id"]["pattern"]
            .as_str()
            .unwrap();
        let schema_id = Regex::new(pattern).unwrap();
        for (identity, expected) in [
            ("re:digits".to_owned(), true),
            (format!("re:{}", "x".repeat(60)), true),
            (format!("re:{}", "x".repeat(61)), false),
            ("re:".to_owned(), false),
            ("r1".to_owned(), false),
            ("re:汉".to_owned(), false),
            ("re:a:b".to_owned(), false),
        ] {
            assert_eq!(schema_id.is_match(&identity), expected);
            let mut value = serde_json::to_value(rule("[0-9]{3}")).unwrap();
            value["id"] = identity.into();
            let decoded: RegexRule = serde_json::from_value(value).unwrap();
            assert_eq!(
                CompiledRegex::compile(&[decoded], &|| false).is_ok(),
                expected
            );
        }
        assert_eq!(scan(&[rule("[0-9]{3}")], "123后面456")[0].start_byte, 0);
    }

    #[test]
    fn metadata_cannot_expand_response_without_bound() {
        for (display_word, category) in [
            ("x".repeat(65), "test".into()),
            ("测试".into(), "x".repeat(65)),
            (String::new(), "test".into()),
            ("label".into(), String::new()),
        ] {
            let mut invalid = rule("a");
            invalid.display_word = display_word;
            invalid.category = category;
            assert!(matches!(
                CompiledRegex::compile(&[invalid], &|| false),
                Err(ErrorCode::InvalidRuleset)
            ));
        }
    }
}
