# Import

`agentgateway import` converts a standalone config from another gateway into an agentgateway config.

The only supported source today is LiteLLM.

## Usage

```bash
agentgateway import --from litellm --file litellm-config.yaml
```

Findings are written to stderr. The generated config is written to stdout. To write the config to a file instead:

```bash
agentgateway import --from litellm --file litellm-config.yaml --output agentgateway-config.yaml
```

From a development checkout:

```bash
cargo run -p agentgateway-app -- import --from litellm --file litellm-config.yaml
```

`agentgateway import --help` reports these flags:

```text
Usage: agentgateway import [OPTIONS] --from <source> --file <file>

Options:
      --from <source>  Source gateway configuration format
  -f, --file <file>    Read source configuration from a file
  -o, --output <file>  Write generated agentgateway configuration to a file instead of stdout
  -h, --help           Print help
```

`--from` must be `litellm`. Any other value exits with: `unsupported import source "..."; supported sources: litellm`.

## Example

This input is the `inbound-api-key` test fixture:

```yaml
model_list:
- model_name: chat
  litellm_params:
    model: openai/gpt-4o
general_settings:
  master_key: os.environ/LITELLM_MASTER_KEY
  litellm_key_header_name: X-LiteLLM-Key
```

Running `agentgateway import --from litellm --file inbound-api-key.yaml` prints findings on stderr:

```text
exact: model_list[0]: Imported model deployment for chat
approximate: general_settings.master_key: Mapped LiteLLM's master_key (an admin-equivalent credential, not a per-user virtual key) and custom API-key header to inbound API-key authentication; LiteLLM also accepts the same key with a Bearer prefix on this header, which this does not replicate. Per-user virtual-key migration needs LiteLLM's database state, not config.yaml alone.
exact: general_settings.litellm_key_header_name: Mapped LiteLLM's custom API-key header name to the inbound API-key policy

Import summary: 2 exact, 1 approximate, 0 manual, 0 unsupported
```

and this config on stdout:

```yaml
config:
  database:
    url: sqlite://data.db
gateways:
  default:
    port: 4000
ui:
  gateways: default
llm:
  gateways: default
  models:
  - name: chat
    provider: openAI
    params:
      model: gpt-4o
  policies:
    apiKey:
      keys:
      - key: $LITELLM_MASTER_KEY
      mode: strict
      location:
        header:
          name: x-litellm-key
```

`os.environ/LITELLM_MASTER_KEY` is emitted as `$LITELLM_MASTER_KEY`. A literal `master_key` is copied as-is. With `--output agentgateway-config.yaml`, the config is written to that file and stdout is:

```text
Imported litellm config: agentgateway-config.yaml
```

The SQLite URL is `sqlite://data.db` by default. When `--output` points at a real file, the database is placed beside it (`sqlite://<dir>/data.db`).

## Findings

Each finding is one line on stderr:

```text
<status>: <source path>: <message>
```

The statuses are:

- `exact`: mapped as in the source.
- `approximate`: mapped, with a difference called out in the message.
- `manual`: not mapped; needs a human decision.
- `unsupported`: not mapped; agentgateway cannot represent it, or the value is invalid.

The command still writes a config when some fields cannot be mapped. It exits 0 in that case. Review every `manual` and `unsupported` line before using the result.

After the per-field lines, the command prints a summary of how many findings landed in each status. If any are `manual` or `unsupported`, it then lists those lines again under `Needs manual review:` so you do not have to scroll back through the full finding list. For example, a per-deployment timeout, which is not mapped, looks like this:

```text
manual: model_list[0].litellm_params.timeout: Per-deployment LiteLLM timeout was not mapped; agentgateway applies at most one router_settings.timeout across all models
exact: model_list[0]: Imported model deployment for gpt-4o
approximate: router_settings.timeout: Mapped LiteLLM router_settings.timeout to the single global llm.policies.timeout backendRequestTimeout; LiteLLM allows a per-deployment override that agentgateway cannot represent

Import summary: 1 exact, 1 approximate, 1 manual, 0 unsupported
Needs manual review:
  - model_list[0].litellm_params.timeout: Per-deployment LiteLLM timeout was not mapped; agentgateway applies at most one router_settings.timeout across all models
```

For example, an invalid `litellm_key_header_name` does not emit an inbound API-key policy:

```text
exact: model_list[0]: Imported model deployment for chat
manual: general_settings.master_key: LiteLLM master_key was not emitted because LiteLLM custom API-key header name is invalid
unsupported: general_settings.litellm_key_header_name: LiteLLM custom API-key header name is invalid and the inbound policy was not emitted
```

## Other LiteLLM fields this command maps

- `router_settings.timeout` becomes a single global `llm.policies.timeout`. A per-deployment `litellm_params.timeout` is reported and not mapped.
- `litellm_params.order` becomes failover priority when every deployment in a model group sets an integer order and at least two distinct values exist. A partial setting (some deployments have no order) or every deployment sharing the same value leaves the existing routing in place; deployments that tie at a shared order value fall into the same priority tier with no weighting between them, since RPM-based weighting does not apply once any ordering is used.
- `general_settings.pass_through_endpoints` become standalone routes. An endpoint with `auth: true` is omitted if there is no usable `master_key`, rather than left open.
- Bedrock guardrails become `llm.policies.guardrails`. Other guardrail types, such as Presidio, are reported and not emitted.
- `mcp_servers` become `mcp.targets`. Names are sanitized to the Gateway API section-name pattern. A restricted server (`allow_all_keys` set to false, omitted, or not a boolean) is not emitted from config.yaml alone, because the per-key grants that protect it live in LiteLLM's database. If MCP targets are emitted, the inbound API-key policy is also applied to the MCP gateway. If there is no inbound key, MCP targets are dropped rather than left unauthenticated.

## What is not migrated

- LiteLLM database settings (`database_url` and similar) are not reused. The generated config always points at a new agentgateway SQLite database. Configure PostgreSQL yourself if you need it.
- Unrecognized top-level fields, unused credentials, model metadata, and environment variables that are not part of a mapped value are reported and not emitted.
- The importer does not read LiteLLM's runtime database. Virtual keys, budgets, and per-key MCP grants are not imported by this command. Restricted MCP servers therefore stay omitted.
- Per-deployment timeouts, non-Bedrock guardrails, and pass-through endpoints that require auth without a usable `master_key` are not mapped, as in the findings above.
