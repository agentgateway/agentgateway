# CEL context Schema

|Field|Type|Description|
|-|-|-|
|`request`|object|`request` contains attributes about the incoming HTTP request|
|`request.method`|string|The HTTP method of the request. For example, `GET`|
|`request.uri`|string|The complete URI of the request. For example, `http://example.com/path`.|
|`request.host`|string|The hostname of the request. For example, `example.com`.|
|`request.scheme`|string|The scheme of the request. For example, `https`.|
|`request.path`|string|The path of the request URI. For example, `/path`.|
|`request.pathAndQuery`|string|The path and query of the request URI. For example, `/path?foo=bar`.|
|`request.version`|string|The version of the request. For example, `HTTP/1.1`.|
|`request.headers`|object|The headers of the request.|
|`request.body`|string|The body of the request. Warning: accessing the body will cause the body to be buffered.|
|`request.startTime`|string|The time the request started|
|`request.endTime`|string|The time the request completed|
|`response`|object|`response` contains attributes about the HTTP response|
|`response.code`|integer|The HTTP status code of the response.|
|`response.headers`|object|The headers of the response.|
|`response.body`|string|The body of the response. Warning: accessing the body will cause the body to be buffered.|
|`env`|object|`env` contains selected process environment attributes exposed to CEL.<br>This does NOT expose raw environment variables, but rather a subset of well-known variables.|
|`env.podName`|string|The name of the pod (when running on Kubernetes)|
|`env.namespace`|string|The namespace of the pod (when running on Kubernetes)|
|`env.gateway`|string|The Gateway we are running as (when running on Kubernetes)|
|`jwt`|object|`jwt` contains the claims from a verified JWT token. This is only present if the JWT policy is enabled.|
|`apiKey`|object|`apiKey` contains the claims from a verified API Key. This is only present if the API Key policy is enabled.|
|`apiKey.key`|string||
|`basicAuth`|object|`basicAuth` contains the claims from a verified basic authentication Key. This is only present if the Basic authentication policy is enabled.|
|`basicAuth.username`|string||
|`llm`|object|`llm` contains attributes about an LLM request or response. This is only present when using an `ai` backend.|
|`llm.streaming`|boolean|Whether the LLM response is streamed.|
|`llm.requestModel`|string|The model requested for the LLM request. This may differ from the actual model used.|
|`llm.responseModel`|string|The model that actually served the LLM response.|
|`llm.provider`|string|The provider of the LLM.|
|`llm.inputTokens`|integer|The number of tokens in the input/prompt.|
|`llm.inputImageTokens`|integer|The number of image tokens in the input/prompt.|
|`llm.inputTextTokens`|integer|The number of text tokens in the input/prompt.<br>Note: this field is only set in multi-modal calls where the total token count is split out by<br>text/image/audio; for standard all-text calls, this is unset.|
|`llm.inputAudioTokens`|integer|The number of audio tokens in the input/prompt.|
|`llm.cachedInputTokens`|integer|The number of tokens in the input/prompt read from cache (savings)|
|`llm.cacheCreationInputTokens`|integer|Tokens written to cache (costs)<br>Not present with OpenAI|
|`llm.outputTokens`|integer|The number of tokens in the output/completion.|
|`llm.outputImageTokens`|integer|The number of image tokens in the output/completion.|
|`llm.outputTextTokens`|integer|The number of text tokens in the output/completion.|
|`llm.outputAudioTokens`|integer|The number of audio tokens in the output/completion.<br>Note: this field is only set in multi-modal calls where the total token count is split out by<br>text/image/audio; for standard all-text calls, this is unset.|
|`llm.reasoningTokens`|integer|The number of reasoning tokens in the output/completion.|
|`llm.totalTokens`|integer|The total number of tokens for the request.|
|`llm.serviceTier`|string|The service tier the provider served the request under.|
|`llm.countTokens`|integer|The number of tokens in the request, when using the token counting endpoint<br>These are not counted as 'input tokens' since they do not consume input tokens.|
|`llm.prompt`|[]object|The prompt sent to the LLM. Warning: accessing this has some performance impacts for large prompts.|
|`llm.prompt[].role`|string||
|`llm.prompt[].content`|string||
|`llm.completion`|[]string|The completion from the LLM. Warning: accessing this has some performance impacts for large responses.|
|`llm.params`|object|The parameters for the LLM request.|
|`llm.params.temperature`|number||
|`llm.params.top_p`|number||
|`llm.params.frequency_penalty`|number||
|`llm.params.presence_penalty`|number||
|`llm.params.seed`|integer||
|`llm.params.max_tokens`|integer||
|`llm.params.encoding_format`|string||
|`llm.params.dimensions`|integer||
|`llmRequest`|any|`llmRequest` contains the raw LLM request before processing. This is only present *during* LLM policies;<br>policies occurring after the LLM policy, such as logs, will not have this field present even for LLM requests.|
|`source`|object|`source` contains attributes about the source of the request.|
|`source.address`|string|The IP address of the downstream connection.|
|`source.port`|integer|The port of the downstream connection.|
|`source.rawAddress`|string|The original TCP peer IP address of the downstream connection.<br>This can differ from the `address` when using tunneling protocols like PROXY.|
|`source.rawPort`|integer|The original TCP peer port of the downstream connection.<br>This can differ from the `port` when using tunneling protocols like PROXY.|
|`source.identity`|object|The (Istio SPIFFE) identity of the downstream connection, if available.|
|`source.identity.trustDomain`|string|The trust domain of the identity.|
|`source.identity.namespace`|string|The namespace of the identity.|
|`source.identity.serviceAccount`|string|The service account of the identity.|
|`source.subjectAltNames`|[]string|The subject alt names from the downstream certificate, if available.|
|`source.issuer`|string|The issuer from the downstream certificate, if available.|
|`source.subject`|string|The subject from the downstream certificate, if available.|
|`source.subjectCn`|string|The CN of the subject from the downstream certificate, if available.|
|`source.unverifiedWorkload`|object|The workload context of the downstream connection, resolved from the<br>workload discovery store by source IP. Available when the source pod is<br>known to the controller's workload discovery store.<br><br>Fields are nested under `unverified` to signal that they are derived<br>from the source IP (not cryptographically authenticated). Policy<br>authors should prefer `source.identity.*` for trust-sensitive checks.|
|`source.unverifiedWorkload.name`|string|The pod name of the source workload.|
|`source.unverifiedWorkload.namespace`|string|The namespace of the source workload.|
|`source.unverifiedWorkload.serviceAccount`|string|The service account of the source workload.|
|`mcp`|object|`mcp` contains attributes about the MCP request.<br>Request-time CEL only includes identity fields such as `tool`, `prompt`, or `resource`.<br>Post-request CEL may also include fields like `methodName`, `sessionId`, and tool payloads.|
|`mcp.methodName`|string||
|`mcp.sessionId`|string||
|`mcp.tool`|object||
|`mcp.tool.target`|string|The target handling the tool call after multiplexing resolution.|
|`mcp.tool.name`|string|The resolved tool name sent to the upstream target.|
|`mcp.tool.arguments`|object|The JSON arguments passed to the tool call.|
|`mcp.tool.result`|any|The terminal tool result payload, if available.|
|`mcp.tool.error`|any|The terminal JSON-RPC error payload, if available.|
|`mcp.prompt`|object||
|`mcp.prompt.target`|string|The target of the resource|
|`mcp.prompt.name`|string|The name of the resource|
|`mcp.resource`|object||
|`mcp.resource.target`|string|The target of the resource|
|`mcp.resource.name`|string|The name of the resource|
|`backend`|object|`backend` contains information about the backend being used.|
|`backend.name`|string|The name of the backend being used. For example, `my-service` or `service/my-namespace/my-service:8080`.|
|`backend.type`|string|The type of backend. For example, `ai`, `mcp`, `static`, `dynamic`, or `service`.|
|`backend.protocol`|string|The protocol of backend. For example, `http`, `tcp`, `a2a`, `mcp`, or `llm`.|
|`extauthz`|object|`extauthz` contains dynamic metadata from ext_authz filters|
|`extproc`|object|`extproc` contains dynamic metadata from ext_proc filters|
|`metadata`|object|`metadata` contains values set by transformation metadata expressions.|
