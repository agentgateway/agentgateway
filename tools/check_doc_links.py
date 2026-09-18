#!/usr/bin/env python3
"""
Repoint agentgateway.dev doc links that have moved.

WHY THIS EXISTS. Doc comments across this repo cite absolute agentgateway.dev URLs, and
the docs site reorganises from time to time. When a page moves, the site keeps serving the
old path through a redirect, so nothing looks broken and the stale URL sits here
indefinitely. It stops being harmless the day that redirect is retired.

The staleness is invisible from inside this repo, because a 301 is not an error. So this
script asks the live site instead of guessing: it follows each URL and, when the site
answers with a permanent redirect, rewrites the link to wherever it actually landed.

Release tags make it worse. The docs site generates its reference pages from a release
TAG, so a link fixed here after a tag is cut cannot reach the published docs until the
next release. Fixing them promptly here is what keeps that window short.

WHAT IT WILL NOT DO. A URL that 404s is reported and left alone: there is nothing to
resolve it to, and inventing a target would be worse than leaving a link a human can find.
Only permanent redirects are followed, so a temporary redirect (a maintenance page, a
login interstitial) never gets baked in.

GENERATED FILES. controller-gen copies these doc comments verbatim into the CRD manifests,
and the URLs never wrap, so replacing the same string in both the Go source and the
generated YAML produces exactly what regenerating would produce. That keeps the tree
consistent without running codegen here. If a URL ever does wrap, the repo's own
generated-code check catches the mismatch.

Usage:
  python3 tools/check_doc_links.py                      # report only
  python3 tools/check_doc_links.py --write              # rewrite in place
  python3 tools/check_doc_links.py --write --summary out.md
"""

import argparse
import os
import re
import subprocess
import sys
import urllib.error
import urllib.request
from urllib.parse import urljoin

SITE = "https://agentgateway.dev"

# Stops at whitespace and at the punctuation that usually ends a sentence or wraps a link
# in Markdown, Go comments and YAML. A trailing full stop is peeled off separately, since a
# period is legal inside a URL and only the LAST one is likely to be prose.
URL_RE = re.compile(r"https://agentgateway\.dev/[^\s<>\"'`)\]}|\\]*")

# Extensions worth reading. Anything else in a source tree is either binary or not prose.
TEXT_SUFFIXES = {
    ".go", ".md", ".yaml", ".yml", ".json", ".txt", ".rs", ".ts", ".tsx",
    ".js", ".jsx", ".toml", ".sh", ".tmpl", ".gotmpl", ".proto", ".dockerfile",
}

# Permanent redirects only. A 302/307 is by definition not a settled home, so following one
# would bake a temporary answer into the source.
PERMANENT = {301, 308}

MAX_HOPS = 5
TIMEOUT = 20


class NoRedirects(urllib.request.HTTPRedirectHandler):
    """Surface each redirect instead of following it, so hops can be inspected one at a time."""

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


OPENER = urllib.request.build_opener(NoRedirects)
# Some CDNs answer a bare scripted request differently than a browser. Identify honestly
# rather than defaulting to the Python UA, which is more likely to be rate limited.
OPENER.addheaders = [("User-Agent", "agentgateway-doc-link-check")]


def fetch_status(url):
    """Return (status, location) for one request, without following the redirect."""
    request = urllib.request.Request(url, method="HEAD")
    try:
        with OPENER.open(request, timeout=TIMEOUT) as response:
            return response.status, None
    except urllib.error.HTTPError as e:
        return e.code, e.headers.get("Location")
    except (urllib.error.URLError, TimeoutError, OSError) as e:
        print(f"    could not reach {url}: {e}", file=sys.stderr)
        return None, None


def resolve(url):
    """Follow permanent redirects to a final URL.

    Returns (final_url, status). final_url is unchanged when nothing moved. status is the
    final response code, so a caller can tell a healthy destination from a 404.
    """
    current = url
    for _ in range(MAX_HOPS):
        status, location = fetch_status(current)
        if status is None:
            return url, None
        if status not in PERMANENT:
            return current, status
        if not location:
            return current, status
        current = urljoin(current, location)
    print(f"    too many redirects from {url}", file=sys.stderr)
    return url, None


def same_but_for_slash(before, after):
    """True when two URLs differ only by a trailing slash.

    Hugo serves /page and /page/ as the same page and redirects one to the other, so this
    kind of redirect says nothing about a page having moved.
    """
    return before.rstrip("/") == after.rstrip("/")


