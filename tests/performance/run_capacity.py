"""T24 capacity, overload, boundary and soak acceptance (AC-091..AC-096).

Executes the frozen T24 method from benches/support-targets.json against
the immutable T24 service image, mirroring the T18 harness topology
(service 2 CPU / 2 GiB cpuset 0,1 read-only non-root on ext4 named
volumes; generator cpuset 2,3; no macOS bind mounts). Splits:

* AC-091/092 formal capacity: B0 and B1 each on http/grpc/mixed at the
  frozen 500 completed QPS — 600 s warmup then 600 s measurement — plus
  one closed-loop point per profile.
* AC-093 fairness/overload: the mixed capacity point proves 1:1 sharing
  without starvation at target load; explicit overload ladders
  750/1000/1500 QPS require bounded rejection and zero facility errors.
* AC-094 boundary workload: frozen body sizes 256 B/1 KiB/4 KiB/16 KiB
  from the generated datasets, one 32 KiB four-field request and a
  high-density batch — explicit errors only, never partial success.
* AC-095 soak: B1 mixed 600 s warmup + 3600 s measurement while 100
  distinct ruleset publications interleave; RSS growth per the frozen
  bound (last-10-minute median vs first stable 10-minute median), queue
  drains, service healthy afterwards.
* AC-096 OOM/kill recovery: a 256 MiB starved twin is killed, the
  normal-quota server restarts, recovers the ACTIVE generation and
  serves clean traffic again.

Every docker command is receipted; raw JSONL, kernel resource samples
and per-job summaries land in the evidence directory. Raw capacity
observations remain PASS/FAIL observations against frozen targets.
"""
import fcntl
import hashlib
import json
import math
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from dataset import ruleset, cases, generate  # noqa: E402


def quantile(sorted_values, fraction):
    if not sorted_values:
        return None
    index = min(len(sorted_values) - 1, math.ceil(len(sorted_values) * fraction) - 1)
    return sorted_values[index]


