import json
import os
from pathlib import Path
import re
import subprocess
import sys
import unittest


ROOT = Path(__file__).resolve().parent.parent


def registry_packages(metadata):
    root_id = metadata["resolve"]["root"]
    assert root_id and metadata["workspace_members"] == [root_id]
    external = []
    for package in metadata["packages"]:
        if package["id"] != root_id:
            assert package["source"] == "registry+https://github.com/rust-lang/crates.io-index"
            external.append(package)
    return external


class BuildContract(unittest.TestCase):
    def command(self, arguments):
        print(json.dumps({"argv": arguments, "cwd": str(ROOT)}), flush=True)
        result = subprocess.run(arguments, cwd=ROOT, capture_output=True, text=True,
                                timeout=600)
        if arguments[:2] != ["cargo", "metadata"]:
            print(result.stdout, end="")
            print(result.stderr, end="")
        if result.returncode:
            self.fail(f"{arguments}: exit {result.returncode}\n{result.stdout}\n{result.stderr}")
        return result.stdout

    def test_format(self):
        self.command(["cargo", "fmt", "--all", "--", "--check"])

    def test_lints(self):
        self.command(["cargo", "clippy", "--locked", "--all-targets", "--", "-D", "warnings"])

    def test_release_build(self):
        self.command(["cargo", "build", "--locked", "--release"])
        target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
        binary = target / "release/imboy-textguard"
        result = subprocess.run([str(binary), "--version"], capture_output=True,
                                text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("imboy-textguard", result.stdout)
        result = subprocess.run([str(binary)], capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 2)
        self.assertIn("NOT_READY", result.stderr)

    def test_registry_resolution_is_locked(self):
        metadata = json.loads(self.command(["cargo", "metadata", "--locked", "--format-version", "1"]))
        manifest = (ROOT / "Cargo.toml").read_text()
        lock = (ROOT / "Cargo.lock").read_text()
        self.assertEqual(Path(metadata["workspace_root"]).resolve(), ROOT)
        self.assertEqual(len(metadata["workspace_members"]), 1)
        self.assertNotRegex(manifest, r"(?m)^\[(?:patch|replace)(?:\]|\.)")
        external = registry_packages(metadata)
        sources = re.findall(r'^source = "([^"]+)"$', lock, re.M)
        self.assertEqual(set(sources), {"registry+https://github.com/rust-lang/crates.io-index"})
        self.assertEqual(len(re.findall(r'^checksum = "[0-9a-f]{64}"$', lock, re.M)), len(sources))
        ac = [package for package in external if package["name"] == "aho-corasick"]
        self.assertEqual([package["version"] for package in ac], ["1.1.3"])
        print(json.dumps({"dependency_mode": "registry_locked", "resolved_packages": len(external),
                          "aho_corasick": [{"version": package["version"], "source": package["source"]}
                                           for package in ac]}))

    def test_source_validator_rejects_nonroot_path(self):
        metadata = {"resolve": {"root": "product"}, "workspace_members": ["product"],
                    "packages": [{"id": "product", "source": None},
                                 {"id": "registry-ac", "source": "registry+https://github.com/rust-lang/crates.io-index"}]}
        self.assertEqual(len(registry_packages(metadata)), 1)
        metadata["packages"].append({"id": "foreign-path", "source": None})
        with self.assertRaises(AssertionError):
            registry_packages(metadata)

    def test_independent_product_root(self):
        root = self.command(["git", "rev-parse", "--show-toplevel"]).strip()
        self.assertEqual(Path(root).resolve(), ROOT)
        self.assertRegex((ROOT / "Cargo.toml").read_text(), r'(?m)^name = "imboy-textguard"$')
        for filename in ["Cargo.toml", "Cargo.lock", "build.rs", "rust-toolchain.toml"]:
            contents = (ROOT / filename).read_text()
            self.assertNotIn("/Users/", contents)
            self.assertNotIn("../aho-corasick", contents)


if __name__ == "__main__":
    if sys.argv[1:] == ["--list"]:
        print(json.dumps(unittest.defaultTestLoader.getTestCaseNames(BuildContract)))
    else:
        unittest.main(verbosity=2)
