FROM python:3.11-slim
WORKDIR /app
RUN pip install --no-cache-dir flask httpx
COPY authz-service.py .
CMD ["python", "authz-service.py"]
