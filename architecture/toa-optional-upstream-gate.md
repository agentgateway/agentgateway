# Optional Tool Outcome Attestation (TOA) for upstream MCP

Agentgateway already authenticates clients and authorizes MCP tools with CEL (`mcpAuthorization`, JWT, and related policies). That answers *who may call what*. It does not answer whether an upstream MCP tool recently delivered a real result under an outside probe.

[TOA](https://github.com/Carmel-Labs-Inc/toa) (`toa/0.1`) is an Apache-2.0 signed JSON evidence format for MCP tool delivery (reach, invoke, functional, shape, and related layers). It is not a wire protocol. It is not meant to run on every live `tools/call`.

## Suggested fit (optional, off by default)

Before promoting or attaching a new upstream MCP backend in CI or change control, require a recent attestation and verify it offline with a pinned emitter public key.

- Any party can emit if they sign the schema.
- AgentStatus is one optional emitter.
- No AgentStatus account is required to verify.

```yaml
      # After your existing MCP smoke / auth policy checks.
      - name: Verify tool delivery attestation
        if: hashFiles('toa.json') != ''
        run: |
          pip install "git+https://github.com/Carmel-Labs-Inc/toa.git@345f24607919b5bdf143719b9ea062543cdfe88e#subdirectory=python"
          toa-verify toa.json --require-layer functional=pass
```

Copy-paste example: [`examples/mcp-toa-verify`](../examples/mcp-toa-verify/README.md).

## Out of scope for this doc

- Replacing JWT / CEL `mcpAuthorization`
- Signing every production `tools/call`
- Changing the proxy hot path

This is documentation of an adjacent gate only.
