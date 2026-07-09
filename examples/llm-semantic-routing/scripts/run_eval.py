#!/usr/bin/env python3
import argparse
import json
import os
import random
import sys
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone


DEFAULT_RATES = {
    "gpt-5.4-nano": {"input": 0.20, "cached_input": 0.02, "output": 1.25},
    "gpt-5.5": {"input": 5.00, "cached_input": 0.50, "output": 30.00},
}

VSR_HEADERS = [
    "x-vsr-selected-model",
    "x-vsr-selected-decision",
    "x-vsr-selected-confidence",
    "x-vsr-selected-category",
    "x-vsr-selected-reasoning",
    "x-vsr-matched-keywords",
    "x-vsr-matched-embeddings",
    "x-vsr-matched-complexity",
    "x-vsr-matched-context",
    "x-vsr-matched-structure",
    "x-vsr-matched-projection",
]


def parse_args():
    default_gateway = os.environ.get("INGRESS_GW_ADDRESS", "")
    parser = argparse.ArgumentParser(
        description="Run routed and forced-model LLM eval lanes through agentgateway."
    )
    parser.add_argument("--gateway-url", default=default_gateway)
    parser.add_argument("--path", default="/v1/chat/completions")
    parser.add_argument("--dataset", default="examples/llm-semantic-routing/data/eval-corpus.jsonl")
    parser.add_argument("--output", default="")
    parser.add_argument("--run-id", default=datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ"))
    parser.add_argument("--lanes", default="routed,always_low_cost,always_expensive")
    parser.add_argument("--low-cost-model", default="gpt-5.4-nano")
    parser.add_argument("--expensive-model", default="gpt-5.5")
    parser.add_argument("--auto-model", default="auto")
    parser.add_argument("--system-prompt", default="You are a concise technical assistant. Answer directly.")
    parser.add_argument("--temperature", type=float, default=None)
    parser.add_argument("--timeout", type=float, default=180.0)
    parser.add_argument("--limit", type=int, default=0)
    parser.add_argument("--seed", type=int, default=7)
    parser.add_argument("--delay-sec", type=float, default=0.0)
    parser.add_argument("--capture-output", action="store_true")
    parser.add_argument("--dry-run", action="store_true")
    return parser.parse_args()


def load_dataset(path, limit):
    rows = []
    with open(path, "r", encoding="utf-8") as f:
        for line_no, line in enumerate(f, 1):
            line = line.strip()
            if not line:
                continue
            try:
                rows.append(json.loads(line))
            except json.JSONDecodeError as e:
                raise SystemExit(f"{path}:{line_no}: invalid JSON: {e}") from e
            if limit and len(rows) >= limit:
                break
    return rows


def normalize_url(gateway_url, path):
    if not gateway_url:
        raise SystemExit("Set --gateway-url or INGRESS_GW_ADDRESS.")
    base = gateway_url.rstrip("/")
    suffix = path if path.startswith("/") else "/" + path
    return base + suffix


def request_model_for_lane(args, lane):
    if lane == "routed":
        return args.auto_model
    if lane == "always_low_cost":
        return args.low_cost_model
    if lane == "always_expensive":
        return args.expensive_model
    raise SystemExit(f"Unknown lane {lane!r}")


def headers_for(args, item, lane):
    request_id = f"vsr-{args.run_id}-{lane}-{item['id']}"
    headers = {
        "Content-Type": "application/json",
        "X-Request-ID": request_id,
        "X-Experiment-ID": args.run_id,
        "X-Eval-ID": item["id"],
        "X-Eval-Lane": lane,
        "X-User-ID": f"vsr-{args.run_id}-{lane}",
    }
    if lane == "routed":
        headers["X-VSR-Debug"] = "true"
    return headers


def payload_for(args, item, lane):
    payload = {
        "model": request_model_for_lane(args, lane),
        "messages": [
            {"role": "system", "content": args.system_prompt},
            {"role": "user", "content": item["prompt"]},
        ],
        "max_tokens": item.get("max_tokens", 180),
    }
    if args.temperature is not None:
        payload["temperature"] = args.temperature
    return payload


def lower_headers(headers):
    return {k.lower(): v for k, v in headers.items()}


def usage_from_response(body):
    usage = body.get("usage") if isinstance(body, dict) else {}
    if not isinstance(usage, dict):
        usage = {}
    prompt_details = usage.get("prompt_tokens_details") or {}
    if not isinstance(prompt_details, dict):
        prompt_details = {}
    return {
        "input_tokens": usage.get("prompt_tokens", usage.get("input_tokens", 0)) or 0,
        "cached_input_tokens": prompt_details.get("cached_tokens", usage.get("cached_input_tokens", 0)) or 0,
        "output_tokens": usage.get("completion_tokens", usage.get("output_tokens", 0)) or 0,
        "total_tokens": usage.get("total_tokens", 0) or 0,
        "raw": usage,
    }


def canonical_model(model):
    if not model:
        return ""
    model = model.lower()
    if model.startswith("gpt-5.5"):
        return "gpt-5.5"
    if model.startswith("gpt-5.4-nano"):
        return "gpt-5.4-nano"
    return model


def cost_estimate(model, usage):
    model = canonical_model(model)
    rates = DEFAULT_RATES.get(model)
    if not rates:
        return None
    input_tokens = usage["input_tokens"]
    cached_tokens = min(usage["cached_input_tokens"], input_tokens)
    uncached_tokens = max(input_tokens - cached_tokens, 0)
    output_tokens = usage["output_tokens"]
    return (
        (uncached_tokens * rates["input"]) +
        (cached_tokens * rates["cached_input"]) +
        (output_tokens * rates["output"])
    ) / 1_000_000


def extract_text(body):
    if not isinstance(body, dict):
        return ""
    choices = body.get("choices") or []
    if not choices:
        return ""
    message = choices[0].get("message") or {}
    content = message.get("content", "")
    if isinstance(content, str):
        return content
    return json.dumps(content, ensure_ascii=False)


def post_json(url, payload, headers, timeout):
    encoded = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(url, data=encoded, headers=headers, method="POST")
    started = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            raw = resp.read()
            elapsed_ms = (time.perf_counter() - started) * 1000
            return resp.status, lower_headers(resp.headers), raw, elapsed_ms, ""
    except urllib.error.HTTPError as e:
        raw = e.read()
        elapsed_ms = (time.perf_counter() - started) * 1000
        return e.code, lower_headers(e.headers), raw, elapsed_ms, str(e)
    except Exception as e:
        elapsed_ms = (time.perf_counter() - started) * 1000
        return 0, {}, b"", elapsed_ms, str(e)


def run_one(args, url, item, lane):
    payload = payload_for(args, item, lane)
    headers = headers_for(args, item, lane)
    status, response_headers, raw, latency_ms, error = post_json(url, payload, headers, args.timeout)
    try:
        body = json.loads(raw.decode("utf-8")) if raw else {}
    except json.JSONDecodeError:
        body = {"raw_body": raw.decode("utf-8", errors="replace")}
    usage = usage_from_response(body)
    response_model = body.get("model", "") if isinstance(body, dict) else ""
    selected_model = response_headers.get("x-vsr-selected-model") or response_model or payload["model"]
    canonical_selected = canonical_model(selected_model)
    canonical_expected = canonical_model(item.get("expected_model", ""))
    record = {
        "run_id": args.run_id,
        "timestamp": datetime.now(timezone.utc).isoformat(),
        "id": item["id"],
        "family": item.get("family", ""),
        "lane": lane,
        "expected_model": item.get("expected_model", ""),
        "request_model": payload["model"],
        "selected_model": selected_model,
        "response_model": response_model,
        "status": status,
        "ok": 200 <= status < 300,
        "latency_ms": round(latency_ms, 3),
        "usage": usage,
        "cost_estimate_usd": cost_estimate(selected_model, usage),
        "routing_correct": (
            canonical_selected == canonical_expected if lane == "routed" and canonical_expected else None
        ),
        "request_headers": {
            "x-request-id": headers["X-Request-ID"],
            "x-experiment-id": headers["X-Experiment-ID"],
            "x-eval-lane": headers["X-Eval-Lane"],
            "x-user-id": headers["X-User-ID"],
        },
        "vsr_headers": {name: response_headers.get(name, "") for name in VSR_HEADERS},
        "error": error,
    }
    if args.capture_output:
        record["response_text"] = extract_text(body)
    elif not record["ok"]:
        record["error_body"] = body
    return record


def main():
    args = parse_args()
    lanes = [lane.strip() for lane in args.lanes.split(",") if lane.strip()]
    items = load_dataset(args.dataset, args.limit)
    url = normalize_url(args.gateway_url, args.path)
    output = args.output or f"examples/llm-semantic-routing/results/{args.run_id}.jsonl"
    jobs = [(item, lane) for item in items for lane in lanes]
    random.Random(args.seed).shuffle(jobs)

    print(f"run_id={args.run_id}")
    print(f"url={url}")
    print(f"dataset_items={len(items)} lanes={','.join(lanes)} total_requests={len(jobs)}")
    print(f"output={output}")

    if args.dry_run:
        for item, lane in jobs[:10]:
            print(f"dry-run {lane} {item['id']} model={request_model_for_lane(args, lane)}")
        return

    os.makedirs(os.path.dirname(output), exist_ok=True)
    ok_count = 0
    with open(output, "w", encoding="utf-8") as f:
        for idx, (item, lane) in enumerate(jobs, 1):
            record = run_one(args, url, item, lane)
            f.write(json.dumps(record, ensure_ascii=False) + "\n")
            f.flush()
            ok_count += 1 if record["ok"] else 0
            selected = record["selected_model"] or "-"
            print(
                f"{idx:03d}/{len(jobs)} {lane:15s} {item['id']:14s} "
                f"status={record['status']} selected={selected} "
                f"latency_ms={record['latency_ms']:.1f}"
            )
            if args.delay_sec > 0 and idx < len(jobs):
                time.sleep(args.delay_sec)
    print(f"completed ok={ok_count}/{len(jobs)} output={output}")


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        sys.exit(130)
