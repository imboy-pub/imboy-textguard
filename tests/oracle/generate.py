"""Seeded corpus generator for the oracle differential (T05 -> T08/T11).

Produces 10,000 deterministic cases with frozen expectations. The seed, pool
sizes and category mix are part of the manifest; regeneration must be
byte-identical (checked in memory by the selftest, never by rewriting the
frozen fixtures). Expectations come from the oracle scanner only - never from
any production engine.
"""

import hashlib
import json
import random
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import expected
import rulepool

SEED = 20261016
CASE_COUNT = 10000
ENHANCED_SCENES = ["channel_message", "moment_post"]
SCENES = ENHANCED_SCENES + ["profile_field"]  # profile_field is compat-v1

# category -> count, must sum to CASE_COUNT
CATEGORY_PLAN = {
    "literal_mixed": 1500,      # plain literal hits with overlap/prefix/suffix
    "zero_width": 1400,         # zero-width characters inside or around words
    "fullwidth": 800,           # fullwidth ASCII folding variants
    "emoji_boundary": 600,      # emoji directly before/inside hit spans
    "t2s": 1300,                # traditional variants (T view, enhanced scenes)
    "gap": 1300,                # separator-injected canonical (gap-enabled rules)
    "hard_boundary_negative": 600,  # boundaries that must prevent gap hits
    "alias_positive": 1000,     # configured aliases that must hit (enhanced only)
    "alias_negative": 700,      # fence violations and unconfigured variants
    "noise_no_hit": 800,        # realistic no-hit majority traffic
}

WORDS = [rule["word"] for rule in rulepool.FIXTURE_RULESET["literals"]
         if not rule["word"].isascii()]
GAP_ENABLED_WORDS = [rule["word"] for rule in rulepool.FIXTURE_RULESET["literals"]
                     if rule["gap"]["enabled"] and not rule["word"].isascii()]
TRADITIONAL = {"测试词": "測試詞", "测试": "測試", "森林": "森林",
               "测试词条": "測試詞條"}


def random_noise_text(rng, length):
    parts = []
    for _ in range(length):
        roll = rng.random()
        if roll < 0.7:
            parts.append(rng.choice(rulepool.CHINESE_NOISE))
        elif roll < 0.95:
            parts.append(rng.choice(rulepool.ASCII_NOISE))
        else:
            parts.append(rng.choice(rulepool.EMOJI))
    return "".join(parts)


def wrap_with_noise(rng, core):
    left = random_noise_text(rng, rng.randint(0, 6))
    right = random_noise_text(rng, rng.randint(0, 6))
    return left + core + right


def multi_char_word(rng):
    word = rng.choice(WORDS)
    while len(word) < 2:
        word = rng.choice(WORDS)
    return word