def tracked_files():
    """Every tracked text file. git ls-files keeps vendored and ignored trees out for free."""
    out = subprocess.run(
        ["git", "ls-files", "-z"], capture_output=True, text=True, check=True
    ).stdout
    for path in out.split("\0"):
        if not path:
            continue
        suffix = os.path.splitext(path)[1].lower()
        if suffix in TEXT_SUFFIXES and os.path.isfile(path):
            yield path


def collect_urls(paths):
    """Map each distinct URL to the files citing it."""
    found = {}
    for path in paths:
        try:
            with open(path, encoding="utf-8") as f:
                content = f.read()
        except (UnicodeDecodeError, OSError):
            continue
        for match in URL_RE.finditer(content):
            url = match.group(0).rstrip(".,;:")
            found.setdefault(url, set()).add(path)
    return found


def rewrite(paths, replacements):
    """Apply every replacement across the given files. Returns the files that changed."""
    changed = set()
    for path in paths:
        try:
            with open(path, encoding="utf-8") as f:
                content = f.read()
        except (UnicodeDecodeError, OSError):
            continue
        updated = content
        for before, after in replacements.items():
            updated = updated.replace(before, after)
        if updated != content:
            with open(path, "w", encoding="utf-8") as f:
                f.write(updated)
            changed.add(path)
    return changed


def write_summary(path, moved, broken):
    lines = [
        "Some doc links in this repo point at pages that have moved on agentgateway.dev.",
        "The site still redirects the old paths, so nothing reports them as broken, but they",
        "stop working the day those redirects are retired.",
        "",
    ]
    if moved:
        lines += ["Each link below was followed to where it actually resolves:", "",
                  "| Was | Now |", "|-----|-----|"]
        lines += [f"| `{before}` | `{after}` |" for before, after in sorted(moved.items())]
        lines.append("")
    if broken:
        lines += [
            "These returned an error and were left untouched, since there is nothing to",
            "resolve them to:",
            "",
        ]
        lines += [f"- `{url}` returned {status}" for url, status in sorted(broken.items())]
        lines.append("")
    lines += [
        "Generated CRD manifests are updated alongside their Go doc comments, matching what",
        "controller-gen would emit, so no regeneration is required.",
        "",
        "Opened automatically by the check-doc-links workflow.",
    ]
    with open(path, "w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true", help="rewrite files in place")
    parser.add_argument("--summary", help="write a pull request body to this path")
    args = parser.parse_args()

    paths = sorted(tracked_files())
    urls = collect_urls(paths)
    print(f"Checking {len(urls)} distinct agentgateway.dev link(s) across {len(paths)} file(s)")

    moved, broken = {}, {}
    for url in sorted(urls):
        # A fragment never reaches the server, so it cannot come back in a Location header.
        # Resolve the page, then put the anchor back, or the rewrite silently drops the
        # reader at the top of the page instead of the section the link was written for.
        base, _, fragment = url.partition("#")
        resolved, status = resolve(base)
        if status is None:
            continue
        final = resolved if "#" in resolved or not fragment else f"{resolved}#{fragment}"

        if final != url and status == 200:
            if same_but_for_slash(url, final):
                # The server canonicalising /page to /page/ is not a page move, and a pull
                # request that only adds trailing slashes is noise a reviewer has to wade
                # through to find the real fixes.
                continue
            moved[url] = final
            print(f"  MOVED {url}\n     -> {final}")
        elif status >= 400:
            broken[url] = status
            print(f"  BROKEN ({status}) {url}")

    if not moved:
        print("No moved links found")
        if broken:
            print(f"{len(broken)} link(s) returned an error; see above")
        if args.summary and broken:
            write_summary(args.summary, moved, broken)
        set_output(False)
        return

    if args.write:
        changed = rewrite(paths, moved)
        print(f"Rewrote {len(changed)} file(s)")
        for path in sorted(changed):
            print(f"  {path}")

    if args.summary:
        write_summary(args.summary, moved, broken)

    set_output(bool(moved))


def set_output(changed):
    """Tell the workflow whether anything moved. Echoed too, so a local run reads the same."""
    value = "true" if changed else "false"
    print(f"changed={value}")
    github_output = os.environ.get("GITHUB_OUTPUT")
    if github_output:
        with open(github_output, "a", encoding="utf-8") as f:
            f.write(f"changed={value}\n")


if __name__ == "__main__":
    main()
