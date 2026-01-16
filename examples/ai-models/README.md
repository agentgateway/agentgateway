# AI Models Aggregation Example

This example demonstrates how to configure agentgateway to return a list of models at the `/v1/models` endpoint.

## Overview

The `/v1/models` endpoint is commonly used by LLM clients to discover available models. By declaring models in your agentgateway configuration, you can:

- Advertise available models to clients
- Provide a unified model list across multiple backends
- Control which models are visible to clients

## Configuration

Models are declared in the `ai` policy section:

```yaml
policies:
  ai:
    models:
    - id: gpt-4o-mini
      ownedBy: openai
      created: 1704067200
    - id: gpt-4-turbo
      ownedBy: openai
      created: 1704067200
    routes:
      "/v1/models": Models
      "/v1/chat/completions": Completions
```

### Fields

- `id` (required): The model identifier (e.g., "gpt-4o-mini", "claude-3-5-sonnet")
- `ownedBy` (required): The organization that owns/provides the model (e.g., "openai", "anthropic")
- `created` (optional): Unix timestamp of when the model was created

### Routes

The `routes` configuration maps URL paths to handler types. For models aggregation:
- Map `/v1/models` to `Models` handler
- Map other endpoints like `/v1/chat/completions` to `Completions` handler

## Usage

1. Start agentgateway with this configuration:
   ```bash
   agentgateway -f examples/ai-models/config.yaml
   ```

2. Query the models endpoint:
   ```bash
   curl http://localhost:3000/v1/models
   ```

3. Expected response (OpenAI-compatible format):
   ```json
   {
     "object": "list",
     "data": [
       {
         "id": "gpt-4o-mini",
         "object": "model",
         "owned_by": "openai",
         "created": 1704067200
       },
       {
         "id": "gpt-4-turbo",
         "object": "model",
         "owned_by": "openai",
         "created": 1704067200
       },
       {
         "id": "gpt-3.5-turbo",
         "object": "model",
         "owned_by": "openai",
         "created": 1677649963
       }
     ]
   }
   ```

## Use Cases

### Multi-Backend Aggregation

When using multiple AI backends, you can declare all available models in a single configuration:

```yaml
policies:
  ai:
    models:
    # OpenAI models
    - id: gpt-4o-mini
      ownedBy: openai
    - id: gpt-4-turbo
      ownedBy: openai
    # Anthropic models
    - id: claude-3-5-sonnet
      ownedBy: anthropic
    - id: claude-3-opus
      ownedBy: anthropic
```

### Model Visibility Control

You can control which models are visible to clients by only declaring the models you want to expose:

```yaml
policies:
  ai:
    models:
    # Only expose specific models
    - id: gpt-4o-mini
      ownedBy: openai
```

## Notes

- The `/v1/models` endpoint returns models declared in the policy configuration
- If no models are configured, the endpoint returns an empty list
- The response format is compatible with the OpenAI API specification
- Models are returned in the order they are declared in the configuration
