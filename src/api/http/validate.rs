//! Strict request parsing (AC-052/AC-053).
//!
//! `serde_json` keeps the last duplicate key by default, so the body is first
//! walked through [`StrictValue`] — a shape-only tree that preserves every
//! object entry — and rejected when any object (top-level or nested inside a
//! `fields` entry) contains a duplicate key. Only then is the body
//! deserialized into the typed [`CheckRequest`], whose
//! `deny_unknown_fields`/enum/`String` definitions reject unknown fields,
//! unknown scenes and implicit (`null`/missing) text. The final
//! `CheckRequest::validate` pass enforces the identity, field-count and byte
//! limits.

use super::error::ApiError;
use crate::model::{CheckRequest, ErrorCode};
use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use std::collections::HashSet;
use std::fmt;

/// A JSON document reduced to what duplicate detection needs: object entry
/// order and nesting. Scalar values are not inspected.
enum StrictValue {
    Scalar,
    Array(Vec<StrictValue>),
    Object(Vec<(String, StrictValue)>),
}

struct StrictVisitor;

impl<'de> Visitor<'de> for StrictVisitor {
    type Value = StrictValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("any JSON value")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(StrictValue::Scalar)
    }

    fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E> {
        Ok(StrictValue::Scalar)
    }

    fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E> {
        Ok(StrictValue::Scalar)
    }

    fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E> {
        Ok(StrictValue::Scalar)
    }

    fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E> {
        Ok(StrictValue::Scalar)
    }

    fn visit_str<E>(self, _value: &str) -> Result<Self::Value, E> {
        Ok(StrictValue::Scalar)
    }

    fn visit_string<E>(self, _value: String) -> Result<Self::Value, E> {
        Ok(StrictValue::Scalar)
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element::<StrictValue>()? {
            items.push(item);
        }
        Ok(StrictValue::Array(items))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut entries = Vec::new();
        while let Some((key, value)) = map.next_entry::<String, StrictValue>()? {
            entries.push((key, value));
        }
        Ok(StrictValue::Object(entries))
    }
}

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictVisitor)
    }
}

fn has_duplicate_keys(value: &StrictValue) -> bool {
    match value {
        StrictValue::Object(entries) => {
            let mut seen = HashSet::with_capacity(entries.len());
            entries
                .iter()
                .any(|(key, inner)| !seen.insert(key.as_str()) || has_duplicate_keys(inner))
        }
        StrictValue::Array(items) => items.iter().any(has_duplicate_keys),
        StrictValue::Scalar => false,
    }
}

/// Parse and fully validate a check request body (already size-bounded).
pub(super) fn parse_check_request(bytes: &[u8]) -> Result<CheckRequest, ApiError> {
    let text = std::str::from_utf8(bytes).map_err(|_| ApiError::new(ErrorCode::InvalidUtf8))?;
    let strict = serde_json::from_str::<StrictValue>(text)
        .map_err(|_| ApiError::new(ErrorCode::InvalidArgument))?;
    if has_duplicate_keys(&strict) {
        return Err(ApiError::new(ErrorCode::InvalidArgument));
    }
    let request = serde_json::from_str::<CheckRequest>(text)
        .map_err(|_| ApiError::new(ErrorCode::InvalidArgument))?;
    request.validate().map_err(ApiError::new)?;
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_keys_anywhere_are_rejected_but_last_value_would_parse() {
        for input in [
            r#"{"request_id":"a","request_id":"a","scene":"channel_message","fields":[{"field_id":"f","text":""}]}"#,
            r#"{"request_id":"a","scene":"channel_message","scene":"profile_field","fields":[{"field_id":"f","text":""}]}"#,
            r#"{"request_id":"a","scene":"channel_message","fields":[{"field_id":"f","field_id":"f","text":""}]}"#,
            r#"{"request_id":"a","scene":"channel_message","fields":[{"field_id":"f","text":"","text":""}]}"#,
        ] {
            // serde_json's untyped Value keeps the last duplicate and accepts the
            // document; the explicit scan is what rejects it
            assert!(serde_json::from_str::<serde_json::Value>(input).is_ok());
            let error = parse_check_request(input.as_bytes()).unwrap_err();
            assert_eq!(error.code(), ErrorCode::InvalidArgument);
        }
    }

    #[test]
    fn non_object_documents_and_invalid_utf8_fail() {
        assert!(parse_check_request(b"[1,2]").is_err());
        assert!(parse_check_request(b"null").is_err());
        assert_eq!(
            parse_check_request(b"\xff\xfe{}").unwrap_err().code(),
            ErrorCode::InvalidUtf8
        );
    }
}
