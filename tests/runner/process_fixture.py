import os
import signal
import subprocess
import sys
import time


mode = sys.argv[1]
if mode == "ignore-term":
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
elif mode in {"descendant", "orphan"}:
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    child = subprocess.Popen([sys.executable, "-B", __file__, "ignore-term"])
    print(f"DESCENDANT_PID={child.pid}", flush=True)
    if mode == "orphan":
        sys.exit(0)
print(f"READY_PID={os.getpid()}", flush=True)
time.sleep(30)
