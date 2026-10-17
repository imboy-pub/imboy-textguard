import hashlib
import json
from pathlib import Path
import random
import sys
import unittest

SEED = 20261017
SCENES = ["channel_message", "moment_post", "profile_field"]
FILLER = "大家分享春天的风景，交流日常生活，阅读书籍，认真思考，友善讨论。"


def word(index):
    if index < 2500:
        return "测试" + format(index, "04x")
    length = 2 + index % 11
    return chr(0x4E00 + length) + chr(0x7000 + index // 11) + "开" * (length - 2)


def ruleset(count, enhanced):
    literals = []
    for index in range(count):
        aliases = []
        if enhanced and index < 2500:
            suffix = format(index, "04x")
            for kind, pattern in [("pinyin", "ceshi" + suffix),
                                  ("homophone", "侧视" + suffix),
                                  ("lookalike", "测试" + suffix.replace("0", "o")),
                                  ("decomposed", "氵则讠式" + suffix)]:
                aliases.append({"variant_id": kind, "type": kind, "pattern": pattern,
                                "action": "review", "boundary": "ascii_word" if kind == "pinyin" else "none"})
        literals.append({"id": str(100000 + index), "word": word(index), "category": "synthetic",
                         "severity": "low", "scenes": SCENES, "aliases": aliases,
                         "gap": {"enabled": enhanced and 100 <= index < 5100, "action": "review"}})
    regex = [{"id": f"re:perf-regex-{index}", "expression": f"正则锚点{index:02}[0-9]{{2}}",
              "display_word": f"正则锚点{index:02}", "category": "synthetic", "severity": "low",
              "action": "review", "scenes": SCENES} for index in range(32)] if enhanced else []
    exceptions = [{"id": f"perf-exception-{index}", "phrase": "示例" + word(index), "scenes": SCENES,
                   "rule_id": str(100000 + index), "variant_ids": ["canonical"], "views": ["B", "T"]}
                  for index in range(min(100, count))] if enhanced else []
    semantics = "enhanced-v1" if enhanced else "compat-v1"
    return {"schema_version": 1, "normalization_id": "imboy-base-v1",
            "t2s_table_id": "opencc-1.1.9-single-v1", "limit_profile_id": "mvp-default-v1",
            "semantics_by_scene": dict.fromkeys(SCENES, semantics), "allow_empty": False,
            "literals": literals, "regex_rules": regex, "exceptions": exceptions}


def padded(prefix, size):
    encoded = (prefix + " " + FILLER * (size // 3 + 1)).encode()[:size]
    text = encoded.decode("utf-8", errors="ignore")
    return text + " " * (size - len(text.encode()))


def cases(count, enhanced, size=1024):
    rng = random.Random(SEED)
    rows = []
    for index in range(100):
        prefix = ""
        expected = "allow"
        kind = "no_hit"
        witnesses = []
        if enhanced and index == 0:
            prefix, kind = "示例" + word(0), "exception_no_hit"
        if 90 <= index < 99:
            rule_index = rng.randrange(min(count, 2500))
            prefix = word(rule_index)
            kind, expected = "literal", "review"
            if enhanced:
                variant = index - 90
                suffix = format((variant + 1) % min(count, 2500), "04x")
                variants = [("literal", "测试" + suffix), ("pinyin", "ceshi" + suffix),
                            ("homophone", "侧视" + suffix), ("lookalike", "测试" + suffix.replace("0", "o")),
                            ("decomposed", "氵则讠式" + suffix), ("gap", "测 试0064"),
                            ("t2s", "測試" + suffix), ("regex", "正则锚点0012"), ("literal", "测试" + suffix)]
                kind, prefix = variants[variant]
                rule_index = (variant + 1) % min(count, 2500)
                if kind == "gap":
                    rule_index = 100
            method = {"gap": "gap", "t2s": "t2s", "regex": "regex", "literal": "literal"}.get(kind, "alias")
            witnesses = [{"rule_id": "re:perf-regex-0" if kind == "regex" else str(100000 + rule_index),
                          "variant_id": "regex" if kind == "regex" else kind if method == "alias" else "canonical",
                          "method": method}]
        elif index == 99:
            prefix = " ".join(word(number) for number in range(20))
            kind, expected = "high_density", "review"
            witnesses = [{"rule_id": str(100000 + number), "variant_id": "canonical", "method": "literal"}
                         for number in range(20)]
        rows.append({"request": {"request_id": f"perf-{index}", "scene": "channel_message",
                                  "fields": [{"field_id": "body", "text": padded(prefix, size)}]},
                     "expected_decision": expected, "expected_hits": witnesses, "kind": kind})
    return rows


def generate(directory):
    directory.mkdir(parents=True, exist_ok=False)
    manifest = {"seed": SEED, "source": "deterministic synthetic mechanism/performance fixtures; no business approval",
                "distribution": {"no_hit": 90, "low_density": 9, "high_density": 1}, "files": {}}
    for count in [1000, 10000, 50000]:
        for enhanced in [False, True]:
            name = f"{'B1' if enhanced else 'B0'}-{count}"
            rules = ruleset(count, enhanced)
            outputs = {name + ".rules.json": json.dumps(rules, ensure_ascii=False, separators=(",", ":"))}
            rules["literals"][-1]["severity"] = "medium"
            outputs[name + ".update.rules.json"] = json.dumps(rules, ensure_ascii=False, separators=(",", ":"))
            for size in [256, 1024, 4096, 16384]:
                outputs[f"{name}-{size}.cases.jsonl"] = "".join(json.dumps(row, ensure_ascii=False) + "\n"
                                                                                for row in cases(count, enhanced, size))
            for filename, content in outputs.items():
                data = content.encode()
                (directory / filename).write_bytes(data)
                manifest["files"][filename] = {"sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)}
            manifest[name] = {"canonical": count, "alias": sum(len(rule["aliases"]) for rule in rules["literals"]),
                              "gap": sum(rule["gap"]["enabled"] for rule in rules["literals"]),
                              "regex": len(rules["regex_rules"]), "exception": len(rules["exceptions"]),
                              "canonical_utf8_bytes": sum(len(rule["word"].encode()) for rule in rules["literals"]),
                              "canonical_scalar_lengths": sorted(set(len(rule["word"]) for rule in rules["literals"]))}
            manifest[name]["canonical_scalar_length_counts"] = {
                str(length): sum(len(rule["word"]) == length for rule in rules["literals"])
                for length in range(2, 13)}
    (directory / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


class DatasetTests(unittest.TestCase):
    def test_formal_counts_and_four_explicit_alias_types(self):
        rules = ruleset(50000, True)
        self.assertEqual(len(rules["literals"]), 50000)
        self.assertEqual(len({rule["word"] for rule in rules["literals"]}), 50000)
        self.assertEqual({len(rule["word"]) for rule in rules["literals"]}, set(range(2, 13)))
        self.assertEqual(sum(rule["gap"]["enabled"] for rule in rules["literals"]), 5000)
        self.assertEqual(len(rules["regex_rules"]), 32)
        self.assertEqual(len(rules["exceptions"]), 100)
        aliases = [alias for rule in rules["literals"] for alias in rule["aliases"]]
        self.assertEqual(len(aliases), 10000)
        for kind in ["pinyin", "homophone", "lookalike", "decomposed"]:
            self.assertEqual(sum(alias["type"] == kind for alias in aliases), 2500)
        self.assertTrue(all(rule["id"].startswith("re:") for rule in rules["regex_rules"]))
        self.assertTrue(all(not rules["literals"][index]["gap"]["enabled"] for index in range(100)))

    def test_baseline_disables_all_enhancements(self):
        rules = ruleset(1000, False)
        self.assertTrue(all(not rule["aliases"] and not rule["gap"]["enabled"] for rule in rules["literals"]))
        self.assertEqual(rules["regex_rules"], [])
        self.assertEqual(rules["exceptions"], [])
        self.assertEqual(set(rules["semantics_by_scene"].values()), {"compat-v1"})

    def test_cases_have_exact_byte_sizes_distribution_and_deterministic_expectations(self):
        for enhanced in [False, True]:
            for size in [256, 1024, 4096, 16384]:
                rows = cases(50000, enhanced, size)
                self.assertEqual(rows, cases(50000, enhanced, size))
                self.assertEqual(sum(row["expected_decision"] == "allow" for row in rows), 90)
                self.assertEqual(sum(row["expected_decision"] == "review" for row in rows), 10)
                self.assertEqual([len(row["expected_hits"]) for row in rows], [0]*90 + [1]*9 + [20])
                self.assertEqual({len(row["request"]["fields"][0]["text"].encode()) for row in rows}, {size})


if __name__ == "__main__":
    if sys.argv[1:] == ["--list"]:
        print(json.dumps(unittest.defaultTestLoader.getTestCaseNames(DatasetTests)))
    elif sys.argv[1:] == ["--selftest"]:
        unittest.main(argv=[sys.argv[0]])
    elif len(sys.argv) == 3 and sys.argv[1] == "generate":
        generate(Path(sys.argv[2]))
    else:
        raise SystemExit(64)
