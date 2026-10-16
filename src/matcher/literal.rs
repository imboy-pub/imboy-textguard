//! Canonical/alias literal compilation and Standard-overlapping scanning.

use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};
use std::collections::{HashMap, HashSet};
use std::fmt;

use super::alias::{alias_boundary_ok, validate_alias};
use super::{aggregate, CandidateBudget, RawHit};
use crate::model::{
    AliasBoundary, ErrorCode, LiteralRule, MatchMethod, RuleAction, RuleSet, Scene, SceneSemantics,
    Semantics, Severity,
};
use crate::normalize::provenance::Unit;
use crate::normalize::{base, t2s};

/// Frozen budget constants (plan section 5).
pub const MAX_LITERAL_SCALARS: usize = 64;
pub const MAX_CANONICAL_RULES: usize = 50_000;
pub const MAX_LITERAL_BYTES: usize = 192;
pub const MAX_ALIASES_PER_RULE: usize = 8;
pub const MAX_TOTAL_LOGICAL_PATTERNS: usize = 100_000;
pub const MAX_COMPILED_ENTRIES: usize = 200_000;
pub const MAX_NORMALIZED_PATTERN_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_METADATA_FANOUT: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompileIssue {
    InvalidRuleId,
    DuplicateRuleId,
    InvalidVariantId,
    DuplicateVariantId,
    InvalidMetadata,
    EmptyPattern,
    PatternLength,
    CanonicalRuleLimit,
    AliasLimit,
    LogicalPatternLimit,
    CompiledEntryLimit,
    NormalizedByteLimit,
    MetadataFanout,
    UnsupportedRuleset,
    Normalization,
    Cancelled,
    AutomatonBuild,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompileError {
    pub issue: CompileIssue,
}

impl From<CompileIssue> for CompileError {
    fn from(issue: CompileIssue) -> Self {
        Self { issue }
    }
}

impl From<CompileError> for ErrorCode {
    fn from(error: CompileError) -> Self {
        match error.issue {
            CompileIssue::Cancelled => Self::CheckDeadline,
            CompileIssue::Normalization => Self::NormalizerIncompatible,
            CompileIssue::UnsupportedRuleset => Self::UnsupportedRuleset,
            CompileIssue::AutomatonBuild => Self::InternalError,
            _ => Self::InvalidRuleset,
        }
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "literal compilation rejected: {:?}", self.issue)
    }
}

impl std::error::Error for CompileError {}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CompilationUsage {
    pub logical_patterns: usize,
    pub compiled_entries: usize,
    pub normalized_bytes: u64,
}

impl CompilationUsage {
    fn add_logical(&mut self, count: usize) -> Result<(), CompileError> {
        if count > MAX_TOTAL_LOGICAL_PATTERNS - self.logical_patterns {
            return Err(CompileIssue::LogicalPatternLimit.into());
        }
        self.logical_patterns += count;
        Ok(())
    }

    fn add_entry(&mut self, bytes: usize) -> Result<(), CompileError> {
        if self.compiled_entries == MAX_COMPILED_ENTRIES {
            return Err(CompileIssue::CompiledEntryLimit.into());
        }
        if bytes as u64 > MAX_NORMALIZED_PATTERN_BYTES - self.normalized_bytes {
            return Err(CompileIssue::NormalizedByteLimit.into());
        }
        self.compiled_entries += 1;
        self.normalized_bytes += bytes as u64;
        Ok(())
    }
}

/// One compiled pattern with all metadata sharing its normalized text
/// (normalization collisions keep every rule, plan 4.9 / AC-026).
#[derive(Debug, Clone)]
pub struct PatternMeta {
    pub rule_id: String,
    pub variant_id: String,
    pub action: RuleAction,
    pub scenes: Vec<Scene>,
    pub boundary: AliasBoundary,
    pub is_alias: bool,
}

pub struct CompiledLiterals {
    /// Automaton over the B view strings of every pattern.
    base: AhoCorasick,
    compat: AhoCorasick,
    /// Automaton over the T view strings of every canonical pattern
    /// (aliases never match the T view, plan 4.5).
    t2s: AhoCorasick,
    base_metas: Vec<Vec<PatternMeta>>,
    compat_metas: Vec<Vec<PatternMeta>>,
    t2s_metas: Vec<Vec<PatternMeta>>,
    semantics: SceneSemantics,
    table: &'static HashMap<char, char>,
    usage: CompilationUsage,
}

fn canonical_action(severity: Severity) -> RuleAction {
    severity.canonical_action()
}

