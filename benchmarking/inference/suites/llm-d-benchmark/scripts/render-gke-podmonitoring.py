#!/usr/bin/env python3
"""Render GKE Managed Prometheus resources for one benchmark namespace."""

from __future__ import annotations

import argparse
from pathlib import Path

import yaml


def pod_monitoring(name: str, namespace: str, selector: dict, endpoints: list) -> dict:
    return {
        "apiVersion": "monitoring.googleapis.com/v1",
        "kind": "PodMonitoring",
        "metadata": {"name": name, "namespace": namespace},
        "spec": {
            "selector": selector,
            "endpoints": endpoints,
            "targetLabels": {
                "metadata": ["pod", "container"],
                "fromPod": [
                    {"from": "llm-d.ai/role", "to": "llm_d_role"},
                    {"from": "llm-d.ai/model", "to": "llm_d_model"},
                ],
            },
        },
    }


def endpoint(port: int | str, interval: str, auth_secret: str = "") -> dict:
    result: dict = {"port": port, "path": "/metrics", "interval": interval}
    if auth_secret:
        result["authorization"] = {
            "type": "Bearer",
            "credentials": {
                "secret": {"name": auth_secret, "key": "token"}
            },
        }
    return result


def documents(args: argparse.Namespace) -> list[dict]:
    result = [
        pod_monitoring(
            "benchmark-vllm",
            args.namespace,
            {"matchLabels": {"llm-d.ai/inferenceServing": "true"}},
            [endpoint(args.vllm_port, args.interval)],
        )
    ]
    if args.epp_enabled:
        result.append(
            pod_monitoring(
                "benchmark-epp",
                args.namespace,
                {
                    "matchExpressions": [
                        {"key": "inferencepool", "operator": "Exists"}
                    ]
                },
                [endpoint(args.epp_port, args.interval, args.epp_auth_secret)],
            )
        )
    if args.agentgateway_mode == "standalone":
        result.append(
            pod_monitoring(
                "benchmark-agentgateway",
                args.namespace,
                {
                    "matchExpressions": [
                        {"key": "inferencepool", "operator": "Exists"}
                    ]
                },
                [endpoint(args.agentgateway_port, args.interval)],
            )
        )
    elif args.agentgateway_mode == "gateway":
        result.append(
            pod_monitoring(
                "benchmark-agentgateway",
                args.namespace,
                {
                    "matchExpressions": [
                        {
                            "key": "gateway.networking.k8s.io/gateway-name",
                            "operator": "Exists",
                        }
                    ]
                },
                [endpoint(args.agentgateway_port, args.interval)],
            )
        )

    if args.epp_enabled and args.epp_auth_secret and args.collector_namespace:
        result.extend(
            [
                {
                    "apiVersion": "rbac.authorization.k8s.io/v1",
                    "kind": "Role",
                    "metadata": {
                        "name": "gmp-epp-metrics-secret-reader",
                        "namespace": args.namespace,
                    },
                    "rules": [
                        {
                            "apiGroups": [""],
                            "resources": ["secrets"],
                            "resourceNames": [args.epp_auth_secret],
                            "verbs": ["get", "list", "watch"],
                        }
                    ],
                },
                {
                    "apiVersion": "rbac.authorization.k8s.io/v1",
                    "kind": "RoleBinding",
                    "metadata": {
                        "name": "gmp-epp-metrics-secret-reader",
                        "namespace": args.namespace,
                    },
                    "subjects": [
                        {
                            "kind": "ServiceAccount",
                            "name": "collector",
                            "namespace": args.collector_namespace,
                        }
                    ],
                    "roleRef": {
                        "apiGroup": "rbac.authorization.k8s.io",
                        "kind": "Role",
                        "name": "gmp-epp-metrics-secret-reader",
                    },
                },
            ]
        )
    return result


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser()
    result.add_argument("--output", required=True, type=Path)
    result.add_argument("--namespace", required=True)
    result.add_argument("--interval", default="5s")
    result.add_argument("--vllm-port", default=8200, type=int)
    result.add_argument("--epp-port", default=9090, type=int)
    result.add_argument("--agentgateway-port", default=15020, type=int)
    result.add_argument("--epp-auth-secret", default="")
    result.add_argument("--epp-enabled", action="store_true")
    result.add_argument("--collector-namespace", default="")
    result.add_argument(
        "--agentgateway-mode", choices=("none", "standalone", "gateway"), default="none"
    )
    return result


def main() -> None:
    args = parser().parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w", encoding="utf-8") as stream:
        yaml.safe_dump_all(documents(args), stream, sort_keys=False)


if __name__ == "__main__":
    main()
