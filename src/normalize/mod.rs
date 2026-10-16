//! Base normalization and original-text provenance (plan section 4.3).

pub mod base;
pub mod provenance;
pub mod t2s;

pub use base::{normalize_bytes, normalize_str, NormalizationError, Semantics};
pub use provenance::Unit;
pub use t2s::{t2s_units, table_by_id, FIXTURE_TABLE_ID, REAL_TABLE_ID, SUPPORTED_TABLE_IDS};
