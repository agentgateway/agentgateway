"""CSV and Markdown rendering for inference comparison reports."""

from __future__ import annotations

import csv
import math
from pathlib import Path
from typing import Any

from common.model import TreatmentResults


def display_name(name: str) -> str:
    names = {
        "service": "k8s service (RR)",
        "agentgateway-standalone": "agentgateway standalone",
        "agentgateway-gateway": "agentgateway on Kubernetes",
        "envoy-standalone": "Envoy standalone",
    }
    return names.get(name, name.replace("-", " "))


def agentgateway_mode_note(name: str) -> str | None:
    notes = {
        "agentgateway-standalone": (
            "**agentgateway mode:** In standalone request scheduler mode, "
            "agentgateway runs as a sidecar proxy with the EPP and communicates "
            "with it over localhost, without a full Gateway API stack. "
            "[Learn more about standalone inference routing]"
            "(https://agentgateway.dev/docs/standalone/latest/inference/"
            "#standalone-request-scheduler-mode)."
        ),
        "agentgateway-gateway": (
            "**agentgateway mode:** In Kubernetes Gateway API mode, agentgateway "
            "is the Gateway data plane; an HTTPRoute targets an InferencePool, "
            "whose EPP selects the model-server endpoint. "
            "[Learn more about Kubernetes inference routing]"
            "(https://agentgateway.dev/docs/kubernetes/latest/llm/inference/"
            "inference-routing/)."
        ),
    }
    return notes.get(name)


def percent_delta(candidate: float, baseline: float) -> str:
    if not math.isfinite(candidate) or not math.isfinite(baseline) or baseline == 0:
        return "n/a"
    value = f"{((candidate - baseline) / baseline) * 100:+.1f}%"
    return value.replace("-", "−")


def fmt(value: float, digits: int = 2) -> str:
    if not math.isfinite(value):
        return "n/a"
    return f"{value:,.{digits}f}"


def itl_interpretation(
    baseline: TreatmentResults,
    candidate: TreatmentResults,
    baseline_peak: float,
    candidate_peak: float,
) -> list[str]:
    """Explain a lower Service ITL without presenting it as overall latency."""
    if baseline.name != "service":
        return []

    baseline_top = baseline.stages[-1]
    candidate_top = candidate.stages[-1]
    compared_values = (
        baseline_top.itl_p50_seconds,
        candidate_top.itl_p50_seconds,
    )
    if not all(math.isfinite(value) for value in compared_values):
        return []
    if baseline_top.itl_p50_seconds >= candidate_top.itl_p50_seconds:
        return []

    b_name = display_name(baseline.name)
    c_name = display_name(candidate.name)
    lines = [
        "## Interpreting ITL",
        "",
        f"At rate {baseline_top.requested_qps:g}, {b_name} has a lower p50 ITL "
        f"({fmt(baseline_top.itl_p50_seconds * 1000, 1)} ms) than {c_name} "
        f"({fmt(candidate_top.itl_p50_seconds * 1000, 1)} ms). ITL measures token "
        "cadence only after the first response token; time waiting to enter prefill "
        "or decode appears in TTFT instead. Lower ITL therefore does not by itself "
        "mean better overall inference latency.",
        "",
    ]

    tradeoff_values = (
        baseline_top.ttft_p50_seconds,
        candidate_top.ttft_p50_seconds,
        baseline_peak,
        candidate_peak,
    )
    if (
        all(math.isfinite(value) for value in tradeoff_values)
        and candidate_top.ttft_p50_seconds < baseline_top.ttft_p50_seconds
        and candidate_peak > baseline_peak
    ):
        tradeoff = (
            f"Here, {b_name} peaks at {fmt(baseline_peak, 0)} output tokens/s and "
            f"has {fmt(baseline_top.ttft_p50_seconds, 1)} s TTFT p50, while "
            f"{c_name} reaches {fmt(candidate_peak, 0)} output tokens/s with "
            f"{fmt(candidate_top.ttft_p50_seconds, 1)} s TTFT p50. This is "
            "consistent with the Service queueing longer while the routed treatment "
            "keeps more work decoding; vLLM continuous batching can trade higher "
            "per-stream ITL for greater throughput and lower TTFT."
        )
        baseline_low = baseline.stages[0]
        candidate_low = candidate.stages[0]
        low_values = (baseline_low.itl_p50_seconds, candidate_low.itl_p50_seconds)
        if (
            all(math.isfinite(value) and value > 0 for value in low_values)
            and 0.9
            <= candidate_low.itl_p50_seconds / baseline_low.itl_p50_seconds
            <= 1.1
        ):
            tradeoff += (
                f" At {baseline_low.requested_qps:g} QPS, ITL is "
                f"{fmt(baseline_low.itl_p50_seconds * 1000, 1)} ms versus "
                f"{fmt(candidate_low.itl_p50_seconds * 1000, 1)} ms, indicating "
                "that the larger high-load gap is not a fixed proxy penalty."
            )
        lines.extend([tradeoff, ""])
    return lines


