FROM python:3.11-slim
WORKDIR /app
RUN pip install --no-cache-dir \
    --index-url https://pypi.org/simple \
    --trusted-host pypi.org \
    --trusted-host files.pythonhosted.org \
    flask
COPY admin.py .
EXPOSE 8080
CMD ["python", "admin.py"]