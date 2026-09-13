# /// script
# requires-python = ">=3.11"
# dependencies = ["opentelemetry-proto==1.36.0", "prometheus-client==0.22.1"]
# ///
"""Exercise the real v1.5.0 proxy against the deterministic provider fixture."""

import argparse
import base64
import json
import math
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path

from opentelemetry.proto.collector.trace.v1.trace_service_pb2 import (
    ExportTraceServiceRequest,
)
from prometheus_client.parser import text_string_to_metric_families


def get(url):
    with urllib.request.urlopen(url, timeout=10) as response:
        return response.read()


def samples(text):
    return [
        sample
        for family in text_string_to_metric_families(text)
        for sample in family.samples
    ]


def total(data, name, **labels):
    return sum(
        s.value
        for s in data
        if s.name == name and all(s.labels.get(k) == v for k, v in labels.items())
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--datadog",
        action="store_true",
        help="Generate traffic for Datadog and validate metrics without local trace assertions",
    )
    parser.add_argument(
        "--capture-content",
        action="store_true",
        help="Validate the explicit synthetic content-capture configuration",
    )
    args = parser.parse_args()
    for attempt in range(20):
        try:
            get("http://127.0.0.1:18520/metrics")
            break
        except (urllib.error.URLError, ConnectionError):
            if attempt == 19:
                raise
            time.sleep(1)
    before = samples(get("http://127.0.0.1:18520/metrics").decode())
    trace_id = uuid.uuid4().hex
    parent_id = "b0" * 8
    cases = [
        ("datadog-test", False, 200),
        ("datadog-test", True, 200),
        ("datadog-unpriced", False, 200),
        ("datadog-error", False, 500),
        ("datadog-rate-limit", False, 429),
    ]
    for model, stream, expected in cases:
        payload = {
            "model": model,
            "stream": stream,
            "messages": [{"role": "user", "content": "Synthetic test prompt."}],
        }
        request = urllib.request.Request(
            "http://127.0.0.1:13000/v1/chat/completions",
            json.dumps(payload).encode(),
            {
                "Content-Type": "application/json",
                "traceparent": f"00-{trace_id}-{parent_id}-01",
            },
        )
        try:
            response = urllib.request.urlopen(request, timeout=15)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            body = response.read()
            assert response.status == expected, (model, response.status, body)
            if stream:
                assert b"[DONE]" in body and b"Synthetic" in body, body
    text = get("http://127.0.0.1:18520/metrics").decode()
    after = samples(text)

    def delta(name, **labels):
        return total(after, name, **labels) - total(before, name, **labels)

    assert delta("agentgateway_requests_total") == 5
    assert delta("agentgateway_requests_total", status="500") == 1
    assert delta("agentgateway_requests_total", status="429") == 1
    assert (
        delta("agentgateway_gen_ai_client_token_usage_sum", gen_ai_token_type="input")
        == 30
    )
    assert (
        delta("agentgateway_gen_ai_client_token_usage_sum", gen_ai_token_type="output")
        == 12
    )
    assert (
        delta(
            "agentgateway_gen_ai_client_token_usage_sum",
            gen_ai_token_type="input_cache_read",
        )
        == 6
    )
    assert math.isclose(
        delta("agentgateway_gen_ai_client_cost_usd_total"), 0.000034, rel_tol=1e-6
    )
    assert delta("agentgateway_gen_ai_server_time_to_first_token_count") > 0
    assert delta("agentgateway_gen_ai_server_time_per_output_token_count") > 0
    Path("var").mkdir(exist_ok=True)
    Path("var/metrics.txt").write_text(text)
    print(
        "PASS: HTTP success/errors, streaming, input/output/cache tokens, synthetic USD cost, TTFT and TPOT"
    )
    if args.datadog:
        print(
            "Traffic sent. Datadog UI/API verification is still required; --datadog does not assert ingestion."
        )
        return
    spans = []
    for _ in range(30):
        spans = []
        for encoded in json.loads(get("http://127.0.0.1:18080/captured-traces")):
            export = ExportTraceServiceRequest.FromString(base64.b64decode(encoded))
            for resource in export.resource_spans:
                if any(
                    s.trace_id.hex() == trace_id
                    for scope in resource.scope_spans
                    for s in scope.spans
                ):
                    service = {
                        a.key: a.value.string_value
                        for a in resource.resource.attributes
                    }
                    assert service["service.name"] == "agentgateway"
                for scope in resource.scope_spans:
                    spans.extend(scope.spans)
        spans = [s for s in spans if s.trace_id.hex() == trace_id]
        if len([s for s in spans if s.parent_span_id.hex() == parent_id]) >= len(cases):
            break
        time.sleep(1)
    assert len(spans) >= len(cases), (
        "OTLP traces did not arrive at the local capture sink"
    )
    roots = [s for s in spans if s.parent_span_id.hex() == parent_id]
    assert len(roots) == len(cases), (
        "Each request should retain the supplied W3C parent"
    )
    known_parents = {s.span_id.hex() for s in spans} | {parent_id}
    for span in roots:
        attributes = {a.key: a.value for a in span.attributes}
        failed = attributes["http.status"].int_value >= 400
        assert (span.status.code == 2) == failed, (
            "HTTP errors must be marked as OTLP errors"
        )
        if failed:
            assert attributes["error.type"].string_value == "http_error"
    for span in spans:
        attributes = {a.key: a.value for a in span.attributes}
        assert span.parent_span_id.hex() in known_parents
        if not args.capture_content:
            assert "gen_ai.input.messages" not in attributes
            assert "gen_ai.output.messages" not in attributes
            assert not any(
                "Synthetic test prompt" in str(v) or "Synthetic reply" in str(v)
                for v in attributes.values()
            )
    llm_spans = [{a.key: a.value for a in s.attributes} for s in spans]
    assert any(
        a.get("gen_ai.provider.name")
        and a["gen_ai.provider.name"].string_value == "openai"
        for a in llm_spans
    )
    if args.capture_content:
        messages = [
            json.loads(a["gen_ai.input.messages"].string_value)
            for a in llm_spans
            if "gen_ai.input.messages" in a
        ]
        outputs = [
            json.loads(a["gen_ai.output.messages"].string_value)
            for a in llm_spans
            if "gen_ai.output.messages" in a
        ]
        assert any(
            isinstance(m, list) and m[0]["content"] == "Synthetic test prompt."
            for m in messages
        )
        assert any(
            isinstance(m, list) and m[0]["content"] == "Synthetic reply."
            for m in outputs
        )
    assert any(
        a.get("gen_ai.usage.input_tokens")
        and a["gen_ai.usage.input_tokens"].int_value == 10
        for a in llm_spans
    )
    assert any(
        a.get("gen_ai.operation.name")
        and a["gen_ai.operation.name"].string_value == "chat"
        for a in llm_spans
    )
    print(
        "PASS: OTLP/HTTP protobuf export, GenAI attributes, HTTP error normalization, W3C parent propagation, and "
        + (
            "explicit synthetic content capture"
            if args.capture_content
            else "metadata-only privacy"
        )
    )


if __name__ == "__main__":
    main()
