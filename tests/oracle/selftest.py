"""T05 oracle selftest: naive reference, frozen vectors, corpus, mutations.

Covers AC-011 (naive all-occurrence self-check), AC-012 (deleted or corrupted
witnesses must make any differential fail) and AC-013 registration (10,000
seeded cases frozen with oracle expectations; production comparison happens
in T08/T11, never here).
"""

import hashlib
import json
import random
import sys
import unittest
from pathlib import Path

ORACLE = Path(__file__).resolve().parent
sys.path.insert(0, str(ORACLE))

import expected
import gap_reference
import generate
import naive
import rulepool
import views
import unicode16
from unicode_selftest import Unicode16Tests, SemanticsPropagationTests

FIXTURES = ORACLE.parent / "fixtures/oracle"

# Frozen lowercase vectors from the run probes (OTP 29 / ERTS 17.0.2 vs the
# Rust 1.90 full-string lowercase, Unicode 16.0.0). divergent=True marks the
# documented Final_Sigma difference: OTP string:lowercase has no context rule.
OTP_VECTORS = [
    {"input": "\u0130", "otp_bytes": [105, 204, 135], "divergent": False},
    {"input": "\u039f\u03a3", "otp_bytes": [206, 191, 207, 131],
     "divergent": True, "reference_bytes": [206, 191, 207, 130]},
    {"input": "\u039f\u03a3\u0391", "otp_bytes": [206, 191, 207, 131, 206, 177],
     "divergent": False},
    {"input": "\u03a3", "otp_bytes": [207, 131], "divergent": False},
    {"input": "\u039f\u03a3\u0301", "otp_bytes": [206, 191, 207, 131, 204, 129],
     "divergent": True, "reference_bytes": [206, 191, 207, 130, 204, 129]},
]


def load_cases():
    manifest = json.loads((FIXTURES / "manifest.json").read_text())
    payload = (FIXTURES / manifest["cases_file"]).read_bytes()
    cases = [json.loads(line) for line in payload.decode("utf-8").splitlines()]
    return manifest, payload, cases


class NaiveReferenceTests(unittest.TestCase):
    def test_ac011_overlapping_self_check(self):
        # AC-011: patterns a, ab, b over text "ab" -> exactly three hits.
        hits = []
        for pattern in ("a", "ab", "b"):
            pattern_units = views.base_units(pattern, "enhanced-v1")
            text_units = views.base_units("ab", "enhanced-v1")
            hits.extend(naive.scan_literal(pattern, pattern_units, text_units))
        ranges = sorted((hit["start"], hit["end"]) for hit in hits)
        self.assertEqual(ranges, [(0, 1), (0, 2), (1, 2)])

    def test_plan_410_offset_hand_cases(self):
        units = views.base_units("🙂测试词", "enhanced-v1")
        hits = naive.scan_literal("1001", views.base_units("测试词", "enhanced-v1"), units)
        self.assertEqual([(hit["start"], hit["end"]) for hit in hits], [(4, 13)])

        units = views.base_units("测\u200b试词", "enhanced-v1")
        hits = naive.scan_literal("1001", views.base_units("测试词", "enhanced-v1"), units)
        self.assertEqual([(hit["start"], hit["end"]) for hit in hits], [(0, 12)])

    def test_gap_hand_case_t2s_and_range(self):
        # plan 4.10: 🙂測*試_詞 with t2s+gap hits the original range [4,15).
        units = views.t2s_units(views.base_units("🙂測*試_詞", "enhanced-v1"),
                                rulepool.TEST_T2S_TABLE)
        pattern = views.t2s_units(views.base_units("测试词", "enhanced-v1"),
                                  rulepool.TEST_T2S_TABLE)
        hits = gap_reference.scan_gap("1001", pattern, units, ["literal", "t2s", "gap"])
        self.assertEqual([(hit["start"], hit["end"]) for hit in hits], [(4, 15)])
        self.assertEqual(hits[0]["methods"], ["literal", "t2s", "gap"])

    def test_appendix_b2_golden_request(self):
        case = {"id": "golden-b2", "category": "hand", "scene": "channel_message",
                "t2s": True, "gap": True, "text": "🙂測*試_詞"}
        hits = expected.scan_case(case)
        matching = [hit for hit in hits
                    if hit["rule_id"] == "1001" and hit["variant_id"] == "canonical"]
        self.assertEqual(matching, [
            {"rule_id": "1001", "variant_id": "canonical", "start": 4, "end": 15,
             "methods": ["literal", "t2s", "gap"], "action": "review"}])

    def test_alias_ascii_word_fence(self):
        alias = {"variant_id": "pinyin-01", "boundary": "ascii_word"}
        for text, count in (("ceshici", 1), ("myceshicitool", 0),
                            ("中文ceshici中文", 1), ("ce shi ci", 0)):
            units = views.base_units(text, "enhanced-v1")
            hits = naive.scan_alias("1001", alias, views.base_units("ceshici", "enhanced-v1"), units)
            self.assertEqual(len(hits), count, text)


