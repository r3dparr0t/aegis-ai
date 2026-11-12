#!/usr/bin/env python3
"""
tls_raw_path_poc — یک سرور HTTPS عمداً آسیب‌پذیر، فقط با stdlib پایتون
(بدون Flask/urllib framework) که دو چیز رو تست می‌کنه:

۱. RawHttpExecutor واقعاً می‌تونه به یه هدف HTTPS وصل بشه (handshake،
   نوشتن request، خوندن response).
۲. بایت‌های خامِ path، حتی از پشت یه لایه‌ی TLS، دقیقاً همون‌طور که
   کلاینت فرستاده به دست کد مدیریت درخواست می‌رسن — یعنی همون قرارداد
   RawHttpExecutor (نه reqwest) بعد از اضافه شدن TLS هم برقراره.

آسیب‌پذیری: الگوی کلاسیکِ «چک قبل از decode» (ریشه‌ی خیلی از CVEهای
path traversal، از جمله چیزی که CVE-2021-41773 رو ممکن می‌کنه):
  - روی self.path خامِ HTTP (قبل از urldecode) فقط دنبال ".." تحت‌اللفظی
    می‌گردیم.
  - بعد percent-decode می‌کنیم.
  - یعنی "/.%2e/.%2e/secret/flag.txt" از چک رد می‌شه (چون ".." تحت‌اللفظی
    توش نیست)، ولی بعد از decode به "/../../secret/flag.txt" تبدیل می‌شه
    و از public/ بیرون می‌زنه.

این مسیر فقط با یه کلاینتی قابل اکسپلویته که بایت خامِ ".%2e" رو دست‌نخورده
بفرسته — دقیقاً همون کاری که RawHttpExecutor می‌کنه و reqwest/`url` (که
طبق استاندارد WHATWG این سگمنت‌ها رو موقع parse کردن collapse می‌کنه)
نمی‌کنه.
"""
import http.server
import os
import ssl
import sys
import urllib.parse

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "public")
CERT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "cert.pem")
KEY = os.path.join(os.path.dirname(os.path.abspath(__file__)), "key.pem")
PORT = int(os.environ.get("PORT", "8443"))


class Handler(http.server.BaseHTTPRequestHandler):
    server_version = "tls_raw_path_poc/1.0"

    def _serve(self):
        raw_path = self.path  # دقیقاً همون بایت‌هایی که روی سیم اومدن

        # چک «امن‌نما» روی مسیر خام، قبل از decode — همین‌جا باگه.
        if ".." in raw_path:
            self.send_response(403)
            self.end_headers()
            self.wfile.write(b"blocked: literal dot-segment")
            return

        decoded = urllib.parse.unquote(raw_path.split("?", 1)[0])
        if decoded.endswith("/") or decoded == "":
            decoded += "index.html"
        # اینجا دیگه دیر شده: ".%2e" الان به ".." تبدیل شده.
        target = os.path.normpath(os.path.join(ROOT, decoded.lstrip("/")))

        try:
            with open(target, "rb") as f:
                data = f.read()
            self.send_response(200)
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
        except (OSError, IsADirectoryError):
            self.send_response(404)
            self.end_headers()
            self.wfile.write(b"not found")

    def do_GET(self):
        self._serve()

    def log_message(self, fmt, *args):
        sys.stderr.write("tls_raw_path_poc: " + (fmt % args) + "\n")


def main():
    server = http.server.ThreadingHTTPServer(("0.0.0.0", PORT), Handler)
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx.load_cert_chain(certfile=CERT, keyfile=KEY)
    server.socket = ctx.wrap_socket(server.socket, server_side=True)
    print(f"tls_raw_path_poc listening on :{PORT}", file=sys.stderr)
    server.serve_forever()


if __name__ == "__main__":
    main()
