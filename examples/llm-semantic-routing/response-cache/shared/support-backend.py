import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

lock = threading.Lock()
state = {"invocations": 0}

def support_answer(prompt):
    text = prompt.lower()
    if "x2" in text and any(word in text for word in ("factory", "reset", "restore")):
        return (
            "To factory-reset a HomeHub X2, hold the recessed Reset button "
            "for 10 seconds until the status ring flashes amber, then release it."
        )
    if "outdoor" in text or "freezing" in text:
        return (
            "HomeHub devices are rated for indoor use only and should not "
            "be exposed to rain or freezing temperatures."
        )
    return "Contact HomeHub support with the product model and diagnostic code."

class Handler(BaseHTTPRequestHandler):
    server_version = "HomeHubSupport/1.0"

    def log_message(self, format, *args):
        return

    def write_json(self, status, payload):
        body = json.dumps(payload, separators=(",", ":")).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def read_json(self):
        length = int(self.headers.get("Content-Length", "0"))
        return json.loads(self.rfile.read(length) or b"{}")

    def do_GET(self):
        if self.path == "/healthz":
            self.write_json(200, {"status": "ok"})
            return
        if self.path == "/stats":
            with lock:
                payload = {
                    "invocations": state["invocations"],
                }
            self.write_json(200, payload)
            return
        self.write_json(404, {"error": "not found"})

    def do_POST(self):
        if self.path != "/v1/chat/completions":
            self.write_json(404, {"error": "not found"})
            return
        try:
            request = self.read_json()
            prompt = "\n".join(
                str(message.get("content", ""))
                for message in request.get("messages", [])
                if message.get("role") == "user"
            )
        except (ValueError, TypeError) as error:
            self.write_json(400, {"error": str(error)})
            return
        with lock:
            state["invocations"] += 1
        response = {
            "id": "chatcmpl-homehub-support",
            "object": "chat.completion",
            "created": 0,
            "model": "homehub-support",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": support_answer(prompt),
                },
                "finish_reason": "stop",
            }],
            "usage": {
                "prompt_tokens": 12,
                "completion_tokens": 24,
                "total_tokens": 36,
            },
        }
        self.write_json(200, response)

ThreadingHTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
