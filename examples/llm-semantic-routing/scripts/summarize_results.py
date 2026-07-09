#!/usr/bin/env python3
import argparse
import csv
import json
import math
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


def summarize(rows, ratings):
    by_lane = defaultdict(list)
    for row in rows:
        by_lane[row["lane"]].append(row)

    print("Lane summary")
    print("lane,requests,ok,input_tokens,output_tokens,cost_estimate,p50_ms,p95_ms")
    for lane in sorted(by_lane):
        lane_rows = by_lane[lane]
        ok_rows = [r for r in lane_rows if r.get("ok")]
        input_tokens = sum((r.get("usage") or {}).get("input_tokens", 0) or 0 for r in ok_rows)
        output_tokens = sum((r.get("usage") or {}).get("output_tokens", 0) or 0 for r in ok_rows)
        costs = [r.get("cost_estimate_usd") for r in ok_rows if r.get("cost_estimate_usd") is not None]
        latencies = [r.get("latency_ms") for r in ok_rows]
        print(
            f"{lane},{len(lane_rows)},{len(ok_rows)},{input_tokens},{output_tokens},"
            f"{fmt_money(sum(costs))},{percentile(latencies, 0.50):.1f},{percentile(latencies, 0.95):.1f}"
        )

    routed = [r for r in by_lane.get("routed", []) if r.get("ok")]
    if routed:
        correct = [r for r in routed if r.get("routing_correct") is True]
        print()
        print(f"Routing accuracy: {len(correct)}/{len(routed)} = {len(correct) / len(routed):.1%}")
        confusion = defaultdict(int)
        for row in routed:
            expected = canonical_model(row.get("expected_model"))
            selected = canonical_model(row.get("selected_model"))
            confusion[(expected, selected)] += 1
        print("expected_model,selected_model,count")
        for (expected, selected), count in sorted(confusion.items()):
            print(f"{expected},{selected},{count}")

        routed_cost = sum(r.get("cost_estimate_usd") or 0 for r in routed)
        routed_tokens_as_expensive = sum(cost_as_model("gpt-5.5", r.get("usage") or {}) for r in routed)
        if routed_tokens_as_expensive:
            savings = 1 - (routed_cost / routed_tokens_as_expensive)
            print()
            print(
                "Counterfactual savings on routed token counts: "
                f"{fmt_money(routed_tokens_as_expensive)} always_expensive vs "
                f"{fmt_money(routed_cost)} routed = {savings:.1%}"
            )

    if by_lane.get("routed") and by_lane.get("always_expensive"):
        routed_cost = sum(r.get("cost_estimate_usd") or 0 for r in by_lane["routed"] if r.get("ok"))
        expensive_cost = sum(
            r.get("cost_estimate_usd") or 0 for r in by_lane["always_expensive"] if r.get("ok")
        )
        if expensive_cost:
            print(
                "Actual lane savings: "
                f"{fmt_money(expensive_cost)} always_expensive vs "
                f"{fmt_money(routed_cost)} routed = {1 - routed_cost / expensive_cost:.1%}"
            )

    if ratings:
        print()
        print("Satisfaction")
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
        for lane, values in sorted(by_rating_lane.items()):
            print(f"{lane}: avg={sum(values) / len(values):.2f} n={len(values)}")
        if right_model:
            print(f"human right-model rate: {sum(right_model)}/{len(right_model)} = {sum(right_model) / len(right_model):.1%}")


def main():
    parser = argparse.ArgumentParser(description="Summarize Semantic Router eval JSONL results.")
    parser.add_argument("results")
    parser.add_argument("--ratings", default="", help="Optional CSV with id,lane,satisfaction,right_model columns.")
    args = parser.parse_args()
    summarize(load_results(args.results), load_ratings(args.ratings))


if __name__ == "__main__":
    main()
