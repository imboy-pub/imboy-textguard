//! Base normalization and original-text provenance (plan section 4.3).

pub mod base;
pub mod provenance;

pub use base::{normalize_bytes, normalize_str, NormalizationError, Semantics};
pub use provenance::Unit;
