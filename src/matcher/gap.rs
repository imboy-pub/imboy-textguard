//! Bounded gap matching over the G view (plan section 4.6).
//!
//! Only canonical literals may enable gap. The normalized view is segmented
//! on hard boundaries, ignorable characters are removed inside each segment,
//! and every automaton candidate is re-validated against the adjacency
//! (<=2) and total (<=4) ignored-character budgets before it becomes a hit.
//! Hit ranges always map back to original text bytes and the original text
//! is never modified.

use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};
use std::collections::{HashMap, HashSet};

use super::literal::{
    CompileError, CompileIssue, MAX_COMPILED_ENTRIES, MAX_LITERAL_BYTES, MAX_LITERAL_SCALARS,
    MAX_NORMALIZED_PATTERN_BYTES,
};
use super::{aggregate, CandidateBudget, RawHit};
use crate::model::{ErrorCode, MatchMethod, RuleAction, RuleSet, Scene, SceneSemantics, Semantics};
use crate::normalize::provenance::Unit;
use crate::normalize::{base, t2s};

pub const MAX_ADJACENT_IGNORED: u32 = 2;
pub const MAX_TOTAL_IGNORED: u32 = 4;

const IGNORABLE: [char; 5] = [' ', '\u{00b7}', '*', '-', '_'];
const HARD_BOUNDARIES: [char; 12] = [
    '\r', '\n', '\u{2028}', '\u{2029}', '.', '\u{3002}', '!', '\u{ff01}', '?', '\u{ff1f}', ';',
    '\u{ff1b}',
];

pub fn is_ignorable(scalar: char) -> bool {
    IGNORABLE.contains(&scalar)
}

fn is_hard_boundary(scalar: char) -> bool {
    HARD_BOUNDARIES.contains(&scalar)
}

/// One gap-enabled canonical rule inside a view automaton.
#[derive(Debug, Clone)]
pub struct GapMeta {
    pub rule_id: String,
    pub action: RuleAction,
    pub scenes: Vec<Scene>,
}

struct Bank {
    automaton: AhoCorasick,
    metas: Vec<Vec<GapMeta>>,
}

/// Frozen compilation of every gap rule across the B(enhanced) and T
/// views. `None` banks mean no rule enabled gap for that view. Compat-v1
/// scenes keep the legacy behavior and never scan gap (plan 4.2: gap is an
/// enhanced-v1 capability).
pub struct CompiledGaps {
    base: Option<Bank>,
    t2s: Option<Bank>,
    semantics: SceneSemantics,
    table: &'static HashMap<char, char>,
}

impl CompiledGaps {
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
        let mut base_bank = PatternBank::default();
        let mut t2s_bank = PatternBank::default();
        let mut seen_ids = HashSet::new();
        for rule in &ruleset.literals {
            if cancel() {
                return Err(CompileIssue::Cancelled.into());
            }
            if !seen_ids.insert(rule.id.as_str()) {
                return Err(CompileIssue::DuplicateRuleId.into());
            }
            if !rule.gap.enabled {
                continue;
            }
            let meta = GapMeta {
                rule_id: rule.id.clone(),
                action: rule.gap.action,
                scenes: rule.scenes.clone(),
            };
            let base_units = normalized_pattern(&rule.word, base::Semantics::EnhancedV1, cancel)?;
            let t2s_units = t2s::t2s_units(&base_units, table);
            validate_gap_pattern(&base_units)?;
            validate_gap_pattern(&t2s_units)?;
            base_bank.push(view_string(&base_units), meta.clone())?;
            t2s_bank.push(view_string(&t2s_units), meta)?;
        }
        Ok(Self {
            base: build_bank(base_bank, cancel)?,
            t2s: build_bank(t2s_bank, cancel)?,
            semantics: ruleset.semantics_by_scene.clone(),
            table,
        })
    }

    /// Convenience entry: normalize one field, scan its B view and (when
    /// enabled) its T view, and aggregate. Mirrors `CompiledLiterals`.
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

    /// Scan the B view of already-normalized units. Compat-v1 scenes never
    /// scan gap: the capability is enhanced-v1 only (plan 4.2).
    pub fn scan_base(
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
        match &self.base {
            Some(bank) => scan_bank(bank, units, scene, false, budget, cancel),
            None => Ok(Vec::new()),
        }
    }

    /// Scan the T view units (canonical gap patterns only). Compat scenes
    /// never scan the T view, matching `CompiledLiterals::scan_t2s`.
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
        match &self.t2s {
            Some(bank) => scan_bank(bank, units, scene, true, budget, cancel),
            None => Ok(Vec::new()),
        }
    }
}

