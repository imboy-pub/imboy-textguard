//! View characters carrying the conservative original byte range.

/// One output character of a normalized view. `start`/`end` are byte offsets
/// into the ORIGINAL text and conservatively cover the complete original
/// character that produced this unit (plan 4.3: a fragment produced by only
/// part of an original character still covers the whole original character;
/// grapheme-level highlighting is not promised).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unit {
    pub scalar: char,
    pub start: u32,
    pub end: u32,
}

impl Unit {
    pub fn new(scalar: char, start: u32, end: u32) -> Self {
        Self { scalar, start, end }
    }
}

/// Per-character lowercase pieces with their original byte ranges.
pub(crate) struct Piece {
    /// Lowercased output of one original character (1..=3 chars).
    pub lowered: String,
    /// The original character this piece was built from.
    pub original: char,
    /// Byte range of the original character in the source text.
    pub start: u32,
    pub end: u32,
}

/// Build per-char lowercase pieces for provenance reconstruction.
///
/// Lengths come from the unconditional full mapping of each character. The
/// only context-sensitive lowercase rule (Final_Sigma) is 1:1, so piece
/// lengths always match the full-string output length; callers verify the
/// concatenated values against the authoritative output.
pub(crate) fn lowercase_pieces(
    text: &str,
    cancel: &dyn Fn() -> bool,
) -> Result<Vec<Piece>, super::base::NormalizationError> {
    super::base::check_cancel(cancel)?;
    let mut pieces = Vec::with_capacity(text.len());
    let mut cursor = 0u32;
    for character in text.chars() {
        super::base::check_cancel(cancel)?;
        let width = character.len_utf8() as u32;
        let mut lowered = String::new();
        for mapped in character.to_lowercase() {
            lowered.push(mapped);
        }
        pieces.push(Piece {
            lowered,
            original: character,
            start: cursor,
            end: cursor + width,
        });
        cursor += width;
    }
    Ok(pieces)
}
