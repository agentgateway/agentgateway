## Standalone EPP Example

This example shows the v1 static config shape for running `agentgateway` as the sidecar proxy next
to a [standalone EPP](https://gateway-api-inference-extension.sigs.k8s.io/guides/standalone/) deployment
on Kubernetes.

### Config shape

```yaml
binds:
- port: 8081
  listeners:
  - routes:
    - backends:
      - service:
          name: default/my-model
          port: 8000
        policies:
          inferenceRouting:
            endpointPicker:
              host: 127.0.0.1:9002
```

### What it does

* `agentgateway` listens on port `8081`.
* The route forwards requests to the Kubernetes `Service` `default/my-model:8000`.
* Before choosing an upstream endpoint, `agentgateway` calls the local EPP over
  ext-proc at `127.0.0.1:9002`.
* EPP returns the selected backend `ip:port`, and `agentgateway` uses that
  endpoint for the request.

### Current v1 constraints

* `inferenceRouting` is only supported on `service` route backends.
* Standalone local config is fail-closed for now. If EPP is unavailable, the
  request fails instead of falling back to direct service endpoint balancing.
* This example is meant to be mounted into the `agentgateway` sidecar in a
  Kubernetes deployment. It is a reference config, not a standalone local demo.
