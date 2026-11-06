# Fortytwo

Route Chat Completions requests to [Fortytwo](https://fortytwo.network) through agentgateway.
Requires a build that includes the `fortytwo` provider preset.

## Add your API key

Create a key in the [Fortytwo Platform](https://platform.fortytwo.network/api-keys), then set:

```sh
export FORTYTWO_API_KEY="your-api-key"
```

## Run

From this directory:

```sh
agentgateway -f config.yaml
```

## Send a request

```sh
curl --no-buffer http://localhost:4000/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "qwen/qwen3.8-27b",
    "messages": [{"role": "user", "content": "What is Rust?"}],
    "reasoning_effort": "medium",
    "stream": true,
    "stream_options": {"include_usage": true}
  }'
```

The preset uses `https://api.fortytwo.network/v1` and advertises Chat Completions only.
Send function tools through the same endpoint; the client executes tools and sends their results back.
Current capabilities and model IDs: [Models](https://docs.fortytwo.network/docs/models).
Current prices: [Pricing](https://docs.fortytwo.network/docs/pricing).

## Existing releases

Without the preset, replace `provider: fortytwo` with the following custom provider
and add `baseUrl: https://api.fortytwo.network/v1` under `params`:

```yaml
provider:
  custom:
    providerOverride: fortytwo
    formats:
    - type: completions
```
