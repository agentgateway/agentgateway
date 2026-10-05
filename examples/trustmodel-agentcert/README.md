## TrustModel AgentCert integration

This example verifies a calling agent's **TrustModel AgentCert + TrustScore** on
every request, using agentgateway's `extAuthz` policy — **no code in agentgateway
itself**. agentgateway calls the [TAG verify sidecar](https://github.com/pdxlab/agentcert-tag)
over HTTP; the sidecar validates the agent's cert (chain + revocation), looks up
its live TrustScore, and returns `200` (allow) / `403` (deny) plus `x-agentcert-*`
verdict headers that are surfaced onto the upstream request and the access log.

It runs in **shadow mode by default**: the sidecar decides and logs but returns
`200` on every request, so you can drop it into real traffic without blocking
anything. Flip to enforcement only once you've seen the verdicts you expect.

### Running the example

Start the TAG verify sidecar (you supply a trust-anchor bundle at
`examples/trustmodel-agentcert/anchors/roots.pem` — see the
[sidecar docs](https://github.com/pdxlab/agentcert-tag/blob/main/docs/ext_authz.md)):

```bash
docker compose -f examples/trustmodel-agentcert/docker-compose.yaml up
```

Then run agentgateway (the route forwards to an MCP/agent upstream on
`localhost:9000` — adjust the backend in `config.yaml`):

```bash
agentgateway -f examples/trustmodel-agentcert/config.yaml
```

Send a request with an AgentCert in the `x-agent-cert` header (plus
`x-agent-cert-proof` for the stapled proof-of-possession, or
`x-agent-cert-carriage: mtls` if agentgateway already terminated mTLS with the
agent's cert). The access log shows `agent.verdict` and `agent.trustscore`, and
the upstream receives the `x-agentcert-*` headers.

### Shadow → enforce

- **Shadow (default):** `TAG_MODE=shadow` on the sidecar — verifies and logs,
  blocks nothing. Watch `x-agentcert-verdict` / the `x-agentcert-shadow-would`
  header to see what enforcement would do.
- **Enforce:** set `TAG_MODE=enforce` (optionally `TAG_MIN_SCORE=N` for a score
  floor). `VERIFIED` is allowed; everything else is denied. Keep the sidecar's
  `TAG_FAIL_MODE` aligned with `failureMode` in `config.yaml`.

See the [ext_authz contract](https://github.com/pdxlab/agentcert-tag/blob/main/docs/ext_authz.md)
for the full header/verdict reference.
