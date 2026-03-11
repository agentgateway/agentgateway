# testbox

Single e2e helper image used by multiple tests.

Ports:
- `80/443/7070/9090`: Istio echo app server (`backend` service)
- `8443`: dummy-idp/auth0-mock
- `18080`: ext-proc gRPC server
- `9000`: ext-authz gRPC server
- `8000`: mcp-website-fetcher
- `3001`: mcp-admin-server
- `9999`: a2a-helloworld

Build/load:
```bash
make -C controller testbox-docker kind-load-testbox
```