class OtpVectorTests(unittest.TestCase):
    def test_frozen_lowercase_vectors_both_flavors(self):
        for vector in OTP_VECTORS:
            with self.subTest(vector=vector["input"]):
                otp = views.otp_compat_lower(vector["input"]).encode()
                self.assertEqual(list(otp), vector["otp_bytes"])
                reference = unicode16.lowercase(vector["input"], "enhanced-v1").encode()
                if vector["divergent"]:
                    self.assertEqual(list(reference), vector["reference_bytes"])
                    self.assertNotEqual(list(otp), list(reference))
                    self.assertIn(0xCF, reference)  # final sigma present
                else:
                    self.assertEqual(list(otp), list(reference))

    def test_base_view_reconstruction_matches_reference(self):
        for vector in OTP_VECTORS:
            units = views.base_units(vector["input"], "enhanced-v1")
            self.assertEqual("".join(unit.scalar for unit in units),
                             unicode16.lowercase(vector["input"], "enhanced-v1"))


class CorpusTests(unittest.TestCase):
    def test_manifest_identity_and_deterministic_regeneration(self):
        manifest, payload, cases = load_cases()
        self.assertEqual(len(cases), generate.CASE_COUNT)
        self.assertEqual(manifest["seed"], generate.SEED)
        self.assertEqual(manifest["cases_sha256"],
                         hashlib.sha256(payload).hexdigest())
        # in-memory only: the frozen fixtures must never be rewritten by a check
        self.assertEqual(generate.cases_digest(generate.generate()),
                         manifest["cases_sha256"])

    def test_category_plan_sums_and_coverage(self):
        _, _, cases = load_cases()
        counts = {}
        hits_by_category = {}
        for case in cases:
            counts[case["category"]] = counts.get(case["category"], 0) + 1
            hits_by_category.setdefault(case["category"], 0)
            hits_by_category[case["category"]] += len(case["expected"])
        self.assertEqual(sum(counts.values()), generate.CASE_COUNT)
        self.assertEqual(counts, generate.CATEGORY_PLAN)
        self.assertGreaterEqual(hits_by_category["literal_mixed"], 1200)
        self.assertGreaterEqual(hits_by_category["zero_width"], 1100)
        self.assertGreaterEqual(hits_by_category["fullwidth"], 300)
        self.assertGreaterEqual(hits_by_category["emoji_boundary"], 450)
        self.assertGreaterEqual(hits_by_category["t2s"], 800)
        self.assertGreaterEqual(hits_by_category["gap"], 700)
        self.assertGreaterEqual(hits_by_category["alias_positive"], 700)
        self.assertEqual(hits_by_category["alias_negative"], 0)

    def test_compat_scene_never_uses_enhanced_evidence(self):
        _, _, cases = load_cases()
        compat = [case for case in cases
                  if case["scene"] == "profile_field"]
        self.assertGreater(len(compat), 1800)
        for case in compat:
            for hit in case["expected"]:
                self.assertNotIn("t2s", hit["methods"], case["id"])
                self.assertNotIn("gap", hit["methods"], case["id"])
                self.assertNotIn("alias", hit["methods"], case["id"])
        # traditional text under a compat scene stays undetected (oracle gate)
        gated = expected.scan_case({"id": "compat-gate", "category": "hand",
                                    "scene": "profile_field", "t2s": True,
                                    "gap": True, "text": "測試詞"})
        self.assertEqual(gated, [])

    def test_hit_ranges_are_utf8_boundaries_inside_original(self):
        _, _, cases = load_cases()
        checked = 0
        for case in cases:
            raw = case["text"].encode("utf-8")
            for hit in case["expected"]:
                self.assertTrue(0 <= hit["start"] < hit["end"] <= len(raw), case["id"])
                raw[hit["start"]:hit["end"]].decode("utf-8")  # must not raise
                checked += 1
        self.assertGreater(checked, 4000)

    def test_hard_boundary_cases_have_no_gap_hits(self):
        _, _, cases = load_cases()
        boundary_cases = [case for case in cases
                          if case["category"] == "hard_boundary_negative"]
        for case in boundary_cases:
            self.assertFalse(any("gap" in hit["methods"]
                                 for hit in case["expected"]), case["id"])


