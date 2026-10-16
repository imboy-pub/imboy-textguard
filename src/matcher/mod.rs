//! Literal/alias matching over normalized views (plan sections 4.4-4.5).

pub mod alias;
pub mod exceptions;
pub mod gap;
pub mod literal;
pub mod regex;

use crate::model::ErrorCode;
use crate::model::{MatchMethod, RuleAction};

pub const MAX_REQUEST_CANDIDATES: usize = 20_000;

#[derive(Debug, Default)]
pub struct CandidateBudget {
    consumed: usize,
    failure: Option<ErrorCode>,
}

impl CandidateBudget {
    pub fn consumed(&self) -> usize {
        self.consumed
    }

    pub fn check(&mut self, cancel: &dyn Fn() -> bool) -> Result<(), ErrorCode> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        if cancel() {
            self.failure = Some(ErrorCode::CheckDeadline);
            return Err(ErrorCode::CheckDeadline);
        }
        Ok(())
    }

    pub fn consume(&mut self, cancel: &dyn Fn() -> bool) -> Result<(), ErrorCode> {
        self.check(cancel)?;
        if self.consumed == MAX_REQUEST_CANDIDATES {
            self.failure = Some(ErrorCode::MatchBudgetExceeded);
            return Err(ErrorCode::MatchBudgetExceeded);
        }
        self.consumed += 1;
        Ok(())
    }
}

/// One raw candidate before aggregation. Byte ranges are into the ORIGINAL
/// field text (plan 4.10), mapped through view provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawHit {
    pub rule_id: String,
    pub variant_id: String,
    pub action: RuleAction,
    pub start: u32,
    pub end: u32,
    pub methods: Vec<MatchMethod>,
}

/// Plan 4.9 aggregation: key rule/variant/start/end keeps the strongest
/// action, then the fewest transforms, then fixed order. Returns a stable,
/// deterministic order identical to the frozen oracle aggregation.
pub fn aggregate(mut hits: Vec<RawHit>) -> Vec<RawHit> {
    hits.sort_by(|left, right| {
        left.rule_id
            .cmp(&right.rule_id)
            .then_with(|| left.variant_id.cmp(&right.variant_id))
            .then_with(|| left.start.cmp(&right.start))
            .then_with(|| left.end.cmp(&right.end))
            .then_with(|| right.action.cmp(&left.action))
            .then_with(|| left.methods.len().cmp(&right.methods.len()))
            .then_with(|| methods_key(&left.methods).cmp(&methods_key(&right.methods)))
    });
    hits.dedup_by(|candidate, kept| {
        candidate.rule_id == kept.rule_id
            && candidate.variant_id == kept.variant_id
            && candidate.start == kept.start
            && candidate.end == kept.end
    });
    hits
}

fn methods_key(methods: &[MatchMethod]) -> Vec<u8> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(
        rule: &str,
        variant: &str,
        action: RuleAction,
        start: u32,
        end: u32,
        methods: &[MatchMethod],
    ) -> RawHit {
        RawHit {
            rule_id: rule.into(),
            variant_id: variant.into(),
            action,
            start,
            end,
            methods: methods.to_vec(),
        }
    }

    #[test]
    fn aggregation_keeps_strongest_action_then_fewest_transforms() {
        let hits = vec![
            hit(
                "r1",
                "canonical",
                RuleAction::Review,
                0,
                3,
                &[MatchMethod::Literal, MatchMethod::T2s, MatchMethod::Gap],
            ),
            hit(
                "r1",
                "canonical",
                RuleAction::Block,
                0,
                3,
                &[MatchMethod::Literal],
            ),
            hit(
                "r1",
                "canonical",
                RuleAction::Block,
                0,
                3,
                &[MatchMethod::Literal, MatchMethod::T2s],
            ),
        ];
        let aggregated = aggregate(hits);
        assert_eq!(aggregated.len(), 1);
        assert_eq!(aggregated[0].action, RuleAction::Block);
        assert_eq!(aggregated[0].methods, vec![MatchMethod::Literal]);
    }
}