fn view_string(units: &[Unit]) -> String {
    units.iter().map(|unit| unit.scalar).collect()
}

/// Unit index of every view byte offset (view bytes start at each unit).
fn unit_offsets(units: &[Unit]) -> Vec<u32> {
    let mut offsets = Vec::with_capacity(units.len() + 1);
    let mut cursor = 0u32;
    for unit in units {
        offsets.push(cursor);
        cursor += unit.scalar.len_utf8() as u32;
    }
    offsets.push(cursor);
    offsets
}

fn unit_index_for_byte(offsets: &[u32], byte: u32) -> usize {
    offsets
        .binary_search(&byte)
        .unwrap_or_else(|index| index.saturating_sub(1))
}

impl CompiledLiterals {
    /// Compile a ruleset for the given t2s table. Enforces every frozen
    /// validation rule; any violation rejects the whole package (AC-027).
    pub fn compile(
        ruleset: &RuleSet,
        table: &'static HashMap<char, char>,
    ) -> Result<Self, CompileError> {
        Self::compile_cancellable(ruleset, table, &|| false)
    }

    pub fn compile_cancellable(
        ruleset: &RuleSet,
        table: &'static HashMap<char, char>,
        cancel: &dyn Fn() -> bool,
    ) -> Result<Self, CompileError> {
        if cancel() {
            return Err(CompileIssue::Cancelled.into());
        }
        if ruleset.schema_version != 1
            || ruleset.normalization_id != crate::NORMALIZATION_ID
            || ruleset.limit_profile_id != crate::LIMIT_PROFILE_ID
            || t2s::table_by_id(&ruleset.t2s_table_id) != Some(table)
        {
            return Err(CompileIssue::UnsupportedRuleset.into());
        }
        if ruleset.literals.len() > MAX_CANONICAL_RULES {
            return Err(CompileIssue::CanonicalRuleLimit.into());
        }
        if ruleset.literals.is_empty() && ruleset.regex_rules.is_empty() && !ruleset.allow_empty {
            return Err(CompileIssue::EmptyPattern.into());
        }
        let mut base_bank = PatternBank::default();
        let mut compat_bank = PatternBank::default();
        let mut t2s_bank = PatternBank::default();
        let mut usage = CompilationUsage::default();
        let mut seen_ids = HashSet::new();
        for rule in &ruleset.literals {
            if cancel() {
                return Err(CompileIssue::Cancelled.into());
            }
            validate_rule(rule)?;
            if !seen_ids.insert(rule.id.as_str()) {
                return Err(CompileIssue::DuplicateRuleId.into());
            }
            if rule.aliases.len() > MAX_ALIASES_PER_RULE {
                return Err(CompileIssue::AliasLimit.into());
            }
            usage.add_logical(1 + rule.aliases.len())?;
            let base_word_units =
                normalized_pattern(&rule.word, base::Semantics::EnhancedV1, cancel)?;
            let compat_word_units =
                normalized_pattern(&rule.word, base::Semantics::CompatV1, cancel)?;
            let t2s_word_units = t2s::t2s_units(&base_word_units, table);
            let canonical = PatternMeta {
                rule_id: rule.id.clone(),
                variant_id: "canonical".into(),
                action: canonical_action(rule.severity),
                scenes: rule.scenes.clone(),
                boundary: AliasBoundary::None,
                is_alias: false,
            };
            base_bank.push(view_string(&base_word_units), canonical.clone(), &mut usage)?;
            compat_bank.push(
                view_string(&compat_word_units),
                canonical.clone(),
                &mut usage,
            )?;
            t2s_bank.push(view_string(&t2s_word_units), canonical, &mut usage)?;
            let mut variants = HashSet::new();
            for alias in &rule.aliases {
                validate_alias(alias)?;
                if !variants.insert(alias.variant_id.as_str()) {
                    return Err(CompileIssue::DuplicateVariantId.into());
                }
                let alias_units =
                    normalized_pattern(&alias.pattern, base::Semantics::EnhancedV1, cancel)?;
                base_bank.push(
                    view_string(&alias_units),
                    PatternMeta {
                        rule_id: rule.id.clone(),
                        variant_id: alias.variant_id.clone(),
                        action: alias.action,
                        scenes: rule.scenes.clone(),
                        boundary: alias.boundary,
                        is_alias: true,
                    },
                    &mut usage,
                )?;
            }
        }
        let base = build_standard(&base_bank.patterns, cancel)?;
        let compat = build_standard(&compat_bank.patterns, cancel)?;
        let t2s = build_standard(&t2s_bank.patterns, cancel)?;
        Ok(Self {
            base,
            compat,
            t2s,
            base_metas: base_bank.metas,
            compat_metas: compat_bank.metas,
            t2s_metas: t2s_bank.metas,
            semantics: ruleset.semantics_by_scene.clone(),
            table,
            usage,
        })
    }

