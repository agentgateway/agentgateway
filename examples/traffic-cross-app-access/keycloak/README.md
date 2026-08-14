# End-to-end ID-JAG / Cross App Access with agentgateway + Keycloak

A complete working demo fo ID-JAG with Keycloak and agentgateway locally. This demo automates all the
Keycloak configuration, and drives a real backend call where agentgateway turns a user's ID token
into a backend access token using the OAuth **Identity Assertion Authorization Grant (ID-JAG /
Cross App Access)** with **no other external identity provider**.

## Prerequisites

| Component | What |
|---|---|
| **Keycloak (ID-JAG)** | the `ceposta/keycloak:id-jag` container image (Keycloak with Identity Assertion / ID-JAG support) |
| **agentgateway** | built from source or a `v1.4.0-alpha.1`+ release binary (with the Cross App Access policy) — see step 4 |
| tools | Docker, **python3** (echo backend + JWT decoding), `curl` |

> Building the Keycloak image yourself: see [`docker/README.md`](docker/README.md).

All commands below run from this directory:

```bash
cd examples/traffic-cross-app-access/keycloak
```

You'll use **three terminals** (Keycloak, echo backend, gateway) plus one to drive requests.

---

## 1. Start Keycloak — Terminal 1

Run Keycloak on `:8480` and map `8480:8480` so the **same URL is used inside and outside** the
container (see "Why one URL everywhere" below):

```bash
docker run --name kc-idjag --rm -p 8480:8480 \
  -e KC_BOOTSTRAP_ADMIN_USERNAME=admin -e KC_BOOTSTRAP_ADMIN_PASSWORD=admin \
  ceposta/keycloak:id-jag start-dev --http-port=8480
# ... Keycloak ... started in N s. Listening on: http://localhost:8480
```

The ID-JAG feature is already enabled in the image. Admin console: <http://localhost:8480> (admin/admin).

Sanity check in another shell:
```bash
curl -s -o /dev/null -w "%{http_code}\n" http://localhost:8480/realms/master   # -> 200
```

## 2. Configure Keycloak — one command

> **Order matters:** configure Keycloak **before** starting the gateway (step 4). The gateway
> fetches the realm's signing keys at startup and caches them; this script (re)creates the realm,
> which rotates those keys. If you re-run configuration later, **restart the gateway** afterward.

```bash
KCADM="$PWD/docker/kcadm.sh" SERVER=http://localhost:8480 ./configure-keycloak.sh
```

`docker/kcadm.sh` runs Keycloak's admin CLI inside the container, so you don't need a Keycloak
install on the host. It creates realm **`idjag-demo`**, user **`alice`** / `alice`, and the two
clients + supporting config described in "What the setup creates" below. Inspect it all in the
admin console at <http://localhost:8480> (admin/admin).

## 3. Start the echo backend — Terminal 2

A stand-in "downstream API" that returns the request headers as JSON, so you can see the token
the gateway attaches.

```bash
python3 echo-backend.py       # listening on :9000
```

## 4. Start agentgateway — Terminal 3

[`gateway.yaml`](gateway.yaml) validates the inbound Keycloak ID token and runs the Cross App
Access two-leg exchange against Keycloak, forwarding to the echo backend. The client secrets come
from env vars (matching what the setup scripts create) — export them, then run it in the
**foreground in its own terminal** (it binds **`:3030`**):

```bash
export KC_AGENT_SECRET=agent-secret KC_RESOURCE_SECRET=resource-secret
cargo run --release --bin agentgateway -- -f examples/traffic-cross-app-access/keycloak/gateway.yaml
```

(Or use a released agentgateway binary of `v1.4.0-alpha.1`+, the first release with Cross App
Access. Validate without starting: append `--validate-only`.)

> **Port :3030.** agentgateway's usual default is `:3000`; this demo uses `:3030` so it won't
> collide with any other agentgateway you already have running on `:3000`. Change the `port:` in
> `gateway.yaml` if `:3030` is also taken. (Validate config without starting: append `--validate-only`.)

---

## 5. Drive the flow — Terminal 4

### 5a. Get alice's Keycloak ID token
In reality the agent obtains this via a normal OIDC login; here we use a direct-access-grant
shortcut (which also creates the user session the exchange requires).

