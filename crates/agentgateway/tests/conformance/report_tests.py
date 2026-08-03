import importlib.util
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location("report", Path(__file__).with_name("report.py"))
report = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(report)


def check(check_id, status, name=None):
    return {"id": check_id, "name": name or check_id, "status": status}


class ClassificationTests(unittest.TestCase):
    def classify(self, gateway, baseline=(), direct=()):
        return report.classify("scenario", list(direct), list(gateway), list(baseline))

    def test_whole_scenario_failure_is_gap(self):
        result = self.classify([check("a", "FAILURE")], [{"scenario": "scenario"}])
        self.assertEqual(result["category"], "gap")

    def test_per_check_failure_is_gap(self):
        result = self.classify([check("a", "FAILURE")], [{"scenario": "scenario", "checkId": "a"}])
        self.assertEqual(result["category"], "gap")

    def test_unbaselined_failure_is_regression(self):
        result = self.classify([check("a", "FAILURE")])
        self.assertEqual(result["category"], "regression")

    def test_mixed_baselined_and_unbaselined_failure_is_regression(self):
        result = self.classify(
            [check("a", "FAILURE"), check("b", "FAILURE")],
            [{"scenario": "scenario", "checkId": "a"}],
        )
        self.assertEqual(result["category"], "regression")

    def test_passed_baseline_is_stale_even_with_another_gap(self):
        result = self.classify(
            [check("a", "FAILURE"), check("b", "SUCCESS")],
            [{"scenario": "scenario", "checkId": "a"}, {"scenario": "scenario", "checkId": "b"}],
        )
        self.assertEqual(result["category"], "stale")

    def test_absent_and_skipped_baselines_are_no_signal(self):
        for checks in ([], [check("a", "SKIPPED")]):
            result = self.classify(checks, [{"scenario": "scenario", "checkId": "a"}])
            self.assertEqual(result["category"], "pass")

    def test_duplicate_id_uses_most_severe_status(self):
        self.assertEqual(report.status_by_id([check("a", "SUCCESS"), check("a", "FAILURE")])["a"], "FAILURE")

    def test_check_counts_collapses_duplicate_ids(self):
        self.assertEqual(
            report.check_counts([check("a", "SUCCESS"), check("a", "FAILURE")]),
            {"total": 1, "failed": 1},
        )

    def test_id_not_display_name_controls_baseline(self):
        result = self.classify([check("id", "FAILURE", name="friendly")], [{"scenario": "scenario", "checkId": "id"}])
        self.assertEqual(result["category"], "gap")

    def test_expected_failure_rationale_is_attached_to_status(self):
        rationale = {
            "kind": "intentional-proxy-behavior",
            "summary": "The gateway rejects the request before opening upstream connections.",
        }
        result = report.classify(
            "scenario",
            [],
            [check("id", "FAILURE")],
            [{"scenario": "scenario", "checkId": "id"}],
            {"scenario:id": rationale},
        )
        self.assertEqual(result["details"][0]["rationale"], rationale)

    def test_direct_failure_has_no_control_attribution(self):
        result = report.classify_direct(
            "scenario", [check("a", "FAILURE")], [{"scenario": "scenario", "checkId": "a"}]
        )
        detail = result["details"][0]
        self.assertEqual(detail["direct"], "FAILURE")
        self.assertNotIn("gateway", detail)
        self.assertNotIn("attribution", detail)

    def test_attribution_table(self):
        self.assertEqual(report.attribution("SUCCESS", "FAILURE"), "gateway-attributed")
        self.assertEqual(report.attribution("FAILURE", "FAILURE"), "control-blocked")
        self.assertEqual(report.attribution("FAILURE", "SUCCESS"), "gateway-changes-behavior")
        self.assertEqual(report.attribution(None, "FAILURE"), "investigate")
        self.assertEqual(report.attribution("SKIPPED", "FAILURE"), "investigate")

    def test_rendered_summary_includes_the_suite_total(self):
        status = {
            "gateway": {},
            "suites": {
                "2026-07-28": {
                    "summary": {
                        "total": 19,
                        "direct": {"pass": 19},
                        "gateway": {"pass": 18, "gap": 1},
                    },
                    "scenarios": [],
                }
            },
        }
        rendered = report.render_markdown(status)
        self.assertIn("Direct: 19/19 pass.", rendered)
        self.assertIn("Gateway: 18/19 pass, 1/19 gap.", rendered)

    def test_rendered_scenario_includes_check_counts(self):
        status = {
            "gateway": {},
            "suites": {
                "2026-07-28": {
                    "summary": {
                        "total": 1,
                        "direct": {"pass": 1},
                        "gateway": {"gap": 1},
                    },
                    "scenarios": [
                        {
                            "scenario": "server-stateless",
                            "direct": {
                                "category": "pass",
                                "checkCounts": {"total": 30, "failed": 0},
                                "details": [],
                            },
                            "gateway": {
                                "category": "gap",
                                "checkCounts": {"total": 30, "failed": 1},
                                "details": [],
                            },
                        }
                    ],
                }
            },
        }
        rendered = report.render_markdown(status)
        self.assertIn(
            "| `server-stateless` | pass (30 checks) | gap (1/30 checks) | — | — |", rendered
        )

    def test_rendered_rationale_explains_intentional_behavior(self):
        status = {
            "gateway": {},
            "suites": {
                "2026-07-28": {
                    "summary": {
                        "total": 1,
                        "direct": {"pass": 1},
                        "gateway": {"gap": 1},
                    },
                    "scenarios": [
                        {
                            "scenario": "server-stateless",
                            "direct": {
                                "category": "pass",
                                "checkCounts": {"total": 30, "failed": 0},
                                "details": [],
                            },
                            "gateway": {
                                "category": "gap",
                                "checkCounts": {"total": 30, "failed": 1},
                                "details": [
                                    {
                                        "entry": "server-stateless:unsupported-version",
                                        "category": "gap",
                                        "attribution": "gateway-attributed",
                                        "rationale": {
                                            "kind": "intentional-proxy-behavior",
                                            "summary": "The gateway rejects before contacting upstreams.",
                                        },
                                    }
                                ],
                            },
                        }
                    ],
                }
            },
        }
        rendered = report.render_markdown(status)
        self.assertIn(
            "| `server-stateless` | pass (30 checks) | gap (1/30 checks) | "
            "server-stateless:unsupported-version (gap; gateway-attributed) | "
            "intentional proxy behavior: The gateway rejects before contacting upstreams. |",
            rendered,
        )

    def test_rendered_2025_suite_explains_upstream_active_name(self):
        status = {
            "gateway": {},
            "suites": {
                "2025-11-25": {
                    "summary": {
                        "total": 31,
                        "direct": {"pass": 31},
                        "gateway": {"pass": 31},
                    },
                    "scenarios": [],
                }
            },
        }
        self.assertIn("## 2025-11-25 (upstream active)", report.render_markdown(status))

    def test_reviewed_pending_suite_uses_only_the_selected_scenario(self):
        self.assertEqual(
            report.scenarios_for_suite({}, "pending-json-schema-2020-12"),
            ["json-schema-2020-12"],
        )

    def test_graded_suites_derive_from_gated_pending_scenarios(self):
        inventory = {"gatedPendingScenarios": ["json-schema-2020-12", "tasks-lifecycle"]}
        self.assertEqual(
            report.graded_suites(inventory),
            (
                "2025-11-25",
                "2026-07-28",
                "pending-json-schema-2020-12",
                "pending-tasks-lifecycle",
            ),
        )

    def test_baselines_pair_adapter_entries_with_their_files(self):
        files = report.expected_failures_files(Path("dir"), ("2025-11-25", "2026-07-28"))
        parsed = [
            {"path": str(path), "expectedFailures": {"server": [{"scenario": f"{topology}-{suite}"}]}}
            for topology, suite, path in files
        ]
        baselines = report.baselines_by_topology(files, parsed)
        self.assertEqual(
            baselines["gateway"]["2026-07-28"],
            [{"scenario": "gateway-2026-07-28"}],
        )

    def test_baselines_reject_misordered_adapter_output(self):
        files = report.expected_failures_files(Path("dir"), ("2025-11-25",))
        parsed = [
            {"path": str(path), "expectedFailures": {"server": []}} for _, _, path in reversed(files)
        ]
        with self.assertRaisesRegex(ValueError, "does not match"):
            report.baselines_by_topology(files, parsed)

    def test_pending_json_schema_status_is_grouped_with_2025_coverage(self):
        self.assertEqual(report.status_suite("pending-json-schema-2020-12"), "2025-11-25")


if __name__ == "__main__":
    unittest.main()