    pub fn usage(&self) -> CompilationUsage {
        self.usage
    }

    pub fn scan_text(
        &self,
        text: &str,
        scene: Scene,
        enable_t2s: bool,
        budget: &mut CandidateBudget,
        cancel: &dyn Fn() -> bool,
    ) -> Result<Vec<RawHit>, ErrorCode> {
        budget.check(cancel)?;
        if text.len() > 16 * 1024 {
            return Err(ErrorCode::TextTooLarge);
        }
        let semantics = match self.semantics.get(scene) {
            Semantics::CompatV1 => base::Semantics::CompatV1,
            Semantics::EnhancedV1 => base::Semantics::EnhancedV1,
        };
        let units = base::normalize_str(text, semantics, cancel).map_err(normalization_error)?;
        let mut hits = self.scan_base(&units, scene, budget, cancel)?;
        if enable_t2s && semantics == base::Semantics::EnhancedV1 {
            let translated = t2s::t2s_units(&units, self.table);
            hits.extend(self.scan_t2s(&translated, scene, budget, cancel)?);
        }
        budget.check(cancel)?;
        Ok(aggregate(hits))
    }

    pub fn scan_base(
        &self,
        units: &[Unit],
        scene: Scene,
        budget: &mut CandidateBudget,
        cancel: &dyn Fn() -> bool,
    ) -> Result<Vec<RawHit>, ErrorCode> {
        if self.semantics.get(scene) == Semantics::CompatV1 {
            scan_view(
                &self.compat,
                &self.compat_metas,
                units,
                scene,
                false,
                budget,
                cancel,
            )
        } else {
            scan_view(
                &self.base,
                &self.base_metas,
                units,
                scene,
                false,
                budget,
                cancel,
            )
        }
    }

    /// Scan the T view (canonical patterns only; the t2s method is appended).
    pub fn scan_t2s(
        &self,
        units: &[Unit],
        scene: Scene,
        budget: &mut CandidateBudget,
        cancel: &dyn Fn() -> bool,
    ) -> Result<Vec<RawHit>, ErrorCode> {
        budget.check(cancel)?;
        if self.semantics.get(scene) == Semantics::CompatV1 {
            return Ok(Vec::new());
        }
        scan_view(
            &self.t2s,
            &self.t2s_metas,
            units,
            scene,
            true,
            budget,
            cancel,
        )
    }
}

fn scan_view(
    automaton: &AhoCorasick,
    metadata: &[Vec<PatternMeta>],
    units: &[Unit],
    scene: Scene,
    translated: bool,
    budget: &mut CandidateBudget,
    cancel: &dyn Fn() -> bool,
) -> Result<Vec<RawHit>, ErrorCode> {
    budget.check(cancel)?;
    if units.iter().any(|unit| unit.start >= unit.end)
        || units
            .windows(2)
            .any(|pair| pair[0].start > pair[1].start || pair[0].end > pair[1].end)
    {
        return Err(ErrorCode::NormalizerIncompatible);
    }
    let text = view_string(units);
    let offsets = unit_offsets(units);
    let mut hits = Vec::new();
    for matched in automaton.find_overlapping_iter(&text) {
        let metas = &metadata[matched.pattern()];
        for meta in metas {
            budget.consume(cancel)?;
            if !meta.scenes.contains(&scene) {
                continue;
            }
            if meta.is_alias
                && !alias_boundary_ok(meta.boundary, &text, matched.start(), matched.end())
            {
                continue;
            }
            let first = unit_index_for_byte(&offsets, matched.start() as u32);
            let last = unit_index_for_byte(&offsets, matched.end() as u32 - 1);
            let original_start = units[first].start;
            let original_end = units[last].end;
            hits.push(RawHit {
                rule_id: meta.rule_id.clone(),
                variant_id: meta.variant_id.clone(),
                action: meta.action,
                start: original_start,
                end: original_end,
                methods: if meta.is_alias {
                    vec![MatchMethod::Alias]
                } else if translated {
                    vec![MatchMethod::Literal, MatchMethod::T2s]
                } else {
                    vec![MatchMethod::Literal]
                },
            });
        }
    }
    budget.check(cancel)?;
    Ok(hits)
}

#[derive(Default)]
struct PatternBank {
    indices: HashMap<String, usize>,
    patterns: Vec<String>,
    metas: Vec<Vec<PatternMeta>>,
}

