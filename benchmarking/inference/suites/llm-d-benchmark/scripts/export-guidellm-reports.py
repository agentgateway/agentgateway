#!/usr/bin/env python3
"""Export every GuideLLM benchmark point as Benchmark Report v0.2."""

from __future__ import annotations

import argparse
import copy
from pathlib import Path

import yaml


def import_guidellm_all(path: str):
    """Load llm-d-benchmark only when the GuideLLM exporter is executed."""
    from llmdbenchmark.analysis.benchmark_report.native_to_br0_2 import (
        import_guidellm_all as upstream_import_guidellm_all,
    )

    return upstream_import_guidellm_all(path)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--results", required=True, type=Path)
    parser.add_argument("--template", required=True, type=Path)
    parser.add_argument("--output-dir", required=True, type=Path)
    args = parser.parse_args()

    reports = import_guidellm_all(str(args.results))
    if not reports:
        raise ValueError(f"GuideLLM produced no benchmark points: {args.results}")
    args.output_dir.mkdir(parents=True, exist_ok=True)
    with args.template.open(encoding="utf-8") as stream:
        template = yaml.safe_load(stream) or {}
    for index, report in enumerate(reports):
        # llm-d-benchmark's in-process conversion has the deployment metadata
        # environment available, while this all-points expansion runs after the
        # command returns. Carry the invariant stack metadata from its point-0
        # report; load and result fields remain specific to each GuideLLM run.
        report.scenario.stack = copy.deepcopy(template["scenario"]["stack"])
        report.export_yaml(
            str(
                args.output_dir
                / f"benchmark_report_v0.2,_guidellm_run_{index}.yaml"
            )
        )


if __name__ == "__main__":
    main()
