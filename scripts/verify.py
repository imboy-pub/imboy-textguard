import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys


ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "tests/manifest.json"
TOOLS = ROOT / "scripts"


def load_helper(name):
    spec = importlib.util.spec_from_file_location(name, TOOLS / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def digest(value):
    return hashlib.sha256(value).hexdigest()


def git(*arguments):
    return subprocess.check_output(["git", "-C", str(ROOT), *arguments])


def fingerprint(registry, commands):
    changed = git("diff", "HEAD", "--binary")
    files = git("ls-files", "-z", "--cached", "--others", "--exclude-standard").split(b"\0")
    contents = {}
    for filename in sorted(set(files) - {b""}):
        source = ROOT / os.fsdecode(filename)
        contents[os.fsdecode(filename)] = digest(source.read_bytes()) if source.is_file() else "MISSING"
    helper_digests = {name: digest((TOOLS / name).read_bytes())
                      for name in ("command_receipt.py", "checkpoint.py")}
    lock = ROOT / "Cargo.lock"
    executables = {}
    for name, definition in commands.items():
        for key in ("argv", "discovery_argv"):
            argv = definition.get(key, [])
            executable = shutil.which(argv[0]) if argv else None
            executables[f"{name}#{key}"] = (
                {"path": executable, "sha256": digest(Path(executable).read_bytes())}
                if executable and Path(executable).is_file() else None)
    environment = {key: os.environ.get(key) for definition in commands.values()
                   for key in definition.get("environment_allowlist", [])}
    details = {"tree": git("rev-parse", "HEAD^{tree}").decode().strip(),
               "files": contents, "helpers": helper_digests,
               "diff": digest(changed), "manifest": digest(MANIFEST.read_bytes()),
               "commands": digest(registry.read_bytes()),
               "lock": digest(lock.read_bytes()) if lock.is_file() else None,
               "environment_digest": digest(json.dumps(environment, sort_keys=True).encode()),
               "python": sys.version, "executables": executables}
    return digest(json.dumps(details, sort_keys=True).encode()), details


def validate_argv(argv):
    return (isinstance(argv, list) and bool(argv)
            and all(isinstance(value, str) and value and "\0" not in value for value in argv)
            and Path(argv[0]).name not in {"sh", "bash", "zsh", "fish", "cmd", "powershell"})


def discover(log, count_format):
    if count_format == "unittest":
        names = json.loads(log)
        if not isinstance(names, list) or not all(isinstance(name, str) for name in names):
            raise ValueError("Discovery must return test IDs")
    elif count_format == "rust":
        names = re.findall(r"^(.+): test$", log, re.M)
    else:
        raise ValueError("Unimplemented count parser")
    if not names or len(names) != len(set(names)):
        raise ValueError("No tests discovered or duplicate test IDs")
    return names


def executed_counts(log, count_format):
    if count_format == "unittest":
        match = re.search(r"^Ran (\d+) tests? in [\d.]+s$", log, re.M)
        if not match:
            raise ValueError("Missing actual unittest execution summary")
        skipped = re.search(r"^OK \(skipped=(\d+)\)$", log, re.M)
        return int(match.group(1)) - (int(skipped.group(1)) if skipped else 0)
    if count_format == "rust":
        summaries = re.findall(r"^test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;", log, re.M)
        if not summaries:
            raise ValueError("Missing Rust test execution summary")
        return sum(int(passed) + int(failed) for passed, failed, _ in summaries)
    raise ValueError("Unimplemented count parser")


class Parser(argparse.ArgumentParser):
    def error(self, message):
        self.exit(64, f"argument error: {message}\n")


def main():
    parser = Parser()
    selector = parser.add_mutually_exclusive_group(required=True)
    selector.add_argument("--task")
    selector.add_argument("--group")
    selector.add_argument("--list", action="store_true")
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    if not args.list and args.evidence is None:
        parser.error("--evidence is required")
    if args.list and args.dry_run:
        parser.error("--list cannot be combined with --dry-run")
    if args.task and not re.fullmatch(r"T(?:[012]\d)|U0[0-5]", args.task):
        parser.error("invalid task ID")
    try:
        manifest = json.loads(MANIFEST.read_text())
        registry = (MANIFEST.parent / manifest["commands_file"]).resolve()
        registry.relative_to(ROOT)
        commands = json.loads(registry.read_text())["commands"]
        if args.list:
            print(json.dumps({"tasks": manifest["tasks"], "groups": manifest["groups"],
                              "commands": commands}, indent=2))
            return 0
        if args.group and args.group not in manifest["groups"]:
            parser.error("unknown group")
        task = manifest["tasks"].get(args.task, {}) if args.task else {}
        selected = task.get("commands", []) if args.task else manifest["groups"][args.group]
        required = task.get("acceptance", []) if args.task else sorted({
            ac for name in selected for ac in commands.get(name, {}).get("acceptance", [])})
        helper = load_helper("command_receipt")
    except (OSError, ValueError, KeyError, ImportError) as error:
        print(f"BLOCKED: runner registration or control tools unavailable ({type(error).__name__})", file=sys.stderr)
        return 2
    evidence = args.evidence.resolve()
    if evidence == ROOT or ROOT in evidence.parents:
        parser.error("evidence must be outside the source worktree")
    evidence.mkdir(parents=True, exist_ok=True, mode=0o700)
    if any(evidence.iterdir()):
        parser.error("evidence directory must be empty; preserve previous attempts")
    (evidence / "raw").mkdir(mode=0o700)
    phase = task.get("phase", "runner")
    result = {"task_id": args.task, "lane": "T", "phase": phase,
              "status": "NOT_RUN", "applicability": "local", "started_at": helper.timestamp(),
              "finished_at": None, "repo_sha_map": {}, "working_tree_diff_digest": None,
              "dependency_mode": "registry_locked", "resolved_dependency_digest": None,
              "ruleset_revision": None, "generation": None, "normalization_id": None,
              "t2s_table_id": None, "environment": {"python": sys.version, "platform": sys.platform},
              "command_argv": [], "exit_code": 2, "tests_discovered": 0, "tests_executed": 0,
              "ac_results": {ac: {"status": "NOT_RUN", "phase": phase, "applicability": "local",
                                   "reason": "No completed selector", "subresults": []} for ac in required},
              "evidence_paths": [], "limitations": manifest.get("limitations", []),
              "reviewer_type": "engineering_agent_selftest", "evidence_validity": "UNKNOWN",
              "commands": []}
    def interrupt(signum, frame):
        raise helper.Interrupted(signum)

    handlers = {signum: signal.signal(signum, interrupt) for signum in (signal.SIGINT, signal.SIGTERM)}
    try:
        selected_commands = {name: commands.get(name, {}) for name in selected}
        before, details = fingerprint(registry, selected_commands)
        result.update(input_fingerprint=before, fingerprint_details=details,
                      repo_sha_map={str(ROOT): git("rev-parse", "HEAD").decode().strip()},
                      working_tree_diff_digest=details["diff"], resolved_dependency_digest=details["lock"])
        result["control_validation"] = {
            "applicability": "explicit_control_group" if args.group == "control" else "NOT_APPLICABLE",
            "reason": "External control verification requires --group control and TEXTGUARD_CONTROL_TOOLS; ordinary runner tests use repository helpers only"}
        if not selected:
            result.update(status="BLOCKED", reason="Task implementation not registered")
        elif args.dry_run:
            result.update(status="NOT_RUN", reason="Dry run; no command executed")
            result["command_argv"] = [commands.get(name, {}).get("argv") for name in selected]
        else:
            result.update(status="PASS", exit_code=0)
            for index, name in enumerate(selected):
                definition = commands.get(name, {})
                entry = {"id": name, "status": "BLOCKED", "tests_discovered": 0, "tests_executed": 0}
                result["commands"].append(entry)
                argv, discovery = definition.get("argv"), definition.get("discovery_argv")
                if any(not os.environ.get(key) for key in definition.get("required_environment", [])):
                    entry["reason"] = "Explicit control environment missing; no automatic fallback"
                elif not validate_argv(argv) or not validate_argv(discovery):
                    entry["reason"] = "Executable argv or discovery not implemented"
                else:
                    cwd = (ROOT / definition.get("cwd", ".")).resolve()
                    cwd.relative_to(ROOT)
                    options = {"timeout": definition.get("timeout_seconds", 300),
                               "env_allowlist": definition.get("environment_allowlist", [])}
                    result["command_argv"].extend([discovery, argv])
                    counts = helper.run_command(discovery, cwd, evidence / f"raw/{index}-discover", **options)
                    entry["discovery_receipt"] = counts
                    blocked = counts["exit_code"] in definition.get("blocked_exit_codes", [])
                    if counts["status"] == "PASS":
                        try:
                            names = discover(Path(counts["log"]).read_text(), definition.get("count_format"))
                            entry["test_ids"] = names
                            entry["tests_discovered"] = len(names)
                            actual = helper.run_command(argv, cwd, evidence / f"raw/{index}-execute", **options)
                            entry["execution_receipt"] = actual
                            entry["status"] = ("BLOCKED" if actual["exit_code"] in definition.get("blocked_exit_codes", [])
                                               else actual["status"])
                            try:
                                count = executed_counts(Path(actual["log"]).read_text(), definition.get("count_format"))
                                entry["tests_executed"] = count
                                if actual["status"] == "PASS" and (count <= 0 or count != len(names)):
                                    raise ValueError("Zero, skipped, filtered or mismatched tests executed")
                            except ValueError:
                                if actual["status"] == "PASS":
                                    raise
                        except (ValueError, OSError) as error:
                            entry.update(status="FAIL", reason=str(error))
                    else:
                        entry["status"] = "BLOCKED" if blocked else counts["status"]
                if entry["status"] != "PASS":
                    if entry["status"] != "BLOCKED":
                        result.update(status="FAIL", exit_code=1)
                    elif result["status"] != "FAIL":
                        result.update(status="BLOCKED", exit_code=2)
                for ac in definition.get("acceptance", []):
                    if ac in result["ac_results"]:
                        result["ac_results"][ac]["subresults"].append({
                            "selector": name, "status": entry["status"],
                            "tests_discovered": entry["tests_discovered"], "tests_executed": entry["tests_executed"]})
                result["tests_discovered"] += entry["tests_discovered"]
                result["tests_executed"] += entry["tests_executed"]
                if entry["status"] == "INTERRUPTED":
                    interrupted = entry.get("execution_receipt", entry["discovery_receipt"])
                    result["exit_code"] = interrupted["outcome_exit_code"]
                    break
            for ac in result["ac_results"].values():
                statuses = [item["status"] for item in ac["subresults"]]
                ac.update(status=("PASS" if statuses and set(statuses) == {"PASS"} else
                                  "FAIL" if any(status in {"FAIL", "INTERRUPTED"} for status in statuses) else "BLOCKED"),
                          reason="Actual selector receipts" if statuses else "Required selector not registered")
            if result["status"] == "PASS" and any(ac["status"] != "PASS" for ac in result["ac_results"].values()):
                result.update(status="BLOCKED", exit_code=2, reason="Required AC selectors not registered")
            if result["status"] == "PASS" and result["tests_executed"] == 0:
                result.update(status="FAIL", exit_code=1, reason="Zero tests executed")
            if task.get("acceptance_scope") == "phase":
                result["acceptance_scope"] = "phase"
                for ac in result["ac_results"].values():
                    completed_phase = dict(ac)
                    ac.update(status="NOT_RUN" if ac["status"] == "PASS" else ac["status"],
                              phase="aggregate", reason="Final acceptance requires remaining phases",
                              phase_results={phase: completed_phase})
        after, _ = fingerprint(registry, selected_commands)
        result["fingerprint_after"] = after
        result["evidence_validity"] = "VALID" if before == after else "STALE"
        if before != after:
            result.update(status="FAIL", exit_code=1, reason="Inputs changed during execution")
    except (KeyboardInterrupt, helper.Interrupted) as error:
        signum = getattr(error, "signum", signal.SIGINT)
        result.update(status="FAIL", exit_code=128 + signum, reason="Runner interrupted")
    except Exception as error:
        result.update(status="FAIL", exit_code=1, reason=helper.redactor()(f"{type(error).__name__}: {error}"))
    finally:
        for signum in handlers:
            signal.signal(signum, signal.SIG_IGN)
        result["finished_at"] = helper.timestamp()
        result["evidence_paths"] = [str(evidence / filename) for filename in ("result.json", "commands.log", "test-counts.json")]
        helper.atomic_json(evidence / "test-counts.json", {
            "tests_discovered": result["tests_discovered"], "tests_executed": result["tests_executed"],
            "selectors": [{key: entry.get(key) for key in ("id", "tests_discovered", "tests_executed", "test_ids")}
                          for entry in result["commands"]]})
        (evidence / "commands.log").write_text("\n".join(json.dumps(argv) for argv in result["command_argv"]) + "\n")
        helper.atomic_json(evidence / "result.json", result)
        for signum, previous in handlers.items():
            signal.signal(signum, previous)
    print(json.dumps({"status": result["status"], "exit_code": result["exit_code"],
                      "tests_discovered": result["tests_discovered"], "tests_executed": result["tests_executed"],
                      "evidence": str(evidence)}))
    return result["exit_code"]


if __name__ == "__main__":
    sys.exit(main())