def write_csv(path: Path, baseline: TreatmentResults, candidate: TreatmentResults) -> None:
    columns = (
        "requested_qps",
        "input_tokens_per_second",
        "output_tokens_per_second",
        "total_tokens_per_second",
        "requests_per_second",
        "ttft_mean_seconds",
        "ttft_p50_seconds",
        "ttft_p90_seconds",
        "itl_mean_seconds",
        "itl_p50_seconds",
        "ntpot_mean_seconds",
        "ntpot_p50_seconds",
        "failures",
        "requests",
        "failure_rate",
    )
    with path.open("w", encoding="utf-8", newline="") as stream:
        writer = csv.DictWriter(
            stream, fieldnames=("treatment", *columns), lineterminator="\n"
        )
        writer.writeheader()
        for treatment in (baseline, candidate):
            for stage in treatment.stages:
                row = {
                    column: (
                        value
                        if not isinstance(value := getattr(stage, column), float)
                        or math.isfinite(value)
                        else ""
                    )
                    for column in columns
                }
                row["treatment"] = treatment.name
                writer.writerow(row)


def write_markdown(
    path: Path,
    manifest: dict[str, Any],
    baseline: TreatmentResults,
    candidate: TreatmentResults,
    include_images: bool,
    evidence_paths: dict[str, str],
) -> None:
    identity = manifest["identity"]
    baseline_top = baseline.stages[-1]
    candidate_top = candidate.stages[-1]
    baseline_peak = max(stage.output_tokens_per_second for stage in baseline.stages)
    candidate_peak = max(stage.output_tokens_per_second for stage in candidate.stages)
    b_name = display_name(baseline.name)
    c_name = display_name(candidate.name)
    engine = candidate.metadata.get("inference_engine", {})
    components = candidate.metadata.get("components", [])
    candidate_manifest = manifest.get("treatments", {}).get(candidate.name, {})
    candidate_repetitions = candidate_manifest.get("repetitions", {})
    first_repetition = next(
        (candidate_repetitions[key] for key in sorted(candidate_repetitions)), {}
    )
    gateway_image = first_repetition.get("gateway", {}).get("image")
    component_versions = ", ".join(
        f"{component.get('tool', 'component')} "
        f"{component.get('tool_version') or gateway_image or 'unknown'}"
        for component in components
    ) or "none"

    accelerator = str(identity["accelerator_model"]).upper()
    backend = {
        "vllm": "vLLM",
    }.get(identity["backend_type"], identity["backend_type"])
    gpu_count = identity["replicas"] * identity["tensor_parallelism"]
    routing_policy = identity.get("routing_policy", "unknown")
    comparison_title = (
        f"Comparing {c_name} to a Simple Kubernetes Service ({backend})"
        if baseline.name == "service"
        else f"Comparing {c_name} to {b_name}"
    )
    delta_heading = "Δ% vs k8s" if baseline.name == "service" else f"Δ% vs {b_name}"
    comparison_description = (
        f"Graphs below compare {c_name} routing to a stock Kubernetes Service "
        f"that round-robins requests across the same {identity['replicas']} "
        f"{backend} pods (no EPP, no scoring). Tables abbreviate this baseline "
        "as `k8s service (RR)`, where RR means round-robin."
        if baseline.name == "service"
        else f"Graphs below compare {c_name} to {b_name} across the same "
        f"{identity['replicas']} {backend} pods."
    )

    lines = [
        "# Benchmark Report",
        "",
        f"The benchmark runs model {identity['model']} on {gpu_count} × {accelerator} "
        f"GPUs, distributed across {identity['replicas']} {backend} "
        f"model servers ({identity['tensor_parallelism']} GPUs per server with "
        f"TP={identity['tensor_parallelism']}). Workload is "
        f"`{identity['workload']}` driven across the configured request-rate ladder.",
        "",
        "> [!NOTE]",
        f"> These results use the `{routing_policy}` routing policy. Latency is "
        "reported as median (p50) in the summary; NTPOT is end-to-end latency "
        "divided by output tokens. Missing latency histograms are shown as `n/a` "
        "and dropped from latency lines. Both treatments use the same model-server "
        "topology and workload.",
        "",
        f"## {comparison_title}",
        "",
    ]
    mode_note = agentgateway_mode_note(candidate.name)
    if mode_note:
        lines.extend([mode_note, ""])
    lines.extend([comparison_description, ""])
    if include_images:
        lines.extend(
            [
                '<img src="./throughput_vs_qps.png" width="900" alt="Throughput vs QPS">',
                '<img src="./latency_vs_qps.png" width="900" alt="Latency vs QPS">',
                '<img src="./ttft_p90_vs_qps.png" width="900" alt="TTFT p90 vs QPS">',
            ]
        )

    lines.extend(
        [
            f"Summary (throughput is peak sustained; latencies at the top of the "
            f"ladder, rate {baseline_top.requested_qps:g}):",
            "",
            f"| Metric | {b_name} | {c_name} | {delta_heading} |",
            "| :--- | :--- | :--- | :--- |",
            f"| Peak output tokens/s | {fmt(baseline_peak, 0)} | {fmt(candidate_peak, 0)} | {percent_delta(candidate_peak, baseline_peak)} |",
            f"| Requests/sec (@ rate {baseline_top.requested_qps:g}) | {fmt(baseline_top.requests_per_second)} | {fmt(candidate_top.requests_per_second)} | {percent_delta(candidate_top.requests_per_second, baseline_top.requests_per_second)} |",
            f"| TTFT p50 (s) | {fmt(baseline_top.ttft_p50_seconds, 1)} | {fmt(candidate_top.ttft_p50_seconds, 1)} | {percent_delta(candidate_top.ttft_p50_seconds, baseline_top.ttft_p50_seconds)} |",
            f"| TTFT p90 (s) | {fmt(baseline_top.ttft_p90_seconds, 1)} | {fmt(candidate_top.ttft_p90_seconds, 1)} | {percent_delta(candidate_top.ttft_p90_seconds, baseline_top.ttft_p90_seconds)} |",
            f"| ITL p50 (ms) | {fmt(baseline_top.itl_p50_seconds * 1000, 1)} | {fmt(candidate_top.itl_p50_seconds * 1000, 1)} | {percent_delta(candidate_top.itl_p50_seconds, baseline_top.itl_p50_seconds)} |",
            "",
            "<details>",
            "<summary><b><i>Click</i></b> to view the per-rate breakdown across the full ladder</summary>",
            "",
            "Output tokens/sec — higher is better; TTFT in seconds — lower is "
            "better. `n/a` means the stage had no successful-request latency "
            "histogram.",
            "",
            f"| Rate | {b_name} Output | {c_name} Output | {b_name} TTFT p50 | {c_name} TTFT p50 | {b_name} TTFT p90 | {c_name} TTFT p90 |",
            "| ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
        ]
    )
    for b_stage, c_stage in zip(baseline.stages, candidate.stages, strict=True):
        lines.append(
            f"| {b_stage.requested_qps:g} | {fmt(b_stage.output_tokens_per_second, 0)} "
            f"| {fmt(c_stage.output_tokens_per_second, 0)} "
            f"| {fmt(b_stage.ttft_p50_seconds, 1)} | {fmt(c_stage.ttft_p50_seconds, 1)} "
            f"| {fmt(b_stage.ttft_p90_seconds, 1)} | {fmt(c_stage.ttft_p90_seconds, 1)} |"
        )

    lines.extend(
        [
            "</details>",
            "",
            "Latency must be interpreted together with failures. A treatment can "
            "show deceptively low latency under overload when slower requests time "
            "out and are excluded from successful-request latency distributions. "
            "Failure counts and rates are retained in `metrics.csv`.",
            "",
        ]
    )
    lines.extend(
        itl_interpretation(
            baseline,
            candidate,
            baseline_peak,
            candidate_peak,
        )
    )
    lines.extend(
        [
            "## Configuration",
            "",
            f"- Campaign: `{manifest['campaign_id']}`",
            f"- Suite: `{manifest['suite']}`",
            f"- Scenario: `{identity['scenario']}`",
            f"- Model: `{identity['model']}`",
            f"- Backend: `{identity['backend_type']}`",
            f"- Backend image/version: `{engine.get('tool_version', 'unknown')}`",
            f"- Accelerator: `{identity['accelerator_model']}`",
            f"- Model servers: {identity['replicas']} with TP={identity['tensor_parallelism']}",
            f"- Workload: `{identity['workload']}` (`{identity['workload_variant']}`)",
            f"- Repetitions per treatment: {baseline.repetitions}",
            f"- Candidate components: {component_versions}",
            "",
            "Warm-up stages whose requested rate is repeated in the measured ladder "
            "are excluded. With multiple repetitions, each table and graph point is "
            "the median at that requested QPS.",
            "",
            "## Evidence",
            "",
        ]
    )
    for treatment, evidence in evidence_paths.items():
        lines.append(f"- [{display_name(treatment)} native results]({evidence})")
    lines.extend(
        [
            "- [Campaign manifest](../campaign-manifest.yaml)",
            "- [Normalized metrics](metrics.csv)",
            "",
        ]
    )
    path.write_text("\n".join(lines), encoding="utf-8")
