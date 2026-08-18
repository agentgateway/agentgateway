#!/usr/bin/env python3
"""Render the wrapper-owned runtime metrics collector Kubernetes resources."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

import yaml


HISTOGRAM_METRICS = [
    "llm_d_epp_scheduler_e2e_duration_seconds",
    "llm_d_epp_plugin_duration_seconds",
    "llm_d_epp_prefix_indexer_hit_ratio",
    "llm_d_epp_flow_control_request_queue_duration_seconds",
    "agentgateway_request_processing_seconds",
    "agentgateway_response_processing_seconds",
    "agentgateway_upstream_connect_duration_seconds",
    "agentgateway_upstream_call_duration_seconds",
    "agentgateway_request_duration_seconds",
]

TIME_SERIES_METRICS = [
    "vllm:kv_cache_usage_perc",
    "vllm:num_requests_running",
    "vllm:num_requests_waiting",
    "vllm:prefix_cache_hits_total",
    "vllm:prefix_cache_queries_total",
    "vllm:num_preemptions_total",
    "vllm:request_success_total",
    "llm_d_epp_scheduler_attempts_total",
    "llm_d_epp_average_kv_cache_utilization",
    "llm_d_epp_average_queue_size",
    "llm_d_epp_average_running_requests",
    "llm_d_epp_ready_endpoints",
    "llm_d_epp_datalayer_poll_errors_total",
    "llm_d_epp_datalayer_extract_errors_total",
    "llm_d_epp_flow_control_queue_size",
    "llm_d_epp_flow_control_pool_saturation",
    "agentgateway_requests_total",
    "agentgateway_downstream_connections_total",
    "agentgateway_tokio_global_queue_depth",
    "agentgateway_tokio_num_alive_tasks",
    "agentgateway_tokio_num_workers",
] + [
    f"{name}_{suffix}"
    for name in HISTOGRAM_METRICS
    for suffix in ("bucket", "sum", "count")
]


def resources(args: argparse.Namespace) -> list[dict]:
    labels = {"app": args.name, "app.kubernetes.io/part-of": "llm-d-benchmark"}
    role = {
        "apiVersion": "rbac.authorization.k8s.io/v1",
        "kind": "Role",
        "metadata": {"name": args.name, "namespace": args.namespace},
        "rules": [
            {
                "apiGroups": [""],
                "resources": ["pods", "pods/log"],
                "verbs": ["get", "list", "watch"],
            },
            {
                "apiGroups": [""],
                "resources": ["secrets"],
                "resourceNames": [args.epp_auth_secret],
                "verbs": ["get"],
            },
            {
                "apiGroups": ["apps"],
                "resources": ["deployments", "statefulsets"],
                "verbs": ["get", "list", "watch"],
            },
            {
                "apiGroups": ["metrics.k8s.io"],
                "resources": ["pods"],
                "verbs": ["get", "list"],
            },
        ],
    }
    binding = {
        "apiVersion": "rbac.authorization.k8s.io/v1",
        "kind": "RoleBinding",
        "metadata": {"name": args.name, "namespace": args.namespace},
        "subjects": [
            {
                "kind": "ServiceAccount",
                "name": args.service_account,
                "namespace": args.namespace,
            }
        ],
        "roleRef": {
            "apiGroup": "rbac.authorization.k8s.io",
            "kind": "Role",
            "name": args.name,
        },
    }
    pod_spec = {
        "serviceAccountName": args.service_account,
        "restartPolicy": "Never",
        "terminationGracePeriodSeconds": 10,
        "containers": [
            {
                "name": "collector",
                "image": args.image,
                "imagePullPolicy": "IfNotPresent",
                "command": ["/scripts/runtime-metrics-collector.sh"],
                "env": [
                    {
                        "name": "LLMDBENCH_RUN_EXPERIMENT_RESULTS_DIR",
                        "value": "/runtime-metrics",
                    },
                    {
                        "name": "LLMDBENCH_VLLM_COMMON_NAMESPACE",
                        "value": args.namespace,
                    },
                    {
                        "name": "LLMDBENCH_VLLM_COMMON_METRICS_PORT",
                        "value": str(args.vllm_port),
                    },
                    {
                        "name": "LLMDBENCH_VLLM_COMMON_INFERENCE_PORT",
                        "value": "8000",
                    },
                    {
                        "name": "LLMDBENCH_EPP_METRICS_SECRET",
                        "value": args.epp_auth_secret,
                    },
                    {"name": "METRICS_COLLECTION_INTERVAL", "value": str(args.interval)},
                    {
                        "name": "LLMDBENCH_TIME_SERIES_METRICS",
                        "value": json.dumps(TIME_SERIES_METRICS),
                    },
                ],
                "resources": {
                    "requests": {"cpu": "100m", "memory": "128Mi"},
                    "limits": {"cpu": "1", "memory": "1Gi"},
                },
                "securityContext": {
                    "allowPrivilegeEscalation": False,
                    "capabilities": {"drop": ["ALL"]},
                },
                "volumeMounts": [
                    {"name": "scripts", "mountPath": "/scripts", "readOnly": True},
                    {"name": "results", "mountPath": "/runtime-metrics"},
                    {"name": "control", "mountPath": "/control"},
                ],
            }
        ],
        "volumes": [
            {
                "name": "scripts",
                "configMap": {"name": args.configmap, "defaultMode": 0o555},
            },
            {"name": "results", "emptyDir": {}},
            {"name": "control", "emptyDir": {}},
        ],
    }
    if args.nodepool:
        pod_spec["nodeSelector"] = {
            "cloud.google.com/gke-nodepool": args.nodepool
        }
    pod = {
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {"name": args.name, "namespace": args.namespace, "labels": labels},
        "spec": pod_spec,
    }
    return [role, binding, pod]


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser()
    result.add_argument("--output", required=True, type=Path)
    result.add_argument("--namespace", required=True)
    result.add_argument("--service-account", required=True)
    result.add_argument("--image", required=True)
    result.add_argument("--name", default="agentgateway-runtime-metrics")
    result.add_argument("--configmap", default="agentgateway-runtime-metrics-scripts")
    result.add_argument("--interval", default=5, type=int)
    result.add_argument("--vllm-port", default=8200, type=int)
    result.add_argument(
        "--epp-auth-secret", default="inference-gateway-sa-metrics-reader-secret"
    )
    result.add_argument("--nodepool", default="")
    return result


def main() -> None:
    args = parser().parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w", encoding="utf-8") as stream:
        yaml.safe_dump_all(resources(args), stream, sort_keys=False)


if __name__ == "__main__":
    main()
