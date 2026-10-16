from contextlib import contextmanager
import copy
import datetime
import fcntl
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import uuid


CONTROL = Path(__file__).resolve().parent.parent
PASS_STATES = {"PASS", "PASSED", "LOCAL_CANDIDATE_PASS", "PASS_WITH_DECLARED_LIMITS"}


def process_identity(pid):
    result = subprocess.run(["/bin/ps", "-p", str(pid), "-o", "lstart=", "-o", "command="],
                            capture_output=True, text=True, timeout=2)
    if result.returncode == 1 and not result.stdout:
        return None
    if result.returncode:
        raise RuntimeError("Cannot verify process identity")
    return {"host": socket.gethostname(), "pid": pid,
            "identity": result.stdout.strip()}


@contextmanager
def coordinator_lock(control_root, session_id):
    lock_path = Path(control_root) / "locks/coordinator.json"
    lock_path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    descriptor = os.open(lock_path, os.O_RDWR | os.O_CREAT, 0o600)
    with os.fdopen(descriptor, "r+") as stream:
        try:
            fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise RuntimeError("Coordinator already locked") from error
        try:
            content = stream.read()
            previous = json.loads(content) if content else {}
            if previous and not previous.get("released"):
                if previous.get("host") != socket.gethostname():
                    raise RuntimeError("Unknown coordinator host; reconciliation required")
                actual = process_identity(previous["pid"])
                if actual is not None:
                    if previous.get("identity") != actual["identity"]:
                        raise RuntimeError("PID reuse or unverified legacy identity; reconciliation required")
                    raise RuntimeError("Coordinator still active")
            owner = {**process_identity(os.getpid()), "session_id": session_id}
            stream.seek(0)
            stream.truncate()
            json.dump(owner, stream)
            stream.flush()
            os.fsync(stream.fileno())
            try:
                yield owner
            finally:
                stream.seek(0)
                stream.truncate()
                json.dump({"released": True, "previous_owner": owner}, stream)
                stream.flush()
                os.fsync(stream.fileno())
        finally:
            fcntl.flock(stream, fcntl.LOCK_UN)


def evidence_references(item):
    references = list(item.get("evidence_paths", []))
    evidence = item.get("evidence", [])
    references.extend([evidence] if isinstance(evidence, str) else evidence)
    phases = item.get("phase_results", {})
    for phase in (phases.values() if isinstance(phases, dict) else phases):
        if isinstance(phase, dict):
            references.extend(evidence_references(phase))
    return references


def evidence_path(reference, control_root):
    candidate = Path(reference)
    return candidate if candidate.is_absolute() else Path(control_root or CONTROL) / candidate


def validate(state, require_evidence=False, control_root=None):
    if state.get("state_schema_version") not in (2, 3):
        raise ValueError("Unknown state schema; preserve original state")
    for key in ("project_id", "run_id", "session_id", "plan_digest"):
        if not isinstance(state.get(key), str) or not state[key]:
            raise ValueError(f"Missing identity: {key}")
    tasks = state.get("tasks")
    if not isinstance(tasks, dict):
        raise ValueError("tasks must be a map")
    visiting, visited = set(), set()

    def visit(task_id):
        if task_id not in tasks or task_id in visiting:
            raise ValueError("Invalid task dependency DAG")
        if task_id in visited:
            return
        visiting.add(task_id)
        for dependency in tasks[task_id].get("depends_on", []):
            visit(dependency)
        visiting.remove(task_id)
        visited.add(task_id)

    for task_id in tasks:
        visit(task_id)
    for repo in state.get("repos", {}).values():
        if repo.get("path") and not Path(repo["path"]).is_dir():
            raise ValueError("Repository path missing")
    if require_evidence:
        for namespace in ("tasks", "acceptance", "gates", "upstream_acceptance", "upstream_gates"):
            for item in state.get(namespace, {}).values():
                if isinstance(item, dict) and item.get("status") in PASS_STATES:
                    references = evidence_references(item)
                    if not references or not all(evidence_path(reference, control_root).is_file() for reference in references):
                        raise ValueError("Completed status requires existing evidence")
    return state


