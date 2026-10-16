"""T07 generator selftest: reproducibility and artifact consistency.

Re-runs the table generation twice from the locked OpenCC source and proves
the committed artifact is exactly what the pinned input + generator produce
(AC-022). Runs entirely in temporary directories; never rewrites the frozen
data files.
"""

import hashlib
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools/t2s"))

import generate_table  # noqa: E402

SOURCE = os.environ.get("TEXTGUARD_T2S_SOURCE")
COMMITTED_TABLE = (ROOT / "data/t2s/single_table.tsv").read_text(encoding="utf-8")
COMMITTED_REPORT = json.loads((ROOT / "data/t2s/table_report.json").read_text())


class GeneratorTests(unittest.TestCase):
    def test_locked_source_is_available(self):
        self.assertTrue(SOURCE and Path(SOURCE).is_file(),
                        "TEXTGUARD_T2S_SOURCE must point at the locked TSCharacters.txt")

    def test_regeneration_is_reproducible_and_matches_artifact(self):
        first, report_one, exclusions_one = generate_table.build_from_input(SOURCE)
        second, report_two, exclusions_two = generate_table.build_from_input(SOURCE)
        self.assertEqual(exclusions_one, exclusions_two)
        self.assertEqual(
            (ROOT / "data/t2s/excluded.tsv").read_text(encoding="utf-8"),
            exclusions_one, "committed exclusions must match regeneration")
        self.assertEqual(first, second)
        self.assertEqual(first, COMMITTED_TABLE,
                         "committed table must equal pinned-source regeneration")
        self.assertEqual(report_one["table_sha256"], report_two["table_sha256"])
        self.assertEqual(report_one, COMMITTED_REPORT)

    def test_source_digest_is_pinned_in_the_report(self):
        raw = Path(SOURCE).read_bytes()
        self.assertEqual(COMMITTED_REPORT["source_sha256"],
                         hashlib.sha256(raw).hexdigest())
        self.assertEqual(COMMITTED_REPORT["table_id"], generate_table.TABLE_ID)

    def test_selftest_flag_detects_divergence(self):
        import subprocess
        with tempfile.TemporaryDirectory(prefix="t2s-selftest-") as temporary:
            result = subprocess.run(
                [sys.executable, "-B", str(ROOT / "tools/t2s/generate_table.py"),
                 "--input", SOURCE, "--selftest", temporary],
                capture_output=True, text=True, timeout=60)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(json.loads(result.stdout)["reproducible"])

    def test_rust_fixture_table_matches_the_python_rulepool(self):
        source = (ROOT / "src/normalize/t2s.rs").read_text(encoding="utf-8")
        start = source.index("FIXTURE_TABLE_ENTRIES: &str = concat!(")
        end = source.index(");", start)
        pieces = source[start:end].split('"')[1::2]
        joined = "".join(pieces).replace("\\t", "\t").replace("\\n", "\n")
        embedded = dict(line.split("\t") for line in joined.split("\n") if "\t" in line)
        sys.path.insert(0, str(ROOT / "tests/oracle"))
        import rulepool
        self.assertEqual(embedded, dict(rulepool.TEST_T2S_TABLE))


if __name__ == "__main__":
    arguments = [argument for argument in sys.argv[1:]]
    if arguments == ["--list"]:
        print(json.dumps(unittest.defaultTestLoader.getTestCaseNames(GeneratorTests)))
    elif arguments:
        sys.exit(64)
    else:
        result = unittest.TextTestRunner(verbosity=2).run(
            unittest.defaultTestLoader.loadTestsFromTestCase(GeneratorTests))
        print(json.dumps({"tests_discovered": len(unittest.defaultTestLoader.getTestCaseNames(GeneratorTests)), "tests_executed": result.testsRun,
                          "failures": len(result.failures), "errors": len(result.errors)}))
        sys.exit(0 if result.wasSuccessful() and result.testsRun > 0 else 1)