fn scan_bank(
    bank: &Bank,
    units: &[Unit],
    scene: Scene,
    translated: bool,
    budget: &mut CandidateBudget,
    cancel: &dyn Fn() -> bool,
) -> Result<Vec<RawHit>, ErrorCode> {
    budget.check(cancel)?;
    let mut hits = Vec::new();
    for segment in segments(units) {
        budget.check(cancel)?;
        let effective = EffectiveView::build(segment);
        effective.scan(bank, scene, translated, budget, cancel, &mut hits)?;
    }
    budget.check(cancel)?;
    Ok(hits)
}

/// Split view units on hard boundaries; boundary characters belong to no
/// segment so candidates can never be stitched across them.
fn segments(units: &[Unit]) -> Vec<&[Unit]> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for (index, unit) in units.iter().enumerate() {
        if is_hard_boundary(unit.scalar) {
            if index > start {
                out.push(&units[start..index]);
            }
            start = index + 1;
        }
    }
    if start < units.len() {
        out.push(&units[start..]);
    }
    out
}

/// One hard-boundary segment with its ignorable characters removed.
struct EffectiveView<'a> {
    units: Vec<&'a Unit>,
    positions: Vec<usize>,
    body: String,
    offsets: Vec<u32>,
}

impl<'a> EffectiveView<'a> {
    fn build(segment: &'a [Unit]) -> Self {
        let mut units = Vec::with_capacity(segment.len());
        let mut positions = Vec::with_capacity(segment.len());
        let mut body = String::with_capacity(segment.len() * 3);
        let mut offsets = Vec::with_capacity(segment.len() + 1);
        for (position, unit) in segment.iter().enumerate() {
            if is_ignorable(unit.scalar) {
                continue;
            }
            offsets.push(body.len() as u32);
            body.push(unit.scalar);
            units.push(unit);
            positions.push(position);
        }
        offsets.push(body.len() as u32);
        Self {
            units,
            positions,
            body,
            offsets,
        }
    }

    fn scan(
        &self,
        bank: &Bank,
        scene: Scene,
        translated: bool,
        budget: &mut CandidateBudget,
        cancel: &dyn Fn() -> bool,
        hits: &mut Vec<RawHit>,
    ) -> Result<(), ErrorCode> {
        for matched in bank.automaton.find_overlapping_iter(&self.body) {
            let metas = &bank.metas[matched.pattern()];
            for meta in metas {
                budget.consume(cancel)?;
                if !meta.scenes.contains(&scene) {
                    continue;
                }
                if let Some(hit) = self.candidate(meta, matched.start(), matched.end(), translated)
                {
                    hits.push(hit);
                }
            }
        }
        Ok(())
    }

    /// Re-validate one candidate against the adjacency and total budgets;
    /// identical arithmetic to the independent Python oracle.
    fn candidate(
        &self,
        meta: &GapMeta,
        start_byte: usize,
        end_byte: usize,
        translated: bool,
    ) -> Option<RawHit> {
        let first = unit_index_for_byte(&self.offsets, start_byte as u32);
        let last = unit_index_for_byte(&self.offsets, end_byte as u32 - 1);
        let mut total = 0u32;
        for window in first..last {
            let gap = (self.positions[window + 1] - self.positions[window] - 1) as u32;
            if gap > MAX_ADJACENT_IGNORED {
                return None;
            }
            total += gap;
        }
        if total > MAX_TOTAL_IGNORED {
            return None;
        }
        Some(RawHit {
            rule_id: meta.rule_id.clone(),
            variant_id: "canonical".into(),
            action: meta.action,
            start: self.units[first].start,
            end: self.units[last].end,
            methods: if translated {
                vec![MatchMethod::Literal, MatchMethod::T2s, MatchMethod::Gap]
            } else {
                vec![MatchMethod::Literal, MatchMethod::Gap]
            },
        })
    }
}

