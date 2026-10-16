use crate::model::{valid_identifier, AliasBoundary, AliasRule};

use super::literal::{CompileError, CompileIssue, MAX_LITERAL_BYTES, MAX_LITERAL_SCALARS};

pub fn validate_alias(alias: &AliasRule) -> Result<(), CompileError> {
    if !valid_identifier(&alias.variant_id)
        || matches!(alias.variant_id.as_str(), "canonical" | "regex")
    {
        return Err(CompileIssue::InvalidVariantId.into());
    }
    if alias.pattern.is_empty() {
        return Err(CompileIssue::EmptyPattern.into());
    }
    if alias.pattern.len() > MAX_LITERAL_BYTES
        || alias.pattern.chars().count() > MAX_LITERAL_SCALARS
    {
        return Err(CompileIssue::PatternLength.into());
    }
    Ok(())
}

pub fn alias_boundary_ok(boundary: AliasBoundary, text: &str, start: usize, end: usize) -> bool {
    if start >= end || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
        return false;
    }
    if boundary == AliasBoundary::None {
        return true;
    }
    let ascii_word = |character: char| character.is_ascii_alphanumeric() || character == '_';
    !text[..start].chars().next_back().is_some_and(ascii_word)
        && !text[end..].chars().next().is_some_and(ascii_word)
}
