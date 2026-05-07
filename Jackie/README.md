# PayloadProcessor POC

This directory contains a proof-of-concept implementation of the `PayloadProcessor` CRD
for the [wg-ai-gateway Payload Processing proposal](https://github.com/kubernetes-sigs/wg-ai-gateway/blob/main/proposals/7-payload-processing.md).

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

```yaml
apiVersion: ainetworking.x-k8s.io/v0alpha0
kind: PayloadProcessor
metadata:
  name: model-header-setter
spec:
  targetRef:
    group: gateway.networking.k8s.io
    kind: Gateway
    name: ai-gateway
  phase: PreRouting
  processors:
  - name: extract-model
    type: InProcess
    failureMode: FailClosed
    inProcess:
      request:
        set:
        - name: X-Gateway-Model-Name
          value: 'json(request.body).model'
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
- A Kubernetes cluster with agentgateway installed
- The `PayloadProcessor` CRD registered

### Deploy
```bash
kubectl apply -f Jackie/testdata/payload-processor-bbr.yaml
```

### Verify
```bash
# Should route to gpt4-backend
curl -X POST http://gateway:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "gpt-4", "messages": [{"role": "user", "content": "hello"}]}'

# Should route to claude-backend
curl -X POST http://gateway:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "claude", "messages": [{"role": "user", "content": "hello"}]}'

# Should route to default-backend (no model match)
curl -X POST http://gateway:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "llama", "messages": [{"role": "user", "content": "hello"}]}'
```
