from __future__ import annotations

import importlib.util
import os
import subprocess
import tempfile
import time
import unittest
from pathlib import Path


BENCHMARKING = Path(__file__).resolve().parents[1]


def load_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


GKE = load_module(
    "render_gke_podmonitoring",
    BENCHMARKING / "scripts" / "render-gke-podmonitoring.py",
)
COLLECTOR = load_module(
    "render_runtime_collector",
    BENCHMARKING / "scripts" / "render-runtime-collector.py",
)


class Args:
    pass


class RuntimeMonitoringTest(unittest.TestCase):
    def test_gke_agentgateway_standalone_uses_google_resources_and_auth(self) -> None:
        args = Args()
        args.namespace = "benchmark"
        args.interval = "5s"
        args.vllm_port = 8200
        args.epp_port = 9090
        args.agentgateway_port = 15020
        args.epp_auth_secret = "metrics-token"
        args.epp_enabled = True
        args.collector_namespace = "gmp-system"
        args.agentgateway_mode = "standalone"
        documents = GKE.documents(args)

        monitors = [d for d in documents if d["kind"] == "PodMonitoring"]
        self.assertEqual(len(monitors), 3)
        self.assertTrue(
            all(d["apiVersion"] == "monitoring.googleapis.com/v1" for d in monitors)
        )
        by_name = {d["metadata"]["name"]: d for d in monitors}
        self.assertEqual(
            by_name["benchmark-epp"]["spec"]["endpoints"][0]["authorization"]
            ["credentials"]["secret"],
            {"name": "metrics-token", "key": "token"},
        )
        self.assertEqual(
            by_name["benchmark-agentgateway"]["spec"]["endpoints"][0]["port"],
            15020,
        )
        binding = next(d for d in documents if d["kind"] == "RoleBinding")
        self.assertEqual(binding["subjects"][0]["namespace"], "gmp-system")

    def test_gke_gateway_mode_selects_gateway_api_pods(self) -> None:
        args = Args()
        args.namespace = "benchmark"
        args.interval = "5s"
        args.vllm_port = 8200
        args.epp_port = 9090
        args.agentgateway_port = 15020
        args.epp_auth_secret = ""
        args.epp_enabled = True
        args.collector_namespace = ""
        args.agentgateway_mode = "gateway"
        documents = GKE.documents(args)
        gateway = next(
            d
            for d in documents
            if d.get("metadata", {}).get("name") == "benchmark-agentgateway"
        )
        expression = gateway["spec"]["selector"]["matchExpressions"][0]
        self.assertEqual(
            expression["key"], "gateway.networking.k8s.io/gateway-name"
        )

    def test_runtime_collector_is_low_resource_and_tracks_proxy_metrics(self) -> None:
        args = Args()
        args.name = "runtime-metrics"
        args.configmap = "runtime-scripts"
        args.namespace = "benchmark"
        args.service_account = "runner"
        args.image = "example.invalid/benchmark:v1"
        args.interval = 5
        args.vllm_port = 8200
        args.epp_auth_secret = "metrics-token"
        args.nodepool = "bench-cpu"
        resources = COLLECTOR.resources(args)
        pod = next(d for d in resources if d["kind"] == "Pod")
        container = pod["spec"]["containers"][0]
        env = {item["name"]: item["value"] for item in container["env"]}
        self.assertEqual(pod["spec"]["nodeSelector"], {
            "cloud.google.com/gke-nodepool": "bench-cpu"
        })
        self.assertEqual(container["resources"]["requests"]["cpu"], "100m")
        self.assertIn("agentgateway_request_processing_seconds_sum", env[
            "LLMDBENCH_TIME_SERIES_METRICS"
        ])
        self.assertIn("llm_d_epp_scheduler_attempts_total", env[
            "LLMDBENCH_TIME_SERIES_METRICS"
        ])

    def test_runtime_entrypoint_adds_agentgateway_to_upstream_results(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binaries = root / "bin"
            binaries.mkdir()
            results = root / "results"
            control = root / "control"
            control.mkdir()

            (binaries / "kubectl").write_text(
                """#!/usr/bin/env bash
if [[ " $* " == *" get pods "* ]]; then
  echo '{"items":[{"metadata":{"name":"agentgateway-pod","labels":{}},"spec":{"containers":[{"name":"proxy","image":"example/agentgateway:test"}]},"status":{"podIP":"10.0.0.2"}}]}'
elif [[ " $* " == *" top pods "* ]]; then
  echo 'agentgateway-pod proxy 10m 20Mi'
elif [[ " $* " == *" logs "* ]]; then
  echo 'component log'
fi
""",
                encoding="utf-8",
            )
            (binaries / "curl").write_text(
                """#!/usr/bin/env bash
echo 'agentgateway_request_processing_seconds_count 1'
""",
                encoding="utf-8",
            )
            upstream = root / "collect_metrics.sh"
            upstream.write_text(
                """#!/usr/bin/env bash
if [[ "$1" == start ]]; then
  while true; do sleep 1; done
else
  mkdir -p "${LLMDBENCH_RUN_EXPERIMENT_RESULTS_DIR}/metrics/processed"
  echo '{}' > "${LLMDBENCH_RUN_EXPERIMENT_RESULTS_DIR}/metrics/processed/metrics_summary.json"
fi
""",
                encoding="utf-8",
            )
            for executable in (binaries / "kubectl", binaries / "curl", upstream):
                executable.chmod(0o755)

            environment = os.environ.copy()
            environment.update(
                {
                    "PATH": f"{binaries}:{environment['PATH']}",
                    "LLMDBENCH_RUN_EXPERIMENT_RESULTS_DIR": str(results),
                    "LLMDBENCH_VLLM_COMMON_NAMESPACE": "benchmark",
                    "METRICS_COLLECTION_INTERVAL": "1",
                    "RUNTIME_METRICS_STOP_FILE": str(control / "stop"),
                    "RUNTIME_METRICS_COMPLETE_FILE": str(control / "complete"),
                    "RUNTIME_METRICS_COPIED_FILE": str(control / "copied"),
                    "UPSTREAM_COLLECTOR": str(upstream),
                }
            )
            process = subprocess.Popen(
                ["bash", str(BENCHMARKING / "scripts" / "runtime-metrics-collector.sh")],
                env=environment,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            time.sleep(0.3)
            (control / "stop").touch()
            for _ in range(100):
                if (control / "complete").exists():
                    break
                time.sleep(0.05)
            self.assertTrue((control / "complete").exists())
            self.assertIsNone(process.poll())
            (control / "copied").touch()
            stdout, stderr = process.communicate(timeout=10)
            self.assertEqual(process.returncode, 0, f"{stdout}\n{stderr}")
            raw = list((results / "metrics" / "raw").glob("*_agentgateway_metrics.log"))
            self.assertTrue(raw)
            self.assertIn(
                "agentgateway_request_processing_seconds_count 1",
                raw[0].read_text(encoding="utf-8"),
            )
            self.assertTrue(
                (results / "metrics" / "processed" / "metrics_summary.json").is_file()
            )


if __name__ == "__main__":
    unittest.main()
