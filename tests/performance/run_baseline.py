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
import uuid

from dataset import generate


class Baselines(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.image = os.environ["TEXTGUARD_PERF_IMAGE"]
        if not cls.image.startswith("sha256:") or len(cls.image) != 71:
            raise ValueError("immutable locally built image required")
        cls.evidence = Path(os.environ["TEXTGUARD_PERF_EVIDENCE"])
        cls.evidence.mkdir(mode=0o700, parents=True, exist_ok=False)
        targets = Path(__file__).resolve().parents[2] / "benches/support-targets.json"
        cls.targets = json.loads(targets.read_text())
        cls.observations = {"phase": "T18_measurement", "capacity_acceptance": "T24_CAPACITY_NOT_ACCEPTED",
                            "targets_sha256": hashlib.sha256(targets.read_bytes()).hexdigest(), "jobs": []}
        shutil.copyfile(targets, cls.evidence / "support-targets.json")
        cls.lock = open(os.environ["TEXTGUARD_PERF_LOCK"], "a+")
        fcntl.flock(cls.lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        cls.addClassCleanup(cls.lock.close)
        cls.prefix = "textguard-t18-" + uuid.uuid4().hex[:12]
        cls.containers = []
        cls.volumes = []
        cls.sequence = 0
        cls.addClassCleanup(cls.cleanup)
        cls.lock.seek(0)
        cls.lock.truncate()
        json.dump({"pid": os.getpid(), "started_at_unix": time.time(), "identity": subprocess.check_output(
            ["ps", "-p", str(os.getpid()), "-o", "lstart=,command="], text=True).strip(),
                   "task": "T18", "prefix": cls.prefix, "evidence": str(cls.evidence)}, cls.lock)
        cls.lock.flush()
        os.fsync(cls.lock.fileno())
        info = json.loads(cls.docker("info", "--format", "{{json .}}").stdout)
        if info["NCPU"] < 4:
            raise RuntimeError("four VM CPUs required to isolate service and load generator")
        (cls.evidence / "docker-environment.json").write_text(json.dumps({key: info.get(key) for key in [
            "Architecture", "KernelVersion", "NCPU", "MemTotal", "OSType", "OperatingSystem", "Driver"]}, indent=2))
        cls.data_volume, cls.rules_volume = [cls.prefix + suffix for suffix in ["-data", "-rules"]]
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
        with tempfile.TemporaryDirectory(prefix="textguard-t18-") as directory:
            files = Path(directory) / "perf"
            files.mkdir(mode=0o700)
            generate(files / "datasets")
            (files / "jobs").mkdir()
            (files / "results").mkdir()
            for scope in ["check", "publish"]:
                secret = files / scope
                secret.write_text(secrets.token_hex(32))
                secret.chmod(0o600)
            cls.write_jobs(files)
            shutil.copyfile(files / "datasets/manifest.json", cls.evidence / "dataset-manifest.json")
            cls.docker("cp", str(files), cls.initializer + ":/tmp/perf")
        cls.docker("start", "--attach", cls.initializer)
        cls.docker("run", "-d", "--name", cls.server, "--label", f"imboy.task={cls.prefix}",
                   "--network", "none", "--cpus", "2", "--cpuset-cpus", "0,1", "--memory", "2g",
                   "--pids-limit", "64", "--read-only", "--cap-drop", "ALL", "--security-opt", "no-new-privileges:true",
                   "--mount", f"type=volume,src={cls.data_volume},dst=/perf,readonly",
                   "--mount", f"type=volume,src={cls.rules_volume},dst=/var/lib/textguard",
                   "-e", "TEXTGUARD_CHECK_SECRET_FILE=/perf/check", "-e", "TEXTGUARD_PUBLISH_SECRET_FILE=/perf/publish",
                   cls.image)
        cls.containers.append(cls.server)
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            if cls.docker("exec", cls.server, "/usr/local/bin/imboy-textguard", "--healthcheck", check=False).returncode == 0:
                break
            time.sleep(0.1)
        else:
            raise RuntimeError("isolated service not live")
        cls.verify_quota(cls.server, "0,1")
        cls.docker("exec", cls.server, "sh", "-c", "uname -a; cat /proc/cpuinfo; cat /proc/mounts", save="linux-environment.txt")

    @classmethod
    def write_jobs(cls, files):
        for profile in ["B0", "B1"]:
            for mode, arrival in [("core", "closed"), ("publish", ""), ("http", "open"), ("grpc", "open"),
                                  ("mixed", "open"), ("http", "closed"), ("grpc", "closed"), ("mixed", "closed")]:
                name = f"{profile}-{mode}-{arrival}".rstrip("-")
                job = {"mode": mode, "output": f"/perf/results/{name}.jsonl",
                       "rules": f"/perf/datasets/{profile}-50000.rules.json",
                       "cases": f"/perf/datasets/{profile}-50000-1024.cases.jsonl",
                       "secret_file": "/perf/publish" if mode == "publish" else "/perf/check",
                       "seconds": 10, "qps": 500, "concurrency": 64 if arrival == "open" else 1, "arrival": arrival}
                (files / "jobs" / (name + ".json")).write_text(json.dumps(job))
                if mode == "publish":
                    job["rules"] = job["rules"].replace(".rules.json", ".update.rules.json")
                    job["output"] = job["output"].replace("-publish", "-update")
                    (files / "jobs" / (profile + "-update.json")).write_text(json.dumps(job))

    @classmethod
    def docker(cls, *arguments, check=True, save=None):
        cls.sequence += 1
        started = time.time()
        try:
            result = subprocess.run(["docker", *arguments], capture_output=True, text=True, timeout=180)
        except subprocess.TimeoutExpired as error:
            result = subprocess.CompletedProcess(["docker", *arguments], 124,
                                                (error.stdout or b"").decode() if isinstance(error.stdout, bytes) else error.stdout or "",
                                                (error.stderr or b"").decode() if isinstance(error.stderr, bytes) else error.stderr or "")
        record = {"argv": ["docker", *arguments], "cwd": os.getcwd(), "started_at_unix": started, "elapsed_seconds": time.time() - started,
                  "exit_code": result.returncode, "stdout": result.stdout, "stderr": result.stderr}
        (cls.evidence / f"command-{cls.sequence:03}.json").write_text(json.dumps(record, indent=2))
        if save:
            (cls.evidence / save).write_text(result.stdout)
        if check and result.returncode:
            raise RuntimeError(f"docker step {cls.sequence} exit {result.returncode}; see command receipt")
        return result

    @classmethod
    def verify_quota(cls, name, cpuset):
        observed = json.loads(cls.docker("inspect", name, save=name + "-inspect.json").stdout)[0]
        quota = observed["HostConfig"]
        if (quota["NanoCpus"], quota["Memory"], quota["CpusetCpus"], quota["ReadonlyRootfs"], observed["Config"]["User"]) != (
                2_000_000_000, 2*1024**3, cpuset, True, "10001:10001"):
            raise RuntimeError("actual container quota or nonroot/read-only configuration mismatch")

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

    def job(self, name):
        client = self.prefix + "-" + name.lower()
        self.containers.append(client)
        self.docker("create", "--name", client, "--label", f"imboy.task={self.prefix}", "--network", "container:" + self.server,
                    "--cpus", "2", "--cpuset-cpus", "2,3", "--memory", "2g", "--pids-limit", "64", "--read-only",
                    "--no-healthcheck", "--cap-drop", "ALL", "--security-opt", "no-new-privileges:true",
                    "--mount", f"type=volume,src={self.data_volume},dst=/perf", "--entrypoint", "/usr/local/bin/textguard-perf",
                    self.image, "/perf/jobs/" + name + ".json")
        self.verify_quota(client, "2,3")
        execution = self.docker("start", "--attach", client, check=False)
        state = json.loads(self.docker("inspect", client, save=name + "-exit.json").stdout)[0]["State"]
        target = self.evidence / (name + ".jsonl")
        copy = self.docker("cp", client + ":/perf/results/" + name + ".jsonl", str(target), check=False)
        if execution.returncode or state["ExitCode"] or state["Running"] or copy.returncode:
            raise RuntimeError("performance process failed; partial JSONL, exit and command receipts retained")
        records = [json.loads(line) for line in target.read_text().splitlines()]
        summary = self.summary(name, records)
        peak = self.docker("exec", self.server, "sh", "-c", "cat /proc/1/status; cat /sys/fs/cgroup/cpu.stat; cat /sys/fs/cgroup/memory.peak",
                           save=name + "-kernel-resources.txt").stdout
        summary["service_lifetime_peak_rss"] = next(int(line.split()[1])*1024 for line in peak.splitlines() if line.startswith("VmHWM:"))
        (self.evidence / (name + "-summary.json")).write_text(json.dumps(summary, indent=2))
        print(json.dumps({"job": name, **summary}), flush=True)
        return summary

    @staticmethod
    def summary(name, records):
        requests = [record for record in records if record["event"] == "request"]
        resources = [record for record in records if record["event"] == "resource"]
        memory = [float(line.split()[1]) for record in resources for line in record.get("metrics", [])
                  if line.startswith("textguard_process_resident_memory_bytes ")]
        cpu = [float(line.split()[1]) for record in resources for line in record.get("metrics", [])
               if line.startswith("textguard_process_cpu_seconds_total ")]
        summary = {"name": name, "resources_collected": len(resources), "rss_max": max(memory, default=0),
                   "cpu_seconds_observed": cpu, "rss_samples": len(memory), "cpu_samples": len(cpu),
                   "metrics_errors": sum("error" in record for record in resources)}
        if requests:
            times = sorted(record["elapsed_us"] for record in requests)
            completion = next(record for record in records if record["event"] == "summary")
            summary.update(completion)
            summary.update(executed=len(requests), successful=sum(record["ok"] for record in requests),
                           failed=sum(not record["ok"] for record in requests), p99_us=times[math.ceil(len(times)*0.99)-1],
                           completed_qps=len(requests)/completion["elapsed_seconds"],
                           effective_qps=sum(record["ok"] for record in requests)/completion["elapsed_seconds"],
                           client_schedule_delay_max_us=max(record["schedule_delay_us"] for record in requests))
            if "arrival_window_seconds" in completion:
                window = completion["arrival_window_seconds"]
                within = [record for record in requests if record["completed_at_us"] <= window*1_000_000]
                summary.update(window_completed=len(within), tail_completed=len(requests)-len(within),
                               window_effective_qps=sum(record["ok"] for record in within)/window,
                               drain_seconds=max(0, completion["elapsed_seconds"]-window))
            summary["protocols"] = {}
            for protocol in sorted({record["transport"] for record in requests}):
                subset = [record for record in requests if record["transport"] == protocol]
                latencies = sorted(record["elapsed_us"] for record in subset)
                summary["protocols"][protocol] = {"completed": len(subset), "failed": sum(not record["ok"] for record in subset),
                    "p99_us": latencies[math.ceil(len(latencies)*0.99)-1],
                    "kinds": {kind: sum(record["kind"] == kind for record in subset) for kind in sorted({record["kind"] for record in subset})}}
        else:
            summary.update(next(record for record in records if record["event"] == "publish"))
        return summary

    def profile(self, profile):
        self.docker("stats", "--no-stream", "--format", "{{json .}}", save=profile + "-preflight-container-load.jsonl")
        publication = self.job(profile + "-publish")
        self.record_capacity(profile, publication, "publish")
        self.assertEqual(publication["metrics_errors"], 0)
        self.assertGreater(publication["rss_max"], 0)
        update = self.job(profile + "-update")
        self.record_capacity(profile, update, "update")
        self.assertEqual(update["metrics_errors"], 0)
        self.assertGreater(update["rss_max"], 0)
        for mode, arrival in [("core", "closed"), ("http", "open"), ("grpc", "open"), ("mixed", "open"),
                              ("http", "closed"), ("grpc", "closed"), ("mixed", "closed")]:
            with self.subTest(mode=mode, arrival=arrival):
                result = self.job(f"{profile}-{mode}-{arrival}")
                self.record_capacity(profile, result, mode)
                self.assertGreater(result["executed"], 0)
                self.assertEqual(result["failed"], 0)
                self.assertEqual(result.get("generator_rejected", 0), 0)
                if mode != "core":
                    self.assertEqual(result["metrics_errors"], 0)
                    self.assertGreater(result["cpu_samples"], 0)
                    self.assertGreater(result["rss_max"], 0)
                if arrival == "open":
                    self.assertEqual(result["offered"], 5000)
                    self.assertEqual(result["completed"], result["offered"])

    @staticmethod
    def comparisons(targets, result, mode):
        if mode in {"publish", "update"}:
            metrics = [("elapsed_us", 30000000, "<="), ("service_lifetime_peak_rss", targets["update_rss_bytes"], "<=")]
        else:
            metrics = [("effective_qps", targets["completed_qps"], ">="), ("p99_us", targets["p99_us"], "<=")]
            if mode != "core":
                metrics.append(("rss_max", targets["steady_rss_bytes"], "<="))
        return [{"metric": key, "actual": result[key], "target": target, "comparison": comparison,
                 "status": "PASS" if (result[key] >= target if comparison == ">=" else result[key] <= target) else "FAIL"}
                for key, target, comparison in metrics]

    def record_capacity(self, profile, result, mode):
        self.observations["jobs"].append({"name": result["name"], "profile": profile,
            "source": result["name"] + ".jsonl", "image": self.image,
            "comparisons": self.comparisons(self.targets["profiles"][profile], result, mode),
            "measurement_classification": "unphased_post_update_rss_not_steady_acceptance" if mode not in {"publish", "update", "core"} else mode})
        (self.evidence / "capacity-observations.json").write_text(json.dumps(self.observations, indent=2))

    def test_b0_compat_core_http_grpc_and_mixed(self):
        self.profile("B0")

    def test_b1_enhanced_core_http_grpc_and_mixed(self):
        self.profile("B1")


class HarnessTests(unittest.TestCase):
    def test_capacity_failures_remain_exact_observations(self):
        observations = Baselines.comparisons({"completed_qps": 500, "p99_us": 10000, "steady_rss_bytes": 256},
            {"effective_qps": 499.999, "p99_us": 9999, "rss_max": 257}, "http")
        self.assertEqual([row["status"] for row in observations], ["FAIL", "PASS", "FAIL"])
        self.assertEqual(observations[0]["actual"], 499.999)
    def test_actual_metric_names_are_required_and_preserved(self):
        result = Baselines.summary("publish", [
            {"event": "resource", "metrics": ["textguard_process_resident_memory_bytes 1234", "textguard_process_cpu_seconds_total 1.5"]},
            {"event": "publish", "elapsed_us": 5}])
        self.assertEqual((result["rss_max"], result["cpu_seconds_observed"]), (1234, [1.5]))
        self.assertEqual(Baselines.summary("publish", [{"event": "publish"}])["rss_samples"], 0)

    def test_drain_is_retained_in_elapsed_and_separated_from_window(self):
        rows = [{"event": "request", "transport": "http", "kind": "no_hit", "elapsed_us": 2000,
                 "completed_at_us": completed, "ok": True, "schedule_delay_us": 0} for completed in [9999, 10001]]
        rows.append({"event": "summary", "elapsed_seconds": .010001, "arrival_window_seconds": .01})
        result = Baselines.summary("http", rows)
        self.assertEqual((result["window_completed"], result["tail_completed"]), (1, 1))
        self.assertLess(result["effective_qps"], 200)


if __name__ == "__main__":
    if sys.argv[1:] == ["--list"]:
        print(json.dumps(unittest.defaultTestLoader.getTestCaseNames(Baselines)))
    elif sys.argv[1:] == ["--selftest-list"]:
        print(json.dumps(unittest.defaultTestLoader.getTestCaseNames(HarnessTests)))
    elif sys.argv[1:] == ["--selftest"]:
        unittest.main(argv=[sys.argv[0], "HarnessTests"])
    elif any(not os.environ.get(key) for key in ["TEXTGUARD_PERF_IMAGE", "TEXTGUARD_PERF_EVIDENCE", "TEXTGUARD_PERF_LOCK"]):
        print("BLOCKED_ENV: immutable performance image, external evidence and exclusive resource lock required")
        raise SystemExit(2)
    else:
        unittest.main(argv=[sys.argv[0], "Baselines"])
