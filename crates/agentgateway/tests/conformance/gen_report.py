#!/usr/bin/env python3
"""Render a self-contained HTML conformance status page.

Reads the per-scenario `checks.json` files written by the MCP Conformance Test
Framework (`-o <dir>/<suite>`, via `make mcp-conformance-report`) and the
committed `baseline-<suite>.yml` files, then emits one `status.html` showing,
per suite, which scenarios pass, which are tracked gaps, and which have drifted
from the baseline (regressions or newly-passing/stale entries).

It also appends today's per-suite tally to a committed `status-history.json` and
renders a progression timeline from it, so the shrinking gap is visible over time
(not just as of the latest run). Each snapshot records `failing` as a map from failing
scenario (gap + regression) to the check names that failed under it, so history shows
both which scenarios failed on a date and which checks failed. A partial failure like
`server-stateless` lists only the red checks. Entries written before this field existed
carry counts only.

Each snapshot also records what it tested — the `gateway` (`{sha, ref}`, from
`--gateway-sha`/`--gateway-ref`) and the conformance `framework` version (`--framework`)
— so a date says which gateway commit was graded against which framework, not just a
score. Both render in the page header (latest run) and the per-run timeline caption.

A scenario is "failing" if any of its checks is FAILURE or WARNING, or if it
produced no checks at all. This matches the framework's baseline grader.
Cross-referenced against the baseline, each scenario lands in one bucket:

  pass        passing, not baselined        (green)   nothing to do
  gap         failing, baselined            (amber)   expected; the work ahead
  regression  failing, NOT baselined        (red)     a new failure, fix or baseline
  stale       passing, baselined            (blue)    now passes, remove from baseline

Usage:
  gen_report.py --out-dir <dir> --baseline-dir <dir> --output <status.html>
                [--history <status-history.json>] [--date YYYY-MM-DD]
                [--gateway-sha <sha>] [--gateway-ref <branch|tag>] [--framework <ver>]
"""

import argparse
import datetime
import glob
import html
import json
import math
import os
import re

# Our labels. `modern-draft` maps to the framework's `--suite draft` in
# mcp_conformance.rs; here it names the baseline file, `-o` subdir, and history key.
SUITES = ["active", "modern-draft"]

# Distinct, status-neutral hues for the timeline lines (not the green/amber/red
# used for buckets, so a line is never confused with a status).
LINE_COLORS = {"active": "#1f6feb", "modern-draft": "#a371f7"}

# `createResultDir` names dirs `server-<scenario>-<iso-ts>` with `:`/`.` changed to `-`.
TS_SUFFIX = re.compile(r"-\d{4}-\d{2}-\d{2}T\d{2}-\d{2}-\d{2}-\d{3}Z$")
BASELINE_ENTRY = re.compile(r"^\s*-\s*([A-Za-z0-9._/-]+)")

BUCKET_LABEL = {
    "pass": "PASS",
    "gap": "TRACKED GAP",
    "regression": "REGRESSION",
    "stale": "STALE BASELINE",
}


def parse_baseline(path):
    """Scenario names under the `server:` key, comments and values stripped."""
    if not os.path.isfile(path):
        return set()
    names, in_server = set(), False
    with open(path) as f:
        for line in f:
            stripped = line.strip()
            if stripped.startswith("#") or not stripped:
                continue
            if re.match(r"^server\s*:", stripped):
                in_server = True
                continue
            if re.match(r"^[A-Za-z0-9_]+\s*:", stripped):  # a new top-level key
                in_server = False
            if in_server:
                m = BASELINE_ENTRY.match(line)
                if m:
                    names.add(m.group(1))
    return names


def scenario_name(result_dir):
    base = os.path.basename(result_dir)
    base = re.sub(r"^server-", "", base, count=1)  # strip the mode prefix
    return TS_SUFFIX.sub("", base)


