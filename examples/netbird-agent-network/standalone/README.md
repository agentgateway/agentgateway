# Standalone NetBird Agent Network with agentgateway

This example runs NetBird and agentgateway on one Docker host and routes OpenAI
and Anthropic requests without requiring Kubernetes, Gateway API, or
cert-manager.

The stack uses two agentgateway instances for separate trust boundaries:

```text
NetBird client
    |
    | management HTTPS
    v
public management agentgateway
    |-- NetBird HTTP, WebSocket, and gRPC --> private NetBird server
    `-- all other paths -------------------> private dashboard
```

The management agentgateway is the only public TCP listener. It terminates TLS
and routes protocol-aware management traffic to the combined NetBird server.

```text
Authorized NetBird client
    |
    | HTTPS over the encrypted NetBird tunnel
    v
NetBird Agent Network proxy
    | trusted NetBird identity + virtual API key
    v
private AI agentgateway
    |-- gpt-4o-mini, text-embedding-3-small --> OpenAI
    `-- claude-haiku-4-5 --------------------> Anthropic
```

The NetBird Agent Network proxy is a NetBird peer, not a public reverse proxy.
NetBird DNS resolves each generated Agent Network hostname to the proxy peer's
overlay address. The proxy replaces caller-supplied identity headers with
trusted `x-netbird-user-id` and `x-netbird-groups` values before forwarding to
the private AI agentgateway. Agentgateway requires NetBird's virtual API key
and uses the identity values for attribution.

## Static certificates

`prepare.sh` creates a private demo CA, a management certificate, and a wildcard
certificate for generated Agent Network hostnames. The CA is valid for ten
years and each server certificate is valid for one year. They are static so
the example remains self-contained, not because long-lived certificates are a
production recommendation.

