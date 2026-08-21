#!/usr/bin/env python3
"""Render one llm-d-benchmark scenario for the selected benchmark treatment."""

from __future__ import annotations

import argparse
import copy
import hashlib
import re
from pathlib import Path
from typing import Any

import yaml


COMMON_KEYS = {
    "accelerator",
    "affinity",
    "control",
    "fma",
    "harness",
    "images",
    "model",
    "monitoring",
    "storage",
    "vllmCommon",
}
METHOD_KEYS = {"decode", "gateway", "prefill", "router"}


def merge(left: dict[str, Any], right: dict[str, Any]) -> dict[str, Any]:
    result = copy.deepcopy(left)
    for key, value in right.items():
        if isinstance(value, dict) and isinstance(result.get(key), dict):
            result[key] = merge(result[key], value)
        else:
            result[key] = copy.deepcopy(value)
    return result


def load_entry(path: Path, requested_name: str | None = None) -> dict[str, Any]:
    with path.open(encoding="utf-8") as stream:
        document = yaml.safe_load(stream) or {}
    entries = document.get("scenario", [])
    if requested_name:
        matches = [entry for entry in entries if entry.get("name") == requested_name]
        if len(matches) == 1:
            return copy.deepcopy(matches[0])
    if len(entries) != 1:
        raise ValueError(
            f"{path} must contain exactly one scenario entry or an entry named "
            f"{requested_name!r}"
        )
    return copy.deepcopy(entries[0])


def apply_overlay(scenario: dict[str, Any], overlay: dict[str, Any]) -> None:
    common = scenario.setdefault("common", {})
    method = scenario.setdefault("modelservice", {})
    method["enabled"] = True

    for key, value in overlay.items():
        if key == "name":
            continue
        if key == "common":
            scenario["common"] = merge(common, value)
            common = scenario["common"]
        elif key == "modelservice":
            # Flat wrapper overlays use modelservice for method-level settings
            # such as uriProtocol. Scoped upstream guides use the same key for
            # the complete deployment method.
            scenario["modelservice"] = merge(method, value)
            method = scenario["modelservice"]
        elif key in COMMON_KEYS:
            common[key] = (
                merge(common.get(key, {}), value)
                if isinstance(value, dict)
                else copy.deepcopy(value)
            )
        elif key in METHOD_KEYS:
            method[key] = (
                merge(method.get(key, {}), value)
                if isinstance(value, dict)
                else copy.deepcopy(value)
            )
        else:
            # Preserve guide-specific top-level sections instead of guessing
            # which deployment scope owns them.
            scenario[key] = (
                merge(scenario.get(key, {}), value)
                if isinstance(value, dict)
                else copy.deepcopy(value)
            )


def normalize_scopes(scenario: dict[str, Any]) -> None:
    """Move flat stack keys into the documented scoped representation."""
    common = scenario.setdefault("common", {})
    method = scenario.setdefault("modelservice", {})
    method["enabled"] = True
    for key in COMMON_KEYS:
        if key not in scenario:
            continue
        value = scenario.pop(key)
        common[key] = (
            merge(value, common.get(key, {}))
            if isinstance(value, dict)
            else copy.deepcopy(common.get(key, value))
        )
    for key in METHOD_KEYS:
        if key not in scenario:
            continue
        value = scenario.pop(key)
        method[key] = (
            merge(value, method.get(key, {}))
            if isinstance(value, dict)
            else copy.deepcopy(method.get(key, value))
        )


