#!/usr/bin/env python3
"""Classify direct and gateway MCP conformance results per check."""

import argparse
import datetime as dt
import json
import subprocess
from collections import Counter, defaultdict
from pathlib import Path

FAILED = {"FAILURE", "WARNING"}
PRECEDENCE = {"pass": 0, "gap": 1, "stale": 2, "regression": 3}
RELEASED_SUITES = ("2025-11-25", "2026-07-28")
SUITE_TITLES = {"2025-11-25": "2025-11-25 (upstream active)"}


def graded_suites(inventory):
    """Each gated pending scenario is graded as its own pending-<scenario> suite.

    mcp_conformance.rs asserts the same naming rule on its Suite constants.
    """
    return (
        *RELEASED_SUITES,
        *(f"pending-{scenario}" for scenario in inventory["gatedPendingScenarios"]),
    )


def status_suite(suite):
    # Gated pending scenarios are additional coverage for the deployed baseline.
    return "2025-11-25" if suite.startswith("pending-") else suite


def collapse(checks):
    """Mirror upstream collapseDuplicateChecks: most severe, ties last, INFO kept."""
    severity = {"FAILURE": 3, "WARNING": 2, "SUCCESS": 1}
    winners = {}
    for index, check in enumerate(checks):
        if check.get("status") == "INFO":
            continue
        check_id = check.get("id", "")
        current = winners.get(check_id)
        if current is None or severity.get(check.get("status"), 0) >= severity.get(
            checks[current].get("status"), 0
        ):
            winners[check_id] = index
    return [
        check
        for index, check in enumerate(checks)
        if check.get("status") == "INFO" or winners.get(check.get("id", "")) == index
    ]


def scenario_result_dir(path, scenarios):
    name = path.name
    matches = [scenario for scenario in scenarios if name.startswith(f"server-{scenario}-")]
    if len(matches) != 1:
        raise ValueError(f"unexpected result directory {path}")
    return matches[0]


def load_results(root, scenarios):
    root = Path(root)
    if not root.is_dir():
        raise ValueError(
            f"missing result directory {root}; the report consumes the output root "
            "printed by make mcp-conformance, which grades every suite — capture "
            "roots do not produce a gradable set"
        )
    results = {}
    for checks_path in root.glob("*/checks.json"):
        scenario = scenario_result_dir(checks_path.parent, scenarios)
        if scenario in results:
            raise ValueError(f"duplicate result for {scenario} in {root}")
        results[scenario] = json.loads(checks_path.read_text())
    missing = sorted(set(scenarios) - set(results))
    extra = sorted(set(results) - set(scenarios))
    if missing or extra:
        raise ValueError(
            f"incomplete result set in {root}: missing={missing}, extra={extra}; "
            "the report consumes the complete output of make mcp-conformance — "
            "captures can be partial (a thrown scenario writes no checks.json)"
        )
    return results


def parse_expected_failures(adapter, framework_dir, paths):
    command = [str(Path(framework_dir) / "node_modules/.bin/tsx"), str(adapter), str(framework_dir)]
    command.extend(str(path) for path in paths)
    return json.loads(subprocess.check_output(command, text=True))


def expected_failures_files(expected_failures_dir, suites):
    return [
        (topology, suite, expected_failures_dir / f"expected-failures-{topology}-{suite}.yml")
        for topology in ("direct", "gateway")
        for suite in suites
    ]


def baselines_by_topology(files, parsed):
    if len(parsed) != len(files):
        raise ValueError("adapter must return one entry per expected-failures file")
    baselines = {"direct": {}, "gateway": {}}
    for (topology, suite, path), entry in zip(files, parsed):
        if entry["path"] != str(path):
            raise ValueError(f"adapter entry {entry['path']} does not match {path}")
        baselines[topology][suite] = entry["expectedFailures"].get("server", [])
    return baselines


def entries_by_scenario(entries, scenarios):
    result = defaultdict(list)
    for entry in entries:
        scenario = entry["scenario"]
        if scenario not in scenarios:
            raise ValueError(f"baseline entry names out-of-suite scenario {scenario}")
        result[scenario].append(entry)
    return result


def format_entry(entry):
    return entry["scenario"] + (f":{entry['checkId']}" if "checkId" in entry else "")


def load_rationales(path, baselines):
    rationales = json.loads(Path(path).read_text())
    if not isinstance(rationales, dict):
        raise ValueError("expected failure rationales must be an object")
    for topology, suites in rationales.items():
        if topology not in baselines or not isinstance(suites, dict):
            raise ValueError(f"invalid expected failure rationale topology {topology}")
        for suite, entries in suites.items():
            if suite not in baselines[topology] or not isinstance(entries, dict):
                raise ValueError(f"invalid expected failure rationale suite {topology}/{suite}")
            known_entries = {format_entry(entry) for entry in baselines[topology][suite]}
            for entry, rationale in entries.items():
                if entry not in known_entries:
                    raise ValueError(f"rationale names non-baselined entry {topology}/{suite}/{entry}")
                if not isinstance(rationale, dict) or not all(
                    isinstance(rationale.get(field), str) and rationale[field]
                    for field in ("kind", "summary")
                ):
                    raise ValueError(f"rationale for {topology}/{suite}/{entry} needs kind and summary")
    return rationales


