"""
Dominion Observatory External Authorization Service for AgentGateway.

This lightweight HTTP service implements the ext-authz protocol.
AgentGateway calls this service before routing each MCP request.
The service checks the target server's trust score via Observatory
and returns allow/deny.

Usage:
    pip install flask httpx
    python authz-service.py

Configuration via environment variables:
    TRUST_THRESHOLD  - Minimum trust score (0-100). Default: 60
    OBSERVATORY_URL  - Observatory base URL. Default: https://dominionobservatory.com
    CACHE_TTL        - Score cache TTL in seconds. Default: 300
    PORT             - Service port. Default: 8080
"""

import os
import time
import logging
import json
from flask import Flask, request, jsonify
import httpx

app = Flask(__name__)
logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

TRUST_THRESHOLD = float(os.environ.get("TRUST_THRESHOLD", "60"))
OBSERVATORY_URL = os.environ.get("OBSERVATORY_URL", "https://dominionobservatory.com")
CACHE_TTL = float(os.environ.get("CACHE_TTL", "300"))

# In-memory trust score cache
_cache: dict[str, tuple[float, dict]] = {}


def check_trust(server_url: str) -> dict:
    """Query Observatory trust API with caching."""
    now = time.time()

    if server_url in _cache:
        cached_time, cached_data = _cache[server_url]
        if now - cached_time < CACHE_TTL:
            return cached_data

    try:
        resp = httpx.get(
            f"{OBSERVATORY_URL}/api/trust",
            params={"url": server_url},
            timeout=10.0,
        )
        data = resp.json()
        _cache[server_url] = (now, data)
        return data
    except Exception as e:
        logger.warning(f"Observatory check failed: {e}")
        # Fail open — allow on API error
        return {"found": False, "trust_score": None, "error": str(e)}


@app.route("/authorize", methods=["GET", "POST"])
def authorize():
    """External authorization endpoint called by AgentGateway.

    Checks the X-MCP-Server-URL header against Observatory's trust scores.
    Returns 200 to allow, 403 to deny.
    """
    server_url = request.headers.get("X-MCP-Server-URL", "")

    if not server_url:
        # No server URL in request — allow (non-MCP traffic)
        return "", 200

    trust_data = check_trust(server_url)
    score = trust_data.get("trust_score")

    if score is None:
        # Server not tracked — allow (fail open)
        logger.info(f"ALLOW (untracked): {server_url}")
        return "", 200

    if score >= TRUST_THRESHOLD:
        logger.info(f"ALLOW: {server_url} (score={score})")
        return "", 200
    else:
        logger.warning(f"DENY: {server_url} (score={score} < {TRUST_THRESHOLD})")
        return jsonify({
            "error": "MCP server trust check failed",
            "server_url": server_url,
            "trust_score": score,
            "threshold": TRUST_THRESHOLD,
            "details_url": f"{OBSERVATORY_URL}/api/trust?url={server_url}",
        }), 403


@app.route("/health", methods=["GET"])
def health():
    """Health check endpoint."""
    return jsonify({"status": "ok", "threshold": TRUST_THRESHOLD})


if __name__ == "__main__":
    port = int(os.environ.get("PORT", "8080"))
    logger.info(f"Starting Observatory authz service on port {port}")
    logger.info(f"Trust threshold: {TRUST_THRESHOLD}")
    logger.info(f"Observatory URL: {OBSERVATORY_URL}")
    app.run(host="0.0.0.0", port=port)
