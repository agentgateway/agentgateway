# Fault Injection

This example shows how to inject faults into traffic for chaos testing: aborts
(synthetic error responses) with the `directResponse` policy, and synthetic
latency with the `delay` policy.

## Abort (Synthetic Errors)

To fail requests without forwarding them to the backend, use a conditional
`directResponse`:

```yaml
policies:
  directResponse:
    conditional:
    - condition: random() < 0.1   # abort ~10% of requests
      status: 503
      body: "injected fault"
```

When the condition doesn't match, the request proceeds to the backend as
normal. Any CEL expression works as the condition, so aborts can target a
percentage of traffic (`random()`), specific headers, JWT claims, paths, or any
other request attribute.

## Delay (Synthetic Latency)

To add artificial latency before a request is forwarded, use the `delay`
policy:

```yaml
policies:
  delay:
    duration: 2s       # latency added before the request is forwarded
    probability: 0.1   # a fraction (0.0-1.0), a bool, or a CEL expression evaluating to either
```

`probability` defaults to always. Because it accepts CEL, faults can target a
subset of traffic, for example `request.headers["x-chaos"] == "1"` delays only
requests carrying a chaos-test header, and jwt claims, paths, or any other
request attribute work the same way. Like every policy, `delay` also composes
with the `conditional` wrapper for first-match-wins rules.

Injected delay counts against the route's `requestTimeout` (which measures the
full request from its start); if the delay would pass the deadline the request
fails with a 504 at the deadline rather than sleeping through it. This differs
from Istio/Envoy, where the route timeout only starts after the fault delay.
For a timeout that excludes the injected delay, use `backendRequestTimeout`,
which is measured per backend attempt.

## Running the example

Start an upstream HTTP server:

```bash
python3 -m http.server 8080
```

Start agentgateway:

```bash
cargo run -- -f examples/fault-injection/config.yaml
```

Send requests through the gateway:

```bash
curl http://localhost:3000/random   # ~10% of requests take 2s
curl http://localhost:3000/header -H 'x-chaos: 1'   # always delayed 500ms
curl http://localhost:3000/abort    # ~10% of requests return 503
```
