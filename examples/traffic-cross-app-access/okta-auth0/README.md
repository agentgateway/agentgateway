# ID-JAG / Cross App Access demo — Okta IdP + Auth0 resource

A runnable demo of agentgateway's `crossAppAccess` policy wired to **real** identity providers:

- **Okta** = the Enterprise IdP that mints the **ID-JAG** (RFC 8693 token exchange).
- **Auth0** = the Resource App's authorization server that turns the ID-JAG into a
  Bearer access token (RFC 7523 jwt-bearer grant).

agentgateway acts as the **requesting app** (a confidential OAuth client) and performs
both legs on every backend call, so an agent calls a downstream API *as the end user*
with no second interactive login.

```
agent ──Okta ID token──▶ agentgateway
                          │  leg 1: POST https://<okta>/oauth2/v1/token
                          │         grant_type=token-exchange
                          │         subject_token=<ID token>
                          │         requested_token_type=…:id-jag
                          │         audience=https://<auth0>/          ──▶ Okta ──ID-JAG──▶
                          │  leg 2: POST https://<auth0>/oauth/token
                          │         grant_type=jwt-bearer
                          │         assertion=<ID-JAG>                  ──▶ Auth0 ──access token──▶
                          └▶ GET <resource API>  Authorization: Bearer <access token>
```

## 0. Prerequisites (two separate feature gates)

| Provider | Gate | How to get it |
|---|---|---|
| **Okta** | Cross App Access is a self-service **Early Access** feature | Enable it in the Admin Console (Settings → Features, or **Reports → check EA**). Until enabled, the XAA OIN placeholder apps (Agent0 / Todo0 / "XAA Requesting App") don't appear in the catalog. |
| **Auth0** | XAA is a **private Beta** | Contact Auth0 Support / your TAM to enable it on the tenant. |

You also need: an Okta org, an Auth0 tenant, and a downstream "resource" API to call
(the Auth0-protected API — e.g. a Todos API, or any HTTPS service you point `backends` at).

## 1. Okta setup (Enterprise IdP)

1. **Register the requesting app** (what the gateway *is*). Fastest path uses the OIN
   placeholder **Agent0**: *Applications → Browse App Catalog →* search **Agent0** → add.
   In the app's **Sign On** tab, copy the **Client ID** and **Client secret** — you'll export
   these as `OKTA_CLIENT_ID` and `OKTA_CLIENT_SECRET` (step 3).
   Note the app's **token endpoint auth method** (Basic vs Post) — the config's
   `clientAuth.method` must match it.
2. **Register the resource app** — OIN placeholder **Todo0** (*Browse App Catalog →* **Todo0**),
   or a real XAA resource app entry ("XAA Requesting App" placeholder) pointing at your
   Auth0 tenant's issuer + client id.
3. **Create the managed connection.** Open the *resource* app → **Manage Connections** tab →
   in **Apps granted consent** click **Add apps** (a.k.a. *Add requesting apps*) → select
   the requesting app (Agent0) → **Save**. It should show **Managed**. Without this managed
   connection Okta will **not** mint an ID-JAG.
4. XAA uses the **org authorization server**, so the token endpoint is
   `https://<OKTA_DOMAIN>/oauth2/v1/token` and JWKS is `https://<OKTA_DOMAIN>/oauth2/v1/keys`
   (not a custom `/oauth2/<id>/…` server).

## 2. Auth0 setup (Resource AS)

1. **Create the API** representing your resource. Its **Identifier** is the audience of the
   Auth0 access token. Optionally set it as the tenant **Default Audience** (recommended for
   this demo — see the "Known discrepancy" note about `resources`).
2. **Create the Resource Application** — *Applications → Create Application →* **Regular Web
   Application** (must be a confidential, first-party client; SPAs/native are unsupported).
   In **Settings**, enable the **Cross App Access** toggle. Copy its **Client ID** / **Client
   Secret** → you'll export these as `AUTH0_CLIENT_ID` and `AUTH0_CLIENT_SECRET` (step 3).
3. **Trust the Okta IdP.** Create an **Okta Workforce** enterprise connection in Auth0:
   enter the resource app's Client ID/Secret and the Okta **Issuer URL**, and activate the
   **"Cross App Access – Resource Application"** role on the connection. Link it under the
   connection's **Applications** tab, and confirm the **Callback URL** matches the Redirect
   URI on the Okta app.
4. Auth0's token endpoint is `https://<AUTH0_DOMAIN>/oauth/token`.

