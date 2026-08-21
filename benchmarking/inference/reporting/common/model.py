"""Canonical metrics used by inference comparison reports."""

from __future__ import annotations

from dataclasses import dataclass, field, fields
from statistics import median
from typing import Any


@dataclass(frozen=True)
class StageMetrics:
    stage: int
    requested_qps: float
    input_tokens_per_second: float
    output_tokens_per_second: float
    total_tokens_per_second: float
    requests_per_second: float
    ttft_mean_seconds: float
    ttft_p50_seconds: float
    ttft_p90_seconds: float
    itl_mean_seconds: float
    itl_p50_seconds: float
    ntpot_mean_seconds: float
    ntpot_p50_seconds: float
    failures: float
    requests: float

    @property
    def failure_rate(self) -> float:
        return self.failures / self.requests if self.requests else 0.0


@dataclass(frozen=True)
class TreatmentResults:
    name: str
    repetitions: int
    stages: tuple[StageMetrics, ...]
    total_failures: float = 0
    total_requests: float = 0
    metadata: dict[str, Any] = field(default_factory=dict)


def aggregate_repetitions(
    name: str,
    repetitions: list[list[StageMetrics]],
    metadata: dict[str, Any] | None = None,
) -> TreatmentResults:
    if not repetitions:
        raise ValueError(f"treatment {name!r} has no repetitions")

    by_repetition: list[dict[float, StageMetrics]] = []
    for repetition in repetitions:
        # The optimized-baseline workload has a warm-up stage at a QPS that is
        # repeated in the measured ladder. Keep the later stage for that rate.
        by_rate: dict[float, StageMetrics] = {}
        for stage in sorted(repetition, key=lambda value: value.stage):
            by_rate[stage.requested_qps] = stage
        by_repetition.append(by_rate)

    expected = set(by_repetition[0])
    for index, by_rate in enumerate(by_repetition[1:], start=2):
        if set(by_rate) != expected:
            raise ValueError(
                f"treatment {name!r} repetition {index} has a different QPS ladder"
            )

    numeric_fields = [field.name for field in fields(StageMetrics) if field.name != "stage"]
    aggregated: list[StageMetrics] = []
    for stage_index, rate in enumerate(sorted(expected)):
        samples = [repetition[rate] for repetition in by_repetition]
        values = {
            field: median(getattr(sample, field) for sample in samples)
            for field in numeric_fields
        }
        aggregated.append(StageMetrics(stage=stage_index, **values))

    return TreatmentResults(
        name=name,
        repetitions=len(repetitions),
        stages=tuple(aggregated),
        total_failures=sum(
            stage.failures for repetition in by_repetition for stage in repetition.values()
        ),
        total_requests=sum(
            stage.requests for repetition in by_repetition for stage in repetition.values()
        ),
        metadata=metadata or {},
    )


def validate_comparison(
    baseline: TreatmentResults, candidate: TreatmentResults
) -> None:
    baseline_rates = [stage.requested_qps for stage in baseline.stages]
    candidate_rates = [stage.requested_qps for stage in candidate.stages]
    if baseline_rates != candidate_rates:
        raise ValueError(
            f"QPS ladders differ: {baseline.name}={baseline_rates}, "
            f"{candidate.name}={candidate_rates}"
        )
    if baseline.repetitions != candidate.repetitions:
        raise ValueError(
            f"repetition counts differ: {baseline.name}={baseline.repetitions}, "
            f"{candidate.name}={candidate.repetitions}"
        )
