# Bitcoin MCP Example

This example proxies [bitcoin-mcp](https://github.com/Bortlesboat/bitcoin-mcp) through agentgateway, adding rate limiting and CORS support.

bitcoin-mcp provides 49 tools for querying the Bitcoin network: fee estimation, mempool analysis, block inspection, transaction decoding, mining stats, and more. It works out of the box with the free Satoshi API — no Bitcoin node required.

## Prerequisites

```bash
pip install bitcoin-mcp
```

## Running the example

```bash
agentgateway -f examples/bitcoin/config.yaml
```

Connect any MCP client to `http://localhost:3000`.

## What's included

- **Rate limiting**: 30 burst requests, refill 1 token every 2 seconds
- **CORS**: permissive for development (lock down `allowOrigins` in production)
- **stdio transport**: agentgateway launches `bitcoin-mcp` as a child process

## Example queries

Once connected, try:

- "What are the current Bitcoin fees?"
- "Analyze the mempool"
- "Get the current block height"
- "Give me a Bitcoin situation summary"

## Adding security

See the [authorization example](../authorization/) to add JWT authentication and RBAC policies. For Bitcoin, you might restrict write tools (`send_raw_transaction`, `generate_keypair`) to admin users while keeping the 47 read-only tools open to all authenticated agents.
