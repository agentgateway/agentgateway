# MCP CEL guardrails

This example rejects selected tool calls and masks text in requests and responses
using in-process CEL guardrails. It uses the `echo` tool from the MCP everything
server. You need Node.js/npm (`npx`) and a gateway build containing CEL MCP guardrails.

## Run

From the repository root:

```bash
cargo run -- -f examples/mcp-guardrails/expression/config.yaml
```

In another terminal, start the MCP Inspector:

```bash
npx @modelcontextprotocol/inspector
```

Connect using Streamable HTTP to `http://localhost:3000/mcp`, list the tools,
and select `echo`. Use its advertised name if the client displays a target prefix;
the request guardrails see the resolved upstream name `echo`.

## Try it

Call `echo` with these arguments:

```json
{
  "message": "Contact alice@example.com and bob@example.org; tokens demo-secret-abc and demo-secret-xyz"
}
```

The request processor replaces **both** email addresses with `[EMAIL]` before
forwarding. The response processor replaces both demo secrets with `[SECRET]`
in the returned text blocks. The echoed text therefore contains:

```text
Contact [EMAIL] and [EMAIL]; tokens [SECRET] and [SECRET]
```

Then try:

```json
{"message": "DO_NOT_SEND alice@example.com"}
```

The first processor rejects the call with
`Messages containing DO_NOT_SEND may not be sent upstream`. The tool is not called.

## Expressions and ordering

[config.yaml](config.yaml) runs request denial, request masking, and response
masking in that order within their respective phases. Each processor sees earlier
processors' mutations; the first rejection stops the chain.

- `mcp.params` contains the current request parameters, with the same structure as
  the `params` object in an MCP request.
- `mcp.result` contains the current response result, with the same structure as
  the `result` object in an MCP response.
- Each processor specifies exactly one of `reject` or `transform`.
- `transform` returns a replacement body; returning `null` leaves it unchanged.
  Use `merge` to preserve fields you do not wish to mutate.
- `regexReplaceAll` replaces every non-overlapping match; `regexReplace` replaces only
  the first. Both support replacement capture groups such as `$1`.
- Expression errors and invalid transformed MCP bodies reject the operation.

This example masks email addresses before forwarding the request and demo secrets
before returning the response. Adapt the expressions to mask other fields.
