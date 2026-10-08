FROM python:3.11-slim

WORKDIR /app

RUN pip install --no-cache-dir \
    --index-url https://mirror-pypi.runflare.com/simple/ \
    --trusted-host mirror-pypi.runflare.com \
    flask

COPY admin.py .
EXPOSE 8080
CMD ["python", "admin.py"]