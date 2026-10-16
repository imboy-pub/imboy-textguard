"""Hand-anchored Unicode 16 and scene propagation tests, no Rust imports."""

import ast
import copy
import hashlib
import json
from pathlib import Path
import unittest
from unittest import mock

import expected
import extract_unicode16
import unicode16
import views

DATA = Path(__file__).resolve().parent.parent / "fixtures/oracle/unicode16"


def sample_rules(word="οσ", semantics="enhanced-v1"):
    return {"semantics_by_scene": {"profile_field": semantics},
            "literals": [{"id": "1999", "word": word, "severity": "high",
                          "scenes": ["profile_field"], "aliases": [],
                          "gap": {"enabled": False, "action": "review"}}],
            "exceptions": []}


def sample_case(text):
    return {"text": text, "scene": "profile_field"}


def exception(phrase, variants=("canonical",), view="B"):
    return {"id": "local", "phrase": phrase, "scenes": ["profile_field"],
            "rule_id": "1999", "variant_ids": list(variants), "views": [view]}


def witnesses(hits):
    return [(hit["variant_id"], hit["start"], hit["end"], hit["action"],
             hit["methods"]) for hit in hits]


class Unicode16Tests(unittest.TestCase):
    def test_hand_anchors_and_provenance(self):
        anchors = json.loads((DATA / "anchors.json").read_bytes())
        self.assertEqual(len(anchors), 16)
        for anchor in anchors:
            with self.subTest(anchor=anchor["id"]):
                units = views.base_units(anchor["input"], anchor["semantics"])
                self.assertEqual([[unit.scalar, unit.start, unit.end] for unit in units],
                                 anchor["units"])

    def test_explicit_semantics_required_and_validated(self):
        with self.assertRaises(TypeError):
            views.base_units("ΟΣ")
        for mode in ("unknown", None, ""):
            with self.assertRaises(ValueError):
                views.base_units("", mode)
        with self.assertRaises(UnicodeEncodeError):
            views.base_units("\ud800", "compat-v1")

    def test_official_table_identity_and_license(self):
        manifest = json.loads((DATA / "manifest.json").read_bytes())
        self.assertEqual(manifest["unicode_version"], "16.0.0")
        self.assertEqual(hashlib.sha256((DATA / "lowercase.json").read_bytes()).hexdigest(),
                         manifest["table_sha256"])
        self.assertEqual(hashlib.sha256((DATA / "LICENSE.txt").read_bytes()).hexdigest(),
                         manifest["sources"]["LICENSE.txt"]["sha256"])
        self.assertEqual(unicode16.LOWER[0x1C89], [0x1C8A])
        self.assertTrue(unicode16.has_property(0x1C89, "Cased"))
        self.assertTrue(unicode16.has_property(0x345, "Case_Ignorable"))
        self.assertTrue(unicode16.has_property(0x345, "Cased"))
        self.assertEqual(unicode16.CONTEXTS,
                         {0x3A3: {"condition": "Final_Sigma", "lower": [0x3C2]}})

    def test_extractor_precedence_properties_and_locale_exclusion(self):
        fields = ["0041", "CAPITAL A", "Lu", "0", "L", "", "", "", "", "N",
                  "", "", "", "0061", ""]
        sources = {"UnicodeData.txt": ";".join(fields),
                   "SpecialCasing.txt": "0130; 0069 0307; 0130; 0130;\n"
                                        "0130; 0069; 0130; 0130; tr;\n"
                                        "03A3; 03C2; 03A3; 03A3; Final_Sigma;\n",
                   "DerivedCoreProperties.txt": "0041..0042 ; Cased\n"
                                                "0043 ; Cased\n"
                                                "0301 ; Case_Ignorable\n"}
        table = extract_unicode16.extract(sources)
        self.assertEqual(table["lower"], {"65": [97], "304": [105, 775]})
        self.assertEqual(table["properties"]["Cased"], [[65, 67]])
        self.assertEqual(table["contexts"]["931"]["lower"], [962])
        sources["SpecialCasing.txt"] += "0041; 0061; 0041; 0041; Unknown_Context;\n"
        with self.assertRaises(ValueError):
            extract_unicode16.extract(sources)

    def test_no_host_unicode_or_production_dependencies(self):
        forbidden_calls = {"lower", "casefold", "isalpha", "isalnum", "isupper",
                           "islower", "istitle", "isspace", "normalize", "category"}
        for name in ("unicode16.py", "extract_unicode16.py", "views.py", "expected.py",
                     "naive.py", "gap_reference.py", "generate.py", "rulepool.py"):
            tree = ast.parse((Path(__file__).parent / name).read_bytes())
            for node in ast.walk(tree):
                if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute):
                    self.assertNotIn(node.func.attr, forbidden_calls, name)
                if isinstance(node, ast.Import):
                    self.assertFalse(any(item.name in {"unicodedata", "subprocess", "ctypes"}
                                         for item in node.names), name)
                if isinstance(node, ast.ImportFrom):
                    self.assertNotIn(node.module, {"unicodedata", "subprocess", "ctypes"}, name)

    def test_reference_runs_with_host_casing_disabled(self):
        class NoHostCasing(str):
            def lower(self):
                raise AssertionError("host lowercase used")

        self.assertEqual(unicode16.lowercase(NoHostCasing("AΣ\u1c89"), "enhanced-v1"),
                         "aσ\u1c8a")


