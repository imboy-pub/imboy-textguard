//! Local exceptions for canonical/alias literal candidates (plan 4.8).
//!
//! Exceptions apply only to literal candidates in the B/T views of one
//! field: an exemption happens when the exception phrase fully covers the
//! candidate range in the same view, and only for the exact scene, rule and
//! variant it names. Gap and regex hits are never exempted, and no phrase
//! ever exempts a whole field.

use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};
use std::collections::{HashMap, HashSet};

use super::literal::{CompileError, CompileIssue, MAX_LITERAL_BYTES, MAX_LITERAL_SCALARS};
use super::{CandidateBudget, RawHit};
use crate::model::{
    ErrorCode, ExceptionRule, MatchMethod, RuleSet, Scene, SceneSemantics, Semantics, View,
};
use crate::normalize::provenance::Unit;
use crate::normalize::{base, t2s};

pub const MAX_EXCEPTIONS: usize = 1000;
pub const MAX_VARIANTS_PER_EXCEPTION: usize = 32;

/// One compiled exception inside a view automaton.
#[derive(Debug, Clone)]
struct ExceptionMeta {
    scenes: Vec<Scene>,
    rule_id: String,
    variant_ids: Vec<String>,
}

struct Bank {
    automaton: AhoCorasick,
    entries: Vec<Vec<ExceptionMeta>>,
}

/// Frozen compilation of every exception across the B(enhanced) and T
/// views; `None` means no exception declared itself for that view. Compat-v1
/// scenes keep the legacy behavior and never apply exceptions (plan 4.2:
/// local exceptions are an enhanced-v1 capability).
pub struct CompiledExceptions {
    base: Option<Bank>,
    t2s: Option<Bank>,
    semantics: SceneSemantics,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExemptionSpan {
    start: u32,
    end: u32,
    rule_id: String,
    variant_ids: Vec<String>,
}

impl CompiledExceptions {
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
        if ruleset.exceptions.len() > MAX_EXCEPTIONS {
            return Err(CompileIssue::InvalidMetadata.into());
        }
        let variants_by_rule = literal_variants(ruleset);
        let regex_ids: HashSet<&str> = ruleset.regex_rules.iter().map(|r| r.id.as_str()).collect();
        let mut base_bank = PatternBank::default();
        let mut t2s_bank = PatternBank::default();
        let mut seen_ids = HashSet::new();
        for exception in &ruleset.exceptions {
            if cancel() {
                return Err(CompileIssue::Cancelled.into());
            }
            validate_exception(exception, &variants_by_rule, &regex_ids)?;
            if !seen_ids.insert(exception.id.as_str()) {
                return Err(CompileIssue::InvalidMetadata.into());
            }
            let meta = ExceptionMeta {
                scenes: exception.scenes.clone(),
                rule_id: exception.rule_id.clone(),
                variant_ids: exception.variant_ids.clone(),
            };
            let views = &exception.views;
            let base_units =
                normalized_phrase(&exception.phrase, base::Semantics::EnhancedV1, cancel)?;
            if views.contains(&View::B) {
                base_bank.push(view_string(&base_units), meta.clone())?;
            }
            if views.contains(&View::T) {
                let t2s_units = t2s::t2s_units(&base_units, table);
                if t2s_units.is_empty() {
                    return Err(CompileIssue::EmptyPattern.into());
                }
                t2s_bank.push(view_string(&t2s_units), meta)?;
            }
        }
        Ok(Self {
            base: build_bank(base_bank, cancel)?,
            t2s: build_bank(t2s_bank, cancel)?,
            semantics: ruleset.semantics_by_scene.clone(),
        })
    }

    /// Whether any exception exists at all (fast path for the empty default).
    pub fn is_empty(&self) -> bool {
        self.base.is_none() && self.t2s.is_none()
    }

    /// Remove literal hits exempted by local exceptions. Gap and regex hits
    /// pass through untouched: exceptions never apply to them (plan 4.8).
    /// Compat-v1 scenes never apply exceptions. Exception matching consumes
    /// the shared candidate budget.
    pub fn filter_literal_hits(
        &self,
        base_units: &[Unit],
        t2s_units: Option<&[Unit]>,
        scene: Scene,
        mut hits: Vec<RawHit>,
        budget: &mut CandidateBudget,
        cancel: &dyn Fn() -> bool,
    ) -> Result<Vec<RawHit>, ErrorCode> {
        budget.check(cancel)?;
        if self.is_empty() || hits.is_empty() {
            return Ok(hits);
        }
        if self.semantics.get(scene) == Semantics::CompatV1 {
            return Ok(hits);
        }
        let base_spans = match &self.base {
            Some(bank) => collect_spans(bank, base_units, scene, budget, cancel)?,
            None => Vec::new(),
        };
        let t2s_spans = match (t2s_units, &self.t2s) {
            (Some(units), Some(bank)) => collect_spans(bank, units, scene, budget, cancel)?,
            _ => Vec::new(),
        };
        hits.retain(|hit| {
            let spans = match literal_view(hit) {
                Some(View::B) => &base_spans,
                Some(View::T) => &t2s_spans,
                None => return true,
            };
            !covered(hit, spans)
        });
        budget.check(cancel)?;
        Ok(hits)
    }
}

