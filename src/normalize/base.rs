//! Base view B construction with the two frozen lowercase flavors.

use std::fmt;

use super::provenance::{lowercase_pieces, Unit};

/// Frozen semantics modes (plan 4.2). `CompatV1` reproduces the historical
/// IMBoy normalization exactly: per-character full lowercase (Erlang
/// `string:lowercase/1`, no Final_Sigma context), then fullwidth folding,
/// then zero-width removal. `EnhancedV1` uses the full-string lowercase of
/// `imboy-base-v1` (context-aware, including Final_Sigma).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Semantics {
    CompatV1,
    EnhancedV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalizationError {
    /// Input bytes are not valid UTF-8; the contract rejects instead of
    /// silently repairing (the old Erlang code dropped trailing broken
    /// bytes - an approved, documented convergence).
    InvalidUtf8,
    /// View output exceeds the frozen budget; the scan never continues on a
    /// truncated normalization (plan 4.9 / section 5).
    LimitExceeded,
    /// Source correspondence could not be proven (plan 4.3 item 3).
    NormalizerIncompatible,
    /// Cancellation requested between per-character steps (plan section 5).
    Cancelled,
}

impl fmt::Display for NormalizationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::InvalidUtf8 => "INVALID_UTF8",
            Self::LimitExceeded => "NORMALIZATION_LIMIT",
            Self::NormalizerIncompatible => "NORMALIZER_INCOMPATIBLE",
            Self::Cancelled => "CHECK_DEADLINE",
        };
        f.write_str(name)
    }
}

/// Per-field view output budget (section 5): 48 KiB of view bytes.
pub const MAX_VIEW_BYTES_PER_FIELD: usize = 48 * 1024;

const ZERO_WIDTH: [char; 4] = ['\u{200b}', '\u{200c}', '\u{200d}', '\u{feff}'];

/// Validate raw bytes and build the view for the requested semantics.
///
/// `cancel` is polled once per original character so a deadline can stop the
/// traversal between normalization steps (plan section 5); started work is
/// not silently finished after cancellation.
pub fn normalize_bytes(
    bytes: &[u8],
    semantics: Semantics,
    cancel: &dyn Fn() -> bool,
) -> Result<Vec<Unit>, NormalizationError> {
    let text = std::str::from_utf8(bytes).map_err(|_| NormalizationError::InvalidUtf8)?;
    normalize_str(text, semantics, cancel)
}

/// Build the view of an already validated `&str`.
pub fn normalize_str(
    text: &str,
    semantics: Semantics,
    cancel: &dyn Fn() -> bool,
) -> Result<Vec<Unit>, NormalizationError> {
    let lowered = match semantics {
        Semantics::CompatV1 => per_character_lowercase(text),
        Semantics::EnhancedV1 => text.to_lowercase(),
    };
    let pieces = lowercase_pieces(text);
    let units = assemble(&pieces, &lowered, semantics, cancel)?;
    let view_bytes: usize = units.iter().map(|unit| unit.scalar.len_utf8()).sum();
    if view_bytes > MAX_VIEW_BYTES_PER_FIELD {
        return Err(NormalizationError::LimitExceeded);
    }
    Ok(units)
}

/// Erlang `string:lowercase/1` flavor: per-character full mapping with no
/// context rules. This is the frozen historical behavior (compat-v1).
fn per_character_lowercase(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        for mapped in character.to_lowercase() {
            out.push(mapped);
        }
    }
    out
}

fn assemble(
    pieces: &[super::provenance::Piece],
    lowered: &str,
    semantics: Semantics,
    cancel: &dyn Fn() -> bool,
) -> Result<Vec<Unit>, NormalizationError> {
    let lowered_chars: Vec<char> = lowered.chars().collect();
    let mut expected_len = 0usize;
    for piece in pieces {
        expected_len += piece.lowered.chars().count();
    }
    if expected_len != lowered_chars.len() {
        // A context rule would have to change output length; none exists in
        // the frozen Unicode behavior, so this is an unproven correspondence.
        return Err(NormalizationError::NormalizerIncompatible);
    }
    let mut units = Vec::with_capacity(lowered_chars.len());
    let mut position = 0usize;
    for piece in pieces {
        if cancel() {
            return Err(NormalizationError::Cancelled);
        }
        let width = piece.lowered.chars().count();
        let segment = &lowered_chars[position..position + width];
        let mut segment_iter = segment.iter().copied();
        for expected in piece.lowered.chars() {
            let actual = segment_iter.next().expect("lengths verified above");
            if actual != expected {
                // Final_Sigma is the single 1:1 context divergence of the
                // full-string lowercase; the full-string output stays
                // authoritative. The exception is the exact triple
                // (original Sigma, per-char sigma, full-string final sigma)
                // and only under enhanced semantics. Anything else refuses.
                if !(semantics == Semantics::EnhancedV1
                    && piece.original == '\u{3a3}'
                    && expected == '\u{3c3}'
                    && actual == '\u{3c2}')
                {
                    return Err(NormalizationError::NormalizerIncompatible);
                }
            }
        }
        for actual in segment {
            units.push(Unit::new(*actual, piece.start, piece.end));
        }
        position += width;
    }
    debug_assert_eq!(position, lowered_chars.len());
    apply_folding(units)
}

