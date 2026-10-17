import argparse
import datetime
import json
import math
import os
from pathlib import Path
import re
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time


ALLOWED_ENV = frozenset({"PATH", "LANG", "LC_ALL", "TZ", "TMPDIR", "HOME",
                         "RUSTUP_HOME", "CARGO_HOME", "CARGO_TARGET_DIR",
                         "PYTHONDONTWRITEBYTECODE", "TEXTGUARD_CONTROL_TOOLS",
                         "TEXTGUARD_FORK_ROOT", "TEXTGUARD_T2S_SOURCE"})


def timestamp():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def atomic_json(destination, value):
    destination = Path(destination)
    descriptor, temporary = tempfile.mkstemp(prefix=".receipt-", dir=destination.parent)
    try:
        with os.fdopen(descriptor, "w") as stream:
            json.dump(value, stream, ensure_ascii=False, indent=2)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, destination)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def redactor():
    values = [value for key, value in os.environ.items()
              if re.search(r"SECRET|TOKEN|PASSWORD|CREDENTIAL|PRIVATE_KEY|DATABASE_URL|API_KEY|DSN|COOKIE", key, re.I)
              and len(value) >= 4]

    def redact(text):
        for value in values:
            text = text.replace(value, "[REDACTED]")
        if re.fullmatch(r"[A-Za-z_][A-Za-z0-9_:]*: test\s*", text):
            return text
        return re.sub(r"(?i)(password|token|secret|authorization|api[_-]?key|dsn|cookie)(\s*[:=]\s*)\S+",
                      r"\1\2[REDACTED]", text)

    return redact


class Interrupted(Exception):
    def __init__(self, signum):
        self.signum = signum


def terminate_group(process, grace):
    actions = []
    errors = []

    def live_members():
        listing = subprocess.run(["/bin/ps", "-axo", "pid=,pgid=,stat="],
                                 capture_output=True, text=True, timeout=2)
        if listing.returncode:
            raise RuntimeError("Cannot verify owned process group cleanup")
        return [line for line in listing.stdout.splitlines()
                if len(line.split()) == 3 and line.split()[1] == str(process.pid)
                and not line.split()[2].startswith("Z")]

    for signum in (signal.SIGTERM, signal.SIGKILL):
        try:
            if not live_members():
                break
            os.killpg(process.pid, signum)
            actions.append(signal.Signals(signum).name)
        except ProcessLookupError:
            break
        except (OSError, RuntimeError, subprocess.TimeoutExpired) as error:
            errors.append(type(error).__name__)
        if signum == signal.SIGTERM:
            time.sleep(grace)
    try:
        process.wait(timeout=max(1, grace))
        complete = not live_members()
    except (OSError, RuntimeError, subprocess.TimeoutExpired) as error:
        errors.append(type(error).__name__)
        complete = False
    return actions, errors, complete


