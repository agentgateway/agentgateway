from __future__ import annotations

import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import yaml


BENCHMARKING = Path(__file__).resolve().parents[1]
RENDERER = BENCHMARKING / "scripts" / "render-scenario.py"
SPEC = importlib.util.spec_from_file_location("render_scenario", RENDERER)
assert SPEC and SPEC.loader
RENDER_SCENARIO = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RENDER_SCENARIO)


class RenderScenarioTest(unittest.TestCase):
    def test_flat_guide_keys_are_normalized_without_losing_values(self) -> None:
        scenario = {
            "decode": {"replicas": 1, "resources": {"requests": {"cpu": "4"}}},
            "modelservice": {"enabled": True, "decode": {"replicas": 2}},
        }
        RENDER_SCENARIO.normalize_scopes(scenario)
        decode = scenario["modelservice"]["decode"]
        self.assertEqual(decode["replicas"], 2)
        self.assertEqual(decode["resources"]["requests"]["cpu"], "4")
        self.assertNotIn("decode", scenario)

    def render(self, *extra: str) -> dict:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "scenario.yaml"
            command = [
                sys.executable,
                str(RENDERER),
                "--output",
                str(output),
                "--scenario-name",
                "test-scenario",
                "--workload",
                "test.yaml",
                "--accelerator-type",
                "gpu",
                "--accelerator-model",
                "h100",
                "--backend-type",
                "vllm",
                "--router-chart-version",
                "v0.9.0",
                "--model",
                "Qwen/Qwen3-32B",
                "--replicas",
                "2",
                "--tensor-parallelism",
                "2",
                *extra,
            ]
            subprocess.run(command, check=True, capture_output=True, text=True)
            return yaml.safe_load(output.read_text(encoding="utf-8"))["scenario"][0]

    def test_agentgateway_standalone_replaces_envoy_shape(self) -> None:
        scenario = self.render(
            "--treatment",
            "agentgateway-standalone",
            "--gateway-implementation",
            "agentgateway",
            "--router-mode",
            "standalone",
            "--gateway-image",
            "example.invalid/agentgateway:test",
            "--overlay",
            str(BENCHMARKING / "scenarios/gateways/agentgateway/standalone.yaml"),
        )
        method = scenario["modelservice"]
        proxy = method["router"]["proxy"]
        self.assertEqual(method["gateway"]["className"], "epponly")
        self.assertEqual(scenario["chartVersions"]["llmDRouter"], "v0.9.0")
        self.assertEqual(scenario["common"]["harness"]["name"], "inference-perf")
        self.assertEqual(proxy["proxyType"], "agentgateway")
        self.assertEqual(proxy["args"], ["-f", "/config/config.yaml"])
        self.assertEqual(
            proxy["presets"]["agentgateway"]["image"],
            "example.invalid/agentgateway:test",
        )
        self.assertEqual(
            proxy["agentgateway"]["service"]["name"],
            "test-scenario-model",
        )
        self.assertFalse(method["router"]["inferencePool"]["create"])

    def test_gateway_mode_does_not_retain_a_sidecar(self) -> None:
        scenario = self.render(
            "--treatment",
            "agentgateway-gateway",
            "--gateway-implementation",
            "agentgateway",
            "--router-mode",
            "gateway",
            "--overlay",
            str(BENCHMARKING / "scenarios/gateways/agentgateway/standalone.yaml"),
        )
        method = scenario["modelservice"]
        self.assertEqual(method["gateway"]["className"], "agentgateway")
        self.assertNotIn("proxy", method["router"])
        self.assertTrue(method["router"]["inferencePool"]["create"])

    def test_model_and_workload_storage_are_independent(self) -> None:
        scenario = self.render(
            "--treatment",
            "service",
            "--model-storage-class",
            "premium-rwx",
            "--model-storage-size",
            "2560Gi",
            "--workload-storage-class",
            "standard-rwx",
            "--workload-storage-size",
            "100Gi",
        )
        storage = scenario["common"]["storage"]
        self.assertEqual(storage["modelPvc"]["storageClassName"], "premium-rwx")
        self.assertEqual(storage["modelPvc"]["size"], "2560Gi")
        self.assertEqual(storage["workloadPvc"]["storageClassName"], "standard-rwx")
        self.assertEqual(storage["workloadPvc"]["size"], "100Gi")

    def test_upstream_routing_policy_ablations(self) -> None:
        expected = {
            "optimized-baseline": {
                "approx-prefix-cache-producer",
                "inflight-load-producer",
                "prefix-cache-affinity-filter",
                "token-load-scorer",
            },
            "cache-only": {
                "approx-prefix-cache-producer",
                "prefix-cache-affinity-filter",
            },
            "load-only": {"inflight-load-producer", "token-load-scorer"},
        }
        for policy, plugin_types in expected.items():
            with self.subTest(policy=policy):
                scenario = self.render(
                    "--treatment",
                    "agentgateway-gateway",
                    "--gateway-implementation",
                    "agentgateway",
                    "--router-mode",
                    "gateway",
                    "--routing-policy",
                    policy,
                    "--overlay",
                    str(BENCHMARKING / "scenarios/routing" / f"{policy}.yaml"),
                )
                epp = scenario["modelservice"]["router"]["epp"]
                self.assertEqual(len(epp["pluginsCustomConfig"]), 1)
                config = next(iter(epp["pluginsCustomConfig"].values()))
                plugin_document = yaml.safe_load(config)
                self.assertEqual(
                    {plugin["type"] for plugin in plugin_document["plugins"]},
                    plugin_types,
                )

    def test_gke_vllm_keeps_prefix_caching_enabled(self) -> None:
        scenario = self.render(
            "--treatment",
            "service",
            "--overlay",
            str(BENCHMARKING / "scenarios/backends/vllm.yaml"),
            "--overlay",
            str(BENCHMARKING / "scenarios/providers/gke-gpu.yaml"),
        )
        common = scenario["common"]
        command = scenario["modelservice"]["decode"]["vllm"]["customCommand"]
        self.assertEqual(common["accelerator"]["profile"], "nvidia")
        self.assertEqual(common["accelerator"]["resource"], "nvidia.com/gpu")
        self.assertNotIn("noPrefixCaching", common["vllmCommon"]["flags"])
        self.assertNotIn("--no-enable-prefix-caching", command)
        self.assertEqual(common["images"]["vllm"]["tag"], "v0.27.1")

    def test_upstream_backend_constraints_apply_to_baseline(self) -> None:
        scenario = self.render(
            "--treatment",
            "service",
            "--routing-policy",
            "optimized-baseline",
            "--overlay",
            str(BENCHMARKING / "scenarios/routing/upstream-common.yaml"),
        )
        self.assertEqual(scenario["common"]["model"]["maxModelLen"], 16000)
        self.assertEqual(scenario["common"]["harness"]["waitTimeout"], 5400)
        self.assertEqual(scenario["common"]["harness"]["resources"]["cpu"], "8")
        self.assertEqual(
            scenario["common"]["harness"]["resources"]["memory"], "80Gi"
        )


if __name__ == "__main__":
    unittest.main()