def load_suite(out_dir, suite):
    """Map scenario -> latest checks list for one suite's `-o` output."""
    suite_dir = os.path.join(out_dir, suite)
    # Re-runs accumulate timestamped dirs per scenario; the lexical max is newest
    # because the shared prefix is identical and the suffix is an ISO timestamp.
    newest = {}
    for cj in glob.glob(os.path.join(suite_dir, "*", "checks.json")):
        result_dir = os.path.dirname(cj)
        name = scenario_name(result_dir)
        if name not in newest or result_dir > newest[name]:
            newest[name] = result_dir
    by_scenario = {}
    for name, result_dir in newest.items():
        try:
            with open(os.path.join(result_dir, "checks.json")) as f:
                by_scenario[name] = json.load(f)
        except (OSError, json.JSONDecodeError):
            continue
    return by_scenario


def is_failing(checks):
    if not checks:
        return True  # produced no checks, so the run is incomplete and counts as failing
    return any(c.get("status") in ("FAILURE", "WARNING") for c in checks)


def status_counts(checks):
    counts = {"SUCCESS": 0, "FAILURE": 0, "WARNING": 0, "SKIPPED": 0}
    for c in checks:
        counts[c.get("status", "SKIPPED")] = counts.get(c.get("status"), 0) + 1
    return counts


def sep_group(checks):
    """Most-referenced SEP id across a scenario's checks (its primary spec)."""
    seps = {}
    for c in checks:
        for ref in c.get("specReferences", []) or []:
            sid = ref.get("id")
            if sid:
                seps[sid] = seps.get(sid, 0) + 1
    if not seps:
        return "General"
    return max(seps, key=seps.get)


def classify(failing, baselined):
    if failing:
        return "gap" if baselined else "regression"
    return "stale" if baselined else "pass"


# ---------------------------------------------------------------------------
# History in committed status-history.json, one dated snapshot per run day.
# ---------------------------------------------------------------------------


def load_history(path):
    try:
        with open(path) as f:
            data = json.load(f)
        return data if isinstance(data, list) else []
    except (OSError, json.JSONDecodeError):
        return []


def save_history(path, history):
    with open(path, "w") as f:
        json.dump(history, f, indent=2)
        f.write("\n")


def upsert_snapshot(history, snapshot):
    """Replace any same-day entry, keep the list sorted by date."""
    history = [h for h in history if h.get("date") != snapshot["date"]]
    history.append(snapshot)
    history.sort(key=lambda h: h.get("date", ""))
    return history


# ---------------------------------------------------------------------------
# Rendering
# ---------------------------------------------------------------------------


def render_timeline(history):
    """Inline SVG: % of scenarios passing over time, one line per suite."""
    if not history:
        return ""
    w, h = 760, 300
    ml, mr, mt, mb = 46, 140, 16, 42
    pw, ph = w - ml - mr, h - mt - mb
    n = len(history)

    def px(i):
        return ml + (pw / 2 if n == 1 else pw * i / (n - 1))

    def py(pct):
        return mt + ph * (1 - pct / 100)

    grid = ""
    for g in (0, 25, 50, 75, 100):
        yy = py(g)
        grid += f'<line x1="{ml}" y1="{yy:.1f}" x2="{ml + pw}" y2="{yy:.1f}" class="grid"/>'
        grid += f'<text x="{ml - 8}" y="{yy + 4:.1f}" class="ylab">{g}%</text>'

    step = max(1, math.ceil(n / 12))
    xlabs = ""
    for i, snap in enumerate(history):
        if i % step == 0 or i == n - 1:
            xlabs += f'<text x="{px(i):.1f}" y="{h - mb + 22:.1f}" class="xlab">{snap["date"][5:]}</text>'

    series, legend, ly = "", "", mt + 6
    for suite in SUITES:
        pts = []
        for i, snap in enumerate(history):
            s = snap.get("suites", {}).get(suite)
            if not s or not s.get("total"):
                continue
            pts.append((i, 100 * s["passing"] / s["total"], s))
        if not pts:
            continue
        color = LINE_COLORS.get(suite, "#888")
        if len(pts) >= 2:
            poly = " ".join(f"{px(i):.1f},{py(p):.1f}" for i, p, _ in pts)
            series += f'<polyline points="{poly}" fill="none" stroke="{color}" stroke-width="2.5"/>'
        for i, p, _ in pts:
            series += f'<circle cx="{px(i):.1f}" cy="{py(p):.1f}" r="3.5" fill="{color}"/>'
        li, lp, _ = pts[-1]
        series += (
            f'<circle cx="{px(li):.1f}" cy="{py(lp):.1f}" r="5.5" fill="{color}" '
            f'stroke="var(--bg)" stroke-width="2"/>'
        )
        cur = pts[-1][2]
        series += (
            f'<text x="{px(li) - 8:.1f}" y="{py(lp) - 10:.1f}" class="ptlab" '
            f'fill="{color}">{cur["passing"]}/{cur["total"]}</text>'
        )
        legend += (
            f'<g transform="translate({ml + pw + 18},{ly})">'
            f'<rect width="12" height="12" rx="3" fill="{color}"/>'
            f'<text x="18" y="10" class="leg">{suite} · {cur["passing"]}/{cur["total"]}</text></g>'
        )
        ly += 22
    return (
        f'<svg viewBox="0 0 {w} {h}" class="timeline" role="img" '
        f'aria-label="conformance progression over time">{grid}{series}{xlabs}{legend}</svg>'
    )


