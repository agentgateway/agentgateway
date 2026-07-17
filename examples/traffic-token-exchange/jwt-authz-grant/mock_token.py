#!/usr/bin/env python3
"""Tiny RFC-7523-friendly mock token endpoint. Logs the received form body
and returns a Bearer access token for any POST."""
import json
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import parse_qsl

PORT = 7090

class H(BaseHTTPRequestHandler):
    def do_POST(self):
        n = int(self.headers.get("content-length", 0))
        raw = self.rfile.read(n).decode("utf-8", "replace")
        form = dict(parse_qsl(raw, keep_blank_values=True))
        # Redact the assertion value for log readability, keep its prefix
        printable = dict(form)
        if "assertion" in printable:
            printable["assertion"] = printable["assertion"][:24] + "...(%d chars)" % len(form["assertion"])
        print("MOCK TOKEN REQUEST  path=%s  authz=%r  form=%s" % (
            self.path, self.headers.get("authorization"), json.dumps(printable)), flush=True)
        body = json.dumps({
            "access_token": "mock-jwt-bearer-access-token",
            "token_type": "Bearer",
            "expires_in": 300,
        }).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *a):
        pass

HTTPServer(("127.0.0.1", PORT), H).serve_forever()
