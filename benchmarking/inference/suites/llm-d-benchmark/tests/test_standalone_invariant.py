from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "standalone-invariant.py"
SPEC = importlib.util.spec_from_file_location("standalone_invariant", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class StandaloneInvariantTest(unittest.TestCase):
    def scenario(self, implementation: str, cpu: str = "4") -> dict:
        proxy = {
            "proxyType": implementation,
            "args": [implementation],
            "resources": {"requests": {"cpu": cpu, "memory": "8Gi"}},
        }
        if implementation == "agentgateway":
            proxy["presets"] = {"agentgateway": {"image": "example/ag:test"}}
        return {
            "scenario": [
                {
                    "name": implementation,
                    "chartVersions": {"llmDRouter": "v0.9.0"},
                    "modelservice": {
                        "gateway": {"className": "epponly"},
                        "router": {
                            "epp": {
                                "resources": {"requests": {"cpu": "4"}},
                                "flags": {"v": 2, "secure-serving": False},
                            },
                            "proxy": proxy,
                            "inferencePool": {"create": False},
                        },
                    },
                }
            ]
        }

    def test_implementation_details_are_ignored(self) -> None:
        envoy = MODULE.normalize(self.scenario("envoy"))
        agentgateway = MODULE.normalize(self.scenario("agentgateway"))
        self.assertEqual(envoy, agentgateway)

    def test_proxy_resources_remain_an_invariant(self) -> None:
        envoy = MODULE.normalize(self.scenario("envoy"))
        agentgateway = MODULE.normalize(self.scenario("agentgateway", cpu="1"))
        self.assertNotEqual(envoy, agentgateway)


if __name__ == "__main__":
    unittest.main()