def render_bar(tally, total):
    if not total:
        return ""
    segs = [
        ("pass", tally["pass"]),
        ("stale", tally["stale"]),
        ("gap", tally["gap"]),
        ("regression", tally["regression"]),
    ]
    spans = "".join(
        f'<span class="seg {cls}" style="width:{100 * v / total:.3f}%"></span>'
        for cls, v in segs
        if v
    )
    return f'<div class="bar">{spans}</div>'


def render_compare(counts_by_suite):
    """Side-by-side suite cards for an at-a-glance active-vs-modern-draft comparison,
    each linking down to its detail section."""
    cards = ""
    for suite in SUITES:
        c = counts_by_suite.get(suite)
        if not c:
            continue
        pct = round(100 * c["passing"] / c["total"]) if c["total"] else 0
        tally = {
            "pass": c["passing"] - c["stale"],
            "stale": c["stale"],
            "gap": c["gaps"],
            "regression": c["regressions"],
        }
        drift = c["regressions"] + c["stale"]
        vcls = "ok" if drift == 0 else "warn"
        vtxt = "on baseline" if drift == 0 else f"{drift} drift"
        foot = f'{c["gaps"]} gaps · {c["regressions"]} regressions'
        if c["stale"]:
            foot += f' · {c["stale"]} stale'
        cards += (
            f'<a class="ccard" href="#suite-{suite}" style="--line:{LINE_COLORS.get(suite, "#888")}">'
            f'<div class="cc-head"><span class="cc-name">{suite}</span>'
            f'<span class="verdict {vcls}">{vtxt}</span></div>'
            f'<div class="cc-num"><b>{c["passing"]}</b><span>/ {c["total"]}</span>'
            f'<span class="cc-pct">{pct}%</span></div>'
            f"{render_bar(tally, c['total'])}"
            f'<div class="cc-foot">{foot}<span class="cc-go">details ↓</span></div></a>'
        )
    return f'<div class="compare">{cards}</div>' if cards else ""


