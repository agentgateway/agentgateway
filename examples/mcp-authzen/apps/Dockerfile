FROM python:3.12-slim
WORKDIR /app
RUN pip install --no-cache-dir mcp
COPY payments_mcp_server.py .
EXPOSE 3001
CMD ["python3", "payments_mcp_server.py"]
