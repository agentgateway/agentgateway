# ExtProc Test Server

This is an example external processing service used to test ExtProc functionality in KGateway with agentgateway as the data-plane.

The service is based on the basic-sink example from https://github.com/solo-io/ext-proc-examples

## What It Does

This external processing service implements Envoy's External Processing filter interface. It takes instructions in a request or response header named `instructions` and can:
- Add headers to requests/responses
- Remove headers from requests/responses
- Set body content
- Set trailers

The `instructions` header must be a JSON string in this format:
```json
{
  "addHeaders": {
    "header1": "value1",
    "header2": "value2"
  },
  "removeHeaders": ["header3", "header4"],
  "setBody": "this is the new body",
  "setTrailers": {
    "trailer1": "value1",
    "trailer2": "value2"
  }
}
```

All fields are optional.

## Building and Publishing

The extproc server image is built and loaded into the kind cluster automatically when running

```bash
make e2e-test
```

To build/load only the extproc server, run

```bash
make extproc-server-docker kind-load-extproc-server
```

This will produce a container image `ghcr.io/kgateway-dev/extproc-server:0.0.1`