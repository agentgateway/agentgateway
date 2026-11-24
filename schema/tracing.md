# Trace span attributes

Attributes on the request span. An attribute is set only when the request has a value for it.

## Common

| Attribute | Condition |
| --- | --- |
| `agentgateway.group` | — |
| `agentgateway.session` | — |
| `agentgateway.user` | — |
| `client.address` | — |
| `connection.id` | Debug logging is enabled for the `request` target. |
| `duration` | — |
| `endpoint` | — |
| `error.type` | — |
| `gateway` | — |
| `grpc.status` | — |
| `http.request.method` | — |
| `http.response.status_code` | — |
| `inferencepool.selected_endpoint` | — |
| `jwt.sub` | — |
| `listener` | — |
| `network.protocol.version` | — |
| `protocol` | — |
| `reason` | — |
| `request.id` | Debug logging is enabled for the `request` target. |
| `retry.attempt` | — |
| `route` | — |
| `route_rule` | — |
| `server.address` | — |
| `server.port` | — |
| `span.id` | — |
| `src.identity` | — |
| `tls.sni` | — |
| `trace.id` | — |
| `url.path` | — |
| `url.query` | — |
| `url.scheme` | — |

## LLM

| Attribute | Condition |
| --- | --- |
| `agw.ai.guardrails` | — |
| `agw.ai.original_model` | — |
| `agw.ai.time_per_output_token` | — |
| `agw.ai.time_to_first_token` | — |
| `agw.ai.usage.cost.audio_duration` | — |
| `agw.ai.usage.cost.cache_read` | — |
| `agw.ai.usage.cost.cache_write` | — |
| `agw.ai.usage.cost.input` | — |
| `agw.ai.usage.cost.input_audio` | — |
| `agw.ai.usage.cost.output` | — |
| `agw.ai.usage.cost.output_audio` | — |
| `agw.ai.usage.cost.pages` | — |
| `agw.ai.usage.cost.reasoning` | — |
| `agw.ai.usage.cost.total` | — |
| `gen_ai.embeddings.dimension.count` | — |
| `gen_ai.operation.name` | — |
| `gen_ai.provider.name` | — |
| `gen_ai.request.encoding_formats` | — |
| `gen_ai.request.frequency_penalty` | — |
| `gen_ai.request.max_tokens` | — |
| `gen_ai.request.model` | — |
| `gen_ai.request.presence_penalty` | — |
| `gen_ai.request.seed` | — |
| `gen_ai.request.temperature` | — |
| `gen_ai.request.top_p` | — |
| `gen_ai.response.finish_reasons` | — |
| `gen_ai.response.id` | — |
| `gen_ai.response.model` | — |
| `gen_ai.usage.audio_seconds` | — |
| `gen_ai.usage.cache_creation.input_tokens` | — |
| `gen_ai.usage.cache_read.input_tokens` | — |
| `gen_ai.usage.input_audio_tokens` | — |
| `gen_ai.usage.input_tokens` | — |
| `gen_ai.usage.output_audio_tokens` | — |
| `gen_ai.usage.output_image_tokens` | — |
| `gen_ai.usage.output_tokens` | — |
| `gen_ai.usage.reasoning_tokens` | — |

## MCP

| Attribute | Condition |
| --- | --- |
| `gen_ai.prompt.name` | — |
| `gen_ai.tool.name` | — |
| `mcp.error.code` | — |
| `mcp.error.message` | — |
| `mcp.method.name` | — |
| `mcp.resource.type` | — |
| `mcp.resource.uri` | — |
| `mcp.session.id` | — |
| `mcp.target` | — |

## A2A

| Attribute | Condition |
| --- | --- |
| `a2a.context.id` | — |
| `a2a.method` | — |
| `a2a.response.error_code` | — |
| `a2a.response.outcome` | — |
| `a2a.result.kind` | — |
| `a2a.task.state` | — |

## Substrate

| Attribute | Condition |
| --- | --- |
| `ate.actor.name` | — |
| `ate.actor.uid` | — |
| `ate.atespace` | — |
| `ate.router.resume` | — |
| `ate.router.route.duration` | — |
