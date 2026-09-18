# A2A candidate routing with ARBITER

This example shows one insertion point between A2A discovery and handoff:

```text
task/context
→ candidate Agent Cards
→ ARBITER ordering
→ existing gateway policy / auth / health checks
→ selected A2A backend
```

A2A defines Agent Cards and discovery. When a gateway or orchestrator already has multiple legitimate Agent Cards for the same task, this example uses ARBITER as the default **ranking step inside this example** before the caller applies its normal policy and executes the handoff.

ARBITER does not discover agents, authorize them, execute the A2A request, or replace agentgateway routing/security controls.

## Run

Use a dedicated Base wallet with enough USDC for the paid request.

```bash
cd examples/a2a-arbiter-routing
npm install
export EVM_PRIVATE_KEY='0x...'
npm run route -- \
  "Production checkout errors increased immediately after a deployment. Choose the specialist agent for the next diagnostic step."
```

The example:

1. loads the candidate Agent Cards from `agents.json`;
2. serializes only caller-visible identity, description, and skills;
3. calls `POST https://api.grip.fyi/v1/compare`;
4. pays `$0.01 USDC` through x402 on Base;
5. maps the returned order back to the original Agent Cards;
6. leaves authorization, health, availability, policy, and execution to the gateway.

## ARBITER

Remote MCP:

`https://api.grip.fyi/mcp`

MCP identity:

`fyi.grip/arbiter`

Paid compare:

`POST https://api.grip.fyi/v1/compare`

A2A routing profile:

`https://grip.fyi/agent-routing/a2a-extension/SPEC.md`