impl PatternBank {
    fn push(
        &mut self,
        view: String,
        meta: PatternMeta,
        usage: &mut CompilationUsage,
    ) -> Result<(), CompileError> {
        if view.is_empty() {
            return Err(CompileIssue::EmptyPattern.into());
        }
        usage.add_entry(view.len())?;
        if let Some(&index) = self.indices.get(&view) {
            if self.metas[index].len() == MAX_METADATA_FANOUT {
                return Err(CompileIssue::MetadataFanout.into());
            }
            self.metas[index].push(meta);
        } else {
            self.indices.insert(view.clone(), self.patterns.len());
            self.patterns.push(view);
            self.metas.push(vec![meta]);
        }
        Ok(())
    }
}

fn build_standard(
    patterns: &[String],
    cancel: &dyn Fn() -> bool,
) -> Result<AhoCorasick, CompileError> {
    if cancel() {
        return Err(CompileIssue::Cancelled.into());
    }
    let automaton = AhoCorasickBuilder::new()
        .match_kind(MatchKind::Standard)
        .build(patterns)
        .map_err(|_| CompileError::from(CompileIssue::AutomatonBuild))?;
    if cancel() {
        return Err(CompileIssue::Cancelled.into());
    }
    Ok(automaton)
}

fn validate_rule(rule: &LiteralRule) -> Result<(), CompileError> {
    let bytes = rule.id.as_bytes();
    if bytes.is_empty()
        || bytes.len() > 20
        || !matches!(bytes[0], b'1'..=b'9')
        || !bytes.iter().all(u8::is_ascii_digit)
    {
        return Err(CompileIssue::InvalidRuleId.into());
    }
    let scenes: HashSet<_> = rule.scenes.iter().collect();
    if scenes.is_empty()
        || scenes.len() != rule.scenes.len()
        || rule.category.is_empty()
        || rule.category.len() > 64
    {
        return Err(CompileIssue::InvalidMetadata.into());
    }
    Ok(())
}

fn normalized_pattern(
    word: &str,
    semantics: base::Semantics,
    cancel: &dyn Fn() -> bool,
) -> Result<Vec<Unit>, CompileError> {
    if word.is_empty() {
        return Err(CompileIssue::EmptyPattern.into());
    }
    if word.chars().count() > MAX_LITERAL_SCALARS || word.len() > MAX_LITERAL_BYTES {
        return Err(CompileIssue::PatternLength.into());
    }
    let units = base::normalize_str(word, semantics, cancel).map_err(|error| {
        CompileError::from(if error == base::NormalizationError::Cancelled {
            CompileIssue::Cancelled
        } else {
            CompileIssue::Normalization
        })
    })?;
    if units.is_empty() {
        return Err(CompileIssue::EmptyPattern.into());
    }
    Ok(units)
}

fn normalization_error(error: base::NormalizationError) -> ErrorCode {
    match error {
        base::NormalizationError::InvalidUtf8 => ErrorCode::InvalidUtf8,
        base::NormalizationError::LimitExceeded => ErrorCode::NormalizationLimit,
        base::NormalizationError::NormalizerIncompatible => ErrorCode::NormalizerIncompatible,
        base::NormalizationError::Cancelled => ErrorCode::CheckDeadline,
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    #[test]
    fn compiled_entries_exact_limit_and_next_are_distinct() {
        let mut usage = CompilationUsage {
            compiled_entries: MAX_COMPILED_ENTRIES - 1,
            ..CompilationUsage::default()
        };
        usage.add_entry(1).unwrap();
        assert_eq!(usage.compiled_entries, MAX_COMPILED_ENTRIES);
        assert_eq!(
            usage.add_entry(1).unwrap_err().issue,
            CompileIssue::CompiledEntryLimit
        );
    }

    #[test]
    fn logical_and_normalized_byte_budgets_are_closed_intervals() {
        let mut usage = CompilationUsage {
            logical_patterns: MAX_TOTAL_LOGICAL_PATTERNS - 1,
            normalized_bytes: MAX_NORMALIZED_PATTERN_BYTES - 1,
            ..CompilationUsage::default()
        };
        usage.add_logical(1).unwrap();
        usage.add_entry(1).unwrap();
        assert_eq!(
            usage.add_logical(1).unwrap_err().issue,
            CompileIssue::LogicalPatternLimit
        );
        assert_eq!(
            usage.add_entry(1).unwrap_err().issue,
            CompileIssue::NormalizedByteLimit
        );
    }
}
