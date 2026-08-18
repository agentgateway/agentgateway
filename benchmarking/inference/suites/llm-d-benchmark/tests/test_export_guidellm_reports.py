from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

import yaml


SCRIPT = (
    Path(__file__).resolve().parents[1]
    / "scripts"
    / "export-guidellm-reports.py"
)
SPEC = importlib.util.spec_from_file_location("export_guidellm_reports", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class FakeReport:
    def __init__(self) -> None:
        self.scenario = SimpleNamespace(stack=[])

    def export_yaml(self, path: str) -> None:
        Path(path).write_text(
            yaml.safe_dump({"scenario": {"stack": self.scenario.stack}}),
            encoding="utf-8",
        )


class ExportGuideLLMReportsTest(unittest.TestCase):
    def test_exports_every_point_with_template_stack(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            results = root / "results.json"
            results.write_text("{}", encoding="utf-8")
            template = root / "template.yaml"
            stack = [{"standardized": {"kind": "inference_engine"}}]
            template.write_text(
                yaml.safe_dump({"scenario": {"stack": stack}}),
                encoding="utf-8",
            )
            output = root / "reports"
            argv = [
                str(SCRIPT),
                "--results",
                str(results),
                "--template",
                str(template),
                "--output-dir",
                str(output),
            ]
            with (
                mock.patch.object(
                    MODULE,
                    "import_guidellm_all",
                    return_value=[FakeReport(), FakeReport()],
                ),
                mock.patch.object(sys, "argv", argv),
            ):
                MODULE.main()

            reports = sorted(output.glob("benchmark_report_v0.2,*.yaml"))
            self.assertEqual(len(reports), 2)
            for report in reports:
                document = yaml.safe_load(report.read_text(encoding="utf-8"))
                self.assertEqual(document["scenario"]["stack"], stack)


if __name__ == "__main__":
    unittest.main()
