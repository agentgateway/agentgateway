"""Deterministic plots for inference comparison reports.

Keep this publication renderer local until inference-perf supports custom
series names, requested-rate axes, and percentile plots:
https://github.com/kubernetes-sigs/inference-perf/issues/741
"""

from __future__ import annotations

import math
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402

from common.model import TreatmentResults
from common.rendering import display_name


COLORS = ("#1f77b4", "#ff7f0e")


def series(
    results: TreatmentResults,
    attribute: str,
    scale: float = 1.0,
    positive_only: bool = False,
) -> list[float]:
    values = [getattr(stage, attribute) * scale for stage in results.stages]
    if positive_only:
        return [value if value > 0 else math.nan for value in values]
    return values


def finish(fig: plt.Figure, path: Path) -> None:
    fig.tight_layout(rect=(0, 0, 1, 0.92))
    fig.savefig(path, dpi=100, bbox_inches="tight", metadata={"Software": "agentgateway"})
    plt.close(fig)


def plot_metric(
    axis: plt.Axes,
    treatments: tuple[TreatmentResults, TreatmentResults],
    attribute: str,
    scale: float,
    title: str,
    ylabel: str,
    positive_only: bool = False,
) -> None:
    requested_rates = series(treatments[0], "requested_qps")
    for color, treatment in zip(COLORS, treatments, strict=True):
        axis.plot(
            requested_rates,
            series(treatment, attribute, scale, positive_only),
            marker="o",
            color=color,
            label=display_name(treatment.name),
        )
    axis.set_title(title)
    axis.set_xlabel("Requested Rate (QPS)")
    axis.set_ylabel(ylabel)
    axis.set_xlim(0, max(requested_rates) * 1.05)
    axis.grid(alpha=0.6)


def plot_throughput(
    baseline: TreatmentResults, candidate: TreatmentResults, path: Path
) -> None:
    treatments = (candidate, baseline)
    metrics = (
        ("input_tokens_per_second", "Input Tokens/s"),
        ("output_tokens_per_second", "Output Tokens/s"),
        ("total_tokens_per_second", "Total Tokens/s"),
    )
    fig, axes = plt.subplots(1, 3, figsize=(18, 6))
    for axis, (attribute, title) in zip(axes, metrics, strict=True):
        plot_metric(axis, treatments, attribute, 1.0, title, "Tokens / sec")
    handles, labels = axes[0].get_legend_handles_labels()
    fig.legend(handles, labels, loc="upper center", ncol=2)
    finish(fig, path)


def plot_latency(
    baseline: TreatmentResults, candidate: TreatmentResults, path: Path
) -> None:
    metrics = (
        ("ttft_mean_seconds", "Mean TTFT"),
        ("itl_mean_seconds", "Mean ITL"),
        ("ntpot_mean_seconds", "Mean NTPOT"),
    )
    treatments = (candidate, baseline)
    fig, axes = plt.subplots(1, 3, figsize=(18, 6))
    for axis, (attribute, title) in zip(axes, metrics, strict=True):
        plot_metric(
            axis,
            treatments,
            attribute,
            1000.0,
            title,
            "Latency (ms)",
            positive_only=True,
        )
    handles, labels = axes[0].get_legend_handles_labels()
    fig.legend(handles, labels, loc="upper center", ncol=2)
    finish(fig, path)


def plot_ttft_p90(
    baseline: TreatmentResults, candidate: TreatmentResults, path: Path
) -> None:
    fig, axis = plt.subplots(1, 1, figsize=(6, 6))
    plot_metric(
        axis,
        (candidate, baseline),
        "ttft_p90_seconds",
        1000.0,
        "TTFT 90th Percentile",
        "Latency (ms)",
        positive_only=True,
    )
    handles, labels = axis.get_legend_handles_labels()
    fig.legend(handles, labels, loc="upper center", ncol=2)
    finish(fig, path)
