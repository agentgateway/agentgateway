"""Deterministic OpenAI-compatible test server and local OTLP capture sink.

Only synthetic data belongs here. Neither real credentials nor paid models are used.
"""

import base64
import gzip
import json
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

TRACES = []
LOCK = threading.Lock()


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass  # Do not log request bodies or headers.

    def reply(self, status, data, content_type="application/json"):
        payload = json.dumps(data).encode() if isinstance(data, (dict, list)) else data
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def do_GET(self):
        if self.path == "/health":
            self.reply(200, {"status": "ok"})
        elif self.path == "/captured-traces":
            with LOCK:
                self.reply(200, list(TRACES))
        else:
            self.reply(404, {"error": "not found"})

    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        if self.path == "/v1/traces":
            if self.headers.get("Content-Encoding") == "gzip":
                body = gzip.decompress(body)
            with LOCK:
                TRACES.append(base64.b64encode(body).decode())
                del TRACES[:-100]  # Bound memory even during repeated smoke runs.
            self.reply(200, b"", "application/x-protobuf")
            return
        if self.path != "/v1/chat/completions":
            self.reply(404, {"error": "not found"})
            return
        request = json.loads(body)
        model = request.get("model", "datadog-test")
        status = {"datadog-error": 500, "datadog-rate-limit": 429}.get(model)
        if status:
            self.reply(
                status,
                {
                    "error": {
                        "message": "Synthetic provider failure",
                        "type": "test_error",
                    }
                },
            )
            return
        usage = {
            "prompt_tokens": 10,
            "completion_tokens": 4,
            "total_tokens": 14,
            "prompt_tokens_details": {"cached_tokens": 2},
        }
        common = {"id": "chatcmpl-synthetic", "created": 1700000000, "model": model}
        if not request.get("stream"):
            self.reply(
                200,
                {
                    **common,
                    "object": "chat.completion",
                    "usage": usage,
                    "choices": [
                        {
                            "index": 0,
                            "message": {
                                "role": "assistant",
                                "content": "Synthetic reply.",
                            },
                            "finish_reason": "stop",
                        }
                    ],
                },
            )
            return
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Connection", "close")
        self.end_headers()
        try:
            for delta in (
                {"role": "assistant"},
                {"content": "Synthetic "},
                {"content": "reply."},
            ):
                time.sleep(0.03)
                chunk = {
                    **common,
                    "object": "chat.completion.chunk",
                    "choices": [{"index": 0, "delta": delta, "finish_reason": None}],
                }
                self.wfile.write(f"data: {json.dumps(chunk)}\n\n".encode())
                self.wfile.flush()
            final = {
                **common,
                "object": "chat.completion.chunk",
                "usage": usage,
                "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
            }
            self.wfile.write(f"data: {json.dumps(final)}\n\ndata: [DONE]\n\n".encode())
            self.wfile.flush()
        except (BrokenPipeError, ConnectionResetError):
            pass


if __name__ == "__main__":
    ThreadingHTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
