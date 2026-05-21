#!/usr/bin/env python3
"""
Tests for the Dominion Observatory trust-authz-server sidecar.

Run with: python -m pytest test_trust_authz.py -v
"""

import json
import threading
import time
import urllib.request
import urllib.error
from http.server import HTTPServer
from unittest.mock import patch, MagicMock

import sys
import os

# Add parent directory to path so we can import the server module
sys.path.insert(0, os.path.dirname(__file__))

# We need to import after path manipulation
import importlib
trust_authz = importlib.import_module("trust-authz-server")

TrustAuthzHandler = trust_authz.TrustAuthzHandler
get_trust_score = trust_authz.get_trust_score


class TestGetTrustScore:
    """Tests for the get_trust_score function."""

    def setup_method(self):
        """Clear the cache before each test."""
        trust_authz._cache.clear()

    @patch("trust-authz-server.urllib.request.urlopen")
    def test_fetch_and_cache(self, mock_urlopen):
        """Test fetching a score and caching the result."""
        response_data = {"trust_score": 85}
        mock_resp = MagicMock()
        mock_resp.read.return_value = json.dumps(response_data).encode()
        mock_resp.__enter__ = lambda s: s
        mock_resp.__exit__ = MagicMock(return_value=False)
        mock_urlopen.return_value = mock_resp

        score = get_trust_score("test-server", cache_ttl=300)
        assert score == 85
        assert mock_urlopen.call_count == 1

        # Second call should use cache
        score2 = get_trust_score("test-server", cache_ttl=300)
        assert score2 == 85
        assert mock_urlopen.call_count == 1  # No additional API call

    @patch("trust-authz-server.urllib.request.urlopen")
    def test_cache_expiry(self, mock_urlopen):
        """Test that cache entries expire after TTL."""
        response_data = {"trust_score": 70}
        mock_resp = MagicMock()
        mock_resp.read.return_value = json.dumps(response_data).encode()
        mock_resp.__enter__ = lambda s: s
        mock_resp.__exit__ = MagicMock(return_value=False)
        mock_urlopen.return_value = mock_resp

        score = get_trust_score("expiry-server", cache_ttl=1)
        assert score == 70

        # Simulate cache expiry
        trust_authz._cache["expiry-server"]["timestamp"] = time.monotonic() - 2

        score2 = get_trust_score("expiry-server", cache_ttl=1)
        assert score2 == 70
        assert mock_urlopen.call_count == 2  # Had to fetch again

    @patch("trust-authz-server.urllib.request.urlopen")
    def test_api_error_returns_none(self, mock_urlopen):
        """Test that API errors return None."""
        mock_urlopen.side_effect = urllib.error.URLError("Connection refused")

        score = get_trust_score("unreachable-server", cache_ttl=300)
        assert score is None

    @patch("trust-authz-server.urllib.request.urlopen")
    def test_missing_trust_score_field(self, mock_urlopen):
        """Test handling of API responses without trust_score."""
        response_data = {"status": "unknown"}
        mock_resp = MagicMock()
        mock_resp.read.return_value = json.dumps(response_data).encode()
        mock_resp.__enter__ = lambda s: s
        mock_resp.__exit__ = MagicMock(return_value=False)
        mock_urlopen.return_value = mock_resp

        score = get_trust_score("no-score-server", cache_ttl=300)
        assert score is None


class TestTrustAuthzServer:
    """Integration tests for the ext_authz HTTP server."""

    @classmethod
    def setup_class(cls):
        """Start the authz server in a background thread."""
        cls.server = HTTPServer(("127.0.0.1", 0), TrustAuthzHandler)
        cls.server.threshold = 60
        cls.server.cache_ttl = 300
        cls.port = cls.server.server_address[1]
        cls.base_url = f"http://127.0.0.1:{cls.port}"
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()

    @classmethod
    def teardown_class(cls):
        """Stop the server."""
        cls.server.shutdown()

    def setup_method(self):
        """Clear cache between tests."""
        trust_authz._cache.clear()

    def test_missing_server_param(self):
        """Request without server parameter should return 400."""
        req = urllib.request.Request(f"{self.base_url}/check-trust")
        try:
            urllib.request.urlopen(req)
            assert False, "Should have raised HTTPError"
        except urllib.error.HTTPError as e:
            assert e.code == 400

    def test_not_found_path(self):
        """Unknown path should return 404."""
        req = urllib.request.Request(f"{self.base_url}/unknown")
        try:
            urllib.request.urlopen(req)
            assert False, "Should have raised HTTPError"
        except urllib.error.HTTPError as e:
            assert e.code == 404

    @patch.object(trust_authz, "get_trust_score")
    def test_trusted_server_allowed(self, mock_score):
        """Trusted server should get 200 response."""
        mock_score.return_value = 85
        req = urllib.request.Request(
            f"{self.base_url}/check-trust?server=trusted-server"
        )
        with urllib.request.urlopen(req) as resp:
            assert resp.status == 200
            data = json.loads(resp.read())
            assert data["action"] == "allowed"
            assert data["trust_score"] == 85

    @patch.object(trust_authz, "get_trust_score")
    def test_untrusted_server_denied(self, mock_score):
        """Untrusted server should get 403 response."""
        mock_score.return_value = 30
        req = urllib.request.Request(
            f"{self.base_url}/check-trust?server=untrusted-server"
        )
        try:
            urllib.request.urlopen(req)
            assert False, "Should have raised HTTPError"
        except urllib.error.HTTPError as e:
            assert e.code == 403
            data = json.loads(e.read())
            assert data["action"] == "denied"
            assert data["trust_score"] == 30

    @patch.object(trust_authz, "get_trust_score")
    def test_api_unavailable_denied(self, mock_score):
        """When API returns None, should deny with 403."""
        mock_score.return_value = None
        req = urllib.request.Request(
            f"{self.base_url}/check-trust?server=unknown-server"
        )
        try:
            urllib.request.urlopen(req)
            assert False, "Should have raised HTTPError"
        except urllib.error.HTTPError as e:
            assert e.code == 403
            data = json.loads(e.read())
            assert data["action"] == "denied"
