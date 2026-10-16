//! Restricted traditional->simplified single-scalar mapping (plan 4.4).

use std::collections::HashMap;
use std::sync::OnceLock;

use super::provenance::Unit;

/// Table generated from locked OpenCC ver.1.1.9 TSCharacters data by
/// `tools/t2s/generate_table.py`; provenance in `data/t2s/table_report.json`.
pub const REAL_TABLE_ID: &str = "opencc-1.1.9-single-v1";

/// Frozen fixture table used by the oracle corpus and contract fixtures.
/// Never presented as production data (plan appendix B.1).
pub const FIXTURE_TABLE_ID: &str = "test-t2s-v1";

/// All table ids this binary supports; packages referencing anything else
/// are rejected at publish and recovery time (plan 8.3, AC-023).
pub const SUPPORTED_TABLE_IDS: [&str; 2] = [REAL_TABLE_ID, FIXTURE_TABLE_ID];

const REAL_TABLE_TSV: &str = include_str!("../../data/t2s/single_table.tsv");

/// Fixture table mirroring tests/oracle/rulepool.py TEST_T2S_TABLE.
const FIXTURE_TABLE_ENTRIES: &str = concat!(
    "測\t测\n",
    "試\t试\n",
    "詞\t词\n",
    "們\t们\n",
    "會\t会\n",
    "對\t对\n",
    "時\t时\n",
    "說\t说\n",
    "話\t话\n",
    "機\t机\n",
    "風\t风\n",
    "雲\t云\n",
    "書\t书\n",
    "學\t学\n",
    "國\t国\n",
    "車\t车\n",
    "馬\t马\n",
    "鳥\t鸟\n",
    "龍\t龙\n",
    "廠\t厂\n",
);

fn parse_table(source: &str) -> HashMap<char, char> {
    let mut table = HashMap::new();
    for line in source.lines() {
        if line.is_empty() {
            continue;
        }
        let (key, value) = line
            .split_once('\t')
            .unwrap_or_else(|| panic!("malformed table line: {line:?}"));
        let (key, value) = (
            key.chars()
                .next()
                .unwrap_or_else(|| panic!("empty key: {line:?}")),
            value
                .chars()
                .next()
                .unwrap_or_else(|| panic!("empty value: {line:?}")),
        );
        assert_eq!(
            key.len_utf8(),
            line.split('\t').next().unwrap().len(),
            "multi-scalar key is forbidden: {line:?}"
        );
        let value_text = &line.split_once('\t').unwrap().1;
        assert_eq!(
            value.len_utf8(),
            value_text.len(),
            "multi-scalar value is forbidden: {line:?}"
        );
        if let Some(previous) = table.insert(key, value) {
            assert_eq!(previous, value, "conflicting mapping for {key:?}");
        }
    }
    table
}

fn tables() -> &'static HashMap<&'static str, HashMap<char, char>> {
    static TABLES: OnceLock<HashMap<&'static str, HashMap<char, char>>> = OnceLock::new();
    TABLES.get_or_init(|| {
        let mut tables = HashMap::new();
        tables.insert(REAL_TABLE_ID, parse_table(REAL_TABLE_TSV));
        tables.insert(FIXTURE_TABLE_ID, parse_table(FIXTURE_TABLE_ENTRIES));
        tables
    })
}

/// The table with the given id, or `None` when this binary does not support
/// it. Callers must reject packages referencing unsupported ids instead of
/// silently substituting the current table (plan 8.3).
pub fn table_by_id(table_id: &str) -> Option<&'static HashMap<char, char>> {
    tables().get(table_id)
}

/// Map one scalar; unknown scalars pass through unchanged (plan 4.4).
pub fn map_scalar(table: &HashMap<char, char>, scalar: char) -> char {
    table.get(&scalar).copied().unwrap_or(scalar)
}

/// Apply the table to a view; provenance ranges are preserved exactly.
pub fn t2s_units(units: &[Unit], table: &HashMap<char, char>) -> Vec<Unit> {
    units
        .iter()
        .map(|unit| Unit::new(map_scalar(table, unit.scalar), unit.start, unit.end))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::normalize::base::{normalize_str, Semantics};

    const NO_CANCEL: fn() -> bool = || false;

    fn view(text: &str, table_id: &str) -> String {
        let units = normalize_str(text, Semantics::EnhancedV1, &NO_CANCEL).unwrap();
        let table = table_by_id(table_id).expect("supported table");
        units
            .iter()
            .map(|unit| map_scalar(table, unit.scalar))
            .collect()
    }

    #[test]
    fn ac020_mixed_script_hits_under_the_approved_table() {
        assert_eq!(view("測試詞", REAL_TABLE_ID), "测试词");
        assert_eq!(view("测試词", REAL_TABLE_ID), "测试词");
        // characters outside the table are never guessed
        assert_eq!(view("𠮷測", REAL_TABLE_ID), "𠮷测");
        assert_eq!(view("㐀測", REAL_TABLE_ID), "㐀测");
    }

    #[test]
    fn ac021_ambiguous_sources_are_excluded_not_first_guess() {
        // 乾 has two OpenCC targets (干 / 乾-as-gan) and must be absent from
        // the generated table rather than silently mapped to the first one.
        let table = table_by_id(REAL_TABLE_ID).unwrap();
        assert!(!table.contains_key(&'乾'));
        // multi-target exclusions were recorded by the generator report
        let report = include_str!("../../data/t2s/table_report.json");
        assert!(report.contains("\"excluded_multi_target\": 56"));
        assert!(report.contains("\"excluded_cycle\": 0"));
    }

    #[test]
    fn ac022_embedded_table_is_idempotent_and_sorted() {
        let table = table_by_id(REAL_TABLE_ID).unwrap();
        for (key, value) in table.iter() {
            assert_eq!(map_scalar(table, *value), *value, "non-idempotent {key:?}");
        }
        // the committed TSV is codepoint-sorted with unique keys
        let lines: Vec<&str> = REAL_TABLE_TSV.lines().collect();
        let keys: Vec<u32> = lines
            .iter()
            .map(|line| line.split('\t').next().unwrap().chars().next().unwrap() as u32)
            .collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            keys, sorted,
            "table must be codepoint-sorted with unique keys"
        );
        assert_eq!(keys.len(), table.len());
    }

    #[test]
    fn ac023_unsupported_table_ids_are_rejected() {
        assert!(table_by_id(REAL_TABLE_ID).is_some());
        assert!(table_by_id(FIXTURE_TABLE_ID).is_some());
        assert!(table_by_id("test-t2s-v2").is_none());
        assert!(table_by_id("").is_none());
        assert!(table_by_id("opencc-1.1.9-single-v1-forged").is_none());
    }

    #[test]
    fn provenance_is_preserved_through_the_mapping() {
        let units = normalize_str("測試", Semantics::EnhancedV1, &NO_CANCEL).unwrap();
        let mapped = t2s_units(&units, table_by_id(REAL_TABLE_ID).unwrap());
        assert_eq!(mapped[0].scalar, '测');
        assert_eq!((mapped[0].start, mapped[0].end), (0, 3));
        assert_eq!(mapped[1].scalar, '试');
        assert_eq!((mapped[1].start, mapped[1].end), (3, 6));
    }

    #[test]
    fn fixture_table_matches_the_oracle_rulepool() {
        let table = table_by_id(FIXTURE_TABLE_ID).unwrap();
        assert_eq!(table.len(), 20);
        assert_eq!(view("測試詞", FIXTURE_TABLE_ID), "测试词");
        assert_eq!(view("測試詞條", FIXTURE_TABLE_ID), "测试词條");
    }
}