/// The view a literal hit came from; `None` marks gap/regex hits which are
/// out of scope for exceptions. The method list is frozen by the matcher
/// contract so exact matching is stable.
fn literal_view(hit: &RawHit) -> Option<View> {
    match hit.methods.as_slice() {
        [MatchMethod::Literal] | [MatchMethod::Alias] => Some(View::B),
        [MatchMethod::Literal, MatchMethod::T2s] => Some(View::T),
        _ => None,
    }
}

fn covered(hit: &RawHit, spans: &[ExemptionSpan]) -> bool {
    spans.iter().any(|span| {
        span.rule_id == hit.rule_id
            && span.variant_ids.contains(&hit.variant_id)
            && span.start <= hit.start
            && hit.end <= span.end
    })
}

fn collect_spans(
    bank: &Bank,
    units: &[Unit],
    scene: Scene,
    budget: &mut CandidateBudget,
    cancel: &dyn Fn() -> bool,
) -> Result<Vec<ExemptionSpan>, ErrorCode> {
    budget.check(cancel)?;
    let text = view_string(units);
    let mut offsets = Vec::with_capacity(units.len() + 1);
    let mut cursor = 0u32;
    for unit in units {
        offsets.push(cursor);
        cursor += unit.scalar.len_utf8() as u32;
    }
    offsets.push(cursor);
    let mut spans = Vec::new();
    for matched in bank.automaton.find_overlapping_iter(&text) {
        let entries = &bank.entries[matched.pattern()];
        for entry in entries {
            budget.consume(cancel)?;
            if !entry.scenes.contains(&scene) {
                continue;
            }
            let first = unit_index_for_byte(&offsets, matched.start() as u32);
            let last = unit_index_for_byte(&offsets, matched.end() as u32 - 1);
            spans.push(ExemptionSpan {
                start: units[first].start,
                end: units[last].end,
                rule_id: entry.rule_id.clone(),
                variant_ids: entry.variant_ids.clone(),
            });
        }
    }
    budget.check(cancel)?;
    Ok(spans)
}

fn unit_index_for_byte(offsets: &[u32], byte: u32) -> usize {
    offsets
        .binary_search(&byte)
        .unwrap_or_else(|index| index.saturating_sub(1))
}

/// rule_id -> legal variant ids ("canonical" plus every declared alias).
/// Regex rule ids are recorded separately so an exception pointing at a
/// regex rule (or a dangling id) is rejected at publish time (AC-039).
fn literal_variants(ruleset: &RuleSet) -> HashMap<&str, Vec<&str>> {
    let mut map = HashMap::with_capacity(ruleset.literals.len());
    for rule in &ruleset.literals {
        let mut variants = vec!["canonical"];
        for alias in &rule.aliases {
            variants.push(alias.variant_id.as_str());
        }
        map.insert(rule.id.as_str(), variants);
    }
    map
}

fn validate_exception(
    exception: &ExceptionRule,
    variants_by_rule: &HashMap<&str, Vec<&str>>,
    regex_ids: &HashSet<&str>,
) -> Result<(), CompileError> {
    if !crate::model::valid_identifier(&exception.id)
        || exception.scenes.is_empty()
        || exception.views.is_empty()
    {
        return Err(CompileIssue::InvalidMetadata.into());
    }
    if exception.phrase.is_empty() {
        return Err(CompileIssue::EmptyPattern.into());
    }
    if exception.phrase.chars().count() > MAX_LITERAL_SCALARS
        || exception.phrase.len() > MAX_LITERAL_BYTES
    {
        return Err(CompileIssue::PatternLength.into());
    }
    if regex_ids.contains(exception.rule_id.as_str()) {
        return Err(CompileIssue::InvalidVariantId.into());
    }
    let variants = match variants_by_rule.get(exception.rule_id.as_str()) {
        Some(variants) => variants,
        None => return Err(CompileIssue::InvalidVariantId.into()),
    };
    if exception.variant_ids.len() > MAX_VARIANTS_PER_EXCEPTION {
        return Err(CompileIssue::InvalidMetadata.into());
    }
    for variant in &exception.variant_ids {
        if !variants.contains(&variant.as_str()) {
            return Err(CompileIssue::InvalidVariantId.into());
        }
    }
    Ok(())
}

fn normalized_phrase(
    phrase: &str,
    semantics: base::Semantics,
    cancel: &dyn Fn() -> bool,
) -> Result<Vec<Unit>, CompileError> {
    let units = base::normalize_str(phrase, semantics, cancel).map_err(|error| {
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

#[derive(Default)]
struct PatternBank {
    indices: HashMap<String, usize>,
    patterns: Vec<String>,
    entries: Vec<Vec<ExceptionMeta>>,
}

impl PatternBank {
    fn push(&mut self, view: String, meta: ExceptionMeta) -> Result<(), CompileError> {
        if view.is_empty() {
            return Err(CompileIssue::EmptyPattern.into());
        }
        if let Some(&index) = self.indices.get(&view) {
            if self.entries[index].len() == super::literal::MAX_METADATA_FANOUT {
                return Err(CompileIssue::MetadataFanout.into());
            }
            self.entries[index].push(meta);
        } else {
            self.indices.insert(view.clone(), self.patterns.len());
            self.patterns.push(view);
            self.entries.push(vec![meta]);
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
        entries: bank.entries,
    }))
}
