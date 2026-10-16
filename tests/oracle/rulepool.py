"""Fixed synthetic rule pool and fixture t2s table for the oracle corpus.

All words are artificial test rules (plan section 14: they do not represent
content that should actually be banned). The t2s table here is the frozen
fixture table ``test-t2s-v1`` referenced by the contract ruleset; production
uses the generated restricted table from T07, never this fixture table.
"""

# Restricted, unambiguous single-scalar traditional->simplified fixture table.
TEST_T2S_TABLE = {
    "測": "测", "試": "试", "詞": "词", "們": "们", "會": "会",
    "對": "对", "時": "时", "說": "说", "話": "话", "機": "机",
    "風": "风", "雲": "云", "書": "书", "學": "学", "國": "国",
    "車": "车", "馬": "马", "鳥": "鸟", "龍": "龙", "廠": "厂",
}

FIXTURE_RULESET = {
    "schema_version": 1,
    "normalization_id": "imboy-base-v1",
    "t2s_table_id": "test-t2s-v1",
    "limit_profile_id": "mvp-default-v1",
    "semantics_by_scene": {
        "channel_message": "enhanced-v1",
        "moment_post": "enhanced-v1",
        "profile_field": "compat-v1",
    },
    "allow_empty": False,
    "literals": [
        {"id": "1001", "word": "测试词", "category": "test", "severity": "high",
         "scenes": ["channel_message", "moment_post", "profile_field"],
         "aliases": [{"variant_id": "pinyin-01", "type": "pinyin",
                      "pattern": "ceshici", "action": "review",
                      "boundary": "ascii_word"}],
         "gap": {"enabled": True, "action": "review"}},
        {"id": "1002", "word": "森林", "category": "test", "severity": "medium",
         "scenes": ["channel_message", "moment_post", "profile_field"],
         "aliases": [{"variant_id": "decomp-01", "type": "decomposed",
                      "pattern": "森木木", "action": "review",
                      "boundary": "none"}],
         "gap": {"enabled": False, "action": "review"}},
        {"id": "1003", "word": "测", "category": "test", "severity": "low",
         "scenes": ["channel_message", "moment_post", "profile_field"],
         "aliases": [], "gap": {"enabled": False, "action": "review"}},
        {"id": "1004", "word": "测试", "category": "test", "severity": "low",
         "scenes": ["channel_message", "moment_post", "profile_field"],
         "aliases": [], "gap": {"enabled": False, "action": "review"}},
        {"id": "1005", "word": "词", "category": "test", "severity": "low",
         "scenes": ["channel_message", "moment_post", "profile_field"],
         "aliases": [], "gap": {"enabled": False, "action": "review"}},
        {"id": "1006", "word": "测试词条", "category": "test", "severity": "high",
         "scenes": ["channel_message", "moment_post"],
         "aliases": [], "gap": {"enabled": True, "action": "review"}},
        {"id": "1007", "word": "abc", "category": "test", "severity": "low",
         "scenes": ["channel_message", "profile_field"],
         "aliases": [], "gap": {"enabled": False, "action": "review"}},
        {"id": "1008", "word": "hello", "category": "test", "severity": "low",
         "scenes": ["moment_post"],
         "aliases": [], "gap": {"enabled": False, "action": "review"}},
    ],
    "regex_rules": [],
    "exceptions": [],
}

# Byte pools for corpus text generation (synthetic, non-sensitive).
CHINESE_NOISE = "的一是在了有和人这中大为上个国我以要他时来用们生到作地于出就分对成会可主发年动同工也能下过子说产种面而方后多定行学法之民本得高同进此实长二三四五无利小机其量力种"
ASCII_NOISE = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
EMOJI = ["🙂", "😀", "🚀", "📚", "🎯"]
ZERO_WIDTHS = ["\u200b", "\u200c", "\u200d", "\ufeff"]
SEPARATORS = [" ", "*", "-", "_", "·", "  ", "*_*"]
BOUNDARIES = ["。", ".", "！", "!", "？", "?", "；", ";", "\n", "\r"]
