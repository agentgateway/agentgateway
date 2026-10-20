## Active Health Checking Example

This example probes two self-hosted model replicas on a timer, so a replica that is down takes
none of the traffic and rejoins on its own once it answers again.

Passive health only learns that a provider is down by failing a real request, and that failure is
paid again every eviction window, by a user request that waits out the connect timeout. An active
check moves the cost off the request path: after `unhealthyThreshold` failed probes in a row the
provider is evicted, it stays evicted for as long as the probes keep failing, and it comes back
after `healthyThreshold` probes pass.

```yaml
health:
  eviction:
    duration: 30s
    restoreHealth: 1.0
  active:
    path: /health
    interval: 10s
    timeout: 3s
    unhealthyThreshold: 3
    healthyThreshold: 1
```

The defaults are `/health`, `10s`, `3s`, 3, 1, and any 2xx status; `expectedStatuses` takes a
list when an engine answers with something else. The block is accepted wherever `health` is,
which is on models, provider `defaults`, AI provider policies, and route backend policies.

In local configuration every model is a single-provider backend, so the check earns its keep
together with a `virtualModels` failover entry: clients ask for `qwen3`, and the request goes to
whichever replica is currently healthy.

### Running the example

Point the two replicas at OpenAI-compatible endpoints that serve `/health`, and start
agentgateway:

```bash
cargo run -- -f examples/llm-active-health/config.yaml
```

Stop one replica: within three intervals it is evicted and every request goes to the other one.
Start it again and it takes traffic on the next passing probe.

### When to use it

A readiness probe on a Kubernetes pod already removes it from its Service, so plain Service
backends are not probed at all. What this covers is a provider the gateway reaches by address
rather than through a Service: a `baseUrl` or `hostOverride` in local configuration, an endpoint
outside the cluster, or a pre-emptible replica whose GPUs are taken for other work for days.