class SemanticsPropagationTests(unittest.TestCase):
    def test_body_compat_and_enhanced_are_distinct(self):
        rules = sample_rules(semantics="compat-v1")
        self.assertEqual(witnesses(expected.scan_case(sample_case("ΟΣ"), rules)),
                         [("canonical", 0, 4, "block", ["literal"])])
        rules["semantics_by_scene"]["profile_field"] = "enhanced-v1"
        self.assertEqual(expected.scan_case(sample_case("ΟΣ"), rules), [])

    def test_canonical_uses_scene_semantics(self):
        for mode, body, wanted in (("compat-v1", "οσ", True),
                                    ("enhanced-v1", "οσ", False),
                                    ("enhanced-v1", "ος", True)):
            with self.subTest(mode=mode, body=body):
                self.assertEqual(bool(expected.scan_case(sample_case(body),
                                                        sample_rules("ΟΣ", mode))), wanted)

    def test_alias_uses_scene_semantics(self):
        rules = sample_rules("unused")
        rules["literals"][0]["aliases"] = [
            {"variant_id": "greek", "pattern": "ΟΣ", "boundary": "none",
             "action": "review"}]
        self.assertEqual(witnesses(expected.scan_case(sample_case("ος"), rules)),
                         [("greek", 0, 4, "review", ["alias"])])
        self.assertEqual(expected.scan_case(sample_case("οσ"), rules), [])
        rules["semantics_by_scene"]["profile_field"] = "compat-v1"
        self.assertEqual(expected.scan_case(sample_case("οσ"), rules), [])

    def test_exception_uses_enhanced_semantics(self):
        rules = sample_rules("ος")
        rules["exceptions"] = [exception("ΟΣ")]
        self.assertEqual(expected.scan_case(sample_case("ΟΣ"), rules), [])
        rules["exceptions"][0]["phrase"] = "οσ"
        self.assertEqual(witnesses(expected.scan_case(sample_case("ΟΣ"), rules)),
                         [("canonical", 0, 4, "block", ["literal"])])

    def test_unicode16_applies_to_all_pattern_kinds(self):
        rules = sample_rules("\u1c89")
        self.assertEqual(witnesses(expected.scan_case(sample_case("\u1c8a"), rules)),
                         [("canonical", 0, 3, "block", ["literal"])])
        rules["literals"][0]["word"] = "unused"
        rules["literals"][0]["aliases"] = [
            {"variant_id": "new", "pattern": "\u1c89", "boundary": "none",
             "action": "review"}]
        self.assertEqual(witnesses(expected.scan_case(sample_case("\u1c8a"), rules)),
                         [("new", 0, 3, "review", ["alias"])])
        rules["exceptions"] = [exception("\u1c89", variants=("new",))]
        self.assertEqual(expected.scan_case(sample_case("\u1c8a"), rules), [])

    def test_exception_is_local_scoped_and_compat_gated(self):
        rules = sample_rules("ος")
        rules["exceptions"] = [exception("xΟΣ")]
        self.assertEqual(witnesses(expected.scan_case(sample_case("xΟΣ ος"), rules)),
                         [("canonical", 6, 10, "block", ["literal"])])
        for changed in ({"rule_id": "other"}, {"variant_ids": ["other"]},
                        {"scenes": ["channel_message"]}, {"views": ["T"]}):
            scoped = copy.deepcopy(rules)
            scoped["exceptions"][0].update(changed)
            self.assertEqual(len(expected.scan_case(sample_case("xΟΣ ος"), scoped)), 2)
        rules["literals"][0]["word"] = "οσ"
        rules["semantics_by_scene"]["profile_field"] = "compat-v1"
        rules["exceptions"] = [exception("ΟΣ")]
        self.assertEqual(len(expected.scan_case(sample_case("ΟΣ"), rules)), 1)

    def test_t_view_exception_normalizes_before_t2s(self):
        rules = sample_rules("ος测")
        rules["exceptions"] = [exception("ΟΣ測", view="T")]
        case = {**sample_case("ΟΣ測"), "t2s": True}
        self.assertEqual(expected.scan_case(case, rules), [])
        rules["exceptions"][0]["views"] = ["B"]
        self.assertEqual(witnesses(expected.scan_case(case, rules)),
                         [("canonical", 0, 7, "block", ["literal", "t2s"])])

    def test_exception_does_not_exempt_gap(self):
        rules = sample_rules("ος")
        rules["literals"][0]["gap"]["enabled"] = True
        rules["exceptions"] = [exception("ΟΣ")]
        self.assertEqual(witnesses(expected.scan_case({**sample_case("ΟΣ"), "gap": True},
                                                     rules)),
                         [("canonical", 0, 4, "review", ["literal", "gap"])])

    def test_exception_covers_view_units_not_just_shared_source_bytes(self):
        rules = sample_rules("İ")
        rules["exceptions"] = [exception("i")]
        self.assertEqual(witnesses(expected.scan_case(sample_case("İ"), rules)),
                         [("canonical", 0, 2, "block", ["literal"])])
        rules["exceptions"][0]["phrase"] = "İ"
        self.assertEqual(expected.scan_case(sample_case("İ"), rules), [])

    def test_semantics_mutants_are_detected(self):
        rules = sample_rules(semantics="compat-v1")
        correct = expected.scan_case(sample_case("ΟΣ"), rules)
        original = views.base_units
        with mock.patch.object(views, "base_units",
                               side_effect=lambda text, mode: original(text, "enhanced-v1")):
            wrong = expected.scan_case(sample_case("ΟΣ"), rules)
        self.assertFalse(expected.compare_hits(correct, wrong))
        for kind, pattern in (("canonical", "ΟΣ"), ("alias", "ΟΣ"),
                              ("exception", "ΟΣ")):
            with self.subTest(kind=kind):
                rules = sample_rules("ΟΣ" if kind == "canonical" else "ος")
                if kind == "alias":
                    rules["literals"][0]["word"] = "unused"
                    rules["literals"][0]["aliases"] = [
                        {"variant_id": "greek", "pattern": pattern,
                         "boundary": "none", "action": "review"}]
                elif kind == "exception":
                    rules["exceptions"] = [exception(pattern)]
                correct = expected.scan_case(sample_case("ος"), rules)
                with mock.patch.object(views, "base_units", side_effect=lambda text, mode:
                                       original(text, "compat-v1" if text == pattern else mode)):
                    wrong = expected.scan_case(sample_case("ος"), rules)
                self.assertFalse(expected.compare_hits(correct, wrong))

    def test_stale_cased_property_mutant_is_detected(self):
        original = unicode16.has_property
        def stale(point, name):
            return False if point == 0x1C89 else original(point, name)
        self.assertEqual(unicode16.lowercase("AΣ\u1c89", "enhanced-v1"), "aσ\u1c8a")
        with mock.patch.object(unicode16, "has_property", side_effect=stale):
            self.assertNotEqual(unicode16.lowercase("AΣ\u1c89", "enhanced-v1"), "aσ\u1c8a")