def slug(value: str) -> str:
    normalized = re.sub(r"[^a-z0-9-]+", "-", value.lower()).strip("-")
    normalized = re.sub(r"-+", "-", normalized)
    if len(normalized) <= 63:
        return normalized
    digest = hashlib.sha256(normalized.encode()).hexdigest()[:8]
    return f"{normalized[:54].rstrip('-')}-{digest}"


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser()
    result.add_argument("--output", required=True, type=Path)
    result.add_argument("--scenario-name", required=True)
    result.add_argument("--guide", type=Path)
    result.add_argument("--guide-entry")
    result.add_argument("--overlay", action="append", default=[], type=Path)
    result.add_argument(
        "--treatment",
        choices=(
            "service",
            "agentgateway-standalone",
            "agentgateway-gateway",
            "envoy-standalone",
        ),
        required=True,
    )
    result.add_argument("--gateway-implementation", default="agentgateway")
    result.add_argument("--router-mode", choices=("standalone", "gateway"), default="standalone")
    result.add_argument(
        "--routing-policy",
        choices=("default", "optimized-baseline", "cache-only", "load-only"),
        default="default",
    )
    result.add_argument("--gateway-image", default="")
    result.add_argument("--agentgateway-version", default="")
    result.add_argument("--router-chart-version", required=True)
    result.add_argument("--workload", required=True)
    result.add_argument("--accelerator-type", required=True)
    result.add_argument("--accelerator-model", required=True)
    result.add_argument("--backend-type", required=True)
    result.add_argument("--model", default="")
    result.add_argument("--replicas", type=int)
    result.add_argument("--tensor-parallelism", type=int)
    result.add_argument("--gke-accelerator", default="")
    result.add_argument("--model-storage-class", default="")
    result.add_argument("--model-storage-size", default="")
    result.add_argument("--workload-storage-class", default="")
    result.add_argument("--workload-storage-size", default="")
    result.add_argument("--cpu-nodepool", default="")
    result.add_argument("--harness-nodepool", default="")
    return result