def run_command(argv, cwd, output, timeout=300, env_allowlist=(), grace=0.3):
    output = Path(output)
    output.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    receipt_path = output.with_suffix(".json")
    log_path = output.with_suffix(".log")
    descriptor = os.open(receipt_path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    os.close(descriptor)
    redact = redactor()
    receipt = {"argv": argv, "cwd": str(cwd), "started_at": timestamp(),
               "timeout_seconds": timeout, "status": "RUNNING", "terminal": False,
               "exit_code": None, "outcome_exit_code": 1, "pid": None,
               "environment_keys": [], "log": str(log_path), "cleanup_complete": False}
    atomic_json(receipt_path, receipt)
    process = None
    reader = None
    started = time.monotonic()
    previous_handlers = {}

    def interrupt(signum, frame):
        raise Interrupted(signum)

    try:
        if (not isinstance(argv, list) or not argv
                or not all(isinstance(argument, str) and argument and "\0" not in argument for argument in argv)
                or not isinstance(timeout, (int, float)) or not math.isfinite(timeout) or timeout <= 0
                or not set(env_allowlist).issubset(ALLOWED_ENV)):
            receipt.update(status="INVALID_ARGUMENT", outcome_exit_code=64)
            return receipt
        environment = {key: os.environ[key] for key in env_allowlist if key in os.environ}
        environment.setdefault("PATH", "/usr/bin:/bin")
        environment["PYTHONDONTWRITEBYTECODE"] = "1"
        receipt["environment_keys"] = sorted(environment)
        if threading.current_thread() is threading.main_thread():
            for signum in (signal.SIGINT, signal.SIGTERM):
                previous_handlers[signum] = signal.signal(signum, interrupt)
        with log_path.open("x", encoding="utf-8") as log:
            process = subprocess.Popen(argv, cwd=cwd, env=environment,
                                       stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                       start_new_session=True)
            receipt.update(pid=process.pid, process_group=process.pid,
                           host=socket.gethostname(), owned_process_group=True)
            identity = subprocess.run(["/bin/ps", "-p", str(process.pid), "-o", "lstart=,command="],
                                      capture_output=True, text=True, timeout=2)
            receipt["process_identity"] = redact(identity.stdout.strip())
            atomic_json(receipt_path, receipt)

            def drain():
                for line in iter(process.stdout.readline, b""):
                    log.write(redact(line.decode("utf-8", errors="replace")))
                    log.flush()

            reader = threading.Thread(target=drain, daemon=True)
            reader.start()
            try:
                process.wait(timeout=timeout)
                receipt.update(status="PASS" if process.returncode == 0 else "FAIL",
                               outcome_exit_code=0 if process.returncode == 0 else 1)
            except subprocess.TimeoutExpired:
                receipt.update(status="FAIL", timed_out=True, outcome_exit_code=1)
            except (Interrupted, KeyboardInterrupt) as error:
                signum = getattr(error, "signum", signal.SIGINT)
                receipt.update(status="INTERRUPTED", signal=signum, outcome_exit_code=128 + signum)
            finally:
                for signum in previous_handlers:
                    signal.signal(signum, signal.SIG_IGN)
                actions, errors, complete = terminate_group(process, grace)
                receipt.update(cleanup_signals=actions, cleanup_errors=errors)
                reader.join(timeout=2)
                receipt["cleanup_complete"] = complete and not reader.is_alive()
                process.stdout.close()
    except (Interrupted, KeyboardInterrupt) as error:
        signum = getattr(error, "signum", signal.SIGINT)
        receipt.update(status="INTERRUPTED", signal=signum, outcome_exit_code=128 + signum)
    except FileExistsError:
        receipt.update(status="INVALID_ARGUMENT", outcome_exit_code=64,
                       error="log path already exists; use a new output stem")
    except OSError as error:
        receipt.update(status="BLOCKED", outcome_exit_code=2,
                       error=redact(f"{type(error).__name__}: {error}"))
    except Exception as error:
        receipt.update(status="FAIL", outcome_exit_code=1,
                       error=redact(f"{type(error).__name__}: {error}"))
    finally:
        for signum in previous_handlers:
            signal.signal(signum, signal.SIG_IGN)
        try:
            if process is not None and not receipt["cleanup_complete"]:
                actions, errors, complete = terminate_group(process, grace)
                receipt.update(cleanup_signals=actions, cleanup_errors=errors)
                if reader:
                    reader.join(timeout=2)
                receipt["cleanup_complete"] = complete and (not reader or not reader.is_alive())
            if process is None:
                receipt["cleanup_complete"] = True
            receipt.update(exit_code=process.returncode if process else None,
                           terminal=True, finished_at=timestamp(),
                           elapsed_seconds=time.monotonic() - started)
            if not receipt["cleanup_complete"]:
                receipt.update(status="FAIL", outcome_exit_code=1)
            atomic_json(receipt_path, receipt)
        finally:
            for signum, previous in previous_handlers.items():
                signal.signal(signum, previous)
    return receipt


class ArgumentParser(argparse.ArgumentParser):
    def error(self, message):
        self.exit(64, f"argument error: {message}\n")


def main():
    parser = ArgumentParser()
    parser.add_argument("--cwd", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--timeout", type=float, default=300)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    try:
        receipt = run_command(command, args.cwd, args.out, args.timeout, ALLOWED_ENV)
    except FileExistsError:
        print("Existing receipt preserved; use a new output stem", file=sys.stderr)
        return 64
    print(json.dumps({"receipt": str(Path(args.out).with_suffix('.json')),
                      "exit_code": receipt["exit_code"], "status": receipt["status"]}))
    return receipt["outcome_exit_code"]


if __name__ == "__main__":
    sys.exit(main())
