"""Frozen local-exception differential corpus generator (T09).

Builds a ruleset on top of the fixture rule pool with three local
exceptions (B canonical, T canonical, B alias) and generates random case
texts mixing the exception phrases, plain rule words, gap separators,
traditional forms and noise. Expectations are computed by the independent
Python oracle (expected.scan_case) only; production code never regenerates
them. Plan 14: all words are synthetic and non-sensitive.
"""

import copy
import hashlib
import json
import random
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import rulepool
from expected import scan_case

SEED = 20261017
CASES = 600

RULESET = copy.deepcopy(rulepool.FIXTURE_RULESET)
RULESET["exceptions"] = [
    {"id": "exc-01", "phrase": "软件测试", "scenes": ["channel_message", "moment_post"],
     "rule_id": "1004", "variant_ids": ["canonical"], "views": ["B"]},
    {"id": "exc-02", "phrase": "測試词特殊", "scenes": ["channel_message"],
     "rule_id": "1001", "variant_ids": ["canonical"], "views": ["T"]},
    {"id": "exc-03", "phrase": "森木木系统", "scenes": ["channel_message"],
     "rule_id": "1002", "variant_ids": ["decomp-01"], "views": ["B"]},
]

# Synthetic text fragments exercising exemption positives/negatives, gap
# stitching, t2s evidence and boundary noise around the fixture words.
FRAGMENTS = [
    "软件测试",                     # exception exc-01 exact
    "软件测试还有测试",             # exempted hit + surviving later hit
    "软件测试测",                   # second hit only partially covered
    "前置软件测试",                 # covered with left context
    "測試词特殊",                   # exception exc-02 exact (T view)
    "測試词特殊测试词",             # T exemption + surviving T hit
    "測*試_词特殊",                 # gap evidence inside an exception phrase
    "森木木系统",                   # exception exc-03 (alias variant)
    "森木木系统森木木",             # alias exemption + surviving alias hit
    "森木木",                       # plain alias hit
    "测试词", "测试词条", "测试", "森林",
    "测*试**词", "测**试词", "测***试词",       # gap adjacency/total forms
    "測試詞", "測試词条",                        # t2s plain evidence
    "abc", "ＡＢＣhello", "ceshici", "myceshicitool",
    "。", ".", "；", "\n", " ", "*", "_", "·", "🙂", "🚀",
]
NOISE = ["的", "了", "和", "123", "xyz", "在前", "以后"]

SCENES = ["channel_message", "moment_post", "profile_field"]


def main():
    out_dir = Path(__file__).resolve().parent.parent / "fixtures" / "oracle-exceptions"
    out_dir.mkdir(parents=True, exist_ok=True)
    rng = random.Random(SEED)
    lines = []
    for index in range(CASES):
        parts = []
        for _ in range(rng.randint(1, 4)):
            parts.append(rng.choice(FRAGMENTS))
            if rng.random() < 0.4:
                parts.append(rng.choice(NOISE))
        case = {
            "id": f"exc-{index:04d}",
            "category": "exception_differential",
            "text": "".join(parts),
            "scene": rng.choice(SCENES),
            "t2s": rng.random() < 0.5,
            "gap": True,
        }
        case["expected"] = scan_case(case, ruleset=RULESET)
        lines.append(json.dumps(case, ensure_ascii=False))
    corpus = ("\n".join(lines) + "\n").encode("utf-8")
    (out_dir / "cases-600.jsonl").write_bytes(corpus)
    manifest = {
        "case_count": CASES,
        "cases_file": "cases-600.jsonl",
        "cases_sha256": hashlib.sha256(corpus).hexdigest(),
        "seed": SEED,
        "ruleset_file": "ruleset.json",
        "note": "expectations computed by the independent Python oracle only; "
                "production differential consumers must not regenerate them",
    }
    (out_dir / "manifest.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    (out_dir / "ruleset.json").write_text(
        json.dumps(RULESET, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {CASES} cases to {out_dir}")


if __name__ == "__main__":
    main()