class MutationTests(unittest.TestCase):
    """AC-012: expectations must be sensitive to deleted or corrupted hits."""

    def test_mutations_fail(self):
        _, _, cases = load_cases()
        rng = random.Random(generate.SEED)
        with_hits = [case for case in cases if case["expected"]][:50]
        self.assertEqual(len(with_hits), 50)
        for case in with_hits:
            original = case["expected"]
            fresh = expected.scan_case(case)
            self.assertTrue(expected.compare_hits(original, fresh), case["id"])

            dropped = [hit for hit in original[1:]] or original[:-1]
            self.assertFalse(expected.compare_hits(dropped, fresh), case["id"])

            shifted = [dict(hit) for hit in original]
            shifted[0]["start"] += 1
            self.assertFalse(expected.compare_hits(shifted, fresh), case["id"])

            forged = original + [{"rule_id": "9999", "variant_id": "canonical",
                                  "start": 0, "end": 3, "methods": ["literal"]}]
            self.assertFalse(expected.compare_hits(forged, fresh), case["id"])

    def test_scanner_is_deterministic_per_case(self):
        _, _, cases = load_cases()
        for case in cases[::997]:
            first = expected.scan_case(case)
            second = expected.scan_case(case)
            self.assertTrue(expected.compare_hits(first, second))


def flatten(suite):
    for item in suite:
        if isinstance(item, unittest.TestSuite):
            yield from flatten(item)
        else:
            yield item.id()


if __name__ == "__main__":
    loader = unittest.defaultTestLoader
    suite = unittest.TestSuite([
        loader.loadTestsFromTestCase(NaiveReferenceTests),
        loader.loadTestsFromTestCase(OtpVectorTests),
        loader.loadTestsFromTestCase(CorpusTests),
        loader.loadTestsFromTestCase(MutationTests),
        loader.loadTestsFromTestCase(Unicode16Tests),
        loader.loadTestsFromTestCase(SemanticsPropagationTests),
    ])
    discovered = list(flatten(suite))
    arguments = [argument for argument in sys.argv[1:]]
    if arguments == ["--list"]:
        print(json.dumps(discovered))
    elif arguments:
        sys.exit(64)
    else:
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        print(json.dumps({"tests_discovered": len(discovered),
                          "tests_executed": result.testsRun,
                          "failures": len(result.failures),
                          "errors": len(result.errors)}))
        sys.exit(0 if result.wasSuccessful() and result.testsRun > 0 else 1)
