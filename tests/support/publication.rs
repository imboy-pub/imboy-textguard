use super::TABLE_ID;
use imboy_textguard::model::{
    CheckRequest, GapConfig, LiteralRule, RuleAction, RuleSet, Scene, SceneSemantics, Semantics,
    Severity, TextField,
};
use imboy_textguard::normalize::table_by_id;
use imboy_textguard::rules::RulesState;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) fn table() -> &'static HashMap<char, char> {
    table_by_id(TABLE_ID).unwrap()
}

// ---------------------------------------------------------------- fixtures

pub(super) fn base_ruleset(literals: Vec<LiteralRule>, allow_empty: bool) -> Vec<u8> {
    let ruleset = RuleSet {
        schema_version: 1,
        normalization_id: "imboy-base-v1".to_owned(),
        t2s_table_id: TABLE_ID.to_owned(),
        limit_profile_id: "mvp-default-v1".to_owned(),
        semantics_by_scene: SceneSemantics {
            channel_message: Semantics::EnhancedV1,
            moment_post: Semantics::CompatV1,
            profile_field: Semantics::CompatV1,
        },
        allow_empty,
        literals,
        regex_rules: Vec::new(),
        exceptions: Vec::new(),
    };
    serde_json::to_vec(&ruleset).unwrap()
}

pub(super) fn one_word_ruleset(word: &str) -> Vec<u8> {
    base_ruleset(
        vec![LiteralRule {
            id: "2001".to_owned(),
            word: word.to_owned(),
            category: "test".to_owned(),
            severity: Severity::High,
            scenes: vec![Scene::ChannelMessage],
            aliases: Vec::new(),
            gap: GapConfig {
                enabled: false,
                action: RuleAction::Review,
            },
        }],
        false,
    )
}

pub(super) fn empty_ruleset(allow_empty: bool) -> Vec<u8> {
    base_ruleset(Vec::new(), allow_empty)
}

/// A large ruleset so compilation takes long enough to widen the publish
/// lock race window.
pub(super) fn big_ruleset(seed: usize) -> Vec<u8> {
    let literals = (0..2_000)
        .map(|index| LiteralRule {
            // rule ids are pure digits without a leading zero
            id: format!("1{index:04}"),
            word: format!("坏词{seed}{index:04}"),
            category: "test".to_owned(),
            severity: Severity::Low,
            scenes: vec![Scene::ChannelMessage],
            aliases: Vec::new(),
            gap: GapConfig {
                enabled: false,
                action: RuleAction::Review,
            },
        })
        .collect();
    base_ruleset(literals, false)
}

pub(super) fn expected_base_digest(word: &str) -> String {
    match word {
        "坏词甲" => "1d3973340e2d2cbd2c7c3b89d009ecc0da77342a801f6dcbdfb22114c07b4daa",
        "坏词乙" => "bd21c5dfaceb807afe3ad164574d4ed54448014f173a1e7c133d89a9af46f1f3",
        _ => panic!("unexpected fixture word"),
    }
    .to_owned()
}

pub(super) fn request() -> CheckRequest {
    CheckRequest {
        request_id: "t14".to_owned(),
        scene: Scene::ChannelMessage,
        fields: vec![TextField {
            field_id: "body".to_owned(),
            text: "坏词甲 坏词乙 坏词丙".to_owned(),
        }],
    }
}

pub(super) struct TempDir(PathBuf);

impl TempDir {
    pub(super) fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "textguard-t14-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }

    pub(super) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(super) fn store(dir: &Path) -> RulesState {
    RulesState::load(dir, table()).unwrap()
}

pub(super) fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}
