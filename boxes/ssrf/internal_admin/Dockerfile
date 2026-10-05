FROM python:3.11-slim
WORKDIR /app
RUN pip install --no-cache-dir flask
COPY admin.py .
EXPOSE 8080
CMD ["python", "admin.py"]