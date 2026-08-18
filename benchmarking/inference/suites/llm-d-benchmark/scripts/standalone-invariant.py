#!/usr/bin/env python3
"""Produce the proxy-independent configuration used to compare sidecars."""

from __future__ import annotations

import argparse
import copy
import hashlib
from pathlib import Path
from typing import Any

import yaml


def normalize(document: dict[str, Any]) -> dict[str, Any]:
    result = copy.deepcopy(document)
    scenarios = result.get("scenario", [])
    if len(scenarios) != 1:
        raise ValueError("rendered scenario must contain exactly one entry")

    scenario = scenarios[0]
    scenario["name"] = "standalone-proxy-comparison"
    method = scenario.get("modelservice", {})
    if method.get("gateway", {}).get("className") != "epponly":
        raise ValueError("standalone comparison requires gateway.className=epponly")

    router = method.get("router", {})
    proxy = router.get("proxy", {})
    # Proxy resources and deployment shape are comparison invariants. Image,
    # command, ports and implementation-specific presets are intentionally
    # omitted because they are intrinsic dataplane differences.
    router["proxy"] = {
        key: copy.deepcopy(proxy[key])
        for key in ("enabled", "deploymentMode", "resources")
        if key in proxy
    }

    # agentgateway's standalone ext-proc connection is intentionally insecure
    # over localhost. Envoy's chart path may omit this implementation detail.
    epp = router.get("epp", {})
    flags = epp.get("flags", {})
    flags.pop("secure-serving", None)
    if not flags:
        epp.pop("flags", None)

    # Both implementations disable chart-created InferencePool resources, but
    # retain all non-lifecycle settings should a guide define any.
    inference_pool = router.get("inferencePool", {})
    inference_pool.pop("create", None)
    if not inference_pool:
        router.pop("inferencePool", None)

    return result


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    with args.input.open(encoding="utf-8") as stream:
        normalized = normalize(yaml.safe_load(stream) or {})
    payload = yaml.safe_dump(normalized, sort_keys=True)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(payload, encoding="utf-8")
    print(hashlib.sha256(payload.encode("utf-8")).hexdigest())


if __name__ == "__main__":
    main()
