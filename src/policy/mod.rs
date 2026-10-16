//! Decision aggregation, response ordering and bounded witness truncation
//! (plan section 4.9). The policy layer never turns an error into a
//! success and never lets severity override the frozen effective_action.

use crate::matcher::RawHit;
use crate::model::{Action, Hit, MatchMethod};

/// Maximum witnesses returned in one response; the decision is always
/// computed from the complete evidence before truncation.
pub const MAX_RESPONSE_HITS: usize = 100;

pub fn strongest_action(hits: &[&Hit]) -> Action {
    hits.iter()
        .map(|hit| Action::from(hit.effective_action))
        .fold(Action::Allow, Ord::max)
}

/// Convert one aggregated raw hit into a response witness using the frozen
/// rule metadata. The raw action is the effective action; severity is
/// reported separately and never re-decides the outcome (AC-047).
pub fn to_hit(raw: &RawHit, field_id: &str, meta: &HitMetadata) -> Hit {
    Hit {
        field_id: field_id.to_owned(),
        rule_id: raw.rule_id.clone(),
        variant_id: raw.variant_id.clone(),
        variant_type: meta.variant_type,
        word: meta.word.clone(),
        category: meta.category.clone(),
        severity: meta.severity,
        effective_action: raw.action,
        start_byte: raw.start,
        end_byte: raw.end,
        match_methods: raw.methods.clone(),
    }
}

/// Response ordering (plan 4.9): action descending, request field order,
/// start ascending, end descending, rule/variant byte order, method order.
pub fn sort_response(hits: &mut [(usize, Hit)]) {
    hits.sort_by(|(left_field, left), (right_field, right)| {
        right
            .effective_action
            .cmp(&left.effective_action)
            .then_with(|| left_field.cmp(right_field))
            .then_with(|| left.start_byte.cmp(&right.start_byte))
            .then_with(|| right.end_byte.cmp(&left.end_byte))
            .then_with(|| left.rule_id.cmp(&right.rule_id))
            .then_with(|| left.variant_id.cmp(&right.variant_id))
            .then_with(|| {
                methods_rank(&left.match_methods).cmp(&methods_rank(&right.match_methods))
            })
    });
}

/// Frozen method ordering for the final tie-break (matcher contract).
fn methods_rank(methods: &[MatchMethod]) -> Vec<u8> {
    methods
        .iter()
        .map(|method| match method {
            MatchMethod::Literal => 1,
            MatchMethod::T2s => 2,
            MatchMethod::Gap => 3,
            MatchMethod::Alias => 4,
            MatchMethod::Regex => 5,
        })
        .collect()
}

/// Metadata attached to every rule/variant pair at snapshot build time.
#[derive(Debug, Clone)]
pub struct HitMetadata {
    pub variant_type: crate::model::VariantType,
    pub word: String,
    pub category: String,
    pub severity: crate::model::Severity,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{MatchMethod, RuleAction, Severity, VariantType};

    fn witness(action: RuleAction, field: usize, start: u32, end: u32, rule: &str) -> (usize, Hit) {
        (
            field,
            Hit {
                field_id: format!("f{field}"),
                rule_id: rule.into(),
                variant_id: "canonical".into(),
                variant_type: VariantType::Canonical,
                word: "词".into(),
                category: "test".into(),
                severity: Severity::High,
                effective_action: action,
                start_byte: start,
                end_byte: end,
                match_methods: vec![MatchMethod::Literal],
            },
        )
    }

    #[test]
    fn strongest_action_covers_block_review_allow() {
        assert_eq!(strongest_action(&[]), Action::Allow);
        let review = witness(RuleAction::Review, 0, 0, 3, "1");
        assert_eq!(strongest_action(&[&review.1]), Action::Review);
        let mixed = [
            witness(RuleAction::Review, 0, 0, 3, "1"),
            witness(RuleAction::Block, 1, 0, 3, "2"),
        ];
        let refs: Vec<&Hit> = mixed.iter().map(|(_, hit)| hit).collect();
        assert_eq!(strongest_action(&refs), Action::Block);
    }

    #[test]
    fn response_order_is_action_then_field_then_position() {
        let mut hits = vec![
            witness(RuleAction::Review, 0, 6, 9, "1004"),
            witness(RuleAction::Block, 1, 0, 3, "1002"),
            witness(RuleAction::Review, 0, 0, 3, "1003"),
            witness(RuleAction::Review, 1, 0, 3, "1001"),
        ];
        sort_response(&mut hits);
        let keys: Vec<_> = hits
            .iter()
            .map(|(field, hit)| (hit.effective_action, *field, hit.rule_id.as_str()))
            .collect();
        assert_eq!(
            keys,
            vec![
                (RuleAction::Block, 1, "1002"),
                (RuleAction::Review, 0, "1003"),
                (RuleAction::Review, 0, "1004"),
                (RuleAction::Review, 1, "1001"),
            ]
        );
    }

    #[test]
    fn start_ascending_then_end_descending() {
        let mut hits = vec![
            witness(RuleAction::Review, 0, 0, 9, "1001"),
            witness(RuleAction::Review, 0, 0, 3, "1001"),
            witness(RuleAction::Review, 0, 3, 6, "1001"),
        ];
        sort_response(&mut hits);
        let spans: Vec<_> = hits
            .iter()
            .map(|(_, hit)| (hit.start_byte, hit.end_byte))
            .collect();
        assert_eq!(spans, vec![(0, 9), (0, 3), (3, 6)]);
    }
}
