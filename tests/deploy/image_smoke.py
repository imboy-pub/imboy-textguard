import json
import os
from pathlib import Path
import secrets
import subprocess
import sys
import tempfile
import time
import unittest
import uuid


def docker(*arguments, check=True):
    return subprocess.run(["docker", *arguments], check=check, capture_output=True,
                          text=True, timeout=30)


class ImageSmoke(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.image = os.environ["TEXTGUARD_IMAGE"]
        if not cls.image.startswith("sha256:") or len(cls.image) != 71:
            raise ValueError("TEXTGUARD_IMAGE must be the immutable local image ID")
        cls.prefix = "textguard-t17-" + uuid.uuid4().hex[:12]
        cls.container = cls.prefix + "-server"
        cls.initializer = cls.prefix + "-init"
        cls.volume = cls.prefix + "-secrets"
        cls.rules = cls.prefix + "-rules"
        cls.addClassCleanup(cls.cleanup)
        for volume in [cls.volume, cls.rules]:
            docker("volume", "create", "--label", "imboy.task=textguard-T17", volume)
        docker("create", "--name", cls.initializer, "--user", "0:0",
               "--entrypoint", "/bin/sh", "--network", "none",
               "--mount", f"type=volume,src={cls.volume},dst=/secrets",
               "--mount", f"type=volume,src={cls.rules},dst=/rules", cls.image, "-c",
               "cp /tmp/check /secrets/check && cp /tmp/publish /secrets/publish && "
               "chown -R 10001:10001 /secrets /rules && chmod 700 /secrets /rules && "
               "chmod 600 /secrets/check /secrets/publish")
        with tempfile.TemporaryDirectory(prefix="textguard-t17-secrets-") as directory:
            for name in ["check", "publish"]:
                path = Path(directory) / name
                path.write_text(secrets.token_hex(32))
                path.chmod(0o600)
                docker("cp", str(path), f"{cls.initializer}:/tmp/{name}")
        docker("start", "--attach", cls.initializer)
        status = json.loads(docker("inspect", cls.initializer).stdout)[0]["State"]
        if status["ExitCode"] != 0:
            raise RuntimeError("private-volume initialization failed")
        docker("run", "-d", "--name", cls.container,
               "--label", "imboy.task=textguard-T17", "--network", "none",
               "--cpus", "2", "--memory", "2g", "--pids-limit", "64",
               "--read-only", "--cap-drop", "ALL", "--security-opt", "no-new-privileges:true",
               "--mount", f"type=volume,src={cls.volume},dst=/run/secrets,readonly",
               "--mount", f"type=volume,src={cls.rules},dst=/var/lib/textguard", cls.image)
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            probe = docker("exec", cls.container, "/usr/local/bin/imboy-textguard", "--healthcheck", check=False)
            if probe.returncode == 0:
                return
            time.sleep(0.1)
        raise RuntimeError("container did not become live")

    @classmethod
    def cleanup(cls):
        for name in [cls.container, cls.initializer, cls.prefix + "-missing"]:
            docker("rm", "-f", name, check=False)
        for name in [cls.volume, cls.rules]:
            docker("volume", "rm", name, check=False)

    def test_nonroot_readonly_and_real_resource_quotas(self):
        inspect = json.loads(docker("inspect", self.container).stdout)[0]
        self.assertEqual(inspect["Config"]["User"], "10001:10001")
        host = inspect["HostConfig"]
        self.assertEqual(host["NanoCpus"], 2_000_000_000)
        self.assertEqual(host["Memory"], 2 * 1024 ** 3)
        self.assertEqual(host["PidsLimit"], 64)
        self.assertTrue(host["ReadonlyRootfs"])
        self.assertIn("ALL", host["CapDrop"])
        self.assertIn("no-new-privileges:true", host["SecurityOpt"])
        self.assertEqual(docker("exec", self.container, "id", "-u").stdout.strip(), "10001")
        attempt = docker("exec", self.container, "sh", "-c", "touch /forbidden-write", check=False)
        self.assertNotEqual(attempt.returncode, 0)
        volume = next(item for item in inspect["Mounts"] if item["Destination"] == "/var/lib/textguard")
        self.assertEqual(volume["Type"], "volume")
        self.assertTrue(volume["Source"].startswith("/var/lib/docker/volumes/"))

    def test_z_health_and_sigterm_exit(self):
        self.assertEqual(docker("exec", self.container, "/usr/local/bin/imboy-textguard", "--healthcheck").returncode, 0)
        docker("stop", "--time", "70", self.container)
        state = json.loads(docker("inspect", self.container).stdout)[0]["State"]
        self.assertEqual(state["ExitCode"], 0)
        self.assertFalse(state["OOMKilled"])

    def test_missing_secrets_fail_closed(self):
        result = docker("run", "--name", self.prefix + "-missing", "--network", "none",
                        "--read-only", self.image, check=False)
        self.assertEqual(result.returncode, 2)
        self.assertIn("SECRET_FILE_UNREADABLE", result.stderr)

    def test_metrics_are_real_linux_process_metrics(self):
        result = docker("exec", self.container, "/bin/bash", "-c",
                        "exec 3<>/dev/tcp/127.0.0.1/8083; "
                        "printf 'GET /metrics HTTP/1.1\\r\\nHost: localhost\\r\\nConnection: close\\r\\n\\r\\n' >&3; cat <&3")
        self.assertTrue(result.stdout.startswith("HTTP/1.1 200"))
        metrics = result.stdout.split("\r\n\r\n", 1)[-1] if "\r\n\r\n" in result.stdout else result.stdout.split("\n\n", 1)[1]
        values = dict(line.split(" ", 1) for line in metrics.splitlines() if " " in line and not line.startswith("#"))
        self.assertGreater(float(values["textguard_process_resident_memory_bytes"]), 0)
        self.assertGreaterEqual(float(values["textguard_process_cpu_seconds_total"]), 0)


if __name__ == "__main__":
    if sys.argv[1:] == ["--list"]:
        print(json.dumps(unittest.defaultTestLoader.getTestCaseNames(ImageSmoke)))
    else:
        unittest.main(verbosity=2)