/// Fullwidth folding (U+FF01..U+FF5E -> halfwidth) then zero-width removal,
/// identical in both semantics flavors.
fn apply_folding(mut units: Vec<Unit>) -> Result<Vec<Unit>, NormalizationError> {
    for unit in units.iter_mut() {
        let point = unit.scalar as u32;
        if (0xFF01..=0xFF5E).contains(&point) {
            unit.scalar = char::from_u32(point - 0xFEE0).expect("checked range");
        }
    }
    units.retain(|unit| !ZERO_WIDTH.contains(&unit.scalar));
    Ok(units)
}

/// View text as a plain string (for pattern building and debug checks).
pub fn units_text(units: &[Unit]) -> String {
    units.iter().map(|unit| unit.scalar).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NO_CANCEL: fn() -> bool = || false;

    fn view(text: &str, semantics: Semantics) -> String {
        units_text(&normalize_str(text, semantics, &NO_CANCEL).unwrap())
    }

    #[test]
    fn ac014_lowercase_and_fullwidth_hit_in_view() {
        assert_eq!(view("ABC", Semantics::EnhancedV1), "abc");
        assert_eq!(view("ＡＢＣ", Semantics::EnhancedV1), "abc");
        assert_eq!(view("ＡbＣ", Semantics::CompatV1), "abc");
        // the original text is never modified by the caller of this API
        let original = "ＡＢＣ";
        let _ = view(original, Semantics::EnhancedV1);
        assert_eq!(original, "ＡＢＣ");
    }

    #[test]
    fn ac015_only_the_frozen_zero_widths_are_removed() {
        assert_eq!(view("测\u{200b}试词", Semantics::EnhancedV1), "测试词");
        assert_eq!(view("测\u{feff}试词", Semantics::CompatV1), "测试词");
        // ZWJ inside an emoji sequence is a frozen zero-width and is removed
        assert_eq!(view("👨‍💻", Semantics::EnhancedV1), "👨💻");
        // other invisible-ish characters are NOT deleted
        assert_eq!(view("a\u{00a0}b", Semantics::EnhancedV1), "a\u{a0}b");
        assert_eq!(view("a\u{3000}b", Semantics::EnhancedV1), "a\u{3000}b");
        assert_eq!(view("a\u{200e}b", Semantics::EnhancedV1), "a\u{200e}b");
    }

    #[test]
    fn ac016_special_casing_vectors_match_frozen_probe_outputs() {
        // Frozen probe vectors (OTP 29 / ERTS 17.0.2 vs Rust 1.90 full
        // lowercase). Compat reproduces the OTP bytes; enhanced matches the
        // full-string reference including Final_Sigma.
        let cases: &[(&str, Semantics, &str)] = &[
            ("İ", Semantics::CompatV1, "i\u{307}"),
            ("\u{39f}\u{3a3}", Semantics::CompatV1, "\u{3bf}\u{3c3}"),
            ("\u{39f}\u{3a3}", Semantics::EnhancedV1, "\u{3bf}\u{3c2}"),
            (
                "\u{39f}\u{3a3}\u{391}",
                Semantics::CompatV1,
                "\u{3bf}\u{3c3}\u{3b1}",
            ),
            (
                "\u{39f}\u{3a3}\u{391}",
                Semantics::EnhancedV1,
                "\u{3bf}\u{3c3}\u{3b1}",
            ),
            ("\u{3a3}", Semantics::CompatV1, "\u{3c3}"),
            ("\u{3a3}", Semantics::EnhancedV1, "\u{3c3}"),
            (
                "\u{39f}\u{3a3}\u{301}",
                Semantics::CompatV1,
                "\u{3bf}\u{3c3}\u{301}",
            ),
            (
                "\u{39f}\u{3a3}\u{301}",
                Semantics::EnhancedV1,
                "\u{3bf}\u{3c2}\u{301}",
            ),
        ];
        for (input, semantics, expected) in cases {
            assert_eq!(
                view(input, *semantics),
                *expected,
                "{input:?}/{semantics:?}"
            );
        }
    }

    #[test]
    fn ac017_provenance_ranges_follow_plan_examples() {
        let units = normalize_str("🙂测试词", Semantics::EnhancedV1, &NO_CANCEL).unwrap();
        let text = units_text(&units);
        let at = text[..text.find("测试词").unwrap()].chars().count();
        let first = &units[at];
        let last = &units[at + "测试词".chars().count() - 1];
        assert_eq!((first.start, last.end), (4, 13));

        let units = normalize_str("测\u{200b}试词", Semantics::EnhancedV1, &NO_CANCEL).unwrap();
        assert_eq!((units[0].start, units[2].end), (0, 12));

        // one-to-many lowercase expansion: both outputs conservatively cover
        // the complete original character
        let units = normalize_str("aİb", Semantics::EnhancedV1, &NO_CANCEL).unwrap();
        assert_eq!(units_text(&units), "ai\u{307}b");
        assert_eq!((units[1].start, units[1].end), (1, 3));
        assert_eq!((units[2].start, units[2].end), (1, 3));
    }

    #[test]
    fn ac018_invalid_utf8_is_rejected_not_repaired() {
        let cancel: fn() -> bool = || false;
        // truncated multi-byte sequence
        assert_eq!(
            normalize_bytes(b"\xe6\xb5", Semantics::CompatV1, &cancel),
            Err(NormalizationError::InvalidUtf8)
        );
        // surrogate half encoded as UTF-8 (CESU-8 style)
        assert_eq!(
            normalize_bytes(b"\xed\xa0\x80", Semantics::CompatV1, &cancel),
            Err(NormalizationError::InvalidUtf8)
        );
        // lone continuation byte
        assert_eq!(
            normalize_bytes(b"ok\x80", Semantics::EnhancedV1, &cancel),
            Err(NormalizationError::InvalidUtf8)
        );
    }

    #[test]
    fn ac019_expansion_limit_is_enforced_without_truncation() {
        // İ (2 input bytes) expands to 'i' + U+0307 = 3 view bytes; force
        // the 48 KiB view budget: 16384 units produce exactly 49152 bytes.
        let cancel: fn() -> bool = || false;
        let over = "İ".repeat(MAX_VIEW_BYTES_PER_FIELD / 3 + 8);
        assert_eq!(
            normalize_str(&over, Semantics::EnhancedV1, &cancel),
            Err(NormalizationError::LimitExceeded)
        );
        let boundary = "İ".repeat(MAX_VIEW_BYTES_PER_FIELD / 3);
        assert!(normalize_str(&boundary, Semantics::EnhancedV1, &cancel).is_ok());
    }

    #[test]
    fn cancellation_between_characters_stops_the_traversal() {
        use std::cell::Cell;
        let calls = Cell::new(0u32);
        let cancel = || {
            calls.set(calls.get() + 1);
            calls.get() > 2
        };
        assert_eq!(
            normalize_str("测试词", Semantics::EnhancedV1, &cancel),
            Err(NormalizationError::Cancelled)
        );
    }

    #[test]
    fn compat_flavor_matches_erlang_semantics_on_mixed_text() {
        // per-char lowercase ordering can only differ from full-string on
        // Final_Sigma; mixed content confirms both flavors otherwise equal
        let text = "Abc测试ＸＹｚ词İΣΩΣ";
        let compat = view(text, Semantics::CompatV1);
        let enhanced = view(text, Semantics::EnhancedV1);
        assert_eq!(compat, enhanced.replace('\u{3c2}', "\u{3c3}"));
    }

    #[test]
    fn unproven_source_correspondence_is_refused() {
        use crate::normalize::provenance::lowercase_pieces;
        const NO_CANCEL: fn() -> bool = || false;
        // length divergence between per-char pieces and authoritative output
        let pieces = lowercase_pieces("a");
        let mismatched_len = assemble(&pieces, "ab", Semantics::EnhancedV1, &NO_CANCEL);
        assert_eq!(
            mismatched_len,
            Err(NormalizationError::NormalizerIncompatible)
        );
        // non-sigma value divergence under both semantics
        let pieces = lowercase_pieces("a");
        let mismatched_value = assemble(&pieces, "b", Semantics::CompatV1, &NO_CANCEL);
        assert_eq!(
            mismatched_value,
            Err(NormalizationError::NormalizerIncompatible)
        );
        let pieces = lowercase_pieces("a");
        let mismatched_value = assemble(&pieces, "b", Semantics::EnhancedV1, &NO_CANCEL);
        assert_eq!(
            mismatched_value,
            Err(NormalizationError::NormalizerIncompatible)
        );
        // sigma divergence is accepted ONLY for enhanced semantics
        let pieces = lowercase_pieces("\u{3a3}");
        assert_eq!(
            assemble(&pieces, "\u{3c2}", Semantics::EnhancedV1, &NO_CANCEL).unwrap()[0].scalar,
            '\u{3c2}'
        );
        let pieces = lowercase_pieces("\u{3a3}");
        assert_eq!(
            assemble(&pieces, "\u{3c2}", Semantics::CompatV1, &NO_CANCEL),
            Err(NormalizationError::NormalizerIncompatible)
        );
    }

    #[test]
    fn provenance_survives_fullwidth_and_zero_width_folding() {
        let units = normalize_str("Ａ\u{200b}ｂ", Semantics::EnhancedV1, &NO_CANCEL).unwrap();
        assert_eq!(units_text(&units), "ab");
        assert_eq!((units[0].start, units[0].end), (0, 3)); // Ａ = 3 bytes
        assert_eq!((units[1].start, units[1].end), (6, 9)); // ｂ = 3 bytes
    }
}
