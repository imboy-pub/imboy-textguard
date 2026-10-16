use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scene {
    ChannelMessage,
    MomentPost,
    ProfileField,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Allow,
    Review,
    Block,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleAction {
    Review,
    Block,
}

impl From<RuleAction> for Action {
    fn from(value: RuleAction) -> Self {
        match value {
            RuleAction::Review => Self::Review,
            RuleAction::Block => Self::Block,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Low,
    Medium,
    High,
}

impl Severity {
    pub fn canonical_action(self) -> RuleAction {
        match self {
            Self::High => RuleAction::Block,
            Self::Low | Self::Medium => RuleAction::Review,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchMethod {
    Literal,
    T2s,
    Gap,
    Alias,
    Regex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VariantType {
    Canonical,
    Pinyin,
    Homophone,
    Lookalike,
    Decomposed,
    RegexRule,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextField {
    pub field_id: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckRequest {
    pub request_id: String,
    pub scene: Scene,
    pub fields: Vec<TextField>,
}

impl CheckRequest {
    pub fn validate(&self) -> Result<(), ErrorCode> {
        if !valid_identifier(&self.request_id) || self.fields.is_empty() || self.fields.len() > 8 {
            return Err(ErrorCode::InvalidArgument);
        }
        let mut identities = HashSet::new();
        let mut total = 0;
        for field in &self.fields {
            if field.field_id.is_empty()
                || field.field_id.len() > 64
                || !field
                    .field_id
                    .bytes()
                    .all(|byte| (32..=126).contains(&byte))
                || !identities.insert(&field.field_id)
            {
                return Err(ErrorCode::InvalidArgument);
            }
            total += field.text.len();
            if field.text.len() > 16 * 1024 || total > 32 * 1024 {
                return Err(ErrorCode::TextTooLarge);
            }
        }
        Ok(())
    }
}

pub fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Semantics {
    CompatV1,
    EnhancedV1,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneSemantics {
    pub channel_message: Semantics,
    pub moment_post: Semantics,
    pub profile_field: Semantics,
}

impl SceneSemantics {
    pub fn get(&self, scene: Scene) -> Semantics {
        match scene {
            Scene::ChannelMessage => self.channel_message,
            Scene::MomentPost => self.moment_post,
            Scene::ProfileField => self.profile_field,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleSet {
    pub schema_version: u32,
    pub normalization_id: String,
    pub t2s_table_id: String,
    pub limit_profile_id: String,
    pub semantics_by_scene: SceneSemantics,
    pub allow_empty: bool,
    pub literals: Vec<LiteralRule>,
    pub regex_rules: Vec<RegexRule>,
    pub exceptions: Vec<ExceptionRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiteralRule {
    pub id: String,
    pub word: String,
    pub category: String,
    pub severity: Severity,
    pub scenes: Vec<Scene>,
    pub aliases: Vec<AliasRule>,
    pub gap: GapConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AliasRule {
    pub variant_id: String,
    #[serde(rename = "type")]
    pub variant_type: AliasType,
    pub pattern: String,
    pub action: RuleAction,
    pub boundary: AliasBoundary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AliasType {
    Pinyin,
    Homophone,
    Lookalike,
    Decomposed,
}

impl From<AliasType> for VariantType {
    fn from(value: AliasType) -> Self {
        match value {
            AliasType::Pinyin => Self::Pinyin,
            AliasType::Homophone => Self::Homophone,
            AliasType::Lookalike => Self::Lookalike,
            AliasType::Decomposed => Self::Decomposed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AliasBoundary {
    AsciiWord,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GapConfig {
    pub enabled: bool,
    pub action: RuleAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegexRule {
    pub id: String,
    pub expression: String,
    pub display_word: String,
    pub category: String,
    pub severity: Severity,
    pub action: RuleAction,
    pub scenes: Vec<Scene>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum View {
    B,
    T,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExceptionRule {
    pub id: String,
    pub phrase: String,
    pub scenes: Vec<Scene>,
    pub rule_id: String,
    pub variant_ids: Vec<String>,
    pub views: Vec<View>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hit {
    pub field_id: String,
    pub rule_id: String,
    pub variant_id: String,
    pub variant_type: VariantType,
    pub word: String,
    pub category: String,
    pub severity: Severity,
    pub effective_action: RuleAction,
    pub start_byte: u32,
    pub end_byte: u32,
    pub match_methods: Vec<MatchMethod>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckResponse {
    pub request_id: String,
    pub ruleset_revision: String,
    pub generation: String,
    pub normalization_id: String,
    pub t2s_table_id: String,
    pub semantics_id: String,
    pub base_rules_digest: String,
    pub decision: Action,
    pub scan_complete: bool,
    pub hits_truncated: bool,
    pub hits: Vec<Hit>,
    pub fields_checked: u32,
    pub hit_semantics: String,
    pub build_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    InvalidArgument,
    InvalidUtf8,
    TextTooLarge,
    NormalizationLimit,
    MatchBudgetExceeded,
    Overloaded,
    NotReady,
    ConnectionFailed,
    CheckDeadline,
    Unauthenticated,
    PermissionDenied,
    ProtocolError,
    NormalizerIncompatible,
    InternalError,
    UnsupportedRuleset,
    CasConflict,
    InvalidRuleset,
    PublishDeadline,
    PublishStateUnknown,
    PublishBusy,
    RetiredSnapshotBusy,
}