def status_by_id(checks):
    result = {}
    for check in collapse(checks):
        if check.get("status") != "INFO":
            result[check.get("id", "")] = check.get("status")
    return result


def check_counts(checks):
    checks = [check for check in collapse(checks) if check.get("status") != "INFO"]
    return {
        "total": len(checks),
        "failed": sum(check.get("status") in FAILED for check in checks),
    }


def attribution(direct, gateway):
    if gateway not in FAILED:
        return "gateway-changes-behavior" if direct in FAILED else "pass"
    if direct in (None, "SKIPPED", "INFO"):
        return "investigate"
    return "control-blocked" if direct in FAILED else "gateway-attributed"


def classify(scenario, direct_checks, gateway_checks, baseline_entries, rationales=None):
    """Return check details and a scenario badge with upstream baseline semantics."""
    direct = status_by_id(direct_checks)
    gateway = status_by_id(gateway_checks)
    whole = any("checkId" not in entry for entry in baseline_entries)
    expected_ids = {entry["checkId"] for entry in baseline_entries if "checkId" in entry}
    details = []

    if whole:
        category = "gap" if any(status in FAILED for status in gateway.values()) else "stale"
        details.append({"entry": scenario, "category": category})
    else:
        for check_id, status in gateway.items():
            if status in FAILED:
                category = "gap" if check_id in expected_ids else "regression"
                details.append(
                    {
                        "entry": f"{scenario}:{check_id}",
                        "checkId": check_id,
                        "category": category,
                        "gateway": status,
                        "direct": direct.get(check_id),
                        "attribution": attribution(direct.get(check_id), status),
                    }
                )
        for check_id in expected_ids:
            # The upstream grader only calls a non-INFO SUCCESS stale. Missing
            # and SKIPPED entries are intentionally no-signal.
            if gateway.get(check_id) == "SUCCESS":
                details.append(
                    {
                        "entry": f"{scenario}:{check_id}",
                        "checkId": check_id,
                        "category": "stale",
                        "gateway": "SUCCESS",
                        "direct": direct.get(check_id),
                        "attribution": attribution(direct.get(check_id), "SUCCESS"),
                    }
                )

        # A direct control failure that succeeds through the gateway is not a
        # regression, but it is material: the gateway changed observed behavior.
        for check_id, direct_status in direct.items():
            if direct_status in FAILED and gateway.get(check_id) == "SUCCESS":
                details.append(
                    {
                        "entry": f"{scenario}:{check_id}",
                        "checkId": check_id,
                        "category": "pass",
                        "gateway": "SUCCESS",
                        "direct": direct_status,
                        "attribution": "gateway-changes-behavior",
                    }
                )

    for detail in details:
        rationale = (rationales or {}).get(detail["entry"])
        if rationale:
            detail["rationale"] = rationale
    category = max((item["category"] for item in details), key=PRECEDENCE.get, default="pass")
    return {
        "scenario": scenario,
        "category": category,
        "checkCounts": check_counts(gateway_checks),
        "details": details,
    }


def classify_direct(scenario, checks, baseline_entries, rationales=None):
    result = classify(scenario, (), checks, baseline_entries, rationales)
    for detail in result["details"]:
        if "gateway" in detail:
            detail["direct"] = detail.pop("gateway")
        detail.pop("attribution", None)
    return result


def scenarios_for_suite(inventory, suite):
    if suite.startswith("pending-"):
        return [suite.removeprefix("pending-")]
    return inventory["suites"][suite]


def build_status(inventory, baselines, rationales, out_root):
    suites = {}
    for suite in graded_suites(inventory):
        scenarios = scenarios_for_suite(inventory, suite)
        direct = load_results(Path(out_root) / f"direct-{suite}", scenarios)
        gateway = load_results(Path(out_root) / f"gateway-{suite}", scenarios)
        direct_entries = entries_by_scenario(baselines["direct"][suite], scenarios)
        gateway_entries = entries_by_scenario(baselines["gateway"][suite], scenarios)
        scenarios_status = [
            {
                "scenario": scenario,
                "direct": classify_direct(
                    scenario,
                    direct[scenario],
                    direct_entries[scenario],
                    rationales.get("direct", {}).get(suite, {}),
                ),
                "gateway": classify(
                    scenario,
                    direct[scenario],
                    gateway[scenario],
                    gateway_entries[scenario],
                    rationales.get("gateway", {}).get(suite, {}),
                ),
            }
            for scenario in sorted(scenarios)
        ]
        suites.setdefault(status_suite(suite), {"scenarios": []})["scenarios"].extend(
            scenarios_status
        )
    for suite_status in suites.values():
        suite_status["scenarios"].sort(key=lambda scenario: scenario["scenario"])
        suite_status["summary"] = {
            "total": len(suite_status["scenarios"]),
            "direct": dict(
                Counter(item["direct"]["category"] for item in suite_status["scenarios"])
            ),
            "gateway": dict(
                Counter(item["gateway"]["category"] for item in suite_status["scenarios"])
            ),
        }
    return suites