## 3. Export the config values as environment variables

`gateway.yaml` reads all deployment-specific values from env vars (expanded at load time) — nothing
is hardcoded and there are no secret files. Export them in the shell you'll run the gateway from:

```bash
export OKTA_DOMAIN='dev-12345.okta.com'          # no scheme
export OKTA_CLIENT_ID='0oa...'                   # requesting-app client id (also the ID-token aud)
export OKTA_CLIENT_SECRET='...'                  # requesting-app client secret
export AUTH0_DOMAIN='your-tenant.us.auth0.com'   # no scheme
export AUTH0_CLIENT_ID='...'                     # Auth0 resource-application client id
export AUTH0_CLIENT_SECRET='...'                 # Auth0 resource-application client secret
export AUTH0_API='https://api.example.com'       # Auth0 API Identifier (RFC 8707 resource)
export SCOPE='todos.read'                         # a scope defined on your Auth0 API
export RESOURCE_API_HOST='api.example.com'       # downstream API host the agent calls
```

The `audience` is derived in the config as `https://$AUTH0_DOMAIN/` (trailing slash required).

## 4. Run the gateway

Run **from this directory**, in the **same shell** where you exported the vars (it binds
**`:3032`**). Validate first with `--validate-only`:

```bash
cargo run --release --bin agentgateway -- -f ./gateway.yaml
```

(Or use a released agentgateway binary of `v1.4.0-alpha.1`+ — the first release that includes
Cross App Access.)

## 5. Get a user's Okta ID token (the subject)

The inbound request must carry the user's **Okta ID token**. Any standard OIDC
Authorization-Code login against the requesting app works. Quick option using the Okta org
authorization server:

- authorize: `https://$OKTA_DOMAIN/oauth2/v1/authorize?client_id=$OKTA_CLIENT_ID&response_type=code&scope=openid%20profile%20email&redirect_uri=<REDIRECT>&state=xyz`
- exchange the returned `code` at `https://$OKTA_DOMAIN/oauth2/v1/token` (grant_type=authorization_code) and grab `id_token`.

The Auth0 sample [`auth0-cross-app-access-inspector`](https://github.com/auth0-samples/auth0-cross-app-access-inspector)
does exactly this login (redirect URI `http://localhost:3000/login/callback`) and is a handy
way to capture an ID token if you don't already have one.

## 6. Drive the demo

```bash
export ID_TOKEN="<the Okta ID token from step 5>"
curl -sv http://localhost:3032/<resource-api-path> \
  -H "Authorization: Bearer $ID_TOKEN"
```

What to expect: the gateway validates the ID token (`jwtAuth`), performs leg 1 at Okta and
leg 2 at Auth0, strips the inbound credential, and forwards the request to
`<RESOURCE_API_HOST>` with a fresh `Authorization: Bearer <auth0-access-token>`. The token is
cached (`cache.defaultTtl`) so repeated calls skip the exchange until near expiry.

To watch the two legs (and see why either side rejects a request), prefix your run command with
`RUST_LOG=agentgateway=trace` — it logs each exchange leg and the authorization-server error body.

## Gotchas

- **`audience` needs the trailing slash** — for Auth0 it's the tenant issuer `https://<AUTH0_DOMAIN>/`.
  Getting this wrong is the #1 cause of Auth0 rejecting the ID-JAG on leg 2.
- **Export the vars in the same shell** that runs `cargo run`. Config load fails with
  *"environment variable not found"* if any is unset.
- **TLS: two styles, by position.** The two `crossAppAccess` exchange endpoints use the `https://host`
  form (`https://$OKTA_DOMAIN`, `https://$AUTH0_DOMAIN`), which auto-configures TLS — no `:443` +
  `backendTLS`. The **route backend** (the resource API) takes a plain `host:port`, so HTTPS there
  still needs `$RESOURCE_API_HOST:443` plus a route-level `backendTLS: {}`. Get either wrong and you
  hit *"The plain HTTP request was sent to HTTPS port"*.
- **The `resource` binding.** Per the ID-JAG draft (RFC 8707), `resource` is sent on leg 1 (to Okta)
  and embedded in the ID-JAG; leg 2 sends only the assertion + scope. Set `resources` to your Auth0
  API Identifier to bind the token's target, or omit it and rely on the Auth0 API's Default Audience.
- **If leg 2 fails at Auth0:** check the `audience` trailing slash, that the Okta **managed connection**
  is in place, and that the Auth0 connection's **Cross App Access – Resource Application** role is activated.
