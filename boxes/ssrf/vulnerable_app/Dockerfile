FROM python:3.11-slim

WORKDIR /app

COPY requirements.txt .

RUN pip install --no-cache-dir \
    --index-url https://mirror-pypi.runflare.com/simple/ \
    --trusted-host mirror-pypi.runflare.com \
    -r requirements.txt

COPY app.py .
EXPOSE 5000
CMD ["python", "app.py"]