def render_scenario_row(name, checks, bucket):
    c = status_counts(checks)
    chips = "".join(
        f'<span class="c {sty}">{c[k]}{sym}</span>'
        for k, sty, sym in (
            ("SUCCESS", "ok", " ok"),
            ("FAILURE", "fail", " fail"),
            ("WARNING", "warn", " warn"),
            ("SKIPPED", "skip", " skip"),
        )
        if c[k]
    ) or '<span class="c skip">none</span>'

    nonsuccess = [x for x in checks if x.get("status") != "SUCCESS"]
    if nonsuccess:
        detail_rows = "".join(
            f'<div class="chk {html.escape((x.get("status") or "").lower())}">'
            f'<b>{html.escape(x.get("status", ""))}</b> {html.escape(x.get("name", ""))}'
            + (f'. {html.escape(x.get("errorMessage", ""))}' if x.get("errorMessage") else "")
            + "</div>"
            for x in nonsuccess
        )
        detail = (
            f"<details><summary>{len(nonsuccess)} non-passing check(s)</summary>"
            f"{detail_rows}</details>"
        )
    else:
        detail = '<span class="muted">all checks pass</span>'

    return (
        f'<tr class="b-{bucket}">'
        f'<td class="name">{html.escape(name)}</td>'
        f'<td><span class="badge {bucket}">{BUCKET_LABEL[bucket]}</span></td>'
        f'<td class="chips">{chips}</td>'
        f"<td>{detail}</td></tr>"
    )


def render_suite(suite, scenario_checks, baseline):
    groups, tally = {}, {"pass": 0, "gap": 0, "regression": 0, "stale": 0}
    names = {"pass": [], "gap": [], "regression": [], "stale": []}
    failing_checks = {}  # failing scenario -> failed check names
    for name in sorted(scenario_checks):
        checks = scenario_checks[name]
        bucket = classify(is_failing(checks), name in baseline)
        tally[bucket] += 1
        names[bucket].append(name)
        groups.setdefault(sep_group(checks), []).append((name, checks, bucket))
        if bucket in ("gap", "regression"):
            sub = [c.get("name", "") for c in checks if c.get("status") in ("FAILURE", "WARNING")]
            # A scenario with no checks still counts as failing (is_failing); record why
            # so the entry is never an empty, ambiguous list.
            failing_checks[name] = sub or ["(no checks produced)"]
    not_run = sorted(baseline - set(scenario_checks))

    total = sum(tally.values())
    passing = tally["pass"] + tally["stale"]
    drift = tally["regression"] + tally["stale"]
    verdict_cls = "ok" if drift == 0 else "warn"
    verdict = "matches baseline" if drift == 0 else f"{drift} drift from baseline"

    body = ""
    for group in sorted(groups):
        body += f'<tr class="grp"><td colspan="4">{html.escape(group)}</td></tr>'
        for name, checks, bucket in groups[group]:
            body += render_scenario_row(name, checks, bucket)
    table = (
        '<table class="scen"><colgroup>'
        '<col class="c-name"><col class="c-status"><col class="c-checks"><col class="c-detail">'
        "</colgroup><thead><tr>"
        "<th>scenario</th><th>status</th><th>checks</th><th>detail</th>"
        f"</tr></thead><tbody>{body}</tbody></table>"
    )

    notrun_html = ""
    if not_run:
        items = "".join(f"<li>{html.escape(n)}</li>" for n in not_run)
        notrun_html = (
            '<div class="notrun"><h4>Baselined but not run this pass</h4>'
            f"<ul>{items}</ul></div>"
        )

    stats = f'<b>{passing}/{total}</b> passing · {tally["gap"]} gaps · {tally["regression"]} regressions'
    if tally["stale"]:
        stats += f' · {tally["stale"]} stale'

    section = (
        f'<section class="suite" id="suite-{suite}"><div class="suite-head">'
        f'<h2>{suite} suite</h2><span class="verdict {verdict_cls}">{verdict}</span></div>'
        f'<div class="summary">{render_bar(tally, total)}<div class="stats">{stats}</div></div>'
        f"{table}{notrun_html}</section>"
    )
    counts = {
        "passing": passing,
        "total": total,
        "gaps": tally["gap"],
        "regressions": tally["regression"],
        "stale": tally["stale"],
        # Record what failed this run (gap + regression): scenario -> failed check names.
        # This distinguishes partial failures from fully failed scenarios.
        "failing": dict(sorted(failing_checks.items())),
    }
    return section, counts


