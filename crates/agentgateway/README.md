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
approximate: general_settings.master_key: Mapped LiteLLM master_key and custom API-key header to inbound API-key authentication; LiteLLM also accepts the same key with a Bearer prefix on this header, which this does not replicate
exact: general_settings.litellm_key_header_name: Mapped LiteLLM's custom API-key header name to the inbound API-key policy
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
      - key: ${LITELLM_MASTER_KEY}
      mode: strict
      location:
        header:
          name: x-litellm-key
```

`os.environ/LITELLM_MASTER_KEY` is emitted as `${LITELLM_MASTER_KEY}`. A literal `master_key` is copied as-is, though one containing `$` gets a `manual` finding instead, since agentgateway's config loader would try to expand it. With `--output agentgateway-config.yaml`, the config is written to that file and stdout is:

```text
Imported litellm config: agentgateway-config.yaml
```

The SQLite URL is `sqlite://data.db` by default. With `--output <dir>/config.yaml` it becomes `sqlite://<dir>/data.db`, a path resolved against the gateway process's own working directory at runtime, not against the location of the config file on disk.

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

A `master_key` or `litellm_key_header_name` shape the importer cannot safely interpret — not a string, an empty value, an `os.environ/` reference with no variable name, or a header name that is not valid HTTP — fails the whole import instead: it is never correct to write out a config with no inbound authentication at all because one setting among many could not be read.

If `general_settings.master_key` is absent entirely, the import still succeeds, but with a `manual` finding: this importer only reads config.yaml, so it cannot tell whether LiteLLM is instead authenticating from a `LITELLM_MASTER_KEY` environment variable set outside the file (a common Docker deployment pattern).

## What is not migrated

- LiteLLM database settings (`database_url` and similar) are not reused. The generated config always points at a new agentgateway SQLite database. Configure PostgreSQL yourself if you need it.
- Unrecognized top-level fields, unused credentials, model metadata, and environment variables that are not part of a mapped value are reported and not emitted.
- This command reads only config.yaml. Virtual keys, budgets, and per-key MCP grants that LiteLLM keeps in its own runtime database are out of scope here.
