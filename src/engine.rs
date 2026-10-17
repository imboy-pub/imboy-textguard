//! Shared check engine over one frozen rule snapshot (plan sections 4.9,
//! 4.11 and T11). Every request — HTTP or gRPC — runs against a single
//! immutable snapshot: one revision, one generation, one candidate budget
//! across all fields and B/T/G/exception matching; regex first witnesses
//! are independently bounded (<=32 rules x fields) and share cancellation.

use std::collections::HashMap;

use sha2::{Digest, Sha256};

use crate::matcher::exceptions::CompiledExceptions;
use crate::matcher::gap::CompiledGaps;
use crate::matcher::literal::{CompileError, CompiledLiterals};
use crate::matcher::regex::CompiledRegex;
use crate::matcher::{aggregate, CandidateBudget, RawHit};
use crate::model::{
    CheckRequest, CheckResponse, ErrorCode, Hit, RuleSet, Scene, SceneSemantics, Semantics,
};
use crate::normalize::provenance::Unit;
use crate::normalize::{base, t2s};
use crate::policy::{sort_response, strongest_action, to_hit, HitMetadata, MAX_RESPONSE_HITS};

/// Metadata index: rule_id -> variant_id -> witness metadata.
type MetadataIndex = HashMap<String, HashMap<String, HitMetadata>>;

/// Build-time failure combining the literal/gap/exception compiler errors
/// with the regex compiler error code.
#[derive(Debug)]
pub enum BuildError {
    Matchers(CompileError),
    Regex(ErrorCode),
}

impl From<CompileError> for BuildError {
    fn from(error: CompileError) -> Self {
        Self::Matchers(error)
    }
}

impl From<ErrorCode> for BuildError {
    fn from(error: ErrorCode) -> Self {
        Self::Regex(error)
    }
}

/// One immutable rule snapshot. Cloning is cheap (compile once, share via
/// Arc in the service layer); requests borrow the snapshot for their whole
/// lifetime, which is the AC-050 single-snapshot guarantee.
pub struct Snapshot {
    literals: CompiledLiterals,
    gaps: CompiledGaps,
    exceptions: CompiledExceptions,
    regex: CompiledRegex,
    table: &'static HashMap<char, char>,
    semantics: SceneSemantics,
    metadata: MetadataIndex,
    revision: String,
    generation: String,
    digest: String,
    t2s_table_id: String,
}

impl Snapshot {
    pub fn build(
        ruleset: &RuleSet,
        table: &'static HashMap<char, char>,
    ) -> Result<Self, BuildError> {
        let literals = CompiledLiterals::compile(ruleset, table)?;
        let gaps = CompiledGaps::compile(ruleset, table)?;
        let exceptions = CompiledExceptions::compile(ruleset, table)?;
        let regex = CompiledRegex::compile(&ruleset.regex_rules, &|| false)?;
        let digest = ruleset_digest(ruleset);
        Ok(Self {
            literals,
            gaps,
            exceptions,
            regex,
            table,
            semantics: ruleset.semantics_by_scene.clone(),
            metadata: metadata_index(ruleset),
            revision: digest,
            generation: "0".to_owned(),
            digest: base_rules_digest(ruleset),
            t2s_table_id: ruleset.t2s_table_id.clone(),
        })
    }

    pub fn revision(&self) -> &str {
        &self.revision
    }

