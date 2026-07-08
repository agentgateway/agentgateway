#!/usr/bin/env python3
"""Tiny echo backend: returns the request method, path, and headers as JSON so we
can see the Authorization: Bearer <access token> the gateway attached."""
import json
from http.server import BaseHTTPRequestHandler, HTTPServer

PORT = 9000

class Echo(BaseHTTPRequestHandler):
    def _handle(self):
        body = json.dumps({
            "method": self.command,
            "path": self.path,
            "headers": {k: v for k, v in self.headers.items()},
        }, indent=2).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    do_GET = _handle
    do_POST = _handle

    def log_message(self, *a):  # quiet
        pass

if __name__ == "__main__":
    print(f"echo backend on :{PORT}")
    HTTPServer(("127.0.0.1", PORT), Echo).serve_forever()
