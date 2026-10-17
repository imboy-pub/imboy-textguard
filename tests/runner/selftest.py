import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
CONTROL_MODE = "--control" in sys.argv[1:]
if CONTROL_MODE and not os.environ.get("TEXTGUARD_CONTROL_TOOLS"):
    print("BLOCKED: explicit control tools directory is required", file=sys.stderr)
    sys.exit(2)
TOOLS = Path(os.environ["TEXTGUARD_CONTROL_TOOLS"]) if CONTROL_MODE else ROOT / "scripts"
if CONTROL_MODE:
    for helper_name in ("command_receipt.py", "checkpoint.py"):
        external = TOOLS / helper_name
        if not external.is_file() or external.read_bytes() != (ROOT / "scripts" / helper_name).read_bytes():
            print("BLOCKED: external control helper differs from tested repository implementation", file=sys.stderr)
            sys.exit(2)


def load_module(name):
    spec = importlib.util.spec_from_file_location(name, TOOLS / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


command_receipt = load_module("command_receipt")
checkpoint = load_module("checkpoint")


class Sandbox(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="textguard-runner-")
        self.addCleanup(self.temporary.cleanup)
        self.sandbox = Path(self.temporary.name)

    def command(self, argv, timeout=2):
        return command_receipt.run_command(argv, self.sandbox,
                                           self.sandbox / "command", timeout,
                                           ["PATH"], grace=0.1)

    def assert_terminal(self, result):
        self.assertTrue(result["terminal"])
        self.assertTrue(result["cleanup_complete"])
        self.assertTrue(result["finished_at"])
        self.assertEqual(json.loads((self.sandbox / "command.json").read_text()), result)


class ProcessTests(Sandbox):
    def test_rust_discovery_preserves_safe_ids_and_still_redacts_known_values(self):
        with patch.dict(os.environ, {"RUNNER_TEST_SECRET": "sensitive_test_name"}):
            redact = command_receipt.redactor()
            self.assertEqual(redact("first_state_token: test\n"), "first_state_token: test\n")
            self.assertNotIn("sensitive_test_name", redact("sensitive_test_name: test\n"))
            self.assertNotIn("hidden", redact("token: hidden\n"))

    def test_empty_argv_terminal(self):
        result = self.command([])
        self.assertEqual(result["outcome_exit_code"], 64)
        self.assertIsNone(result["exit_code"])
        self.assert_terminal(result)

    def test_spawn_failure_terminal(self):
        result = self.command([str(self.sandbox / "not-installed")])
        self.assertEqual(result["status"], "BLOCKED")
        self.assertEqual(result["outcome_exit_code"], 2)
        self.assert_terminal(result)

    def test_actual_nonzero_exit_preserved(self):
        result = self.command([sys.executable, "-c", "raise SystemExit(37)"])
        self.assertEqual(result["exit_code"], 37)
        self.assertEqual(result["outcome_exit_code"], 1)
        self.assert_terminal(result)

    def test_timeout_term_reap(self):
        result = self.command([sys.executable, "-B", str(ROOT / "tests/runner/process_fixture.py"), "normal"], .3)
        self.assertTrue(result["timed_out"])
        self.assertEqual(result["exit_code"], -signal.SIGTERM)
        self.assert_terminal(result)

    def test_ignored_term_kill_reap(self):
        result = self.command([sys.executable, "-B", str(ROOT / "tests/runner/process_fixture.py"), "ignore-term"], .3)
        self.assertEqual(result["exit_code"], -signal.SIGKILL)
        self.assertIn("SIGKILL", result["cleanup_signals"])
        self.assert_terminal(result)

    def test_descendant_and_exited_parent_cleanup(self):
        for mode in ("descendant", "orphan"):
            with self.subTest(mode=mode):
                result = command_receipt.run_command(
                    [sys.executable, "-B", str(ROOT / "tests/runner/process_fixture.py"), mode],
                    self.sandbox, self.sandbox / mode, .4, ["PATH"], grace=.1)
                log = Path(result["log"]).read_text()
                child_pid = re.search(r"DESCENDANT_PID=(\d+)", log).group(1)
                state = subprocess.run(["/bin/ps", "-p", child_pid, "-o", "stat="], capture_output=True, text=True)
                self.assertTrue(not state.stdout.strip() or state.stdout.strip().startswith("Z"), state.stdout)
                self.assertTrue(result["cleanup_complete"])

    def test_interrupt_terminal_receipt(self):
        for signum in (signal.SIGINT, signal.SIGTERM):
            with self.subTest(signum=signum):
                stem = self.sandbox / f"interrupt-{signum}"
                process = subprocess.Popen(
                    [sys.executable, "-B", str(TOOLS / "command_receipt.py"), "--cwd", str(self.sandbox),
                     "--out", str(stem), "--", sys.executable, "-B",
                     str(ROOT / "tests/runner/process_fixture.py"), "ignore-term"],
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                try:
                    deadline = time.monotonic() + 5
                    while time.monotonic() < deadline:
                        if stem.with_suffix(".log").exists() and "READY_PID" in stem.with_suffix(".log").read_text():
                            break
                        time.sleep(.02)
                    else:
                        self.fail("Owned fixture never became ready")
                    process.send_signal(signum)
                    process.communicate(timeout=5)
                    receipt = json.loads(stem.with_suffix(".json").read_text())
                    self.assertEqual(process.returncode, 128 + signum)
                    self.assertEqual(receipt["status"], "INTERRUPTED")
                    self.assertEqual(receipt["exit_code"], -signal.SIGKILL)
                    self.assertTrue(receipt["terminal"] and receipt["cleanup_complete"])
                finally:
                    if process.poll() is None:
                        process.kill()
                        process.communicate(timeout=3)

    def test_no_overwrite_previous_receipt(self):
        result = self.command([sys.executable, "-c", "print('first')"])
        original = (self.sandbox / "command.json").read_bytes()
        with self.assertRaises(FileExistsError):
            self.command([sys.executable, "-c", "print('second')"])
        self.assertEqual((self.sandbox / "command.json").read_bytes(), original)
        self.assert_terminal(result)

    def test_environment_whitelist_and_redaction(self):
        with patch.dict(os.environ, {"RUNNER_TEST_SECRET": "synthetic-secret-canary"}):
            result = self.command([sys.executable, "-c",
                                   "import os; assert 'RUNNER_TEST_SECRET' not in os.environ; print('secret=fixture')"])
        self.assertEqual(result["exit_code"], 0)
        self.assertNotIn("fixture", Path(result["log"]).read_text())
        self.assertNotIn("RUNNER_TEST_SECRET", result["environment_keys"])


class CheckpointTests(Sandbox):
    def state(self):
        return {"state_schema_version": 3, "project_id": "synthetic-project", "run_id": "synthetic-run",
                "session_id": "session-old", "plan_digest": "plan-old", "repos": {},
                "tasks": {"T00": {"status": "NOT_RUN", "depends_on": [], "retry_count": 2}},
                "acceptance": {}, "gates": {}, "custom_legacy": {"keep": [1, 2]}}

    def initialize(self, state=None):
        source = state or self.state()
        (self.sandbox / "state.json").write_text(json.dumps(source))
        return source

    def test_atomic_checkpoint_and_append_events(self):
        original = self.initialize()
        checkpoint.save("synthetic-save", lambda state: state.update(marker="saved"), self.sandbox)
        result = json.loads((self.sandbox / "state.json").read_text())
        self.assertEqual(result["marker"], "saved")
        backups = list((self.sandbox / "state.backups").glob("*.json"))
        self.assertEqual(json.loads(backups[0].read_text()), original)
        events = [json.loads(line) for line in (self.sandbox / "events.jsonl").read_text().splitlines()]
        self.assertEqual([event["phase"] for event in events], ["PREPARED", "COMMITTED"])
        self.assertEqual(events[-1]["transaction"], result["checkpoint_event_id"])

    def test_interrupted_replace_preserves_recovery_point(self):
        original = self.initialize()
        with patch.object(checkpoint.os, "replace", side_effect=OSError("synthetic interrupted replace")):
            with self.assertRaises(OSError):
                checkpoint.save("interrupted", lambda state: state.update(marker="new"), self.sandbox)
        self.assertEqual(json.loads((self.sandbox / "state.json").read_text()), original)
        self.assertTrue(list((self.sandbox / "state.backups").glob("*.json")))
        self.assertFalse(list(self.sandbox.glob(".state-*")))

    def test_invalid_dag_and_missing_evidence_refused(self):
        self.initialize()
        before = (self.sandbox / "state.json").read_bytes()
        for update in (lambda state: state["tasks"]["T00"].update(depends_on=["T00"]),
                       lambda state: state["tasks"]["T00"].update(status="PASS", evidence_paths=[str(self.sandbox / "missing")])):
            with self.assertRaises(ValueError):
                checkpoint.save("invalid", update, self.sandbox)
            self.assertEqual((self.sandbox / "state.json").read_bytes(), before)

    def test_v2_migration_preserves_history_identity_retries(self):
        old = self.state()
        old.update(state_schema_version=2, failure_root_causes={"old": {"retry_count": 2}},
                   branches={"legacy": "old-sha"})
        old["tasks"]["T00"].update(status="PASS", evidence_paths=[str(self.sandbox / "missing")])
        original = copy.deepcopy(old)
        upgraded = checkpoint.migrate_v2(old, "plan-new", "session-new")
        self.assertEqual(old, original)
        for key in ("project_id", "run_id", "failure_root_causes", "branches", "custom_legacy"):
            self.assertEqual(upgraded[key], old[key])
        self.assertEqual(upgraded["tasks"]["T00"]["retry_count"], 2)
        self.assertEqual(upgraded["tasks"]["T00"]["previous_status"], "PASS")
        self.assertEqual(upgraded["tasks"]["T00"]["status"], "NOT_RUN")
        self.assertIn("AC-120", upgraded["acceptance"])
        self.assertIn("upstream_acceptance", upgraded)
        self.assertEqual(upgraded["migration"]["upgraded_from"], 2)

    def test_schema3_relative_evidence_and_existing_summary_preserved(self):
        state = self.state()
        (self.sandbox / "prior.json").write_text('{"status":"PASS"}')
        state["tasks"]["T00"].update(status="PASS", evidence_paths=["prior.json"])
        state["acceptance"] = {"AC-001": {"status": "PASS", "phase_results": {
            "baseline": {"status": "PASS", "evidence": ["prior.json"]}}}}
        state["gates"] = {"G0": {"status": "PASS", "evidence_paths": ["prior.json"]}}
        self.initialize(state)
        result = checkpoint.save("schema3-preserve", lambda value: value.update(marker="saved"), self.sandbox)
        self.assertEqual(result["state_schema_version"], 3)
        for key in ("tasks", "acceptance", "gates"):
            self.assertEqual(result[key], state[key])
        with self.assertRaises(ValueError):
            checkpoint.save("new-pass-without-proof", lambda value: value["gates"].update(G1={"status": "PASS"}), self.sandbox)

    def test_double_start_and_pid_reuse_refused(self):
        with checkpoint.coordinator_lock(self.sandbox, "one"):
            with self.assertRaises(RuntimeError):
                with checkpoint.coordinator_lock(self.sandbox, "two"):
                    self.fail("Second coordinator acquired lock")
        lock = self.sandbox / "locks/coordinator.json"
        lock.write_text(json.dumps({"host": socket.gethostname(), "pid": os.getpid(),
                                    "identity": "old-process-with-reused-pid", "expired_at": "2000-01-01"}))
        with self.assertRaisesRegex(RuntimeError, "PID reuse"):
            with checkpoint.coordinator_lock(self.sandbox, "three"):
                self.fail("PID reuse admitted")

    def test_live_coordinator_without_flock_still_refused(self):
        lock = self.sandbox / "locks/coordinator.json"
        lock.parent.mkdir()
        lock.write_text(json.dumps(checkpoint.process_identity(os.getpid())))
        with self.assertRaisesRegex(RuntimeError, "still active"):
            with checkpoint.coordinator_lock(self.sandbox, "two"):
                self.fail("Existing active process admitted")

    def test_restart_never_blindly_inherits_pass(self):
        state = self.state()
        evidence = self.sandbox / "result.json"
        evidence.write_text('{"terminal":true,"exit_code":0}')
        item = state["tasks"]["T00"]
        item.update(status="PASS", input_fingerprint="candidate-one", evidence_paths=[str(evidence)],
                    evidence_sha256={str(evidence): hashlib.sha256(evidence.read_bytes()).hexdigest()})
        self.assertEqual(checkpoint.reconcile(state, "candidate-one")["tasks"]["T00"]["status"], "PASS")
        self.assertEqual(checkpoint.reconcile(state, "candidate-two")["tasks"]["T00"]["status"], "NOT_RUN")
        evidence.write_text("changed")
        self.assertEqual(checkpoint.reconcile(state, "candidate-one")["tasks"]["T00"]["status"], "NOT_RUN")
        evidence.unlink()
        self.assertEqual(checkpoint.reconcile(state, "candidate-one")["tasks"]["T00"]["status"], "NOT_RUN")


class RunnerTests(Sandbox):
    def test_measurement_phase_does_not_pass_aggregate_acceptance(self):
        response = self.invoke("--task", "T18", "--evidence", str(self.evidence))
        self.assertEqual(response.returncode, 0)
        result = json.loads((self.evidence / "result.json").read_text())
        self.assertEqual((result["status"], result["phase"], result["acceptance_scope"]),
                         ("PASS", "T18_measurement", "phase"))
        for acceptance in result["ac_results"].values():
            self.assertEqual(acceptance["status"], "NOT_RUN")
            self.assertEqual(acceptance["phase_results"]["T18_measurement"]["status"], "PASS")

    def setUp(self):
        super().setUp()
        self.source = self.sandbox / "source"
        subprocess.run(["git", "clone", "--shared", "--quiet", str(ROOT), str(self.source)],
                       check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        for name in ("scripts", "tests"):
            shutil.copytree(ROOT / name, self.source / name, dirs_exist_ok=True)
        self.registry = self.source / "tests/runner/commands.json"
        data = json.loads(self.registry.read_text())
        for name, definition in data["commands"].items():
            definition.update(argv=[sys.executable, "-B", "tests/runner/tiny.py"],
                              discovery_argv=[sys.executable, "-B", "tests/runner/tiny.py", "--list"],
                              timeout_seconds=5)
            if not name.startswith("control"):
                definition.pop("required_environment", None)
        self.registry.write_text(json.dumps(data))
        self.fixture = self.source / "tests/runner/tiny.py"
        self.fixture.write_text(
            "import json, sys, unittest\n"
            "class Tiny(unittest.TestCase):\n    def test_ok(self): self.assertEqual(2 + 2, 4)\n"
            "if '--list' in sys.argv: print(json.dumps(['__main__.Tiny.test_ok']))\n"
            "else: unittest.main()\n")
        self.evidence = self.sandbox / "evidence"

    def invoke(self, *arguments, control_tools=None):
        environment = {"PATH": os.environ["PATH"]}
        if control_tools is not None:
            environment["TEXTGUARD_CONTROL_TOOLS"] = str(control_tools)
        return subprocess.run([str(self.source / "scripts/verify_textguard.sh"), *arguments],
                              cwd=self.source, env=environment,
                              capture_output=True, text=True, timeout=15)

    def verify(self):
        response = self.invoke("--group", "runner", "--evidence", str(self.evidence))
        result = json.loads((self.evidence / "result.json").read_text())
        return response, result

    def test_list_and_argument_exit64(self):
        listed = self.invoke("--list")
        self.assertEqual(listed.returncode, 0)
        self.assertIn("runner-selftest", json.loads(listed.stdout)["commands"])
        for arguments in ([], ["--task", "typo"], ["--group", "unknown", "--evidence", str(self.evidence)],
                          ["--group", "runner", "--evidence", str(self.evidence), "--unknown"]):
            self.assertEqual(self.invoke(*arguments).returncode, 64)

    def test_real_counts_fingerprint_and_ac(self):
        response, result = self.verify()
        self.assertEqual(response.returncode, 0, response.stdout + response.stderr)
        self.assertEqual(result["tests_discovered"], 1)
        self.assertEqual(result["tests_executed"], 1)
        self.assertEqual(result["ac_results"]["AC-010"]["status"], "PASS")
        self.assertEqual(result["input_fingerprint"], result["fingerprint_after"])
        self.assertEqual(result["commands"][0]["execution_receipt"]["exit_code"], 0)

    def test_product_runner_independent_of_control(self):
        response = self.invoke("--group", "runner", "--evidence", str(self.evidence),
                               control_tools=self.sandbox / "unavailable-personal-layout")
        self.assertEqual(response.returncode, 0, response.stdout + response.stderr)
        result = json.loads((self.evidence / "result.json").read_text())
        self.assertEqual(result["control_validation"]["applicability"], "NOT_APPLICABLE")
        self.assertEqual(result["tests_executed"], 1)

    def test_external_control_is_explicit_and_blocked_without_env(self):
        response = self.invoke("--group", "control", "--evidence", str(self.evidence))
        self.assertEqual(response.returncode, 2)
        result = json.loads((self.evidence / "result.json").read_text())
        self.assertEqual(result["status"], "BLOCKED")
        self.assertEqual(result["tests_executed"], 0)

    def test_missing_task_and_required_ac_blocked(self):
        manifest = json.loads((self.source / "tests/manifest.json").read_text())
        unregistered = next(f"T{number:02d}" for number in range(30)
                            if f"T{number:02d}" not in manifest["tasks"])
        response = self.invoke("--task", unregistered, "--evidence", str(self.evidence))
        self.assertEqual(response.returncode, 2)
        result = json.loads((self.evidence / "result.json").read_text())
        self.assertEqual(result["status"], "BLOCKED")
        registry = json.loads(self.registry.read_text())
        manifest = json.loads((self.source / "tests/manifest.json").read_text())
        removed = registry["commands"].pop("t04-build-contract")
        manifest["tasks"]["T04"]["commands"].remove("t04-build-contract")
        self.registry.write_text(json.dumps(registry))
        (self.source / "tests/manifest.json").write_text(json.dumps(manifest))
        response = self.invoke("--task", "T04", "--evidence", str(self.sandbox / "t04"))
        self.assertEqual(response.returncode, 2)
        result = json.loads((self.sandbox / "t04/result.json").read_text())
        self.assertEqual(result["ac_results"]["AC-009"]["status"], "BLOCKED")
        self.assertEqual(result["ac_results"]["AC-010"]["status"], "PASS")
        self.assertTrue(removed["scope"].startswith("Frozen toolchain build gate"))

    def test_result_failure_not_downgraded_by_later_block(self):
        self.fixture.write_text(self.fixture.read_text().replace("2 + 2, 4", "2 + 2, 5"))
        registry = json.loads(self.registry.read_text())
        registry["commands"]["t04-build-contract"]["discovery_argv"] = [str(self.sandbox / "missing-tool")]
        self.registry.write_text(json.dumps(registry))
        response = self.invoke("--task", "T04", "--evidence", str(self.evidence))
        self.assertEqual(response.returncode, 1)
        result = json.loads((self.evidence / "result.json").read_text())
        self.assertEqual(result["status"], "FAIL")
        statuses = {command["id"]: command["status"] for command in result["commands"]}
        self.assertEqual(statuses["runner-selftest"], "FAIL")
        self.assertEqual(statuses["t04-build-contract"], "BLOCKED")

    def test_dry_run_not_pass_and_no_execution(self):
        response = self.invoke("--group", "runner", "--dry-run", "--evidence", str(self.evidence))
        self.assertEqual(response.returncode, 2)
        result = json.loads((self.evidence / "result.json").read_text())
        self.assertEqual(result["status"], "NOT_RUN")
        self.assertEqual(result["tests_executed"], 0)
        self.assertFalse(list((self.evidence / "raw").iterdir()))

    def test_zero_tests_fails(self):
        self.fixture.write_text("print('[]')\n")
        response, result = self.verify()
        self.assertEqual(response.returncode, 1)
        self.assertEqual(result["tests_discovered"], 0)
        self.assertEqual(result["status"], "FAIL")

    def test_pass_only_output_fails(self):
        self.fixture.write_text("import json,sys\nprint(json.dumps(['fake']) if '--list' in sys.argv else 'PASS')\n")
        response, result = self.verify()
        self.assertEqual(response.returncode, 1)
        self.assertEqual(result["tests_executed"], 0)

    def test_nonzero_exit_not_swallowed(self):
        self.fixture.write_text("import json,sys\nprint(json.dumps(['fake'])) if '--list' in sys.argv else sys.exit(37)\n")
        response, result = self.verify()
        self.assertEqual(response.returncode, 1)
        self.assertEqual(result["commands"][0]["execution_receipt"]["exit_code"], 37)

    def test_failed_test_still_counts_as_executed(self):
        self.fixture.write_text(self.fixture.read_text().replace("2 + 2, 4", "2 + 2, 5"))
        response, result = self.verify()
        self.assertEqual(response.returncode, 1)
        self.assertEqual(result["tests_discovered"], 1)
        self.assertEqual(result["tests_executed"], 1)

    def test_missing_executable_blocked(self):
        registry = json.loads(self.registry.read_text())
        registry["commands"]["runner-selftest"]["discovery_argv"] = [str(self.sandbox / "missing")]
        self.registry.write_text(json.dumps(registry))
        response, result = self.verify()
        self.assertEqual(response.returncode, 2)
        self.assertEqual(result["status"], "BLOCKED")

    def test_shell_argv_rejected(self):
        registry = json.loads(self.registry.read_text())
        registry["commands"]["runner-selftest"]["argv"] = ["sh", "-c", "echo PASS"]
        self.registry.write_text(json.dumps(registry))
        response, result = self.verify()
        self.assertEqual(response.returncode, 2)
        self.assertEqual(result["tests_executed"], 0)

    def test_evidence_overwrite_refused(self):
        self.verify()
        original = (self.evidence / "result.json").read_bytes()
        response = self.invoke("--group", "runner", "--evidence", str(self.evidence))
        self.assertEqual(response.returncode, 64)
        self.assertEqual((self.evidence / "result.json").read_bytes(), original)

    def test_changed_candidate_invalidates_result(self):
        self.fixture.write_text(self.fixture.read_text().replace(
            "self.assertEqual(2 + 2, 4)", "__import__('pathlib').Path('mutation').write_text('fixture')"))
        response, result = self.verify()
        self.assertEqual(response.returncode, 1)
        self.assertEqual(result["evidence_validity"], "STALE")

    def test_runner_interrupt_during_discovery_terminal(self):
        self.fixture.write_text("import time\nprint('fixture-ready', flush=True)\ntime.sleep(30)\n")
        process = subprocess.Popen(
            [str(self.source / "scripts/verify_textguard.sh"), "--group", "runner", "--evidence", str(self.evidence)],
            cwd=self.source, env={"PATH": os.environ["PATH"]},
            stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            deadline = time.monotonic() + 5
            log = self.evidence / "raw/0-discover.log"
            while time.monotonic() < deadline:
                if log.exists() and "fixture-ready" in log.read_text():
                    break
                time.sleep(.02)
            else:
                self.fail("Discovery fixture not ready")
            process.send_signal(signal.SIGTERM)
            process.communicate(timeout=5)
            result = json.loads((self.evidence / "result.json").read_text())
            self.assertEqual(process.returncode, 143)
            self.assertEqual(result["exit_code"], 143)
            self.assertEqual(result["status"], "FAIL")
            self.assertTrue(result["finished_at"])
            self.assertTrue(result["commands"][0]["discovery_receipt"]["cleanup_complete"])
        finally:
            if process.poll() is None:
                process.send_signal(signal.SIGINT)
                process.communicate(timeout=5)


def flatten(suite):
    for item in suite:
        if isinstance(item, unittest.TestSuite):
            yield from flatten(item)
        else:
            yield item.id()


if __name__ == "__main__":
    suite = (unittest.defaultTestLoader.loadTestsFromTestCase(CheckpointTests) if CONTROL_MODE else
             unittest.defaultTestLoader.loadTestsFromModule(sys.modules[__name__]))
    discovered = list(flatten(suite))
    arguments = [argument for argument in sys.argv[1:] if argument != "--control"]
    if arguments == ["--list"]:
        print(json.dumps(discovered))
    elif arguments:
        sys.exit(64)
    else:
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        print(json.dumps({"tests_discovered": len(discovered), "tests_executed": result.testsRun,
                          "failures": len(result.failures), "errors": len(result.errors), "skipped": len(result.skipped)}))
        sys.exit(0 if result.wasSuccessful() and result.testsRun > 0 else 1)