```bash
TOKEN_URL=http://localhost:8480/realms/idjag-demo/protocol/openid-connect/token
ID_TOKEN=$(curl -s -X POST "$TOKEN_URL" \
  -d grant_type=password -d client_id=agent-client -d client_secret="$KC_AGENT_SECRET" \
  -d username=alice -d password=alice -d scope=openid \
  | python3 -c 'import sys,json;print(json.load(sys.stdin)["id_token"])')
echo "${ID_TOKEN:0:40}..."
```

### 5b. Call the gateway with the ID token
```bash
curl -s http://localhost:3030/todos -H "Authorization: Bearer $ID_TOKEN" | python3 -m json.tool
```

Expected: the echo backend reports it received `GET /todos` with an
`Authorization: Bearer <a DIFFERENT token>` — the Keycloak access token the gateway minted.
Decode it to confirm:

```bash
curl -s http://localhost:3030/todos -H "Authorization: Bearer $ID_TOKEN" | python3 -c '
import sys,json,base64
d=json.load(sys.stdin)
tok=(d["headers"].get("authorization") or d["headers"].get("Authorization")).split()[1]
p=json.loads(base64.urlsafe_b64decode(tok.split(".")[1]+"=="))
print("backend received a", p["typ"], "token for", p["preferred_username"],
      "| azp:", p["azp"], "| scope:", p["scope"])'
# -> backend received a Bearer token for alice | azp: resource-client | scope: todos.read profile email
```

### 5c. (Optional) See each leg yourself with `round-trip.sh`
Runs the three token calls directly against Keycloak (bypassing the gateway) and prints the
decoded ID-JAG and final access token:

```bash
./round-trip.sh
```

### 5d. Negative paths
```bash
curl -s -o /dev/null -w "no token   -> %{http_code}\n" http://localhost:3030/todos
curl -s -o /dev/null -w "bad token  -> %{http_code}\n" http://localhost:3030/todos -H "Authorization: Bearer not.a.jwt"
# -> 400 and 401: the gateway rejects before any exchange happens
```

---

## What just happened

1. The gateway validated the inbound Keycloak **ID token** (issuer, audience, signature). The
   validated token is the exchange *subject*.
2. **Leg 1 (token exchange)** — the gateway, authenticating as `agent-client`, sent the ID token
   to Keycloak asking for an **ID-JAG** bound to `https://resource.idjag.demo`. Keycloak returned
   a signed ID-JAG (`typ=oauth-id-jag+jwt`, `aud=https://resource.idjag.demo`, `client_id=resource-client`).
3. **Leg 2 (jwt-bearer grant)** — the gateway, authenticating as `resource-client`, presented the
   ID-JAG. Keycloak validated it and issued a normal **Bearer access token**.
4. The gateway **stripped** the inbound ID token and attached `Authorization: Bearer <access token>`
   to the upstream request. The result is cached until near expiry.


## agentgateway config mapping (`gateway.yaml`)

- `crossAppAccess.identityProvider` → `http://localhost:8480/realms/idjag-demo/protocol/openid-connect/token`, client `agent-client`
- `crossAppAccess.resourceAuthorizationServer` → same endpoint, client `resource-client`
- `crossAppAccess.audience` → `https://resource.idjag.demo`; `crossAppAccess.scopes` → `[todos.read]`
- `jwtAuth` issuer `http://localhost:8480/realms/idjag-demo`, audience `agent-client`, remote JWKS.

`crossAppAccess.resources` is intentionally unset — Keycloak resolves the target via the
`audience` parameter (the resource client's resource-server identifier), not a separate `resource`
parameter.

> **Why one URL everywhere (`8480:8480` + `--http-port=8480`)?** ID-JAG is self-referential: the
> identity-provider issuer, the token issuer, and the internal key lookup must all agree on one
> base URL. If Keycloak listened on one port internally but tokens were minted via a different
> host/port, the issuer wouldn't match and leg 2 would fail. Using one port everywhere avoids the
> split. (A production deployment pins this with a fixed hostname instead.)


## Teardown

```bash
docker rm -f kc-idjag                       # stop Keycloak
lsof -ti tcp:9000,3030 | xargs kill         # stop echo + gateway
```

