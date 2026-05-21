#!/usr/bin/env python3
"""
Dominion Observatory Trust Verification - ext_authz sidecar for agentgateway.

This lightweight HTTP server acts as an ext_authz backend for agentgateway.
It checks MCP server behavioral trust scores via the Dominion Observatory API
and returns allow/deny decisions.

Usage:
    python trust-authz-server.py [--port 8990] [--threshold 60] [--cache-ttl 300]

API (Dominion Observatory):
    GET https://dominion-observatory.sgdata.workers.dev/benchmark/{server_name}
    Response: {"trust_score": 0-100, ...}
"""

import argparse
import json
import logging
import time
import urllib.request
import urllib.error
import urllib.parse
from http.server import HTTPServer, BaseHTTPRequestHandler

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - %(name)s - %(levelname)s - %(message)s",
)
logger = logging.getLogger("dominion-trust-authz")

# --- Configuration ---
DOMINION_API_BASE = "https://dominion-observatory.sgdata.workers.dev"
DEFAULT_THRESHOLD = 60
DEFAULT_CACHE_TTL = 300  # 5 minutes
DEFAULT_PORT = 8990
REQUEST_TIMEOUT = 5

# --- In-memory cache ---
_cache = {}


def get_trust_score(server_name, cache_ttl):
    """Fetch trust score from Dominion Observatory with caching."""
    now = time.monotonic()

    # Check cache
    if server_name in _cache:
        entry = _cache[server_name]
        if now - entry["timestamp"] < cache_ttl:
            logger.debug(f"Cache hit for '{server_name}': score={entry['score']}")
            return entry["score"]
        else:
            del _cache[server_name]

    # Fetch from API
    url = f"{DOMINION_API_BASE}/benchmark/{urllib.parse.quote(server_name, safe='')}"
    logger.info(f"Fetching trust score for '{server_name}' from {url}")

    try:
        req = urllib.request.Request(
            url,
            headers={
                "Accept": "application/json",
                "User-Agent": "agentgateway-dominion-authz/1.0",
            },
        )
        with urllib.request.urlopen(req, timeout=REQUEST_TIMEOUT) as resp:
            data = json.loads(resp.read().decode("utf-8"))

        score = data.get("trust_score")
        if score is not None:
            _cache[server_name] = {"score": score, "timestamp": now}
            logger.info(f"Trust score for '{server_name}': {score}")
            return score
        else:
            logger.warning(f"No trust_score in response for '{server_name}': {data}")
            return None

    except urllib.error.HTTPError as e:
        logger.warning(f"HTTP {e.code} from Dominion API for '{server_name}': {e.reason}")
        return None
    except urllib.error.URLError as e:
        logger.error(f"Cannot reach Dominion API for '{server_name}': {e.reason}")
        return None
    except Exception as e:
        logger.error(f"Error fetching trust score for '{server_name}': {e}")
        return None


class TrustAuthzHandler(BaseHTTPRequestHandler):
    """HTTP handler for ext_authz trust verification requests."""

    def do_GET(self):
        """Handle ext_authz check requests from agentgateway."""
        parsed = urllib.parse.urlparse(self.path)
        params = urllib.parse.parse_qs(parsed.query)

        if parsed.path != "/check-trust":
            self.send_response(404)
            self.end_headers()
            self.wfile.write(b"Not found")
            return

        server_name = params.get("server", [None])[0]
        if not server_name:
            self.send_response(400)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps({
                "error": "Missing 'server' query parameter"
            }).encode())
            return

        score = get_trust_score(server_name, self.server.cache_ttl)

        if score is None:
            # API unreachable or invalid response - deny by default
            self.send_response(403)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps({
                "error": f"Trust score unavailable for server '{server_name}'",
                "action": "denied",
            }).encode())
            return

        if score < self.server.threshold:
            # Trust score too low - deny
            logger.warning(
                f"DENIED: '{server_name}' has trust score {score} "
                f"(threshold: {self.server.threshold})"
            )
            self.send_response(403)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps({
                "error": f"Server '{server_name}' blocked: trust score {score} is below threshold {self.server.threshold}",
                "trust_score": score,
                "threshold": self.server.threshold,
                "action": "denied",
            }).encode())
            return

        # Trust score OK - allow
        logger.info(
            f"ALLOWED: '{server_name}' has trust score {score} "
            f"(threshold: {self.server.threshold})"
        )
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(json.dumps({
            "trust_score": score,
            "threshold": self.server.threshold,
            "action": "allowed",
        }).encode())

    def log_message(self, format, *args):
        """Redirect HTTP server logs to our logger."""
        logger.debug(f"{self.address_string()} - {format % args}")


def main():
    parser = argparse.ArgumentParser(
        description="Dominion Observatory trust verification sidecar for agentgateway"
    )
    parser.add_argument(
        "--port", type=int, default=DEFAULT_PORT,
        help=f"Port to listen on (default: {DEFAULT_PORT})",
    )
    parser.add_argument(
        "--threshold", type=int, default=DEFAULT_THRESHOLD,
        help=f"Minimum trust score to allow requests (default: {DEFAULT_THRESHOLD})",
    )
    parser.add_argument(
        "--cache-ttl", type=int, default=DEFAULT_CACHE_TTL,
        help=f"Cache TTL in seconds (default: {DEFAULT_CACHE_TTL})",
    )
    args = parser.parse_args()

    server = HTTPServer(("0.0.0.0", args.port), TrustAuthzHandler)
    server.threshold = args.threshold
    server.cache_ttl = args.cache_ttl

    logger.info(
        f"Dominion Observatory trust authz sidecar starting on port {args.port} "
        f"(threshold: {args.threshold}, cache TTL: {args.cache_ttl}s)"
    )
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        logger.info("Shutting down...")
        server.server_close()


if __name__ == "__main__":
    main()