def main() -> None:
    args = parser().parse_args()
    if args.guide:
        scenario = load_entry(args.guide, args.guide_entry)
        if "modelservice" not in scenario:
            raise ValueError(
                f"{args.guide} does not define a modelservice deployment path"
            )
    else:
        scenario = {"common": {}, "modelservice": {"enabled": True}}

    normalize_scopes(scenario)

    # Guide scenarios can describe several deployment methods. The wrapper
    # deliberately benchmarks the modelservice path for both standalone EPP
    # and Gateway API routing so the backend topology stays comparable.
    scenario.setdefault("modelservice", {})["enabled"] = True
    if isinstance(scenario.get("kustomize"), dict):
        scenario["kustomize"]["enabled"] = False
    if isinstance(scenario.get("standalone"), dict):
        scenario["standalone"]["enabled"] = False

    for overlay_path in args.overlay:
        apply_overlay(scenario, load_entry(overlay_path))

    common = scenario.setdefault("common", {})
    method = scenario.setdefault("modelservice", {})
    method["enabled"] = True
    harness = common.setdefault("harness", {})
    harness["name"] = "inference-perf"
    harness["experimentProfile"] = args.workload
    # Both standalone proxy treatments must use the same immutable router
    # chart. Keeping the pin in the rendered scenario makes the comparison
    # reproducible and prevents an implementation overlay from changing it.
    scenario.setdefault("chartVersions", {})["llmDRouter"] = (
        args.router_chart_version
    )
    if args.gateway_implementation == "agentgateway" and args.agentgateway_version:
        # This drives the provider Helmfile. Without an explicit scenario pin,
        # standup can replace a newer cluster controller with its older default.
        scenario["chartVersions"]["agentgateway"] = args.agentgateway_version

    if args.model:
        model = common.setdefault("model", {})
        model.update(
            {
                "name": args.model,
                "shortName": slug(args.model),
                "path": f"models/{args.model}",
                "huggingfaceId": args.model,
            }
        )
    decode = method.setdefault("decode", {})
    if args.replicas is not None:
        decode["replicas"] = args.replicas
    if args.tensor_parallelism is not None:
        decode.setdefault("parallelism", {})["tensor"] = args.tensor_parallelism

    storage = common.setdefault("storage", {})
    model_pvc = storage.setdefault("modelPvc", {})
    workload_pvc = storage.setdefault("workloadPvc", {})
    if args.model_storage_class:
        model_pvc["storageClassName"] = args.model_storage_class
    if args.model_storage_size:
        model_pvc["size"] = args.model_storage_size
    if args.workload_storage_class:
        workload_pvc["storageClassName"] = args.workload_storage_class
    if args.workload_storage_size:
        workload_pvc["size"] = args.workload_storage_size

    if args.cpu_nodepool and args.accelerator_type == "cpu":
        common["affinity"] = {
            "enabled": True,
            "nodeSelector": {"cloud.google.com/gke-nodepool": args.cpu_nodepool},
        }
    if args.harness_nodepool:
        common.setdefault("harness", {})["nodeSelector"] = {
            "cloud.google.com/gke-nodepool": args.harness_nodepool
        }
    if args.accelerator_type == "gpu" and args.gke_accelerator:
        affinity = common.setdefault("affinity", {})
        affinity["enabled"] = True
        affinity.setdefault("nodeSelector", {})[
            "cloud.google.com/gke-accelerator"
        ] = args.gke_accelerator
        decode["acceleratorType"] = {
            "labelKey": "cloud.google.com/gke-accelerator",
            "labelValue": args.gke_accelerator,
        }

    prefill = method.get("prefill", {})
    if args.treatment == "service" and (
        prefill.get("enabled") or int(prefill.get("replicas", 0) or 0) > 0
    ):
        raise ValueError(
            "the service treatment does not support a guide with prefill enabled; "
            "select a decode-only guide or a routed treatment"
        )

    router = method.setdefault("router", {})
    if args.treatment == "service":
        method["gateway"] = {"className": "none"}
        router.pop("proxy", None)
        router.pop("extraServicePorts", None)
    elif args.router_mode == "standalone":
        method["gateway"] = {"className": "epponly"}
        proxy = router.setdefault("proxy", {})
        proxy["proxyType"] = args.gateway_implementation
        if args.gateway_implementation == "agentgateway":
            proxy["args"] = ["-f", "/config/config.yaml"]
            proxy["httpTargetPort"] = "http"
            proxy["agentgateway"] = {
                "service": {
                    "create": True,
                    "name": slug(f"{args.scenario_name}-model"),
                }
            }
            preset = proxy.setdefault("presets", {}).setdefault("agentgateway", {})
            if args.gateway_image:
                preset["image"] = args.gateway_image
            preset.setdefault("pullPolicy", "IfNotPresent")
            router.setdefault("epp", {}).setdefault("flags", {})[
                "secure-serving"
            ] = False
        router.pop("extraServicePorts", None)
        router.setdefault("inferencePool", {})["create"] = False
    else:
        method["gateway"] = {"className": args.gateway_implementation}
        # In Gateway API mode the selected Gateway implementation is the data
        # plane. Do not accidentally retain an Envoy/agentgateway EPP sidecar
        # from the source guide.
        router.pop("proxy", None)
        router.setdefault("inferencePool", {})["create"] = True

    if args.routing_policy != "default" and args.treatment != "service":
        # Guide scenarios may carry other embedded plugin documents. Preserve
        # only the policy selected by this treatment so evidence bundles do
        # not imply that an unselected config participated in the run.
        epp = router.setdefault("epp", {})
        selected = epp.get("pluginsConfigFile")
        configs = epp.get("pluginsCustomConfig", {})
        if not selected or selected not in configs:
            raise ValueError(
                f"routing policy {args.routing_policy!r} did not provide its "
                "selected pluginsConfigFile"
            )
        epp["pluginsCustomConfig"] = {selected: configs[selected]}

    scenario["name"] = slug(args.scenario_name)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w", encoding="utf-8") as stream:
        yaml.safe_dump({"scenario": [scenario]}, stream, sort_keys=False)


if __name__ == "__main__":
    main()
