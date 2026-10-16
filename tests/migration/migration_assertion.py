import json
import os
from pathlib import Path
import subprocess
import sys
import unittest


ROOT = Path(__file__).resolve().parents[2]
FORK_ROOT = Path(os.environ["TEXTGUARD_FORK_ROOT"]) if os.environ.get("TEXTGUARD_FORK_ROOT") else None
EXPECTED_FORK_ORIGIN = "https://github.com/imboy-pub/aho-corasick.git"


def git(fork, *arguments):
    return subprocess.run(["git", "-C", str(fork), *arguments],
                          capture_output=True, text=True, timeout=30)


def resolve_git_dir(reported, repository_root):
    # git may report the common dir relative to the repository, never to the
    # process working directory, so anchor relative values at the repo root.
    candidate = Path(reported)
    return (repository_root / candidate if not candidate.is_absolute() else candidate).resolve()


class MigrationAssertion(unittest.TestCase):
    """AC-113: no prior TextGuard implementation lives inside the aho-corasick fork.

    The product repository was created fresh (bootstrap commit); these checks prove
    that absence reproducibly instead of relying on a one-time survey note.
    """

    def fork(self):
        self.assertTrue(FORK_ROOT is not None and FORK_ROOT.is_dir(),
                        "TEXTGUARD_FORK_ROOT must point at the local aho-corasick checkout")
        return FORK_ROOT

    def test_fork_identity_is_the_tracked_upstream_fork(self):
        remotes = git(self.fork(), "remote", "get-url", "origin")
        self.assertEqual(remotes.returncode, 0, remotes.stderr)
        self.assertEqual(remotes.stdout.strip(), EXPECTED_FORK_ORIGIN)

    def test_no_textguard_paths_are_tracked_in_the_fork(self):
        listing = git(self.fork(), "ls-files", "-z")
        self.assertEqual(listing.returncode, 0, listing.stderr)
        tracked = [name for name in listing.stdout.split("\0") if name]
        self.assertTrue(tracked, "fork unexpectedly empty")
        offenders = [name for name in tracked if "textguard" in name.lower()]
        self.assertEqual(offenders, [], f"prior service paths still tracked: {offenders}")

    def test_no_textguard_untracked_or_ignored_residue_in_fork(self):
        status = git(self.fork(), "status", "--porcelain", "-uall", "--ignored", "-z")
        self.assertEqual(status.returncode, 0, status.stderr)
        residue = [entry for entry in status.stdout.split("\0") if entry and "textguard" in entry.lower()]
        self.assertEqual(residue, [], f"untracked/ignored textguard residue in fork: {residue}")

    def test_no_textguard_branches_or_worktrees_exist_in_the_fork(self):
        refs = git(self.fork(), "for-each-ref", "--format=%(refname)")
        self.assertEqual(refs.returncode, 0, refs.stderr)
        offenders = [name for name in refs.stdout.splitlines() if "textguard" in name.lower()]
        self.assertEqual(offenders, [], f"prior service refs found: {offenders}")
        worktrees = git(self.fork(), "worktree", "list", "--porcelain")
        self.assertEqual(worktrees.returncode, 0, worktrees.stderr)
        offenders = [line for line in worktrees.stdout.splitlines()
                     if line.startswith("worktree ") and "textguard" in line.lower()]
        self.assertEqual(offenders, [], f"prior service worktrees found: {offenders}")

    def test_product_repository_is_independent_of_the_fork(self):
        product_root = subprocess.run(["git", "-C", str(ROOT), "rev-parse", "--show-toplevel"],
                                      capture_output=True, text=True, timeout=30)
        self.assertEqual(product_root.returncode, 0, product_root.stderr)
        product = Path(product_root.stdout.strip()).resolve()
        fork = self.fork().resolve()
        self.assertNotEqual(product, fork)
        self.assertNotIn(fork, product.parents, "product repository nested inside the fork")
        self.assertNotIn(product, fork.parents, "fork nested inside the product repository")
        common_dir = subprocess.run(["git", "-C", str(ROOT), "rev-parse", "--git-common-dir"],
                                    capture_output=True, text=True, timeout=30)
        self.assertEqual(common_dir.returncode, 0, common_dir.stderr)
        shared = resolve_git_dir(common_dir.stdout.strip(), product)
        self.assertNotIn(fork, shared.parents, "product git metadata lives inside the fork")
        self.assertTrue(shared.is_dir(), "product git metadata missing")
        fork_common = git(fork, "rev-parse", "--git-common-dir")
        self.assertEqual(fork_common.returncode, 0, fork_common.stderr)
        fork_shared = resolve_git_dir(fork_common.stdout.strip(), fork)
        self.assertNotIn(product, fork_shared.parents,
                         "fork git metadata nested inside the product repository")


if __name__ == "__main__":
    arguments = [argument for argument in sys.argv[1:]]
    if arguments == ["--list"]:
        print(json.dumps(unittest.defaultTestLoader.getTestCaseNames(MigrationAssertion)))
    elif arguments:
        sys.exit(64)
    else:
        result = unittest.TextTestRunner(verbosity=2).run(
            unittest.defaultTestLoader.loadTestsFromTestCase(MigrationAssertion))
        print(json.dumps({"tests_discovered": 5, "tests_executed": result.testsRun,
                          "failures": len(result.failures), "errors": len(result.errors)}))
        sys.exit(0 if result.wasSuccessful() and result.testsRun > 0 else 1)