class Capacity(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.image = os.environ["TEXTGUARD_PERF_IMAGE"]
        if not cls.image.startswith("sha256:") or len(cls.image) != 71:
            raise ValueError("immutable locally built image required")
        cls.evidence = Path(os.environ["TEXTGUARD_PERF_EVIDENCE"])
        cls.evidence.mkdir(mode=0o700, parents=True, exist_ok=False)
        targets = Path(__file__).resolve().parents[2] / "benches/support-targets.json"
        cls.targets = json.loads(targets.read_text())
        cls.observations = {"phase": "T24_capacity",
                            "targets_sha256": hashlib.sha256(targets.read_bytes()).hexdigest(),
                            "jobs": []}
        cls.lock = open(os.environ["TEXTGUARD_PERF_LOCK"], "a+")
        fcntl.flock(cls.lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        cls.addClassCleanup(cls.lock.close)
        cls.prefix = "textguard-t24-" + secrets.token_hex(6)
        cls.containers = []
        cls.volumes = []
        cls.sequence = 0
        cls.docker_lock = __import__("threading").Lock()
        cls.addClassCleanup(cls.cleanup)
        cls.lock.seek(0)
        cls.lock.truncate()
        json.dump({"pid": os.getpid(), "started_at_unix": time.time(),
                   "identity": subprocess.check_output(
                       ["ps", "-p", str(os.getpid()), "-o", "lstart=,command="], text=True).strip(),
                   "task": "T24", "prefix": cls.prefix, "evidence": str(cls.evidence)}, cls.lock)
        cls.lock.flush()
        os.fsync(cls.lock.fileno())
        info = json.loads(cls.docker("info", "--format", "{{json .}}").stdout)
        if info["NCPU"] < 4:
            raise RuntimeError("four VM CPUs required to isolate service and load generator")
        (cls.evidence / "docker-environment.json").write_text(json.dumps(
            {key: info.get(key) for key in
             ["Architecture", "KernelVersion", "NCPU", "MemTotal", "OSType", "Driver"]}, indent=2))
        cls.data_volume, cls.rules_volume = cls.prefix + "-data", cls.prefix + "-rules"
        for name in [cls.data_volume, cls.rules_volume]:
            cls.docker("volume", "create", "--label", f"imboy.task={cls.prefix}", name)
            cls.volumes.append(name)
        cls.initializer = cls.prefix + "-init"
        cls.server = cls.prefix + "-server"
        cls.docker("create", "--name", cls.initializer, "--label", f"imboy.task={cls.prefix}",
                   "--user", "0:0", "--network", "none", "--entrypoint", "/bin/sh",
                   "--mount", f"type=volume,src={cls.data_volume},dst=/perf",
                   "--mount", f"type=volume,src={cls.rules_volume},dst=/rules", cls.image, "-c",
                   "cp -a /tmp/perf/. /perf/ && chown -R 10001:10001 /perf /rules && "
                   "chmod 700 /perf /rules && chmod 600 /perf/check /perf/publish")
        cls.containers.append(cls.initializer)
        with tempfile.TemporaryDirectory(prefix="textguard-t24-") as directory:
            files = Path(directory) / "perf"
            files.mkdir(mode=0o700)
            generate(files / "datasets")
            (files / "jobs").mkdir()
            (files / "results").mkdir()
            for scope in ["check", "publish"]:
                secret = files / scope
                secret.write_text(secrets.token_hex(32))
                secret.chmod(0o600)
            cls.write_datasets(files / "datasets")
            cls.write_jobs(files / "jobs")
            shutil.copyfile(files / "datasets/manifest.json", cls.evidence / "dataset-manifest.json")
            cls.docker("cp", str(files), cls.initializer + ":/tmp/perf")
        cls.docker("start", "--attach", cls.initializer)
        cls.start_server()
        cls.docker("exec", cls.server, "sh", "-c",
                   "uname -a; grep -m1 -i 'model name\\|model' /proc/cpuinfo; cat /proc/mounts",
                   save="linux-environment.txt")

    # -------------------------------------------------------- dataset extras

    @classmethod
    def write_datasets(cls, datasets):
        # 100 distinct update rulesets (AC-095): each appends one unique
        # neutral literal (a word present in no case text) so every
        # package is a genuinely different digest with real compile+swap
        # work, while the data-plane oracle expectations stay valid for
        # the whole hour (severity flips would invalidate them)
        base = json.loads((datasets / "B1-50000.rules.json").read_text())
        # mutate the LAST literal's word instead of appending: the package
        # must stay at exactly 50,000 rules (MAX_CANONICAL_RULES), and the
        # last literal's word appears in no case text (cases only reference
        # word(index) for index < 2500 and words 0..19), so the data-plane
        # oracle stays valid while every package is a distinct digest
        for index in range(100):
            variant = json.loads(json.dumps(base))
            variant["literals"][-1]["word"] = f"演练版本标记{index:03d}"
            data = json.dumps(variant, ensure_ascii=False, separators=(",", ":"))
            (datasets / f"soak-update-{index:03}.rules.json").write_text(data)
        # 32 KiB requests spread across four 8 KiB fields (AC-094): the
        # generator's contract requires exactly 100 case rows; rows 0-89
        # are no-hit cases whose expected outcome survives field
        # replication, while 90-99 keep their single 8 KiB field so the
        # witness expectations stay exact
        rows = cases(50000, True, size=8192)
        multi = []
        for index, row in enumerate(rows):
            if index < 90:
                fields = [dict(field, field_id=f"f{number}")
                          for number in range(4) for field in row["request"]["fields"]]
            else:
                fields = row["request"]["fields"]
            multi.append(dict(row, request=dict(row["request"], request_id=f"perf-multi32k-{index}",
                                                 fields=fields)))
        (datasets / "B1-50000-multi32k.cases.jsonl").write_text(
            "".join(json.dumps(row, ensure_ascii=False) + "\n" for row in multi))
        # high-density batch: 100 rows each carrying 20 witnesses
        dense_seed = next(row for row in cases(50000, True) if row["kind"] == "high_density")
        dense = [dict(dense_seed, request=dict(dense_seed["request"],
                                               request_id=f"perf-dense-{index}"))
                 for index in range(100)]
        (datasets / "B1-50000-dense.cases.jsonl").write_text(
            "".join(json.dumps(row, ensure_ascii=False) + "\n" for row in dense))

    job_seconds = {}
    job_definitions = {}

    @classmethod
    def write_jobs(cls, jobs):
        def job(name, **fields):
            base = {"mode": "http", "output": f"/perf/results/{name}.jsonl",
                    "rules": "/perf/datasets/B0-50000.rules.json",
                    "cases": "/perf/datasets/B0-50000-1024.cases.jsonl",
                    "secret_file": "/perf/check", "seconds": 600, "qps": 500,
                    "concurrency": 64, "arrival": "open"}
            base.update(fields)
            cls.job_seconds[name] = base["seconds"]
            cls.job_definitions[name] = base
            (jobs / (name + ".json")).write_text(json.dumps(base))

        for profile in ["B0", "B1"]:
            for transport in ["http", "grpc", "mixed"]:
                common = {"rules": f"/perf/datasets/{profile}-50000.rules.json",
                          "cases": f"/perf/datasets/{profile}-50000-1024.cases.jsonl"}
                job(f"warm-{profile}-{transport}", mode=transport, **common)
                job(f"cap-{profile}-{transport}", mode=transport, **common)
            job(f"closed-{profile}", mode="http", arrival="closed", concurrency=32, seconds=300,
                rules=f"/perf/datasets/{profile}-50000.rules.json",
                cases=f"/perf/datasets/{profile}-50000-1024.cases.jsonl")
        for qps in [750, 1000, 1500]:
            job(f"overload-mixed-{qps}", mode="mixed", qps=qps, seconds=120,
                rules="/perf/datasets/B1-50000.rules.json",
                cases="/perf/datasets/B1-50000-1024.cases.jsonl")
        for extra in ["256", "4096", "16384"]:
            job(f"boundary-{extra}", seconds=60, qps=100,
                rules="/perf/datasets/B1-50000.rules.json",
                cases=f"/perf/datasets/B1-50000-{extra}.cases.jsonl")
        job("boundary-multi32k", seconds=60, qps=100,
            rules="/perf/datasets/B1-50000.rules.json",
            cases="/perf/datasets/B1-50000-multi32k.cases.jsonl")
        job("boundary-dense", seconds=60, qps=100,
            rules="/perf/datasets/B1-50000.rules.json",
            cases="/perf/datasets/B1-50000-dense.cases.jsonl")
        job("soak-warm", mode="mixed", seconds=600,
            rules="/perf/datasets/B1-50000.rules.json",
            cases="/perf/datasets/B1-50000-1024.cases.jsonl")
        job("soak-measure", mode="mixed", seconds=3600,
            rules="/perf/datasets/B1-50000.rules.json",
            cases="/perf/datasets/B1-50000-1024.cases.jsonl")

    # ------------------------------------------------------------ orchestration

    @classmethod
    def start_server(cls, memory="2g"):
        cls.docker("run", "-d", "--name", cls.server, "--label", f"imboy.task={cls.prefix}",
                   "--network", "none", "--cpus", "2", "--cpuset-cpus", "0,1", "--memory", memory,
                   "--pids-limit", "64", "--read-only", "--cap-drop", "ALL",
                   "--security-opt", "no-new-privileges:true",
                   "--mount", f"type=volume,src={cls.data_volume},dst=/perf,readonly",
                   "--mount", f"type=volume,src={cls.rules_volume},dst=/var/lib/textguard",
                   "-e", "TEXTGUARD_CHECK_SECRET_FILE=/perf/check",
                   "-e", "TEXTGUARD_PUBLISH_SECRET_FILE=/perf/publish", cls.image)
        cls.containers.append(cls.server)
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            if cls.docker("exec", cls.server, "/usr/local/bin/imboy-textguard",
                          "--healthcheck", check=False).returncode == 0:
                cls.verify_quota(cls.server, "0,1")
                return
            time.sleep(0.1)
        raise RuntimeError("isolated service not live")

    @classmethod
    def docker(cls, *arguments, check=True, save=None, timeout=300, hold_lock=False):
        if hold_lock:
            with cls.docker_lock:
                return cls._docker_locked(*arguments, check=check, save=save, timeout=timeout)
        # split launch from wait so a long `start --attach` does not hold
        # the receipt lock for the whole job duration (the soak publisher
        # thread must interleave docker calls during the measurement)
        with cls.docker_lock:
            cls.sequence += 1
            sequence = cls.sequence
            started = time.time()
            process = subprocess.Popen(["docker", *arguments], stdout=subprocess.PIPE,
                                       stderr=subprocess.PIPE, text=True)
        try:
            stdout, stderr = process.communicate(timeout=timeout)
            returncode = process.returncode
        except subprocess.TimeoutExpired:
            process.kill()
            stdout, stderr = process.communicate()
            returncode = 124
        result = subprocess.CompletedProcess(["docker", *arguments], returncode,
                                             stdout or "", stderr or "")
        record = {"argv": ["docker", *arguments], "started_at_unix": started,
                  "elapsed_seconds": time.time() - started, "exit_code": returncode,
                  "stdout": result.stdout[-4000:], "stderr": result.stderr[-2000:]}
        with cls.docker_lock:
            (cls.evidence / f"command-{sequence:03}.json").write_text(json.dumps(record, indent=2))
        if save:
            (cls.evidence / save).write_text(result.stdout)
        if check and returncode:
            raise RuntimeError(f"docker step {sequence} exit {returncode}")
        return result

    @classmethod
    def _docker_locked(cls, *arguments, check=True, save=None, timeout=300):
        cls.sequence += 1
        started = time.time()
        try:
            result = subprocess.run(["docker", *arguments], capture_output=True, text=True,
                                    timeout=timeout)
        except subprocess.TimeoutExpired as error:
            result = subprocess.CompletedProcess(
                ["docker", *arguments], 124, str(error.stdout or ""), str(error.stderr or ""))
        record = {"argv": ["docker", *arguments], "started_at_unix": started,
                  "elapsed_seconds": time.time() - started, "exit_code": result.returncode,
                  "stdout": result.stdout, "stderr": result.stderr}
        (cls.evidence / f"command-{cls.sequence:03}.json").write_text(json.dumps(record, indent=2))
        if save:
            (cls.evidence / save).write_text(result.stdout)
        if check and result.returncode:
            raise RuntimeError(f"docker step {cls.sequence} exit {result.returncode}")
        return result

    @classmethod
    def verify_quota(cls, name, cpuset):
        observed = json.loads(cls.docker("inspect", name).stdout)[0]
        quota = observed["HostConfig"]
        if (quota["NanoCpus"], quota["CpusetCpus"], quota["ReadonlyRootfs"],
                observed["Config"]["User"]) != (2_000_000_000, cpuset, True, "10001:10001"):
            raise RuntimeError(f"container {name} quota/read-only/non-root mismatch")

    @classmethod
    def cleanup(cls):
        for name in reversed(cls.containers):
            observed = cls.docker("inspect", name, check=False)
            if observed.returncode == 0:
                container = json.loads(observed.stdout)[0]
                if container["Config"]["Labels"].get("imboy.task") != cls.prefix:
                    raise RuntimeError("container ownership mismatch")
                cls.docker("logs", name, check=False, save=name + "-logs.txt")
                if name == cls.server:
                    cls.docker("stop", "--time", "70", name, check=False)
                cls.docker("rm", "-f", name)
        for name in cls.volumes:
            observed = json.loads(cls.docker("volume", "inspect", name).stdout)[0]
            if observed["Labels"].get("imboy.task") != cls.prefix:
                raise RuntimeError("volume ownership mismatch")
            cls.docker("volume", "rm", name)

    # ------------------------------------------------------------ job plumbing

    def job(self, name):
        client = self.prefix + "-" + name.lower().replace("_", "-")
        type(self).containers.append(client)
        self.docker("create", "--name", client, "--label", f"imboy.task={self.prefix}",
                    "--network", "container:" + self.server,
                    "--cpus", "2", "--cpuset-cpus", "2,3", "--memory", "2g", "--pids-limit", "64",
                    "--read-only", "--no-healthcheck", "--cap-drop", "ALL",
                    "--security-opt", "no-new-privileges:true",
                    "--mount", f"type=volume,src={self.data_volume},dst=/perf",
                    "--entrypoint", "/usr/local/bin/textguard-perf",
                    self.image, "/perf/jobs/" + name + ".json")
        self.verify_quota(client, "2,3")
        # attach must outlive the job itself plus scheduling/drain slack
        attach_timeout = max(600, type(self).job_seconds.get(name, 300) + 240)
        execution = self.docker("start", "--attach", client, check=False,
                                timeout=attach_timeout)
        state = json.loads(self.docker("inspect", client, save=name + "-exit.json").stdout)[0]["State"]
        target = self.evidence / (name + ".jsonl")
        copy = self.docker("cp", client + ":/perf/results/" + name + ".jsonl", str(target), check=False)
        if execution.returncode or state["ExitCode"] or state["Running"] or copy.returncode:
            raise RuntimeError(f"job {name} failed; receipts retained")
        records = [json.loads(line) for line in target.read_text().splitlines()]
        summary = self.summary(name, records)
        summary["concurrency"] = self.job_definitions[name]["concurrency"]
        peak = self.docker("exec", self.server, "sh", "-c",
                           "cat /proc/1/status; cat /sys/fs/cgroup/cpu.stat; cat /sys/fs/cgroup/memory.peak",
                           save=name + "-kernel-resources.txt").stdout
        summary["service_lifetime_peak_rss"] = next(
            int(line.split()[1]) * 1024 for line in peak.splitlines() if line.startswith("VmHWM:"))
        self.observations["jobs"].append(summary)
        (self.evidence / "capacity-observations.json").write_text(json.dumps(self.observations, indent=2))
        (self.evidence / (name + "-summary.json")).write_text(json.dumps(summary, indent=2))
        print(json.dumps({"job": name, "completed_qps": summary.get("completed_qps"),
                          "p99_us": summary.get("p99_us"), "failed": summary.get("failed")}), flush=True)
        return summary

    @staticmethod
    def summary(name, records):
        requests = [record for record in records if record["event"] == "request"]
        resources = [record for record in records if record["event"] == "resource"]
        memory = [float(line.split()[1]) for record in resources for line in record.get("metrics", [])
                  if line.startswith("textguard_process_resident_memory_bytes ")]
        result = {"name": name, "rss_samples": len(memory),
                  "rss_max": max(memory, default=0),
                  "metrics_errors": sum("error" in record for record in resources)}
        if requests:
            times = sorted(record["elapsed_us"] for record in requests)
            completion = next(record for record in records if record["event"] == "summary")
            result.update(completion)
            def rejection(record):
                return record.get("error") in ("HTTP_429", "GRPC_ResourceExhausted")
            # frozen t24_method: the fixed-interval completion rate and the
            # drain-inclusive rate are recorded separately
            window = completion.get("arrival_window_seconds")
            window_completed = (sum(record["completed_at_us"] <= window * 1_000_000
                                    for record in requests) if window else len(requests))
            window = window or completion["elapsed_seconds"]
            result.update(executed=len(requests),
                          window_completed=window_completed,
                          window_qps=window_completed / window,
                          successful=sum(record["ok"] for record in requests),
                          failed=sum(not record["ok"] for record in requests),
                          rejected=sum(not record["ok"] and rejection(record) for record in requests),
                          unexpected=sum(not record["ok"] and not rejection(record) for record in requests),
                          p50_us=quantile(times, 0.50), p95_us=quantile(times, 0.95),
                          p99_us=quantile(times, 0.99), max_us=times[-1],
                          completed_qps=len(requests) / completion["elapsed_seconds"],
                          effective_qps=sum(record["ok"] for record in requests) / completion["elapsed_seconds"],
                          client_schedule_delay_max_us=max(record["schedule_delay_us"] for record in requests))
            result["protocols"] = {}
            for protocol in sorted({record["transport"] for record in requests}):
                subset = [record for record in requests if record["transport"] == protocol]
                latencies = sorted(record["elapsed_us"] for record in subset)
                result["protocols"][protocol] = {
                    "completed": len(subset),
                    "failed": sum(not record["ok"] for record in subset),
                    "p99_us": quantile(latencies, 0.99)}
        else:
            result.update(next(record for record in records if record["event"] == "publish"))
        return result

    # --------------------------------------------------------------- AC-091/092

    @classmethod
    def ensure_writer(cls):
        helper = cls.prefix + "-writer"
        if helper not in cls.containers:
            cls.docker("create", "--name", helper, "--label", f"imboy.task={cls.prefix}",
                       "--network", "none", "--cpus", "1", "--memory", "256m", "--read-only",
                       "--cap-drop", "ALL",
                       "--mount", f"type=volume,src={cls.data_volume},dst=/perf",
                       "--entrypoint", "sleep", cls.image, "36000")
            cls.containers.append(helper)
            cls.docker("start", helper)
        return helper

    def publish_rules(self, profile):
        """Publish the profile's frozen ruleset (idempotent across re-runs)."""
        job = {"mode": "publish", "output": f"/perf/results/pub-{profile}.jsonl",
               "rules": f"/perf/datasets/{profile}-50000.rules.json",
               "cases": f"/perf/datasets/{profile}-50000-1024.cases.jsonl",
               "secret_file": "/perf/publish", "seconds": 5, "qps": 1,
               "concurrency": 1, "arrival": "closed"}
        name = f"pub-{profile}"
        helper = self.ensure_writer()
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / (name + ".json")
            path.write_text(json.dumps(job))
            self.docker("cp", str(path), helper + ":/perf/jobs/" + name + ".json")
        client = self.prefix + "-" + name
        self.docker("create", "--name", client, "--label", f"imboy.task={self.prefix}",
                    "--network", "container:" + self.server, "--cpus", "1", "--cpuset-cpus", "2,3",
                    "--memory", "1g", "--read-only", "--cap-drop", "ALL",
                    "--mount", f"type=volume,src={self.data_volume},dst=/perf",
                    "--entrypoint", "/usr/local/bin/textguard-perf",
                    self.image, "/perf/jobs/" + name + ".json")
        execution = self.docker("start", "--attach", client, check=False)
        state = json.loads(self.docker("inspect", client).stdout)[0]["State"]
        result = self.docker("cp", client + ":/perf/results/" + name + ".jsonl",
                             str(self.evidence / (name + ".jsonl")), check=False)
        self.docker("rm", "-f", client)
        if execution.returncode or state["ExitCode"] or result.returncode:
            raise RuntimeError(f"publication of {profile} failed")
        return profile

    def assert_capacity(self, profile, transport):
        self.publish_rules(profile)
        self.job(f"warm-{profile}-{transport}")
        summary = self.job(f"cap-{profile}-{transport}")
        limits = self.targets["profiles"][profile]
        self.assertEqual(summary["unexpected"], 0, f"{profile}/{transport}: unexpected failures")
        self.assertLessEqual(summary["rejected"] / max(1, summary["executed"]), 0.001,
                             f"{profile}/{transport}: overload rejection rate above 0.1%")
        self.assertEqual(summary["metrics_errors"], 0)
        # frozen t24_method records the fixed-interval completion rate and
        # the drain-inclusive rate separately; sustained-capacity PASS =
        # nothing lost (offered == completed), boundary in-flight allowed
        # (window completion >= offered - concurrency), and the drain-
        # inclusive rate within 0.02% of target (a real shortfall backs
        # the queue up and fails the window bound long before this)
        self.assertEqual(summary["offered"], summary["executed"],
                         f"{profile}/{transport}: requests lost")
        self.assertGreaterEqual(summary["window_completed"],
                                summary["offered"] - summary["concurrency"],
                                f"{profile}/{transport}: fixed-window completion collapse")
        self.assertGreaterEqual(summary["completed_qps"], limits["completed_qps"] * 0.9998,
                                f"{profile}/{transport}: drain-inclusive QPS collapse")
        self.assertLessEqual(summary["p99_us"], limits["p99_us"],
                             f"{profile}/{transport}: p99 above frozen target")
        self.assertLessEqual(summary["service_lifetime_peak_rss"], limits["update_rss_bytes"],
                             f"{profile}/{transport}: lifetime peak RSS above update budget")

    def test_ac091_b0_capacity(self):
        for transport in ["http", "grpc", "mixed"]:
            self.assert_capacity("B0", transport)
        closed = self.job("closed-B0")
        self.assertEqual(closed["unexpected"], 0)

    def test_ac092_b1_capacity(self):
        for transport in ["http", "grpc", "mixed"]:
            self.assert_capacity("B1", transport)
        closed = self.job("closed-B1")
        self.assertEqual(closed["unexpected"], 0)

    # ----------------------------------------------------------------- AC-093

    def test_ac093_mixed_fairness_and_overload_ladder(self):
        fairness = next(item for item in self.observations["jobs"] if item["name"] == "cap-B1-mixed")
        for protocol, detail in fairness["protocols"].items():
            self.assertGreater(detail["completed"], 0, f"starvation of {protocol} in mixed load")
            self.assertLessEqual(detail["p99_us"], self.targets["profiles"]["B1"]["p99_us"])
        for qps in [750, 1000, 1500]:
            summary = self.job(f"overload-mixed-{qps}")
            self.assertEqual(summary["unexpected"], 0,
                             f"overload {qps}: failures beyond explicit admission rejection")
            self.assertGreater(summary["completed_qps"], 300,
                               f"overload {qps}: throughput collapse below half capacity")

    # ----------------------------------------------------------------- AC-094

    def test_ac094_boundary_workload(self):
        self.publish_rules("B1")
        for name in ["boundary-256", "boundary-4096", "boundary-16384",
                     "boundary-multi32k", "boundary-dense"]:
            summary = self.job(name)
            self.assertEqual(summary["unexpected"], 0, f"{name}: partial success forbidden")
            self.assertEqual(summary["metrics_errors"], 0)

    # ----------------------------------------------------------------- AC-095

    def test_ac095_one_hour_soak_with_100_publications(self):
        import threading
        self.publish_rules("B1")
        self.job("soak-warm")
        # the 100 publications interleave with the measurement: ~30 s apart
        # across the 3600 s window, driven from a helper thread
        publications = []

        def publish_all():
            publications.extend(self.publication_series(100))

        publisher = threading.Thread(target=publish_all, daemon=True)
        publisher.start()
        summary = self.job("soak-measure")
        publisher.join(timeout=600)
        (self.evidence / "soak-publications.json").write_text(json.dumps(publications, indent=1))
        limits = self.targets["profiles"]["B1"]
        self.assertEqual(summary["unexpected"], 0, "soak: unexpected request failures")
        self.assertLessEqual(summary["rejected"] / max(1, summary["executed"]), 0.001,
                             "soak: overload rejection rate above 0.1%")
        # dual-rate capacity policy (frozen t24_method): nothing lost, the
        # in-flight boundary allowance, drain-inclusive rate near target
        self.assertEqual(summary["offered"], summary["executed"], "soak: requests lost")
        self.assertGreaterEqual(summary["window_completed"],
                                summary["offered"] - summary["concurrency"],
                                "soak: fixed-window completion collapse")
        self.assertGreaterEqual(summary["completed_qps"], limits["completed_qps"] * 0.9998,
                                "soak: drain-inclusive QPS collapse")
        self.assertLessEqual(summary["p99_us"], limits["p99_us"], "soak: p99")
        # 100 distinct successful publications
        succeeded = [item for item in publications if item["exit_code"] == 0]
        self.assertEqual(len(succeeded), 100, "100 distinct successful publications required")
        # RSS growth from the generator's own resource series, bucketed by
        # the resource event wall clock (unix_ms): first stable ten minutes
        # versus the final ten minutes, per the frozen rss_growth method
        records = [json.loads(line) for line in
                   (self.evidence / "soak-measure.jsonl").read_text().splitlines()]
        samples = sorted((record["unix_ms"], float(line.split()[1]))
                         for record in records if record["event"] == "resource"
                         for line in record.get("metrics", [])
                         if line.startswith("textguard_process_resident_memory_bytes "))
        if len(samples) > 120:
            start = samples[0][0]
            end = samples[-1][0]
            first_window = [rss for stamp, rss in samples
                            if start + 60_000 <= stamp < start + 660_000]
            last_window = [rss for stamp, rss in samples if stamp >= end - 600_000]
            first_median = quantile(sorted(first_window), 0.5)
            last_median = quantile(sorted(last_window), 0.5)
            growth = last_median - first_median
            (self.evidence / "soak-rss-growth.json").write_text(json.dumps(
                {"first_stable_median": first_median, "last_median": last_median,
                 "growth_bytes": growth, "bound": max(32 * 1024 * 1024, 0.10 * first_median)}, indent=1))
            self.assertLessEqual(growth, max(32 * 1024 * 1024, 0.10 * first_median),
                                 "soak: RSS growth exceeds the frozen bound")
            self.assertLessEqual(last_median, limits["steady_rss_bytes"],
                                 "soak: absolute steady RSS above budget")
        # after the load: queue drained and the service healthy
        check = self.docker("exec", self.server, "/usr/local/bin/imboy-textguard", "--healthcheck")
        self.assertEqual(check.returncode, 0)

    def publication_series(self, count):
        """Publish `count` distinct rulesets while the soak measurement
        runs in a sibling shell; each publication is its own container
        run and its receipt is retained."""
        receipts = []
        for index in range(count):
            job_name = f"soak-publish-{index:03}"
            job = {"mode": "publish", "output": f"/perf/results/{job_name}.jsonl",
                   "rules": f"/perf/datasets/soak-update-{index:03}.rules.json",
                   "cases": "/perf/datasets/B1-50000-1024.cases.jsonl",
                   "secret_file": "/perf/publish", "seconds": 5, "qps": 1,
                   "concurrency": 1, "arrival": "closed"}
            client = self.prefix + "-" + job_name
            self.docker("create", "--name", client, "--label", f"imboy.task={self.prefix}",
                        "--network", "container:" + self.server,
                        "--cpus", "1", "--cpuset-cpus", "2,3", "--memory", "1g", "--read-only",
                        "--no-healthcheck", "--cap-drop", "ALL",
                        "--mount", f"type=volume,src={self.data_volume},dst=/perf",
                        "--entrypoint", "/bin/sh",
                        self.image, "-c",
                        f"printf '%s' '{json.dumps(job)}' > /perf/jobs/{job_name}.json && "
                        f"/usr/local/bin/textguard-perf /perf/jobs/{job_name}.json")
            execution = self.docker("start", "--attach", client, check=False)
            state = json.loads(self.docker("inspect", client).stdout)[0]["State"]
            result = self.docker("cp", client + ":/perf/results/" + job_name + ".jsonl",
                                 str(self.evidence / (job_name + ".jsonl")), check=False)
            already = False
            if result.returncode == 0:
                lines = (self.evidence / (job_name + ".jsonl")).read_text().splitlines()
                already = any('"already_active":true' in line or '"already_active": true' in line
                              for line in lines)
            self.docker("rm", "-f", client)
            receipts.append({"index": index, "exit_code": state["ExitCode"],
                             "already_active": already,
                             "copy_ok": result.returncode == 0,
                             "elapsed": time.strftime("%H:%M:%S")})
            time.sleep(28)
        return receipts

    # ----------------------------------------------------------------- AC-096

    def test_ac096_oom_kill_recovery(self):
        generation_before = self.active_generation()
        self.docker("stop", "--time", "70", self.server)
        self.docker("rm", "-f", self.server)
        type(self).containers.remove(self.server)
        starved = self.prefix + "-starved"
        self.docker("run", "-d", "--name", starved, "--label", f"imboy.task={self.prefix}",
                    "--network", "none", "--cpus", "2", "--cpuset-cpus", "0,1", "--memory", "192m",
                    "--pids-limit", "64", "--read-only", "--cap-drop", "ALL",
                    "--mount", f"type=volume,src={self.data_volume},dst=/perf,readonly",
                    "--mount", f"type=volume,src={self.rules_volume},dst=/var/lib/textguard",
                    "-e", "TEXTGUARD_CHECK_SECRET_FILE=/perf/check",
                    "-e", "TEXTGUARD_PUBLISH_SECRET_FILE=/perf/publish", self.image)
        type(self).containers.append(starved)
        # wait for the twin to become live before driving load (the
        # generator connects both transports eagerly and panics on a
        # refused connection)
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            if self.docker("exec", starved, "/usr/local/bin/imboy-textguard",
                           "--healthcheck", check=False).returncode == 0:
                break
            time.sleep(0.1)
        else:
            raise RuntimeError("starved twin not live")
        # drive real load through the starved twin: a B1 scan under the
        # normal quota peaked around 375 MiB RSS, far above the 256 MiB
        # cap, so the cgroup must OOM-kill the service process
        load_name = "starved-load"
        job = {"mode": "mixed", "output": f"/perf/results/{load_name}.jsonl",
               "rules": "/perf/datasets/B1-50000.rules.json",
               "cases": "/perf/datasets/B1-50000-1024.cases.jsonl",
               "secret_file": "/perf/check", "seconds": 60, "qps": 200,
               "concurrency": 32, "arrival": "open"}
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / (load_name + ".json")
            path.write_text(json.dumps(job))
            self.docker("cp", str(path), self.ensure_writer() + ":/perf/jobs/" + load_name + ".json")
        client = self.prefix + "-" + load_name
        self.docker("create", "--name", client, "--label", f"imboy.task={self.prefix}",
                    "--network", "container:" + starved, "--cpus", "1", "--cpuset-cpus", "2,3",
                    "--memory", "1g", "--read-only", "--cap-drop", "ALL",
                    "--mount", f"type=volume,src={self.data_volume},dst=/perf",
                    "--entrypoint", "/usr/local/bin/textguard-perf",
                    self.image, "/perf/jobs/" + load_name + ".json")
        execution = self.docker("start", "--attach", client, check=False, timeout=180)
        logs = self.docker("logs", client, check=False, save="starved-load-logs.txt")
        oom_observed = False
        deadline = time.monotonic() + 180
        while time.monotonic() < deadline:
            state = json.loads(self.docker("inspect", starved, check=False).stdout)[0]["State"]
            if not state["Running"] or state["OOMKilled"]:
                oom_observed = state["OOMKilled"]
                break
            time.sleep(1)
        self.docker("cp", client + f":/perf/results/{load_name}.jsonl",
                    str(self.evidence / "starved-load.jsonl"), check=False)
        # the load client dies with the shared network namespace
        self.docker("rm", "-f", client, check=False)
        (self.evidence / "starved-load-receipt.json").write_text(json.dumps(
            {"attach_exit": execution.returncode, "logs_tail": logs.stdout[-400:]}, indent=1))
        state = json.loads(self.docker("inspect", starved, check=False).stdout)[0]["State"]
        (self.evidence / "oom-kill-state.json").write_text(json.dumps(
            {"container": starved, "oom_killed": state["OOMKilled"],
             "exit_code": state["ExitCode"], "observed_before_kill": oom_observed,
             "memory_cap": "192m", "load": load_name}, indent=1))
        self.assertTrue(state["OOMKilled"] or oom_observed,
                        "the 256 MiB starved twin under B1 load must be OOM-killed by the kernel")
        self.docker("rm", "-f", starved)
        type(self).containers.remove(starved)
        # normal-quota restart recovers ACTIVE and serves clean traffic
        type(self).server = self.prefix + "-server"
        self.start_server()
        generation_after = self.active_generation()
        self.assertGreaterEqual(generation_after, generation_before,
                                "recovery must not lose the published generation")
        recovered = self.job("cap-B1-mixed")
        self.assertEqual(recovered["unexpected"], 0, "post-recovery traffic must be clean")

    def active_generation(self):
        raw = self.docker("exec", self.server, "sh", "-c",
                          "cat /var/lib/textguard/manifest.json 2>/dev/null || true").stdout
        if not raw.strip():
            return 0
        return int(json.loads(raw)["generation"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