CSS = """
:root{
  --bg:#fff;--fg:#1f2328;--muted:#656d76;--border:#d0d7de;--card:#f6f8fa;--grid:#d8dee4;
  --ok:#1a7f37;--gap:#9a6700;--reg:#cf222e;--stale:#0969da;
  --ok-bg:#1a7f3714;--gap-bg:#9a670014;--reg-bg:#cf222e14;--stale-bg:#0969da14;
}
@media (prefers-color-scheme:dark){:root{
  --bg:#0d1117;--fg:#e6edf3;--muted:#8b949e;--border:#30363d;--card:#161b22;--grid:#21262d;
  --ok:#3fb950;--gap:#d29922;--reg:#f85149;--stale:#58a6ff;
  --ok-bg:#3fb95022;--gap-bg:#d2992222;--reg-bg:#f8514922;--stale-bg:#58a6ff22;
}}
*{box-sizing:border-box}
body{font:14px/1.55 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,Helvetica,Arial,sans-serif;
  color:var(--fg);background:var(--bg);margin:0;padding:2.5rem 1.5rem}
.wrap{max-width:1040px;margin:0 auto}
h1{font-size:1.55rem;margin:0 0 .25rem;letter-spacing:-.01em}
.sub{color:var(--muted);margin:0 0 2rem;font-size:.9rem;max-width:70ch}
.compare{display:grid;grid-template-columns:1fr 1fr;gap:1rem;margin-bottom:1.25rem}
@media(max-width:620px){.compare{grid-template-columns:1fr}}
.ccard{display:block;text-decoration:none;color:inherit;background:var(--card);
  border:1px solid var(--border);border-left:4px solid var(--line);border-radius:12px;
  padding:1rem 1.15rem .9rem;transition:border-color .15s}
.ccard:hover{border-color:var(--line)}
.cc-head{display:flex;align-items:center;justify-content:space-between;margin-bottom:.45rem}
.cc-name{font-size:.95rem;font-weight:600;text-transform:capitalize;letter-spacing:-.01em}
.cc-num{display:flex;align-items:baseline;gap:.35rem;margin-bottom:.7rem}
.cc-num b{font-size:1.9rem;font-weight:700;line-height:1;color:var(--fg)}
.cc-num>span{color:var(--muted);font-size:.95rem}
.cc-pct{margin-left:auto;font-size:1.15rem;font-weight:700;color:var(--line)}
.ccard .bar{margin-bottom:.55rem}
.cc-foot{display:flex;align-items:center;font-size:.8rem;color:var(--muted)}
.cc-go{margin-left:auto;color:var(--line);font-weight:600}
.overview{background:var(--card);border:1px solid var(--border);border-radius:12px;padding:1.25rem 1.5rem 1rem;margin-bottom:2.25rem}
.overview h2{font-size:.72rem;text-transform:uppercase;letter-spacing:.06em;color:var(--muted);margin:0 0 .5rem;font-weight:600}
svg.timeline{width:100%;height:auto;display:block}
.timeline .grid{stroke:var(--grid);stroke-width:1}
.timeline .ylab{fill:var(--muted);font-size:11px;text-anchor:end}
.timeline .xlab{fill:var(--muted);font-size:11px;text-anchor:middle}
.timeline .ptlab{font-size:11px;font-weight:600;text-anchor:end}
.timeline .leg{fill:var(--fg);font-size:12px}
.cap{color:var(--muted);font-size:.78rem;margin:.4rem 0 0}
section.suite{margin:2.25rem 0}
.suite-head{display:flex;align-items:center;gap:.7rem;margin-bottom:.7rem}
.suite-head h2{font-size:1.15rem;margin:0;text-transform:capitalize;letter-spacing:-.01em}
.verdict{font-size:.68rem;font-weight:600;padding:.22em .7em;border-radius:1em}
.verdict.ok{background:var(--ok-bg);color:var(--ok)}
.verdict.warn{background:var(--reg-bg);color:var(--reg)}
.summary{display:flex;align-items:center;gap:1rem;margin-bottom:1.1rem}
.bar{flex:1;display:flex;height:10px;border-radius:6px;overflow:hidden;background:var(--card);border:1px solid var(--border)}
.bar .seg{height:100%}
.bar .seg.pass{background:var(--ok)}.bar .seg.stale{background:var(--stale)}
.bar .seg.gap{background:var(--gap)}.bar .seg.regression{background:var(--reg)}
.stats{font-size:.85rem;color:var(--muted);white-space:nowrap}
.stats b{color:var(--fg);font-size:1.05rem;font-weight:700}
table.scen{width:100%;border-collapse:collapse;table-layout:fixed}
.scen col.c-name{width:34%}.scen col.c-status{width:15%}.scen col.c-checks{width:15%}.scen col.c-detail{width:36%}
.scen th{text-align:left;font-size:.68rem;text-transform:uppercase;letter-spacing:.05em;color:var(--muted);
  font-weight:600;padding:.4rem .55rem;border-bottom:1px solid var(--border)}
.scen td{padding:.5rem .55rem;border-bottom:1px solid var(--border);vertical-align:top;font-size:.85rem}
.scen tr.grp td{background:var(--card);font-weight:600;font-size:.7rem;text-transform:uppercase;
  letter-spacing:.05em;color:var(--muted);padding:.45rem .55rem}
.scen tr:hover td:not(.grp){background:var(--card)}
.scen tr.grp:hover td{background:var(--card)}
td.name{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:.82rem;word-break:break-all;color:var(--fg)}
.badge{display:inline-block;font-size:.64rem;font-weight:600;padding:.22em .6em;border-radius:1em;white-space:nowrap}
.badge.pass{background:var(--ok-bg);color:var(--ok)}.badge.gap{background:var(--gap-bg);color:var(--gap)}
.badge.regression{background:var(--reg-bg);color:var(--reg)}.badge.stale{background:var(--stale-bg);color:var(--stale)}
.chips{white-space:nowrap}.chips .c{font-family:ui-monospace,monospace;font-size:.78rem;margin-right:.4rem}
.c.ok{color:var(--ok)}.c.fail{color:var(--reg)}.c.warn{color:var(--gap)}.c.skip{color:var(--muted)}
details summary{cursor:pointer;color:var(--stale);font-size:.82rem;user-select:none}
details .chk{font-size:.8rem;margin:.35rem 0 .35rem .15rem;padding-left:.6rem;border-left:3px solid var(--border);word-break:break-word}
.chk.failure{border-color:var(--reg)}.chk.warning{border-color:var(--gap)}
.chk b{font-size:.68rem;letter-spacing:.03em}
.muted{color:var(--muted);font-size:.82rem}
.notrun{margin-top:1rem}.notrun h4{font-size:.78rem;color:var(--muted);margin:0 0 .3rem;font-weight:600}
.notrun ul{margin:0;padding-left:1.2rem;color:var(--muted);font-family:ui-monospace,monospace;font-size:.82rem}
tr.b-regression td:not(.grp){background:var(--reg-bg)}
tr.b-stale td:not(.grp){background:var(--stale-bg)}
"""


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out-dir", required=True, help="dir with <suite>/*/checks.json")
    ap.add_argument("--baseline-dir", required=True, help="dir with baseline-<suite>.yml")
    ap.add_argument("--output", required=True, help="status.html to write")
    ap.add_argument("--history", help="status-history.json (default: alongside --output)")
    ap.add_argument(
        "--date",
        help="snapshot date YYYY-MM-DD (default: today). Use to regenerate a run under a "
        "specific release date, e.g. a main-branch before-state.",
    )
    # What the run tested, recorded per snapshot so a date says exactly what it graded.
    ap.add_argument("--gateway-sha", help="short sha of the gateway commit under test")
    ap.add_argument("--gateway-ref", help="branch or tag of the gateway under test")
    ap.add_argument(
        "--framework",
        help="conformance framework version tested with, e.g. '0.2.0-alpha.9 (1ca3bc3)'",
    )
    args = ap.parse_args()

    now = datetime.datetime.now().astimezone()
    generated = now.strftime("%Y-%m-%d %H:%M %Z")
    snapshot_date = args.date or now.strftime("%Y-%m-%d")
    history_path = args.history or os.path.join(
        os.path.dirname(os.path.abspath(args.output)), "status-history.json"
    )

    # Order keys date -> gateway -> framework -> suites so each snapshot reads as
    # "on this date, this gateway, tested with this framework, scored this".
    snapshot = {"date": snapshot_date}
    if args.gateway_sha or args.gateway_ref:
        gw = {}
        if args.gateway_sha:
            gw["sha"] = args.gateway_sha
        if args.gateway_ref:
            gw["ref"] = args.gateway_ref
        snapshot["gateway"] = gw
    if args.framework:
        snapshot["framework"] = args.framework
    snapshot["suites"] = {}
    sections = ""
    for suite in SUITES:
        checks = load_suite(args.out_dir, suite)
        if not checks:
            sections += (
                f'<section class="suite"><div class="suite-head"><h2>{suite} suite</h2></div>'
                f'<p class="muted">No results under '
                f"{html.escape(os.path.join(args.out_dir, suite))}. Run "
                "<code>make mcp-conformance-report</code>.</p></section>"
            )
            continue
        baseline = parse_baseline(os.path.join(args.baseline_dir, f"baseline-{suite}.yml"))
        section, counts = render_suite(suite, checks, baseline)
        snapshot["suites"][suite] = counts
        sections += section

    history = load_history(history_path)
    if snapshot["suites"]:  # don't pollute history with empty runs
        history = upsert_snapshot(history, snapshot)
        save_history(history_path, history)

    def gw_str(snap):
        g = snap.get("gateway") or {}
        sha, ref = g.get("sha"), g.get("ref")
        if sha and ref:
            return f"{sha} ({ref})"
        return sha or ref or ""

    compare = render_compare(snapshot["suites"])
    timeline = render_timeline(history)
    # Per-run "what was tested" line under the timeline: date -> gateway sha (ref).
    runs = [
        f'{html.escape(snap["date"][5:])} {html.escape(gw_str(snap))}'
        for snap in history
        if gw_str(snap)
    ]
    runs_cap = f' Gateway per run: {"; ".join(runs)}.' if runs else ""
    overview = (
        '<div class="overview"><h2>Progression by passing scenarios</h2>'
        f'{timeline}'
        '<p class="cap">Each run records a dated snapshot in status-history.json; '
        "the lines fill in as tracked gaps close. Point labels show passing / total."
        f"{runs_cap}</p></div>"
        if timeline
        else ""
    )
    # Header line names the gateway + framework this page's latest run tested.
    tested = []
    if gw_str(snapshot):
        tested.append(f"gateway {gw_str(snapshot)}")
    if snapshot.get("framework"):
        tested.append(f"conformance {snapshot['framework']}")
    tested_html = f" · {html.escape(' · '.join(tested))}" if tested else ""

    doc = (
        "<!doctype html><html lang=en><head><meta charset=utf-8>"
        "<meta name=viewport content='width=device-width,initial-scale=1'>"
        "<title>agentgateway MCP conformance</title>"
        f"<style>{CSS}</style></head><body><div class=wrap>"
        "<h1>agentgateway MCP conformance</h1>"
        f'<p class="sub">Generated {html.escape(generated)}{tested_html} · server-mode vs '
        "the everything-server reference upstream. Tracked gaps are the work ahead; "
        "regressions and stale entries are drift from the committed baselines.</p>"
        f"{compare}{overview}{sections}</div></body></html>"
    )

    os.makedirs(os.path.dirname(os.path.abspath(args.output)), exist_ok=True)
    with open(args.output, "w") as f:
        f.write(doc)
    print(f"wrote {args.output} and {history_path}")


if __name__ == "__main__":
    main()
