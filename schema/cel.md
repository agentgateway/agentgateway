# CEL context Schema

|Field|Description|
|-|-|
|`request`|`request` contains attributes about the incoming HTTP request|
|`request.method`|The HTTP method of the request. For example, `GET`|
|`request.uri`|The complete URI of the request. For example, `http://example.com/path`.|
|`request.host`|The hostname of the request. For example, `example.com`.|
|`request.scheme`|The scheme of the request. For example, `https`.|
|`request.path`|The path of the request URI. For example, `/path`.|
|`request.version`|The version of the request. For example, `HTTP/1.1`.|
|`request.headers`|The headers of the request.|
|`request.body`|The body of the request. Warning: accessing the body will cause the body to be buffered.|
|`request.startTime`|The time the request started|
|`request.endTime`|The time the request completed|
|`response`|`response` contains attributes about the HTTP response|
|`response.code`|The HTTP status code of the response.|
|`response.headers`|The headers of the response.|
|`response.body`|The body of the response. Warning: accessing the body will cause the body to be buffered.|
|`jwt`|`jwt` contains the claims from a verified JWT token. This is only present if the JWT policy is enabled.|
|`apiKey`|`apiKey` contains the claims from a verified API Key. This is only present if the API Key policy is enabled.|
|`apiKey.key`||
|`basicAuth`|`basicAuth` contains the claims from a verified basic authentication Key. This is only present if the Basic authentication policy is enabled.|
|`basicAuth.username`||
|`llm`|`llm` contains attributes about an LLM request or response. This is only present when using an `ai` backend.|
|`llm.streaming`|Whether the LLM response is streamed.|
|`llm.requestModel`|The model requested for the LLM request. This may differ from the actual model used.|
|`llm.responseModel`|The model that actually served the LLM response.|
|`llm.provider`|The provider of the LLM.|
|`llm.inputTokens`|The number of tokens in the input/prompt.|
|`llm.cachedInputTokens`|The number of tokens in the input/prompt read from cache (savings)|
|`llm.cacheCreationInputTokens`|Tokens written to cache (costs)<br>Not present with OpenAI|
|`llm.outputTokens`|The number of tokens in the output/completion.|
|`llm.reasoningTokens`|The number of reasoning tokens in the output/completion.|
|`llm.totalTokens`|The total number of tokens for the request.|
|`llm.countTokens`|The number of tokens in the request, when using the token counting endpoint<br>These are not counted as 'input tokens' since they do not consume input tokens.|
|`llm.prompt`|The prompt sent to the LLM. Warning: accessing this has some performance impacts for large prompts.|
|`llm.prompt[].role`||
|`llm.prompt[].content`||
|`llm.completion`|The completion from the LLM. Warning: accessing this has some performance impacts for large responses.|
|`llm.params`|The parameters for the LLM request.|
|`llm.params.temperature`||
|`llm.params.top_p`||
|`llm.params.frequency_penalty`||
|`llm.params.presence_penalty`||
|`llm.params.seed`||
|`llm.params.max_tokens`||
|`llm.params.encoding_format`||
|`llm.params.dimensions`||
|`llmRequest`|`llm_request` contains the raw LLM request before processing. This is only present *during* LLM policies;<br>policies occurring after the LLM policy, such as logs, will not have this field present even for LLM requests.|
|`source`|`source` contains attributes about the source of the request.|
|`source.address`|The IP address of the downstream connection.|
|`source.port`|The port of the downstream connection.|
|`source.identity`|The (Istio SPIFFE) identity of the downstream connection, if available.|
|`source.identity.trustDomain`|The trust domain of the identity.|
|`source.identity.namespace`|The namespace of the identity.|
|`source.identity.serviceAccount`|The service account of the identity.|
|`source.subjectAltNames`|The subject alt names from the downstream certificate, if available.|
|`source.issuer`|The issuer from the downstream certificate, if available.|
|`source.subject`|The subject from the downstream certificate, if available.|
|`source.subjectCn`|The CN of the subject from the downstream certificate, if available.|
|`mcp`|`mcp` contains attributes about the MCP request.|
|`mcp.(any)(1)tool`||
|`mcp.(any)(1)tool.target`|The target of the resource|
|`mcp.(any)(1)tool.name`|The name of the resource|
|`mcp.(any)(1)prompt`||
|`mcp.(any)(1)prompt.target`|The target of the resource|
|`mcp.(any)(1)prompt.name`|The name of the resource|
|`mcp.(any)(1)resource`||
|`mcp.(any)(1)resource.target`|The target of the resource|
|`mcp.(any)(1)resource.name`|The name of the resource|
|`backend`|`backend` contains information about the backend being used.|
|`backend.name`|The name of the backend being used. For example, `my-service` or `service/my-namespace/my-service:8080`.|
|`backend.type`|The type of backend. For example, `ai`, `mcp`, `static`, `dynamic`, or `service`.|
|`backend.protocol`|The protocol of backend. For example, `http`, `tcp`, `a2a`, `mcp`, or `llm`.|
|`extauthz`|`extauthz` contains dynamic metadata from ext_authz filters|
|`extproc`|`extproc` contains dynamic metadata from ext_proc filters|
