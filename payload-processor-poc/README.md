# PayloadProcessor POC

This directory contains a proof-of-concept implementation of the `PayloadProcessor` CRD
for the [wg-ai-gateway Payload Processing proposal](https://github.com/kubernetes-sigs/wg-ai-gateway/blob/main/proposals/7-payload-processing.md).
This POC is scoped to [inline processing](https://github.com/kubernetes-sigs/wg-ai-gateway/issues/53), but the CRD schema is designed to support both inline and external processing.

## What It Does

The POC demonstrates **body-based routing (BBR)** — reading a field from the JSON
request body and setting it as an HTTP header so that standard `HTTPRoute` header
matching can route to the correct backend.

```
Client                    Gateway                          Backends
  │                         │                                │
  │  POST /v1/chat/completions                               │
  │  body: {"model":"gpt-4"}                                 │
  │────────────────────────►│                                │
  │                         │                                │
  │                    PayloadProcessor (PreRouting)          │
  │                    json(request.body).model → "gpt-4"    │
  │                    Set header: X-Gateway-Model-Name      │
  │                         │                                │
  │                    HTTPRoute matches header               │
  │                    X-Gateway-Model-Name: gpt-4           │
  │                         │───────────────────────────────►│ gpt4-backend
  │                         │                                │
```

## CRD

### InProcess Example
```yaml
apiVersion: ainetworking.x-k8s.io/v0alpha0
kind: PayloadProcessor
metadata:
  name: model-header-setter
spec:
  targetRef:
    group: gateway.networking.k8s.io
    kind: Gateway # Allowed targets dependent on phase
    name: ai-gateway
  phase: PreRouting # Models agentgateway's traffic phases (PreRouting, PostRouting). TODO: Consider moving within processors to allow per-processor phase selection
  processors:
  - name: extract-model
    type: InProcess # (InProcess, ExtProc)
    failureMode: FailClosed # (FailClosed, FailOpen)
    inProcess:
      request:
        set:
        - name: X-Gateway-Model-Name # Header name
          value: 'json(request.body).model' # CEL expression
```

## How It Works

1. **Control Plane**: The `PayloadProcessor` CRD is watched by a new plugin
   (`payload_processor_plugin.go`) that translates each `InProcess` processor
   into a standard `TrafficPolicySpec_Transformation` with the correct phase.

2. **Data Plane**: The existing agentgateway Rust data plane receives the
   transformation policy via xDS and processes it identically to an
   `AgentgatewayPolicy` transformation — including automatic body buffering
   when CEL expressions reference `request.body`.

3. **No Rust changes**: The Go plugin emits standard transformation policies,
   so the existing data plane handles everything without modification.

## CRD Components
- **targetRef**: The `Gateway` to which the policy applies. The plugin will
  reject any other kind of target for the `PreRouting` phase.
- **phase**: The phase in which the transformation is applied. Only `PreRouting` is supported in this POC.
   The `PostRouting` phases applies payload processing after a route has been selected and the backend is known.
- **processors**: A list of processors to apply. Each processor has a `name`, `type`, and `failureMode`.
  - Type:
    - **InProcess**: The only supported type in this POC. The processor runs in the agentgateway data plane.
    - **ExtProc**: Not implemented in this POC. The processor would run in an external process.
  - **failureMode**: Determines what happens if the processor fails. `FailClosed` rejects the request, while `FailOpen` continues without applying the transformation.

## Files

| File | Purpose |
|------|---------|
| [plan-payloadProcessorPoc.prompt.md](plan-payloadProcessorPoc.prompt.md) | Implementation plan |
| [implementation-notes.md](implementation-notes.md) | Design decisions and deviations |
| [testdata/payload-processor-bbr.yaml](testdata/payload-processor-bbr.yaml) | Full working K8s example |

### CRD Types (new package)
| File | Purpose |
|------|---------|
| `controller/api/v0alpha0/ainetworking/doc.go` | Package declaration |
| `controller/api/v0alpha0/ainetworking/payload_processor_types.go` | CRD type definitions |
| `controller/api/v0alpha0/ainetworking/zz_generated.register.go` | Scheme registration |
| `controller/api/v0alpha0/ainetworking/zz_generated.deepcopy.go` | DeepCopy implementations |

### Controller Plugin (new file)
| File | Purpose |
|------|---------|
| `controller/pkg/agentgateway/plugins/payload_processor_plugin.go` | Translates CRD → internal policies |

### Modified Files (~10 lines total)
| File | Change |
|------|--------|
| `controller/pkg/agentgateway/plugins/collection.go` | Added `PayloadProcessors` collection |
| `controller/pkg/controller/start.go` | Registered plugin |

## POC Limitations

- **ExtProc not implemented**: The `type: ExtProc` field is defined in the CRD schema but not processed. Only `InProcess` works.
- **FailOpen not enforced**: The `failureMode` field is accepted but not passed to the data plane. Behavior is effectively fail-closed (CEL errors silently skip the header, parsing errors reject).
- **Per-processor timeout not enforced**: The `timeout` field is accepted but not passed through.
- **Processor ordering**: Multiple processors in one CRD become independent transformation policies; ordering is not guaranteed across them.
- **No status reporting**: The CRD `.status` is not populated with Accepted/Attached conditions.

## Testing

### Prerequisites
- A Kubernetes cluster with agentgateway installed.
```bash
# Create kind cluster
ctlptl create cluster kind --name kind-kind --registry=ctlptl-registry

# Build and load images
VERSION=1.0.0-ci1 CLUSTER_NAME=kind make -C controller kind-build-and-load

# Start tilt for live updates
tilt up
```
- The `PayloadProcessor` CRD registered with RBAC permissions for agentgateway
```bash
kubectl apply -f payload-processor-poc/install-crd/
```

### Deploy In Process POC
```bash
# Deploys simulated (llm-d's simulator) claude, gtp4, and default backends for testing
kubectl apply -f payload-processor-poc/testdata/simulator-backends.yaml

# Deploys Gateway, PayloadProcessor CR, and HTTPRoute for routing to backends based on model header with
# in process payload processing
# Note: This includes a Gateway resource which may not be configured to properly receive real time updates
# via tilt. To ensure the Gateway is using the real time updated image, use tilt-gw.
kubectl apply -f payload-processor-poc/testdata/payload-processor-bbr.yaml
```

### Deply Ext Process POC
```bash
# Deploys simulated (llm-d's simulator) claude, gtp4, and default backends for testing
kubectl apply -f payload-processor-poc/testdata/simulator-backends.yaml

# Deploy external payload processor for testing
kubectl apply -f payload-processor-poc/ext-proc-server/deploy.yaml

# Gateway, PayloadProcessor CR, and HTTPRoute for routing to backends based on
# model header with external payload processing
# Note: This includes a Gateway resource which may not be configured to properly receive real time updates
# via tilt. To ensure the Gateway is using the real time updated image, use tilt-gw.
kubectl apply -f payload-processor-poc/testdata/payload-processor-ext-proc.yaml
```

### Verify
```bash
# In process/ext process verification:
# Note: Commands assuming we are port forwarding the gateway to localhost:8080
# Should route to gpt4-backend
curl -X POST http://localhost:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "gpt-4", "messages": [{"role": "user", "content": "hello"}]}'

# Should route to claude-backend
curl -X POST http://localhost:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "claude", "messages": [{"role": "user", "content": "hello"}]}'

# Should route to default-backend (no model match)
curl -X POST http://localhost:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "llama", "messages": [{"role": "user", "content": "hello"}]}'
```