def make_case(rng, index, category):
    # Enhanced-only categories force an enhanced scene; everything else keeps
    # the deterministic rotation, so compat scenes stay well represented.
    if category in ("t2s", "gap", "alias_positive", "alias_negative"):
        scene = ENHANCED_SCENES[index % len(ENHANCED_SCENES)]
    else:
        scene = SCENES[index % len(SCENES)]
    case = {"id": f"case-{index:05d}", "category": category, "scene": scene,
            "t2s": False, "gap": False, "text": ""}
    word = rng.choice(WORDS)
    if category == "literal_mixed":
        # guarantee overlap/prefix/suffix patterns on a subset
        style = rng.choice(["plain", "repeat", "overlap", "prefix", "suffix"])
        if style == "plain":
            core = word
        elif style == "repeat":
            core = word + word
        elif style == "overlap":
            core = "测" + word if word.startswith("测") else word + "词"
        elif style == "prefix":
            core = word + random_noise_text(rng, 2)
        else:
            core = random_noise_text(rng, 2) + word
        case["text"] = wrap_with_noise(rng, core)
    elif category == "zero_width":
        point = rng.randint(0, len(word) - 1)
        case["text"] = wrap_with_noise(
            rng, word[:point] + rng.choice(rulepool.ZERO_WIDTHS) + word[point:])
    elif category == "fullwidth":
        case["text"] = wrap_with_noise(rng, "".join(
            chr(ord(ch) + 0xFEE0) if 0x21 <= ord(ch) <= 0x7E else ch
            for ch in rng.choice(["abc", "hello", "ABC"])))
    elif category == "emoji_boundary":
        case["text"] = wrap_with_noise(
            rng, rng.choice(rulepool.EMOJI) + word + rng.choice(rulepool.EMOJI))
    elif category == "t2s":
        case["t2s"] = True
        case["text"] = wrap_with_noise(rng, TRADITIONAL.get(word, word))
    elif category == "gap":
        word = rng.choice(GAP_ENABLED_WORDS)
        case["gap"] = True
        case["t2s"] = rng.random() < 0.3
        sep = rng.choice(rulepool.SEPARATORS)
        point = rng.randint(1, len(word) - 1)
        source = TRADITIONAL.get(word, word) if case["t2s"] else word
        core = source[:point] + sep + source[point:]
        case["text"] = wrap_with_noise(rng, core)
    elif category == "hard_boundary_negative":
        word = multi_char_word(rng)
        case["gap"] = True
        boundary = rng.choice(rulepool.BOUNDARIES)
        point = rng.randint(1, len(word) - 1)
        case["text"] = wrap_with_noise(rng, word[:point] + boundary + word[point:])
    elif category == "alias_positive":
        variant = rng.choice(["bare", "chinese_left", "chinese_both", "traditional"])
        if variant == "bare":
            core = "ceshici"
        elif variant == "chinese_left":
            core = "中文ceshici"
        elif variant == "chinese_both":
            core = "中文ceshici文本"
        else:
            core = rng.choice(["ceshici", "森木木"])
        case["text"] = wrap_with_noise(rng, core)
    elif category == "alias_negative":
        core = rng.choice(["myceshicitool", "ce shi ci", "csc", "xceshici",
                           "ceshicix"])
        case["text"] = wrap_with_noise(rng, core)
    else:  # noise_no_hit
        case["text"] = random_noise_text(rng, rng.randint(4, 24))
    case["expected"] = expected.scan_case(case)
    return case


def generate():
    rng = random.Random(SEED)
    cases = []
    index = 0
    for category, count in CATEGORY_PLAN.items():
        for _ in range(count):
            cases.append(make_case(rng, index, category))
            index += 1
    return cases


def cases_digest(cases):
    body = "".join(json.dumps(case, ensure_ascii=False, sort_keys=True) + "\n"
                   for case in cases)
    return hashlib.sha256(body.encode("utf-8")).hexdigest()


def build_fixtures(target_dir):
    target = Path(target_dir)
    cases = generate()
    payload = "".join(json.dumps(case, ensure_ascii=False, sort_keys=True) + "\n"
                      for case in cases).encode("utf-8")
    manifest = {
        "seed": SEED,
        "case_count": len(cases),
        "category_plan": CATEGORY_PLAN,
        "cases_sha256": hashlib.sha256(payload).hexdigest(),
        "cases_file": "cases-10k.jsonl",
        "rulepool": "fixture ruleset test-t2s-v1 (see tests/oracle/rulepool.py)",
        "note": "expectations computed by the independent Python oracle only; "
                "production differential consumers must not regenerate them",
    }
    (target / "cases-10k.jsonl").write_bytes(payload)
    (target / "manifest.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2, sort_keys=True) + "\n")
    return manifest


if __name__ == "__main__":
    arguments = sys.argv[1:]
    if "--out" in arguments:
        out = arguments[arguments.index("--out") + 1]
        print(json.dumps(build_fixtures(out), ensure_ascii=False))
    elif arguments == ["--regenerate-check"]:
        # in-memory only: never rewrites the frozen fixtures
        cases = generate()
        print(json.dumps({"cases": len(cases), "sha256": cases_digest(cases)}))
    else:
        sys.exit(64)
