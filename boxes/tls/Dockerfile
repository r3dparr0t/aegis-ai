FROM python:3.12-slim

RUN apt-get update \
 && apt-get install -y --no-install-recommends openssl \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY app.py .
COPY public ./public
COPY secret ./secret

# گواهی self-signed موقع build ساخته می‌شه — هر بار docker compose build
# یه گواهی تازه می‌سازه. عمداً کوتاه‌مدت (۲ روز) چون این فقط یه PoC
# یک‌بارمصرفه، نه چیزی که قراره دائمی بمونه.
RUN openssl req -x509 -newkey rsa:2048 -keyout key.pem -out cert.pem \
    -days 2 -nodes -subj "/CN=localhost"

EXPOSE 8443
CMD ["python3", "app.py"]
