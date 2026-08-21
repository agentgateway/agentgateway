"""Read llm-d-benchmark Benchmark Report v0.2 stage documents."""

from __future__ import annotations

import math
from pathlib import Path
from typing import Any

import yaml

from common.model import StageMetrics, TreatmentResults, aggregate_repetitions


REPORT_GLOB = "benchmark_report_v0.2,*_stage_*_lifecycle_metrics.json.yaml"


def nested(document: dict[str, Any], *keys: str) -> Any:
    value: Any = document
    for key in keys:
        if not isinstance(value, dict) or key not in value:
            raise ValueError(f"missing report field: {'.'.join(keys)}")
        value = value[key]
    return value


def number(document: dict[str, Any], *keys: str) -> float:
    value = nested(document, *keys)
    if isinstance(value, dict) and "mean" in value:
        value = value["mean"]
    if not isinstance(value, (int, float)):
        raise ValueError(f"report field {'.'.join(keys)} is not numeric")
    return float(value)


def read_stage(path: Path) -> StageMetrics:
    with path.open(encoding="utf-8") as stream:
        document = yaml.safe_load(stream) or {}
    aggregate = ("results", "request_performance", "aggregate")
    latency = (*aggregate, "latency")
    requests = (*aggregate, "requests")
    throughput = (*aggregate, "throughput")
    failures = number(document, *requests, "failures")
    total_requests = number(document, *requests, "total")
    all_failed = total_requests > 0 and failures == total_requests

    def measured(*keys: str, zero_when_all_failed: bool = False) -> float:
        try:
            return number(document, *keys)
        except ValueError:
            # Benchmark Report v0.2 omits throughput and latency distributions
            # when a stage has no successful requests. Preserve that distinction:
            # successful throughput is zero, while latency is unavailable.
            if all_failed:
                return 0.0 if zero_when_all_failed else math.nan
            raise

    return StageMetrics(
        stage=int(nested(document, "scenario", "load", "standardized", "stage")),
        # Preserve the configured rate for comparable treatment ladders until
        # inference-perf exposes requested-versus-achieved axis selection:
        # https://github.com/kubernetes-sigs/inference-perf/issues/741
        requested_qps=number(
            document, "scenario", "load", "standardized", "rate_qps"
        ),
        input_tokens_per_second=measured(
            *throughput, "input_token_rate", zero_when_all_failed=True
        ),
        output_tokens_per_second=measured(
            *throughput, "output_token_rate", zero_when_all_failed=True
        ),
        total_tokens_per_second=measured(
            *throughput, "total_token_rate", zero_when_all_failed=True
        ),
        requests_per_second=measured(
            *throughput, "request_rate", zero_when_all_failed=True
        ),
        ttft_mean_seconds=measured(*latency, "time_to_first_token", "mean"),
        ttft_p50_seconds=measured(
            *latency, "time_to_first_token", "p50"
        ),
        ttft_p90_seconds=measured(
            *latency, "time_to_first_token", "p90"
        ),
        itl_mean_seconds=measured(*latency, "inter_token_latency", "mean"),
        itl_p50_seconds=measured(
            *latency, "inter_token_latency", "p50"
        ),
        ntpot_mean_seconds=measured(
            *latency, "normalized_time_per_output_token", "mean"
        ),
        ntpot_p50_seconds=measured(
            *latency, "normalized_time_per_output_token", "p50"
        ),
        failures=failures,
        requests=total_requests,
    )


def read_metadata(path: Path) -> dict[str, Any]:
    with path.open(encoding="utf-8") as stream:
        document = yaml.safe_load(stream) or {}
    stack = nested(document, "scenario", "stack")
    if not isinstance(stack, list):
        raise ValueError(f"scenario.stack is not a list in {path}")
    inference_engine = next(
        (
            component.get("standardized", {})
            for component in stack
            if component.get("standardized", {}).get("kind") == "inference_engine"
        ),
        {},
    )
    components = [
        component.get("standardized", {})
        for component in stack
        if component.get("standardized", {}).get("kind") != "inference_engine"
    ]
    return {"inference_engine": inference_engine, "components": components}


def read_treatment(campaign: Path, treatment: str) -> TreatmentResults:
    treatment_dir = campaign / "runs" / treatment
    if not treatment_dir.is_dir():
        raise ValueError(f"missing treatment directory: {treatment_dir}")
    repetitions: list[list[StageMetrics]] = []
    metadata: dict[str, Any] | None = None
    for repetition in sorted(treatment_dir.glob("repetition-*")):
        reports = sorted(repetition.glob(REPORT_GLOB))
        if not reports:
            raise ValueError(f"no stage reports found in {repetition}")
        repetition_metadata = read_metadata(reports[0])
        if metadata is None:
            metadata = repetition_metadata
        elif repetition_metadata != metadata:
            raise ValueError(
                f"treatment {treatment!r} has different stack metadata across repetitions"
            )
        repetitions.append([read_stage(report) for report in reports])
    return aggregate_repetitions(treatment, repetitions, metadata)