For production, use certificates issued by your organization's PKI or a public
CA, automate renewal and deployment, shorten certificate lifetimes, protect
private keys with a secrets manager, and monitor expiry. Native ACME lifecycle
support for standalone agentgateway is tracked in
[agentgateway#3293](https://github.com/agentgateway/agentgateway/issues/3293).
To test operator-provided certificates, place the management and wildcard leaf
pairs at `runtime/certs/{management,proxy}/tls.{crt,key}` and their trust anchor
at `runtime/certs/ca.crt` before running `prepare.sh`.

## Temporary NetBird images

The example uses the pinned development images in `versions.env`. Replace
the NetBird server and proxy images with the first official release containing
[netbirdio/netbird#6970](https://github.com/netbirdio/netbird/issues/6970), and
replace the dashboard image with the first official release containing
[netbirdio/dashboard#774](https://github.com/netbirdio/dashboard/pull/774).
The temporary server, proxy, and dashboard images are AMD64-only, so Compose
uses emulation on ARM64 Docker hosts. Remove the `platform` overrides after
switching to multi-architecture release images.

## Prerequisites

- A Linux Docker host or Docker Desktop with Docker Compose v2.
- `curl`, `envsubst`, `jq`, and OpenSSL.
- A hostname in a domain you control.
- TCP 443 and UDP 3478 and 51820 reachable on the Docker host.
- OpenAI and Anthropic API credentials.
- A working `/dev/net/tun` device for the included test client. You can instead
  connect an external NetBird client and omit the example client services.

## 1. Configure the environment

Run these commands from this directory:

```bash
cp env.example .env
$EDITOR .env
./prepare.sh
```

The populated `.env`, generated application secrets, certificates, and rendered
NetBird configuration are ignored by Git. Keep `.env` and the `runtime`
directory private. `prepare.sh` restricts `.env` permissions to the current
user.

`prepare.sh` reuses existing certificates and application secrets so restarts
retain the same trust and encryption keys. Run `./cleanup.sh --volumes` before
preparing the example again when you need a completely new installation.

Trust `runtime/certs/ca.crt` on every browser or external NetBird client that
will access this deployment. The scripts pass this CA directly to `curl`, and
the included proxy and client containers mount it automatically.

## 2. Create the management DNS record

Create an A or AAAA record for `NETBIRD_MANAGEMENT_DOMAIN` that points to the
Docker host. No public record is needed for `NETBIRD_PROXY_DOMAIN` or its
wildcard. Those names are resolved privately by NetBird for authorized peers.

Confirm the management name resolves to the current host before continuing:

```bash
dig +short "$(sed -n 's/^NETBIRD_MANAGEMENT_DOMAIN=//p' .env)"
```

Repeated installations that reuse a hostname may require waiting for its DNS
TTL and flushing local resolver or browser caches.

## 3. Configure NetBird

The API mode creates the initial owner, Agent Network settings, client group,
agentgateway provider, access policy, proxy token, and one-use client setup key:

```bash
./configure.sh
```

The dashboard becomes available at `https://${NETBIRD_MANAGEMENT_DOMAIN}`.
Sign in with `NETBIRD_ADMIN_EMAIL` and `NETBIRD_ADMIN_PASSWORD` from `.env`.
When the first-run page offers peer-to-peer or remote-network onboarding, select
**Skip to Dashboard** for this Agent Network example.

To create the provider and access policy in the dashboard instead, run:

```bash
./configure.sh --mode dashboard
```

Then add an enabled `agentgateway` provider with upstream URL
`http://agent-network-agentgateway:3000`, the generated `NETBIRD_VIRTUAL_KEY`
from `runtime/generated.env`, no model restrictions, and identity metadata
enabled. Create an enabled `Agentgateway access` policy from the
`agentgateway-clients` group to that provider. Click **Continue** through the
limits and guardrails steps, then validate the result:

```bash
./configure.sh --check
```

## 4. Verify the integration

The default verification checks management HTTPS, the relay WebSocket upgrade,
strict virtual-key authentication, private proxy exposure, model discovery, and
the complete NetBird path without making billable provider requests:

```bash
./verify.sh
```

Run the live tests to exercise OpenAI Chat Completions, Responses, Embeddings,
streaming, and Anthropic Messages:

```bash
RUN_LIVE_PROVIDER_TESTS=true ./verify.sh
```

To make manual requests, obtain the generated endpoint and define a helper that
runs `curl` in the test container. The container shares the NetBird client's
network namespace, so these requests use the NetBird tunnel:

```bash
set -a
source .env
source runtime/generated.env
source runtime/admin.env
set +a

endpoint=$(curl --cacert runtime/certs/ca.crt -fsS \
  -H "Authorization: Token ${NETBIRD_PAT}" \
  "https://${NETBIRD_MANAGEMENT_DOMAIN}/api/agent-network/settings" \
  | jq -r .endpoint)

client_curl() {
  docker compose --env-file .env --env-file runtime/generated.env \
    exec -T test-client curl -fsS "$@"
}

client_curl "https://${endpoint}/v1/models" | jq
```

The model-discovery request is non-billable. The following requests reach the
configured OpenAI and Anthropic providers and may incur charges:

```bash
client_curl "https://${endpoint}/v1/chat/completions" \
  -H 'Content-Type: application/json' \
  --data-binary '{
    "model": "gpt-4o-mini",
    "messages": [{
      "role": "user",
      "content": "Reply with the word connected."
    }],
    "max_tokens": 16
  }' | jq

client_curl "https://${endpoint}/v1/messages" \
  -H 'Content-Type: application/json' \
  --data-binary '{
    "model": "claude-haiku-4-5",
    "max_tokens": 16,
    "messages": [{
      "role": "user",
      "content": "Reply with the word connected."
    }]
  }' | jq
```

Each live response should contain `connected`. The client does not supply an
authorization header: the NetBird proxy authenticates the peer, adds the
agentgateway virtual key, and forwards trusted NetBird identity headers. The
calls also appear in **Agent Network > Usage & Logs** in the NetBird dashboard.

## Cleanup

Stop the stack while retaining NetBird state, generated credentials, and
certificates:

```bash
./cleanup.sh
```

Remove the named volumes and all generated runtime material for a complete
reset:

```bash
./cleanup.sh --volumes
```

## Production hardening

This example deliberately favors a small, inspectable topology. A production
deployment should automate certificate lifecycle, use a secrets manager,
restrict container and host firewall access, back up NetBird state, configure
request-log storage and retention, monitor every component, and consider HTTPS
or mTLS between the NetBird proxy and private AI agentgateway when the Docker
bridge is not an adequate trust boundary.

Agentgateway's [model catalog](https://agentgateway.dev/docs/standalone/latest/llm/cost-controls/costs/),
[request logging](https://agentgateway.dev/docs/standalone/latest/observability/access-logs/database/),
and [dashboard](https://agentgateway.dev/docs/standalone/latest/llm/cost-controls/dashboard/)
are useful next steps for model discovery, cost reporting, and usage analysis.
