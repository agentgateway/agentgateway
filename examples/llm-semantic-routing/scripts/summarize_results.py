#!/usr/bin/env python3
import argparse
import csv
import json
import math
import os
from collections import defaultdict


RATES = {
    "gpt-5.4-nano": {"input": 0.20, "cached_input": 0.02, "output": 1.25},
    "gpt-5.5": {"input": 5.00, "cached_input": 0.50, "output": 30.00},
}


def canonical_model(model):
    model = (model or "").lower()
    if model.startswith("gpt-5.5"):
        return "gpt-5.5"
    if model.startswith("gpt-5.4-nano"):
        return "gpt-5.4-nano"
    return model


def percentile(values, pct):
    values = sorted(v for v in values if v is not None and not math.isnan(v))
    if not values:
        return 0.0
    if len(values) == 1:
        return values[0]
    rank = (len(values) - 1) * pct
    lower = math.floor(rank)
    upper = math.ceil(rank)
    if lower == upper:
        return values[int(rank)]
    return values[lower] + (values[upper] - values[lower]) * (rank - lower)


def load_results(path):
    rows = []
    with open(path, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if line:
                rows.append(json.loads(line))
    return rows


def cost_as_model(model, usage):
    rates = RATES[model]
    input_tokens = usage.get("input_tokens", 0) or 0
    cached_tokens = min(usage.get("cached_input_tokens", 0) or 0, input_tokens)
    uncached_tokens = max(input_tokens - cached_tokens, 0)
    output_tokens = usage.get("output_tokens", 0) or 0
    return (
        uncached_tokens * rates["input"] +
        cached_tokens * rates["cached_input"] +
        output_tokens * rates["output"]
    ) / 1_000_000


def load_ratings(path):
    if not path:
        return {}
    ratings = {}
    with open(path, "r", encoding="utf-8") as f:
        for row in csv.DictReader(f):
            ratings[(row["id"], row["lane"])] = row
    return ratings


def fmt_money(value):
    return f"${value:.6f}"


def build_summary(rows, ratings):
    by_lane = defaultdict(list)
    for row in rows:
        by_lane[row["lane"]].append(row)

    lanes = {}
    for lane in sorted(by_lane):
        lane_rows = by_lane[lane]
        ok_rows = [r for r in lane_rows if r.get("ok")]
        input_tokens = sum((r.get("usage") or {}).get("input_tokens", 0) or 0 for r in ok_rows)
        output_tokens = sum((r.get("usage") or {}).get("output_tokens", 0) or 0 for r in ok_rows)
        costs = [r.get("cost_estimate_usd") for r in ok_rows if r.get("cost_estimate_usd") is not None]
        latencies = [r.get("latency_ms") for r in ok_rows]
        lanes[lane] = {
            "requests": len(lane_rows),
            "ok": len(ok_rows),
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
            "cost_estimate_usd": sum(costs),
            "latency_ms": {
                "p50": percentile(latencies, 0.50),
                "p95": percentile(latencies, 0.95),
            },
        }

    routing = None
    routed = [r for r in by_lane.get("routed", []) if r.get("ok")]
    if routed:
        correct = [r for r in routed if r.get("routing_correct") is True]
        confusion = defaultdict(int)
        for row in routed:
            expected = canonical_model(row.get("expected_model"))
            selected = canonical_model(row.get("selected_model"))
            confusion[(expected, selected)] += 1
        routing = {
            "correct": len(correct),
            "total": len(routed),
            "accuracy": len(correct) / len(routed),
            "confusion_matrix": [
                {
                    "expected_model": expected,
                    "selected_model": selected,
                    "count": count,
                }
                for (expected, selected), count in sorted(confusion.items())
            ],
        }

    counterfactual_savings = None
    if routed:
        routed_cost = sum(r.get("cost_estimate_usd") or 0 for r in routed)
        routed_tokens_as_expensive = sum(cost_as_model("gpt-5.5", r.get("usage") or {}) for r in routed)
        if routed_tokens_as_expensive:
            counterfactual_savings = {
                "always_expensive_cost_usd": routed_tokens_as_expensive,
                "routed_cost_usd": routed_cost,
                "savings_fraction": 1 - (routed_cost / routed_tokens_as_expensive),
            }

    actual_savings = None
    if by_lane.get("routed") and by_lane.get("always_expensive"):
        routed_cost = sum(r.get("cost_estimate_usd") or 0 for r in by_lane["routed"] if r.get("ok"))
        expensive_cost = sum(
            r.get("cost_estimate_usd") or 0 for r in by_lane["always_expensive"] if r.get("ok")
        )
        if expensive_cost:
            actual_savings = {
                "always_expensive_cost_usd": expensive_cost,
                "routed_cost_usd": routed_cost,
                "savings_fraction": 1 - routed_cost / expensive_cost,
            }

    satisfaction = None
    if ratings:
        by_rating_lane = defaultdict(list)
        right_model = []
        for row in rows:
            rating = ratings.get((row["id"], row["lane"]))
            if not rating:
                continue
            if rating.get("satisfaction"):
                by_rating_lane[row["lane"]].append(float(rating["satisfaction"]))
            if row["lane"] == "routed" and rating.get("right_model"):
                right_model.append(rating["right_model"].strip().lower() in ("1", "true", "yes", "y"))
        satisfaction = {
            "lanes": {
                lane: {"average": sum(values) / len(values), "count": len(values)}
                for lane, values in sorted(by_rating_lane.items())
            },
            "human_right_model": None,
        }
        if right_model:
            satisfaction["human_right_model"] = {
                "correct": sum(right_model),
                "total": len(right_model),
                "rate": sum(right_model) / len(right_model),
            }

    return {
        "lanes": lanes,
        "routing": routing,
        "savings": {
            "counterfactual_on_routed_tokens": counterfactual_savings,
            "actual_lanes": actual_savings,
        },
        "satisfaction": satisfaction,
    }


def render_summary(summary):
    lines = [
        "Lane summary",
        "lane,requests,ok,input_tokens,output_tokens,cost_estimate,p50_ms,p95_ms",
    ]
    for lane, values in summary["lanes"].items():
        lines.append(
            f"{lane},{values['requests']},{values['ok']},{values['input_tokens']},"
            f"{values['output_tokens']},{fmt_money(values['cost_estimate_usd'])},"
            f"{values['latency_ms']['p50']:.1f},{values['latency_ms']['p95']:.1f}"
        )

    routing = summary["routing"]
    if routing:
        lines.extend([
            "",
            f"Routing accuracy: {routing['correct']}/{routing['total']} = {routing['accuracy']:.1%}",
            "expected_model,selected_model,count",
        ])
        for item in routing["confusion_matrix"]:
            lines.append(
                f"{item['expected_model']},{item['selected_model']},{item['count']}"
            )

    counterfactual = summary["savings"]["counterfactual_on_routed_tokens"]
    if counterfactual:
        lines.extend([
            "",
            "Counterfactual savings on routed token counts: "
            f"{fmt_money(counterfactual['always_expensive_cost_usd'])} always_expensive vs "
            f"{fmt_money(counterfactual['routed_cost_usd'])} routed = "
            f"{counterfactual['savings_fraction']:.1%}",
        ])

    actual = summary["savings"]["actual_lanes"]
    if actual:
        lines.append(
            "Actual lane savings: "
            f"{fmt_money(actual['always_expensive_cost_usd'])} always_expensive vs "
            f"{fmt_money(actual['routed_cost_usd'])} routed = "
            f"{actual['savings_fraction']:.1%}"
        )

    satisfaction = summary["satisfaction"]
    if satisfaction:
        lines.extend(["", "Satisfaction"])
        for lane, values in satisfaction["lanes"].items():
            lines.append(f"{lane}: avg={values['average']:.2f} n={values['count']}")
        right_model = satisfaction["human_right_model"]
        if right_model:
            lines.append(
                f"human right-model rate: {right_model['correct']}/{right_model['total']} = "
                f"{right_model['rate']:.1%}"
            )
    return "\n".join(lines) + "\n"


def write_text(path, text):
    os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
    with open(path, "w", encoding="utf-8") as stream:
        stream.write(text)


def write_json(path, summary):
    os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
    with open(path, "w", encoding="utf-8") as stream:
        json.dump(summary, stream, indent=2, sort_keys=True)
        stream.write("\n")


def main():
    parser = argparse.ArgumentParser(description="Summarize Semantic Router eval JSONL results.")
    parser.add_argument("results")
    parser.add_argument(
        "--ratings",
        default="",
        help="Optional CSV with id,lane,satisfaction,right_model columns.",
    )
    parser.add_argument("--json-output", default="", help="Write the summary as JSON.")
    parser.add_argument("--text-output", default="", help="Write the rendered summary as text.")
    args = parser.parse_args()
    summary = build_summary(load_results(args.results), load_ratings(args.ratings))
    text = render_summary(summary)
    if args.json_output:
        write_json(args.json_output, summary)
    if args.text_output:
        write_text(args.text_output, text)
    print(text, end="")


if __name__ == "__main__":
    main()