def render_summary(summary):
    total = summary["total"]
    categories = ("pass", "gap", "stale", "regression")
    return ", ".join(
        f"{summary.get(category, 0)}/{total} {category}"
        for category in categories
        if summary.get(category, 0)
    )


def render_scenario_category(result):
    category = result["category"]
    counts = result["checkCounts"]
    if counts["failed"]:
        return f"{category} ({counts['failed']}/{counts['total']} checks)"
    return f"{category} ({counts['total']} checks)"


def render_detail(detail):
    parts = [detail["category"], detail.get("attribution", "baseline")]
    return f"{detail['entry']} ({'; '.join(parts)})"


def render_rationale(detail):
    rationale = detail.get("rationale")
    if not rationale:
        return None
    kind = rationale["kind"].replace("-", " ")
    reference = f" ({rationale['reference']})" if rationale.get("reference") else ""
    return f"{kind}{reference}: {rationale['summary']}"


def render_markdown(status):
    lines = ["# MCP Conformance Status", ""]
    for suite, suite_status in status["suites"].items():
        summary = suite_status["summary"]
        direct = render_summary({**summary["direct"], "total": summary["total"]})
        gateway = render_summary({**summary["gateway"], "total": summary["total"]})
        lines.extend([f"## {SUITE_TITLES.get(suite, suite)}", "", f"Direct: {direct}.<br>", f"Gateway: {gateway}.", "", "| Scenario | Direct | Gateway | Details | Rationale |", "| --- | --- | --- | --- | --- |"])
        for scenario in suite_status["scenarios"]:
            details = "; ".join(render_detail(detail) for detail in scenario["gateway"]["details"]) or "—"
            rationales = "; ".join(
                rendered
                for detail in scenario["gateway"]["details"]
                if (rendered := render_rationale(detail))
            ) or "—"
            lines.append(
                f"| `{scenario['scenario']}` | {render_scenario_category(scenario['direct'])} | "
                f"{render_scenario_category(scenario['gateway'])} | {details} | {rationales} |"
            )
        lines.append("")
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--inventory", required=True)
    parser.add_argument("--framework-dir", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--framework", required=True)
    parser.add_argument("--typescript-sdk", required=True)
    parser.add_argument("--gateway-sha", required=True)
    parser.add_argument("--gateway-ref", required=True)
    parser.add_argument(
        "--adapter", default=Path(__file__).with_name("parse-expected-failures.ts")
    )
    parser.add_argument("--expected-failures-dir", default=Path(__file__).parent)
    parser.add_argument(
        "--expected-failure-rationales",
        default=Path(__file__).with_name("expected-failure-rationales.json"),
    )
    args = parser.parse_args()

    inventory = json.loads(Path(args.inventory).read_text())
    if inventory["framework"] != args.framework:
        raise ValueError("report framework revision does not match suite inventory")
    files = expected_failures_files(Path(args.expected_failures_dir), graded_suites(inventory))
    parsed = parse_expected_failures(
        args.adapter, args.framework_dir, [path for _, _, path in files]
    )
    baselines = baselines_by_topology(files, parsed)
    rationales = load_rationales(args.expected_failure_rationales, baselines)
    suites = build_status(inventory, baselines, rationales, args.out)
    status = {
        "framework": args.framework,
        "typescriptSdk": args.typescript_sdk,
        "gateway": {
            "sha": args.gateway_sha,
            "ref": args.gateway_ref,
        },
        "generated": dt.datetime.now(tz=dt.timezone.utc).isoformat(),
        "suites": suites,
    }
    target = Path(__file__).parent
    (target / "status.json").write_text(json.dumps(status, indent=2) + "\n")
    (target / "status.md").write_text(render_markdown(status))
    history_path = target / "status-history.json"
    history = json.loads(history_path.read_text()) if history_path.exists() else []
    history.append(
        {
            "generated": status["generated"],
            "framework": args.framework,
            "typescriptSdk": args.typescript_sdk,
            "gateway": status["gateway"],
        }
    )
    history_path.write_text(json.dumps(history, indent=2) + "\n")


if __name__ == "__main__":
    main()
