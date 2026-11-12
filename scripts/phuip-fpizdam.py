#!/usr/bin/env python3
import subprocess, sys, os

if len(sys.argv) < 2:
    print("fail", end="", flush=True)
    sys.exit(0)

GO_BIN = os.path.expanduser("~/go/bin/phuip-fpizdam")
if not os.path.exists(GO_BIN):
    print("fail", end="", flush=True)
    sys.exit(0)

try:
    r = subprocess.run([GO_BIN, sys.argv[1]], capture_output=True, text=True, timeout=90)
    out = (r.stdout or "") + (r.stderr or "")
    print("ok" if "Success!" in out else "fail", end="", flush=True)
except:
    print("fail", end="", flush=True)
sys.exit(0)
