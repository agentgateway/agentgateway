#!/usr/bin/env python3
"""
Tests for check_doc_links.

Run: python3 -m unittest discover -s tools -p 'test_*.py'

No network. Everything that talks to the site goes through fetch(), so these stub that one
function and exercise the logic around it. The check-doc-links workflow runs this before
letting the script rewrite anything, since a misfire there lands in a pull request against
generated API types.

Stdlib unittest only, matching the script's own no-dependencies rule.
"""

import os
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import check_doc_links as c  # noqa: E402

SITE = "https://agentgateway.dev"


class TestMetaRefreshDetection(unittest.TestCase):
    """A moved page often answers 200, so the body is the only place the move shows up."""

    def test_matches_a_hugo_alias_stub(self):
        body = (
            b'<!DOCTYPE html><html><head><title>x</title>'
            b'<meta http-equiv=refresh content="0; url=/docs/new/">'
            b"</head></html>"
        )
        self.assertEqual(c.META_REFRESH_RE.search(body).group(1), b"/docs/new/")

    def test_matches_minified_unquoted_attributes(self):
        """The site builds with --minify, which strips quotes it does not need."""
        body = b'<meta http-equiv=refresh content="0; url=/docs/new/">'
        self.assertEqual(c.META_REFRESH_RE.search(body).group(1), b"/docs/new/")

    def test_matches_quoted_attributes(self):
        body = b'<meta http-equiv="refresh" content="0; url=/docs/new/">'
        self.assertEqual(c.META_REFRESH_RE.search(body).group(1), b"/docs/new/")

    def test_ignores_an_ordinary_page(self):
        body = b"<html><head><title>Real page</title></head><body>content</body></html>"
        self.assertIsNone(c.META_REFRESH_RE.search(body))

    def test_body_limit_covers_a_themed_stub_page(self):
        """A {{< redirect >}} stub renders inside the full template: ~430 KB, tag at ~140 KB."""
        self.assertGreaterEqual(c.MAX_BODY, 500_000)


class TestResolve(unittest.TestCase):
    def _resolve_with(self, responses, url):
        """Drive resolve() against a scripted map of url -> (status, location)."""
        with mock.patch.object(c, "fetch", side_effect=lambda u: responses[u]):
            return c.resolve(url)

    def test_follows_a_permanent_redirect(self):
        final, status = self._resolve_with(
            {
                f"{SITE}/old/": (301, f"{SITE}/new/"),
                f"{SITE}/new/": (200, None),
            },
            f"{SITE}/old/",
        )
        self.assertEqual((final, status), (f"{SITE}/new/", 200))

    def test_follows_a_200_with_a_meta_refresh(self):
        final, status = self._resolve_with(
            {
                f"{SITE}/aliased/": (200, "/moved/"),
                f"{SITE}/moved/": (200, None),
            },
            f"{SITE}/aliased/",
        )
        self.assertEqual(final, f"{SITE}/moved/")

    def test_chains_a_redirect_into_a_stub_page(self):
        """The real multi-hop shape: _redirects 301, then a stub page's meta refresh."""
        final, _ = self._resolve_with(
            {
                f"{SITE}/a/": (301, f"{SITE}/b/"),
                f"{SITE}/b/": (200, "/c/"),
                f"{SITE}/c/": (200, None),
            },
            f"{SITE}/a/",
        )
        self.assertEqual(final, f"{SITE}/c/")

    def test_does_not_follow_a_temporary_redirect(self):
        final, status = self._resolve_with(
            {f"{SITE}/tmp/": (302, f"{SITE}/elsewhere/")}, f"{SITE}/tmp/"
        )
        self.assertEqual((final, status), (f"{SITE}/tmp/", 302))

    def test_never_leaves_the_host(self):
        """The site forwards a few paths to raw.githubusercontent.com downloads."""
        final, _ = self._resolve_with(
            {f"{SITE}/schema/config": (301, "https://raw.githubusercontent.com/x/y.json")},
            f"{SITE}/schema/config",
        )
        self.assertEqual(final, f"{SITE}/schema/config")

    def test_reports_a_404_rather_than_guessing(self):
        final, status = self._resolve_with({f"{SITE}/gone/": (404, None)}, f"{SITE}/gone/")
        self.assertEqual((final, status), (f"{SITE}/gone/", 404))

    def test_unreachable_returns_no_status(self):
        final, status = self._resolve_with({f"{SITE}/x/": (None, None)}, f"{SITE}/x/")
        self.assertIsNone(status)
        self.assertEqual(final, f"{SITE}/x/")


class TestSlashComparison(unittest.TestCase):
    def test_trailing_slash_only_is_not_a_move(self):
        self.assertTrue(c.same_but_for_slash(f"{SITE}/a", f"{SITE}/a/"))

    def test_trailing_slash_only_is_not_a_move_with_an_anchor(self):
        """The fragment has to be stripped first, or every anchored link reads as moved."""
        self.assertTrue(c.same_but_for_slash(f"{SITE}/a#s", f"{SITE}/a/#s"))

    def test_a_real_move_is_still_a_move(self):
        self.assertFalse(c.same_but_for_slash(f"{SITE}/a/#s", f"{SITE}/b/#s"))


class TestRewrite(unittest.TestCase):
    def test_a_prefix_url_does_not_corrupt_a_longer_one(self):
        """/install/ is a prefix of /install/advanced/; the longer URL must win its text."""
        with tempfile.TemporaryDirectory() as root:
            path = os.path.join(root, "values.yaml")
            with open(path, "w", encoding="utf-8") as f:
                f.write(
                    f"a: {SITE}/docs/install/\n"
                    f"b: {SITE}/docs/install/advanced/#namespace-discovery\n"
                )
            c.rewrite([path], {
                f"{SITE}/docs/install/": f"{SITE}/docs/documentation/install/",
                f"{SITE}/docs/install/advanced/#namespace-discovery":
                    f"{SITE}/docs/documentation/install/advanced/#namespace-discovery",
            })
            with open(path, encoding="utf-8") as f:
                out = f.read()
        self.assertIn(f"a: {SITE}/docs/documentation/install/\n", out)
        self.assertIn(
            f"b: {SITE}/docs/documentation/install/advanced/#namespace-discovery\n", out
        )
        self.assertNotIn("documentation/documentation", out)

    def test_rewriting_is_idempotent(self):
        with tempfile.TemporaryDirectory() as root:
            path = os.path.join(root, "x.md")
            with open(path, "w", encoding="utf-8") as f:
                f.write(f"see {SITE}/old/\n")
            repl = {f"{SITE}/old/": f"{SITE}/new/"}
            c.rewrite([path], repl)
            self.assertEqual(c.rewrite([path], repl), set())


if __name__ == "__main__":
    unittest.main(verbosity=2)