fn unit_index_for_byte(offsets: &[u32], byte: u32) -> usize {
    offsets
        .binary_search(&byte)
        .unwrap_or_else(|index| index.saturating_sub(1))
}

/// A gap pattern must not contain ignorable or hard-boundary characters in
/// any view, otherwise enabling gap for that rule is rejected at publish
/// time (plan 4.6).
pub fn validate_gap_pattern(units: &[Unit]) -> Result<(), CompileError> {
    for unit in units {
        if is_ignorable(unit.scalar) || is_hard_boundary(unit.scalar) {
            return Err(CompileIssue::InvalidMetadata.into());
        }
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

fn view_string(units: &[Unit]) -> String {
    units.iter().map(|unit| unit.scalar).collect()
}

/// Section-5 compile budgets enforced independently per gap view bank.
/// Cross-channel totals (literal + gap + regex) are the T11 engine's duty.
#[derive(Default)]
struct PatternBank {
    indices: HashMap<String, usize>,
    patterns: Vec<String>,
    metas: Vec<Vec<GapMeta>>,
    entries: usize,
    normalized_bytes: u64,
}

impl PatternBank {
    fn push(&mut self, view: String, meta: GapMeta) -> Result<(), CompileError> {
        if view.is_empty() {
            return Err(CompileIssue::EmptyPattern.into());
        }
        if self.entries == MAX_COMPILED_ENTRIES {
            return Err(CompileIssue::CompiledEntryLimit.into());
        }
        if view.len() as u64 > MAX_NORMALIZED_PATTERN_BYTES - self.normalized_bytes {
            return Err(CompileIssue::NormalizedByteLimit.into());
        }
        self.entries += 1;
        self.normalized_bytes += view.len() as u64;
        if let Some(&index) = self.indices.get(&view) {
            if self.metas[index].len() == super::literal::MAX_METADATA_FANOUT {
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

fn build_bank(bank: PatternBank, cancel: &dyn Fn() -> bool) -> Result<Option<Bank>, CompileError> {
    if bank.patterns.is_empty() {
        return Ok(None);
    }
    if cancel() {
        return Err(CompileIssue::Cancelled.into());
    }
    let automaton = AhoCorasickBuilder::new()
        .match_kind(MatchKind::Standard)
        .build(&bank.patterns)
        .map_err(|_| CompileError::from(CompileIssue::AutomatonBuild))?;
    if cancel() {
        return Err(CompileIssue::Cancelled.into());
    }
    Ok(Some(Bank {
        automaton,
        metas: bank.metas,
    }))
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

    fn meta(rule: &str) -> GapMeta {
        GapMeta {
            rule_id: rule.into(),
            action: RuleAction::Review,
            scenes: vec![Scene::ChannelMessage],
        }
    }

    #[test]
    fn gap_bank_enforces_the_entry_budget_as_a_closed_interval() {
        let mut bank = PatternBank {
            entries: MAX_COMPILED_ENTRIES - 1,
            ..PatternBank::default()
        };
        bank.push("测".into(), meta("1001")).unwrap();
        assert_eq!(bank.entries, MAX_COMPILED_ENTRIES);
        assert_eq!(
            bank.push("试".into(), meta("1002")).unwrap_err().issue,
            CompileIssue::CompiledEntryLimit
        );
    }

    #[test]
    fn gap_bank_enforces_the_byte_budget_as_a_closed_interval() {
        let mut bank = PatternBank {
            normalized_bytes: MAX_NORMALIZED_PATTERN_BYTES - 1,
            ..PatternBank::default()
        };
        assert_eq!(
            bank.push("测试".into(), meta("1003")).unwrap_err().issue,
            CompileIssue::NormalizedByteLimit
        );
        bank.push("a".into(), meta("1004")).unwrap();
        assert_eq!(bank.normalized_bytes, MAX_NORMALIZED_PATTERN_BYTES);
    }
}