def reconcile(state, fingerprint, control_root=None):
    result = copy.deepcopy(state)
    for namespace in ("tasks", "acceptance", "gates", "upstream_acceptance", "upstream_gates"):
        for item in result.get(namespace, {}).values():
            if not isinstance(item, dict) or item.get("status") not in PASS_STATES:
                continue
            paths = evidence_references(item)
            digests = item.get("evidence_sha256", {})
            valid = bool(fingerprint and paths and item.get("input_fingerprint") == fingerprint)
            for reference in paths:
                evidence = evidence_path(reference, control_root)
                valid = valid and evidence.is_file()
                if valid:
                    valid = digests.get(reference) == hashlib.sha256(evidence.read_bytes()).hexdigest()
            if not valid:
                item.update(previous_status=item["status"], status="NOT_RUN",
                            evidence_validity="STALE", reason="Fingerprint or evidence not verified on resume")
            else:
                item["evidence_validity"] = "VALID"
    return result


def migrate_v2(state, plan_digest, session_id, fingerprint=None):
    validate(state)
    if state["state_schema_version"] != 2:
        raise ValueError("Expected V2 state")
    result = copy.deepcopy(state)
    result.update(state_schema_version=3, session_id=session_id, plan_version="2.1",
                  plan_digest=plan_digest)
    migration = result.setdefault("migration", {})
    migration.update(upgraded_from=2, previous_plan_digest=state["plan_digest"])
    acceptance = result.setdefault("acceptance", {})
    for number in range(109, 121):
        acceptance.setdefault(f"AC-{number:03}", {"status": "NOT_RUN", "evidence_validity": "UNKNOWN"})
    result.setdefault("upstream_acceptance", {})
    result.setdefault("upstream_gates", {})
    result.setdefault("upstream_status", {"status": "NOT_RUN"})
    return validate(reconcile(result, fingerprint))


def atomic_state(state_path, state):
    descriptor, name = tempfile.mkstemp(prefix=".state-", dir=state_path.parent)
    try:
        with os.fdopen(descriptor, "w") as stream:
            json.dump(state, stream, ensure_ascii=False, indent=2)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(name, state_path)
        directory = os.open(state_path.parent, os.O_RDONLY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        if os.path.exists(name):
            os.unlink(name)


def save(event, update, control_root=None):
    control_root = Path(control_root or CONTROL)
    state_path = control_root / "state.json"
    lock_path = control_root / "locks/checkpoint.lock"
    lock_path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    descriptor = os.open(lock_path, os.O_CREAT | os.O_RDWR, 0o600)
    with os.fdopen(descriptor, "r+") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        original = state_path.read_bytes()
        state = validate(json.loads(original))
        update(state)
        validate(state, require_evidence=True, control_root=control_root)
        stamp = datetime.datetime.now(datetime.timezone.utc).isoformat()
        transaction = uuid.uuid4().hex
        state.update(updated_at=stamp, checkpoint_event_id=transaction)
        backups = control_root / "state.backups"
        backups.mkdir(mode=0o700, exist_ok=True)
        backup = backups / (stamp.replace(":", "-") + "-" + transaction + ".json")
        with backup.open("xb") as stream:
            stream.write(original)
            stream.flush()
            os.fsync(stream.fileno())
        entry = {"at": stamp, "session_id": state["session_id"], "event": event,
                 "transaction": transaction, "backup_sha256": hashlib.sha256(original).hexdigest()}
        with (control_root / "events.jsonl").open("a") as stream:
            stream.write(json.dumps({**entry, "phase": "PREPARED"}, ensure_ascii=False) + "\n")
            stream.flush()
            os.fsync(stream.fileno())
            atomic_state(state_path, state)
            stream.write(json.dumps({**entry, "phase": "COMMITTED"}, ensure_ascii=False) + "\n")
            stream.flush()
            os.fsync(stream.fileno())
        for expired in sorted(backups.glob("*.json"))[:-12]:
            expired.unlink()
        return state