    pub fn generation(&self) -> &str {
        &self.generation
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// Override the placeholder generation ("0") with the publication
    /// generation (T14). The publication layer owns this value; the engine
    /// itself never assigns or interprets it.
    pub fn with_generation(mut self, generation: &str) -> Self {
        self.generation = generation.to_owned();
        self
    }

    pub fn with_publication(mut self, revision: &str, generation: u64) -> Self {
        self.revision = revision.to_owned();
        self.generation = generation.to_string();
        self
    }

    /// Run the complete frozen pipeline for one request against this
    /// snapshot. Field order follows the request; the same budget covers
    /// literal, exception and gap candidates across every field.
    pub fn check(
        &self,
        request: &CheckRequest,
        cancel: &dyn Fn() -> bool,
    ) -> Result<CheckResponse, ErrorCode> {
        request.validate()?;
        let scene = request.scene;
        let semantics = match self.semantics.get(scene) {
            Semantics::CompatV1 => base::Semantics::CompatV1,
            Semantics::EnhancedV1 => base::Semantics::EnhancedV1,
        };
        let mut budget = CandidateBudget::default();
        let mut witnesses: Vec<(usize, Hit)> = Vec::new();
        for (field_index, field) in request.fields.iter().enumerate() {
            budget.check(cancel)?;
            let units =
                base::normalize_str(&field.text, semantics, cancel).map_err(normalization_error)?;
            let translated = self.t2s_view(&units, semantics);
            let mut hits = self.literal_hits(&units, scene, &translated, &mut budget, cancel)?;
            hits = self.exceptions.filter_literal_hits(
                &units,
                translated.as_deref(),
                scene,
                hits,
                &mut budget,
                cancel,
            )?;
            hits.extend(self.gaps.scan_base(&units, scene, &mut budget, cancel)?);
            if let Some(translated) = &translated {
                hits.extend(self.gaps.scan_t2s(translated, scene, &mut budget, cancel)?);
            }
            for raw in aggregate(hits) {
                if let Some(hit) = self.witness(&raw, &field.field_id) {
                    witnesses.push((field_index, hit));
                }
            }
            if semantics == base::Semantics::EnhancedV1 {
                let regex_hits = self.regex.scan(scene, &field.field_id, &units, cancel)?;
                for hit in regex_hits {
                    witnesses.push((field_index, hit));
                }
            }
        }
        budget.check(cancel)?;
        let witness_refs: Vec<&Hit> = witnesses.iter().map(|(_, hit)| hit).collect();
        let decision = strongest_action(&witness_refs);
        let hits_truncated = witnesses.len() > MAX_RESPONSE_HITS;
        sort_response(&mut witnesses);
        witnesses.truncate(MAX_RESPONSE_HITS);
        Ok(CheckResponse {
            request_id: request.request_id.clone(),
            ruleset_revision: self.revision.clone(),
            generation: self.generation.clone(),
            normalization_id: crate::NORMALIZATION_ID.to_owned(),
            t2s_table_id: self.t2s_table_id.clone(),
            semantics_id: semantics_id(semantics),
            base_rules_digest: self.digest.clone(),
            decision,
            scan_complete: true,
            hits_truncated,
            hits: witnesses.into_iter().map(|(_, hit)| hit).collect(),
            fields_checked: request.fields.len() as u32,
            hit_semantics: crate::HIT_SEMANTICS.to_owned(),
            build_id: crate::BUILD_ID.to_owned(),
        })
    }

    /// Raw unaggregated literal evidence for both views so the exception
    /// filter keeps every candidate's view identity (plan 4.8/4.9).
    fn literal_hits(
        &self,
        units: &[Unit],
        scene: Scene,
        translated: &Option<Vec<Unit>>,
        budget: &mut CandidateBudget,
        cancel: &dyn Fn() -> bool,
    ) -> Result<Vec<RawHit>, ErrorCode> {
        let mut hits = self.literals.scan_base(units, scene, budget, cancel)?;
        if let Some(translated) = translated {
            hits.extend(self.literals.scan_t2s(translated, scene, budget, cancel)?);
        }
        Ok(hits)
    }

    fn t2s_view(&self, units: &[Unit], semantics: base::Semantics) -> Option<Vec<Unit>> {
        (semantics == base::Semantics::EnhancedV1).then(|| t2s::t2s_units(units, self.table))
    }

    /// Attach frozen metadata; unknown rule/variant pairs (regex variants)
    /// pass through the regex Hit directly and are not looked up here.
    fn witness(&self, raw: &RawHit, field_id: &str) -> Option<Hit> {
        let meta = self
            .metadata
            .get(&raw.rule_id)
            .and_then(|variants| variants.get(&raw.variant_id));
        debug_assert!(
            meta.is_some(),
            "raw hit without frozen metadata: rule {} variant {}",
            raw.rule_id,
            raw.variant_id
        );
        meta.map(|meta| to_hit(raw, field_id, meta))
    }
}

fn semantics_id(semantics: base::Semantics) -> String {
    match semantics {
        base::Semantics::CompatV1 => "compat-v1".to_owned(),
        base::Semantics::EnhancedV1 => "enhanced-v1".to_owned(),
    }
}

fn metadata_index(ruleset: &RuleSet) -> MetadataIndex {
    let mut index = MetadataIndex::with_capacity(ruleset.literals.len());
    for rule in &ruleset.literals {
        let mut variants = HashMap::with_capacity(rule.aliases.len() + 1);
        variants.insert(
            "canonical".to_owned(),
            HitMetadata {
                variant_type: crate::model::VariantType::Canonical,
                word: rule.word.clone(),
                category: rule.category.clone(),
                severity: rule.severity,
            },
        );
        for alias in &rule.aliases {
            variants.insert(
                alias.variant_id.clone(),
                HitMetadata {
                    variant_type: crate::model::VariantType::from(alias.variant_type),
                    word: alias.pattern.clone(),
                    category: rule.category.clone(),
                    severity: rule.severity,
                },
            );
        }
        index.insert(rule.id.clone(), variants);
    }
    index
}

/// Stable digest of the ruleset content (canonical JSON serialization).
/// The publication layer (T14) derives ETag/generation from the same bytes.
fn ruleset_digest(ruleset: &RuleSet) -> String {
    let canonical = serde_json::to_vec(ruleset).expect("ruleset serialization cannot fail");
    format!("{:x}", Sha256::digest(canonical))
}

pub fn base_rules_digest(ruleset: &RuleSet) -> String {
    let mut rows: Vec<_> = ruleset
        .literals
        .iter()
        .map(|rule| {
            let severity = match rule.severity {
                crate::model::Severity::Low => "low",
                crate::model::Severity::Medium => "medium",
                crate::model::Severity::High => "high",
            };
            (
                rule.word.as_bytes(),
                rule.category.as_bytes(),
                severity.as_bytes(),
            )
        })
        .collect();
    rows.sort_unstable();
    let mut digest = Sha256::new();
    digest.update(b"TGBASEv1\0");
    digest.update((rows.len() as u32).to_be_bytes());
    for (word, category, severity) in rows {
        for field in [word, category, severity] {
            digest.update((field.len() as u32).to_be_bytes());
            digest.update(field);
        }
    }
    format!("{:x}", digest.finalize())
}

fn normalization_error(error: base::NormalizationError) -> ErrorCode {
    match error {
        base::NormalizationError::InvalidUtf8 => ErrorCode::InvalidUtf8,
        base::NormalizationError::LimitExceeded => ErrorCode::NormalizationLimit,
        base::NormalizationError::NormalizerIncompatible => ErrorCode::NormalizerIncompatible,
        base::NormalizationError::Cancelled => ErrorCode::CheckDeadline,
    }
}
