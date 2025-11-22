# Schemas
This folder contains JSON schemas for various parts of the project

## Configuration File

|Field|Description|
|-|-|
|`config`||
|`config.enableIpv6`||
|`config.localXdsPath`|Local XDS path. If not specified, the current configuration file will be used.|
|`config.caAddress`||
|`config.caAuthToken`||
|`config.xdsAddress`||
|`config.xdsAuthToken`||
|`config.namespace`||
|`config.gateway`||
|`config.trustDomain`||
|`config.serviceAccount`||
|`config.clusterId`||
|`config.network`||
|`config.adminAddr`|Admin UI address in the format "ip:port"|
|`config.statsAddr`|Stats/metrics server address in the format "ip:port"|
|`config.readinessAddr`|Readiness probe server address in the format "ip:port"|
|`config.connectionTerminationDeadline`||
|`config.connectionMinTerminationDeadline`||
|`config.workerThreads`||
|`config.tracing`||
|`config.tracing.otlpEndpoint`||
|`config.tracing.headers`||
|`config.tracing.otlpProtocol`||
|`config.tracing.fields`||
|`config.tracing.fields.remove`||
|`config.tracing.fields.add`||
|`config.tracing.randomSampling`|Expression to determine the amount of *random sampling*.<br>Random sampling will initiate a new trace span if the incoming request does not have a trace already.<br>This should evaluate to either a float between 0.0-1.0 (0-100%) or true/false.<br>This defaults to 'false'.|
|`config.tracing.clientSampling`|Expression to determine the amount of *client sampling*.<br>Client sampling determines whether to initiate a new trace span if the incoming request does have a trace already.<br>This should evaluate to either a float between 0.0-1.0 (0-100%) or true/false.<br>This defaults to 'true'.|
|`config.logging`||
|`config.logging.filter`||
|`config.logging.fields`||
|`config.logging.fields.remove`||
|`config.logging.fields.add`||
|`config.logging.level`||
|`config.logging.format`||
|`config.metrics`||
|`config.metrics.remove`||
|`config.metrics.fields`||
|`config.metrics.fields.add`||
|`config.backend`||
|`config.backend.keepalives`||
|`config.backend.keepalives.enabled`||
|`config.backend.keepalives.time`||
|`config.backend.keepalives.interval`||
|`config.backend.keepalives.retries`||
|`config.backend.connectTimeout`||
|`config.backend.poolIdleTimeout`|The maximum duration to keep an idle connection alive.|
|`config.backend.poolMaxSize`|The maximum number of connections allowed in the pool, per hostname. If set, this will limit<br>the total number of connections kept alive to any given host.<br>Note: excess connections will still be created, they will just not remain idle.<br>If unset, there is no limit|
|`config.hbone`||
|`config.hbone.windowSize`||
|`config.hbone.connectionWindowSize`||
|`config.hbone.frameSize`||
|`config.hbone.poolMaxStreamsPerConn`||
|`config.hbone.poolUnusedReleaseTimeout`||
|`binds`||
|`binds[].port`||
|`binds[].listeners`||
|`binds[].listeners[].name`||
|`binds[].listeners[].gatewayName`||
|`binds[].listeners[].hostname`|Can be a wildcard|
|`binds[].listeners[].protocol`||
|`binds[].listeners[].tls`||
|`binds[].listeners[].tls.cert`||
|`binds[].listeners[].tls.key`||
|`binds[].listeners[].routes`||
|`binds[].listeners[].routes[].name`||
|`binds[].listeners[].routes[].ruleName`||
|`binds[].listeners[].routes[].hostnames`|Can be a wildcard|
|`binds[].listeners[].routes[].matches`||
|`binds[].listeners[].routes[].matches[].headers`||
|`binds[].listeners[].routes[].matches[].headers[].name`||
|`binds[].listeners[].routes[].matches[].headers[].value`||
|`binds[].listeners[].routes[].matches[].headers[].value.(1)exact`||
|`binds[].listeners[].routes[].matches[].headers[].value.(1)regex`||
|`binds[].listeners[].routes[].matches[].path`||
|`binds[].listeners[].routes[].matches[].path.(1)exact`||
|`binds[].listeners[].routes[].matches[].path.(1)pathPrefix`||
|`binds[].listeners[].routes[].matches[].path.(1)regex`||
|`binds[].listeners[].routes[].matches[].method`||
|`binds[].listeners[].routes[].matches[].query`||
|`binds[].listeners[].routes[].matches[].query[].name`||
|`binds[].listeners[].routes[].matches[].query[].value`||
|`binds[].listeners[].routes[].matches[].query[].value.(1)exact`||
|`binds[].listeners[].routes[].matches[].query[].value.(1)regex`||
|`binds[].listeners[].routes[].policies`||
|`binds[].listeners[].routes[].policies.requestHeaderModifier`|Headers to be modified in the request.|
|`binds[].listeners[].routes[].policies.requestHeaderModifier.add`||
|`binds[].listeners[].routes[].policies.requestHeaderModifier.set`||
|`binds[].listeners[].routes[].policies.requestHeaderModifier.remove`||
|`binds[].listeners[].routes[].policies.responseHeaderModifier`|Headers to be modified in the response.|
|`binds[].listeners[].routes[].policies.responseHeaderModifier.add`||
|`binds[].listeners[].routes[].policies.responseHeaderModifier.set`||
|`binds[].listeners[].routes[].policies.responseHeaderModifier.remove`||
|`binds[].listeners[].routes[].policies.requestRedirect`|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].policies.requestRedirect.scheme`||
|`binds[].listeners[].routes[].policies.requestRedirect.authority`||
|`binds[].listeners[].routes[].policies.requestRedirect.authority.(any)(1)full`||
|`binds[].listeners[].routes[].policies.requestRedirect.authority.(any)(1)host`||
|`binds[].listeners[].routes[].policies.requestRedirect.authority.(any)(1)port`||
|`binds[].listeners[].routes[].policies.requestRedirect.path`||
|`binds[].listeners[].routes[].policies.requestRedirect.path.(any)(1)full`||
|`binds[].listeners[].routes[].policies.requestRedirect.path.(any)(1)prefix`||
|`binds[].listeners[].routes[].policies.requestRedirect.status`||
|`binds[].listeners[].routes[].policies.urlRewrite`|Modify the URL path or authority.|
|`binds[].listeners[].routes[].policies.urlRewrite.authority`||
|`binds[].listeners[].routes[].policies.urlRewrite.authority.(any)(1)full`||
|`binds[].listeners[].routes[].policies.urlRewrite.authority.(any)(1)host`||
|`binds[].listeners[].routes[].policies.urlRewrite.authority.(any)(1)port`||
|`binds[].listeners[].routes[].policies.urlRewrite.path`||
|`binds[].listeners[].routes[].policies.urlRewrite.path.(any)(1)full`||
|`binds[].listeners[].routes[].policies.urlRewrite.path.(any)(1)prefix`||
|`binds[].listeners[].routes[].policies.requestMirror`|Mirror incoming requests to another destination.|
|`binds[].listeners[].routes[].policies.requestMirror.backend`||
|`binds[].listeners[].routes[].policies.requestMirror.backend.(1)service`||
|`binds[].listeners[].routes[].policies.requestMirror.backend.(1)service.name`||
|`binds[].listeners[].routes[].policies.requestMirror.backend.(1)service.name.namespace`||
|`binds[].listeners[].routes[].policies.requestMirror.backend.(1)service.name.hostname`||
|`binds[].listeners[].routes[].policies.requestMirror.backend.(1)service.port`||
|`binds[].listeners[].routes[].policies.requestMirror.backend.(1)host`|Hostname or IP address|
|`binds[].listeners[].routes[].policies.requestMirror.backend.(1)backend`|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].policies.requestMirror.percentage`||
|`binds[].listeners[].routes[].policies.directResponse`|Directly respond to the request with a static response.|
|`binds[].listeners[].routes[].policies.directResponse.body`||
|`binds[].listeners[].routes[].policies.directResponse.status`||
|`binds[].listeners[].routes[].policies.cors`|Handle CORS preflight requests and append configured CORS headers to applicable requests.|
|`binds[].listeners[].routes[].policies.cors.allowCredentials`||
|`binds[].listeners[].routes[].policies.cors.allowHeaders`||
|`binds[].listeners[].routes[].policies.cors.allowMethods`||
|`binds[].listeners[].routes[].policies.cors.allowOrigins`||
|`binds[].listeners[].routes[].policies.cors.exposeHeaders`||
|`binds[].listeners[].routes[].policies.cors.maxAge`||
|`binds[].listeners[].routes[].policies.mcpAuthorization`|Authorization policies for MCP access.|
|`binds[].listeners[].routes[].policies.mcpAuthorization.rules`||
|`binds[].listeners[].routes[].policies.authorization`|Authorization policies for HTTP access.|
|`binds[].listeners[].routes[].policies.authorization.rules`||
|`binds[].listeners[].routes[].policies.mcpAuthentication`|Authentication for MCP clients.|
|`binds[].listeners[].routes[].policies.mcpAuthentication.issuer`||
|`binds[].listeners[].routes[].policies.mcpAuthentication.audiences`||
|`binds[].listeners[].routes[].policies.mcpAuthentication.provider`||
|`binds[].listeners[].routes[].policies.mcpAuthentication.provider.(any)(1)auth0`||
|`binds[].listeners[].routes[].policies.mcpAuthentication.provider.(any)(1)keycloak`||
|`binds[].listeners[].routes[].policies.mcpAuthentication.resourceMetadata`||
|`binds[].listeners[].routes[].policies.mcpAuthentication.jwks`||
|`binds[].listeners[].routes[].policies.mcpAuthentication.jwks.(any)file`||
|`binds[].listeners[].routes[].policies.mcpAuthentication.jwks.(any)url`||
|`binds[].listeners[].routes[].policies.a2a`|Mark this traffic as A2A to enable A2A processing and telemetry.|
|`binds[].listeners[].routes[].policies.ai`|Mark this as LLM traffic to enable LLM processing.|
|`binds[].listeners[].routes[].policies.ai.promptGuard`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].(1)regex`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].(1)regex.action`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].(1)regex.rules`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].(1)regex.rules[].(any)builtin`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].(1)regex.rules[].(any)pattern`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].(1)webhook`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].(1)webhook.target`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].(1)webhook.target.(1)service`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].(1)webhook.target.(1)service.name`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].(1)webhook.target.(1)service.name.namespace`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].(1)webhook.target.(1)service.name.hostname`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].(1)webhook.target.(1)service.port`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].(1)webhook.target.(1)host`|Hostname or IP address|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].(1)webhook.target.(1)backend`|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].(1)webhook.forwardHeaderMatches`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].(1)webhook.forwardHeaderMatches[].name`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].(1)webhook.forwardHeaderMatches[].value`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].(1)webhook.forwardHeaderMatches[].value.(1)exact`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].(1)webhook.forwardHeaderMatches[].value.(1)regex`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].(1)openAIModeration`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].(1)openAIModeration.model`|Model to use. Defaults to `omni-moderation-latest`|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].rejection`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].rejection.body`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].rejection.status`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].rejection.headers`|Optional headers to add, set, or remove from the rejection response|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].rejection.headers.add`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].rejection.headers.set`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].rejection.headers.remove`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].rejection`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].rejection.body`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].rejection.status`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].rejection.headers`|Optional headers to add, set, or remove from the rejection response|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].rejection.headers.add`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].rejection.headers.set`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].rejection.headers.remove`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].kind`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].kind.(1)regex`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].kind.(1)regex.action`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].kind.(1)regex.rules`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].kind.(1)regex.rules[].(any)builtin`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].kind.(1)regex.rules[].(any)pattern`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].kind.(1)webhook`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].kind.(1)webhook.target`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].kind.(1)webhook.target.(1)service`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].kind.(1)webhook.target.(1)service.name`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].kind.(1)webhook.target.(1)service.name.namespace`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].kind.(1)webhook.target.(1)service.name.hostname`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].kind.(1)webhook.target.(1)service.port`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].kind.(1)webhook.target.(1)host`|Hostname or IP address|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].kind.(1)webhook.target.(1)backend`|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].kind.(1)webhook.forwardHeaderMatches`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].kind.(1)webhook.forwardHeaderMatches[].name`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].kind.(1)webhook.forwardHeaderMatches[].value`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].kind.(1)webhook.forwardHeaderMatches[].value.(1)exact`||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].kind.(1)webhook.forwardHeaderMatches[].value.(1)regex`||
|`binds[].listeners[].routes[].policies.ai.defaults`||
|`binds[].listeners[].routes[].policies.ai.overrides`||
|`binds[].listeners[].routes[].policies.ai.prompts`||
|`binds[].listeners[].routes[].policies.ai.prompts.append`||
|`binds[].listeners[].routes[].policies.ai.prompts.append[].role`||
|`binds[].listeners[].routes[].policies.ai.prompts.append[].content`||
|`binds[].listeners[].routes[].policies.ai.prompts.prepend`||
|`binds[].listeners[].routes[].policies.ai.prompts.prepend[].role`||
|`binds[].listeners[].routes[].policies.ai.prompts.prepend[].content`||
|`binds[].listeners[].routes[].policies.ai.modelAliases`||
|`binds[].listeners[].routes[].policies.ai.promptCaching`||
|`binds[].listeners[].routes[].policies.ai.promptCaching.cacheSystem`||
|`binds[].listeners[].routes[].policies.ai.promptCaching.cacheMessages`||
|`binds[].listeners[].routes[].policies.ai.promptCaching.cacheTools`||
|`binds[].listeners[].routes[].policies.ai.promptCaching.minTokens`||
|`binds[].listeners[].routes[].policies.backendTLS`|Send TLS to the backend.|
|`binds[].listeners[].routes[].policies.backendTLS.cert`||
|`binds[].listeners[].routes[].policies.backendTLS.key`||
|`binds[].listeners[].routes[].policies.backendTLS.root`||
|`binds[].listeners[].routes[].policies.backendTLS.hostname`||
|`binds[].listeners[].routes[].policies.backendTLS.insecure`||
|`binds[].listeners[].routes[].policies.backendTLS.insecureHost`||
|`binds[].listeners[].routes[].policies.backendTLS.alpn`||
|`binds[].listeners[].routes[].policies.backendTLS.subjectAltNames`||
|`binds[].listeners[].routes[].policies.backendAuth`|Authenticate to the backend.|
|`binds[].listeners[].routes[].policies.backendAuth.(any)(1)passthrough`||
|`binds[].listeners[].routes[].policies.backendAuth.(any)(1)key`||
|`binds[].listeners[].routes[].policies.backendAuth.(any)(1)key.(any)file`||
|`binds[].listeners[].routes[].policies.backendAuth.(any)(1)gcp`||
|`binds[].listeners[].routes[].policies.backendAuth.(any)(1)aws`||
|`binds[].listeners[].routes[].policies.backendAuth.(any)(1)aws.(any)accessKeyId`||
|`binds[].listeners[].routes[].policies.backendAuth.(any)(1)aws.(any)secretAccessKey`||
|`binds[].listeners[].routes[].policies.backendAuth.(any)(1)aws.(any)region`||
|`binds[].listeners[].routes[].policies.backendAuth.(any)(1)aws.(any)sessionToken`||
|`binds[].listeners[].routes[].policies.backendAuth.(any)(1)azure`||
|`binds[].listeners[].routes[].policies.backendAuth.(any)(1)azure.(1)explicitConfig`||
|`binds[].listeners[].routes[].policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)clientSecret`||
|`binds[].listeners[].routes[].policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)clientSecret.tenant_id`||
|`binds[].listeners[].routes[].policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)clientSecret.client_id`||
|`binds[].listeners[].routes[].policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)clientSecret.client_secret`||
|`binds[].listeners[].routes[].policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)managedIdentity`||
|`binds[].listeners[].routes[].policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)managedIdentity.userAssignedIdentity`||
|`binds[].listeners[].routes[].policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)managedIdentity.userAssignedIdentity.(any)(1)clientId`||
|`binds[].listeners[].routes[].policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)managedIdentity.userAssignedIdentity.(any)(1)objectId`||
|`binds[].listeners[].routes[].policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)managedIdentity.userAssignedIdentity.(any)(1)resourceId`||
|`binds[].listeners[].routes[].policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)workloadIdentity`||
|`binds[].listeners[].routes[].policies.backendAuth.(any)(1)azure.(1)developerImplicit`||
|`binds[].listeners[].routes[].policies.localRateLimit`|Rate limit incoming requests. State is kept local.|
|`binds[].listeners[].routes[].policies.localRateLimit[].maxTokens`||
|`binds[].listeners[].routes[].policies.localRateLimit[].tokensPerFill`||
|`binds[].listeners[].routes[].policies.localRateLimit[].fillInterval`||
|`binds[].listeners[].routes[].policies.localRateLimit[].type`||
|`binds[].listeners[].routes[].policies.remoteRateLimit`|Rate limit incoming requests. State is managed by a remote server.|
|`binds[].listeners[].routes[].policies.remoteRateLimit.(any)(1)service`||
|`binds[].listeners[].routes[].policies.remoteRateLimit.(any)(1)service.name`||
|`binds[].listeners[].routes[].policies.remoteRateLimit.(any)(1)service.name.namespace`||
|`binds[].listeners[].routes[].policies.remoteRateLimit.(any)(1)service.name.hostname`||
|`binds[].listeners[].routes[].policies.remoteRateLimit.(any)(1)service.port`||
|`binds[].listeners[].routes[].policies.remoteRateLimit.(any)(1)host`|Hostname or IP address|
|`binds[].listeners[].routes[].policies.remoteRateLimit.(any)(1)backend`|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].policies.remoteRateLimit.(any)domain`||
|`binds[].listeners[].routes[].policies.remoteRateLimit.(any)descriptors`||
|`binds[].listeners[].routes[].policies.remoteRateLimit.(any)descriptors[].entries`||
|`binds[].listeners[].routes[].policies.remoteRateLimit.(any)descriptors[].entries[].key`||
|`binds[].listeners[].routes[].policies.remoteRateLimit.(any)descriptors[].entries[].value`||
|`binds[].listeners[].routes[].policies.remoteRateLimit.(any)descriptors[].type`||
|`binds[].listeners[].routes[].policies.jwtAuth`|Authenticate incoming JWT requests.|
|`binds[].listeners[].routes[].policies.jwtAuth.(any)(any)mode`||
|`binds[].listeners[].routes[].policies.jwtAuth.(any)(any)providers`||
|`binds[].listeners[].routes[].policies.jwtAuth.(any)(any)providers[].issuer`||
|`binds[].listeners[].routes[].policies.jwtAuth.(any)(any)providers[].audiences`||
|`binds[].listeners[].routes[].policies.jwtAuth.(any)(any)providers[].jwks`||
|`binds[].listeners[].routes[].policies.jwtAuth.(any)(any)providers[].jwks.(any)file`||
|`binds[].listeners[].routes[].policies.jwtAuth.(any)(any)providers[].jwks.(any)url`||
|`binds[].listeners[].routes[].policies.jwtAuth.(any)(any)mode`||
|`binds[].listeners[].routes[].policies.jwtAuth.(any)(any)issuer`||
|`binds[].listeners[].routes[].policies.jwtAuth.(any)(any)audiences`||
|`binds[].listeners[].routes[].policies.jwtAuth.(any)(any)jwks`||
|`binds[].listeners[].routes[].policies.jwtAuth.(any)(any)jwks.(any)file`||
|`binds[].listeners[].routes[].policies.jwtAuth.(any)(any)jwks.(any)url`||
|`binds[].listeners[].routes[].policies.basicAuth`|Authenticate incoming requests using Basic Authentication with htpasswd.|
|`binds[].listeners[].routes[].policies.basicAuth.htpasswd`|.htpasswd file contents/reference|
|`binds[].listeners[].routes[].policies.basicAuth.htpasswd.(any)file`||
|`binds[].listeners[].routes[].policies.basicAuth.realm`|Realm name for the WWW-Authenticate header|
|`binds[].listeners[].routes[].policies.basicAuth.mode`|Validation mode for basic authentication|
|`binds[].listeners[].routes[].policies.apiKey`|Authenticate incoming requests using API Keys|
|`binds[].listeners[].routes[].policies.apiKey.keys`|List of API keys|
|`binds[].listeners[].routes[].policies.apiKey.keys[].key`||
|`binds[].listeners[].routes[].policies.apiKey.keys[].metadata`||
|`binds[].listeners[].routes[].policies.apiKey.mode`|Validation mode for API keys|
|`binds[].listeners[].routes[].policies.extAuthz`|Authenticate incoming requests by calling an external authorization server.|
|`binds[].listeners[].routes[].policies.extAuthz.(any)(1)service`||
|`binds[].listeners[].routes[].policies.extAuthz.(any)(1)service.name`||
|`binds[].listeners[].routes[].policies.extAuthz.(any)(1)service.name.namespace`||
|`binds[].listeners[].routes[].policies.extAuthz.(any)(1)service.name.hostname`||
|`binds[].listeners[].routes[].policies.extAuthz.(any)(1)service.port`||
|`binds[].listeners[].routes[].policies.extAuthz.(any)(1)host`|Hostname or IP address|
|`binds[].listeners[].routes[].policies.extAuthz.(any)(1)backend`|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].policies.extAuthz.(any)context`|Additional context to send to the authorization service.<br>This maps to the `context_extensions` field of the request, and only allows static values.|
|`binds[].listeners[].routes[].policies.extAuthz.(any)metadata`|Additional metadata to send to the authorization service.<br>This maps to the `metadata_context.filter_metadata` field of the request, and allows dynamic CEL expressions.<br>If unset, by default the `envoy.filters.http.jwt_authn` key is set if the JWT policy is used as well, for compatibility.|
|`binds[].listeners[].routes[].policies.extAuthz.(any)failureMode`|Behavior when the authorization service is unavailable or returns an error|
|`binds[].listeners[].routes[].policies.extAuthz.(any)failureMode.(1)denyWithStatus`||
|`binds[].listeners[].routes[].policies.extAuthz.(any)includeRequestHeaders`|Specific headers to include in the authorization request (empty = all headers)|
|`binds[].listeners[].routes[].policies.extAuthz.(any)includeRequestBody`|Options for including the request body in the authorization request|
|`binds[].listeners[].routes[].policies.extAuthz.(any)includeRequestBody.maxRequestBytes`|Maximum size of request body to buffer (default: 8192)|
|`binds[].listeners[].routes[].policies.extAuthz.(any)includeRequestBody.allowPartialMessage`|If true, send partial body when max_request_bytes is reached|
|`binds[].listeners[].routes[].policies.extAuthz.(any)includeRequestBody.packAsBytes`|If true, pack body as raw bytes in gRPC|
|`binds[].listeners[].routes[].policies.extAuthz.(any)timeout`|Timeout for the authorization request (default: 200ms)|
|`binds[].listeners[].routes[].policies.extProc`|Extend agentgateway with an external processor|
|`binds[].listeners[].routes[].policies.extProc.(any)(1)service`||
|`binds[].listeners[].routes[].policies.extProc.(any)(1)service.name`||
|`binds[].listeners[].routes[].policies.extProc.(any)(1)service.name.namespace`||
|`binds[].listeners[].routes[].policies.extProc.(any)(1)service.name.hostname`||
|`binds[].listeners[].routes[].policies.extProc.(any)(1)service.port`||
|`binds[].listeners[].routes[].policies.extProc.(any)(1)host`|Hostname or IP address|
|`binds[].listeners[].routes[].policies.extProc.(any)(1)backend`|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].policies.extProc.(any)failureMode`||
|`binds[].listeners[].routes[].policies.transformations`|Modify requests and responses|
|`binds[].listeners[].routes[].policies.transformations.request`||
|`binds[].listeners[].routes[].policies.transformations.request.add`||
|`binds[].listeners[].routes[].policies.transformations.request.set`||
|`binds[].listeners[].routes[].policies.transformations.request.remove`||
|`binds[].listeners[].routes[].policies.transformations.request.body`||
|`binds[].listeners[].routes[].policies.transformations.response`||
|`binds[].listeners[].routes[].policies.transformations.response.add`||
|`binds[].listeners[].routes[].policies.transformations.response.set`||
|`binds[].listeners[].routes[].policies.transformations.response.remove`||
|`binds[].listeners[].routes[].policies.transformations.response.body`||
|`binds[].listeners[].routes[].policies.csrf`|Handle CSRF protection by validating request origins against configured allowed origins.|
|`binds[].listeners[].routes[].policies.csrf.additionalOrigins`||
|`binds[].listeners[].routes[].policies.timeout`|Timeout requests that exceed the configured duration.|
|`binds[].listeners[].routes[].policies.timeout.requestTimeout`||
|`binds[].listeners[].routes[].policies.timeout.backendRequestTimeout`||
|`binds[].listeners[].routes[].policies.retry`|Retry matching requests.|
|`binds[].listeners[].routes[].policies.retry.attempts`||
|`binds[].listeners[].routes[].policies.retry.backoff`||
|`binds[].listeners[].routes[].policies.retry.codes`||
|`binds[].listeners[].routes[].backends`||
|`binds[].listeners[].routes[].backends[].(1)service`||
|`binds[].listeners[].routes[].backends[].(1)service.name`||
|`binds[].listeners[].routes[].backends[].(1)service.name.namespace`||
|`binds[].listeners[].routes[].backends[].(1)service.name.hostname`||
|`binds[].listeners[].routes[].backends[].(1)service.port`||
|`binds[].listeners[].routes[].backends[].(1)host`||
|`binds[].listeners[].routes[].backends[].(1)dynamic`||
|`binds[].listeners[].routes[].backends[].(1)mcp`||
|`binds[].listeners[].routes[].backends[].(1)mcp.targets`||
|`binds[].listeners[].routes[].backends[].(1)mcp.targets[].(1)sse`||
|`binds[].listeners[].routes[].backends[].(1)mcp.targets[].(1)sse.host`||
|`binds[].listeners[].routes[].backends[].(1)mcp.targets[].(1)sse.port`||
|`binds[].listeners[].routes[].backends[].(1)mcp.targets[].(1)sse.path`||
|`binds[].listeners[].routes[].backends[].(1)mcp.targets[].(1)mcp`||
|`binds[].listeners[].routes[].backends[].(1)mcp.targets[].(1)mcp.host`||
|`binds[].listeners[].routes[].backends[].(1)mcp.targets[].(1)mcp.port`||
|`binds[].listeners[].routes[].backends[].(1)mcp.targets[].(1)mcp.path`||
|`binds[].listeners[].routes[].backends[].(1)mcp.targets[].(1)stdio`||
|`binds[].listeners[].routes[].backends[].(1)mcp.targets[].(1)stdio.cmd`||
|`binds[].listeners[].routes[].backends[].(1)mcp.targets[].(1)stdio.args`||
|`binds[].listeners[].routes[].backends[].(1)mcp.targets[].(1)stdio.env`||
|`binds[].listeners[].routes[].backends[].(1)mcp.targets[].(1)openapi`||
|`binds[].listeners[].routes[].backends[].(1)mcp.targets[].(1)openapi.host`||
|`binds[].listeners[].routes[].backends[].(1)mcp.targets[].(1)openapi.port`||
|`binds[].listeners[].routes[].backends[].(1)mcp.targets[].(1)openapi.path`||
|`binds[].listeners[].routes[].backends[].(1)mcp.targets[].(1)openapi.schema`||
|`binds[].listeners[].routes[].backends[].(1)mcp.targets[].name`||
|`binds[].listeners[].routes[].backends[].(1)mcp.statefulMode`||
|`binds[].listeners[].routes[].backends[].(1)mcp.prefixMode`||
|`binds[].listeners[].routes[].backends[].(1)ai`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)name`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)provider`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)provider.(1)openAI`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)provider.(1)openAI.model`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)provider.(1)gemini`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)provider.(1)gemini.model`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)provider.(1)vertex`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)provider.(1)vertex.model`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)provider.(1)vertex.region`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)provider.(1)vertex.projectId`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)provider.(1)anthropic`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)provider.(1)anthropic.model`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)provider.(1)bedrock`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)provider.(1)bedrock.model`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)provider.(1)bedrock.region`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)provider.(1)bedrock.guardrailIdentifier`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)provider.(1)bedrock.guardrailVersion`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)provider.(1)azureOpenAI`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)provider.(1)azureOpenAI.model`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)provider.(1)azureOpenAI.host`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)provider.(1)azureOpenAI.apiVersion`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)hostOverride`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)pathOverride`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)tokenize`|Whether to tokenize on the request flow. This enables us to do more accurate rate limits,<br>since we know (part of) the cost of the request upfront.<br>This comes with the cost of an expensive operation.|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)routes`|Routes defines how to identify the type of traffic we should handle<br>The keys are URL suffix matches, like `/v1/models`. The special `*` can be used to match anything.|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.requestHeaderModifier`|Headers to be modified in the request.|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.requestHeaderModifier.add`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.requestHeaderModifier.set`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.requestHeaderModifier.remove`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.responseHeaderModifier`|Headers to be modified in the response.|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.responseHeaderModifier.add`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.responseHeaderModifier.set`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.responseHeaderModifier.remove`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.requestRedirect`|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.requestRedirect.scheme`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.requestRedirect.authority`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.requestRedirect.authority.(any)(1)full`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.requestRedirect.authority.(any)(1)host`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.requestRedirect.authority.(any)(1)port`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.requestRedirect.path`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.requestRedirect.path.(any)(1)full`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.requestRedirect.path.(any)(1)prefix`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.requestRedirect.status`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.mcpAuthorization`|Authorization policies for MCP access.|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.mcpAuthorization.rules`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.a2a`|Mark this traffic as A2A to enable A2A processing and telemetry.|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai`|Mark this as LLM traffic to enable LLM processing.|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.request`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.request[].(1)regex`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.request[].(1)regex.action`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.request[].(1)regex.rules`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.request[].(1)regex.rules[].(any)builtin`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.request[].(1)regex.rules[].(any)pattern`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.request[].(1)webhook`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.request[].(1)webhook.target`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.request[].(1)webhook.target.(1)service`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.request[].(1)webhook.target.(1)service.name`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.request[].(1)webhook.target.(1)service.name.namespace`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.request[].(1)webhook.target.(1)service.name.hostname`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.request[].(1)webhook.target.(1)service.port`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.request[].(1)webhook.target.(1)host`|Hostname or IP address|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.request[].(1)webhook.target.(1)backend`|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.request[].(1)webhook.forwardHeaderMatches`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.request[].(1)webhook.forwardHeaderMatches[].name`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.request[].(1)webhook.forwardHeaderMatches[].value`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.request[].(1)webhook.forwardHeaderMatches[].value.(1)exact`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.request[].(1)webhook.forwardHeaderMatches[].value.(1)regex`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.request[].(1)openAIModeration`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.request[].(1)openAIModeration.model`|Model to use. Defaults to `omni-moderation-latest`|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.request[].rejection`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.request[].rejection.body`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.request[].rejection.status`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.request[].rejection.headers`|Optional headers to add, set, or remove from the rejection response|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.request[].rejection.headers.add`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.request[].rejection.headers.set`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.request[].rejection.headers.remove`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.response`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.response[].rejection`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.response[].rejection.body`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.response[].rejection.status`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.response[].rejection.headers`|Optional headers to add, set, or remove from the rejection response|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.response[].rejection.headers.add`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.response[].rejection.headers.set`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.response[].rejection.headers.remove`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.response[].kind`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.response[].kind.(1)regex`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.response[].kind.(1)regex.action`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.response[].kind.(1)regex.rules`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.response[].kind.(1)regex.rules[].(any)builtin`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.response[].kind.(1)regex.rules[].(any)pattern`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.response[].kind.(1)webhook`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.response[].kind.(1)webhook.target`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.response[].kind.(1)webhook.target.(1)service`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.response[].kind.(1)webhook.target.(1)service.name`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.response[].kind.(1)webhook.target.(1)service.name.namespace`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.response[].kind.(1)webhook.target.(1)service.name.hostname`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.response[].kind.(1)webhook.target.(1)service.port`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.response[].kind.(1)webhook.target.(1)host`|Hostname or IP address|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.response[].kind.(1)webhook.target.(1)backend`|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.response[].kind.(1)webhook.forwardHeaderMatches`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.response[].kind.(1)webhook.forwardHeaderMatches[].name`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.response[].kind.(1)webhook.forwardHeaderMatches[].value`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.response[].kind.(1)webhook.forwardHeaderMatches[].value.(1)exact`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptGuard.response[].kind.(1)webhook.forwardHeaderMatches[].value.(1)regex`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.defaults`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.overrides`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.prompts`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.prompts.append`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.prompts.append[].role`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.prompts.append[].content`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.prompts.prepend`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.prompts.prepend[].role`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.prompts.prepend[].content`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.modelAliases`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptCaching`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptCaching.cacheSystem`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptCaching.cacheMessages`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptCaching.cacheTools`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.ai.promptCaching.minTokens`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendTLS`|Send TLS to the backend.|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendTLS.cert`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendTLS.key`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendTLS.root`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendTLS.hostname`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendTLS.insecure`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendTLS.insecureHost`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendTLS.alpn`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendTLS.subjectAltNames`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendAuth`|Authenticate to the backend.|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendAuth.(any)(1)passthrough`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendAuth.(any)(1)key`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendAuth.(any)(1)key.(any)file`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendAuth.(any)(1)gcp`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendAuth.(any)(1)aws`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendAuth.(any)(1)aws.(any)accessKeyId`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendAuth.(any)(1)aws.(any)secretAccessKey`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendAuth.(any)(1)aws.(any)region`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendAuth.(any)(1)aws.(any)sessionToken`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendAuth.(any)(1)azure`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendAuth.(any)(1)azure.(1)explicitConfig`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)clientSecret`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)clientSecret.tenant_id`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)clientSecret.client_id`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)clientSecret.client_secret`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)managedIdentity`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)managedIdentity.userAssignedIdentity`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)managedIdentity.userAssignedIdentity.(any)(1)clientId`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)managedIdentity.userAssignedIdentity.(any)(1)objectId`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)managedIdentity.userAssignedIdentity.(any)(1)resourceId`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)workloadIdentity`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)policies.backendAuth.(any)(1)azure.(1)developerImplicit`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].name`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].provider`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].provider.(1)openAI`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].provider.(1)openAI.model`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].provider.(1)gemini`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].provider.(1)gemini.model`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].provider.(1)vertex`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].provider.(1)vertex.model`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].provider.(1)vertex.region`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].provider.(1)vertex.projectId`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].provider.(1)anthropic`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].provider.(1)anthropic.model`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].provider.(1)bedrock`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].provider.(1)bedrock.model`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].provider.(1)bedrock.region`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].provider.(1)bedrock.guardrailIdentifier`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].provider.(1)bedrock.guardrailVersion`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].provider.(1)azureOpenAI`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].provider.(1)azureOpenAI.model`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].provider.(1)azureOpenAI.host`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].provider.(1)azureOpenAI.apiVersion`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].hostOverride`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].pathOverride`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].tokenize`|Whether to tokenize on the request flow. This enables us to do more accurate rate limits,<br>since we know (part of) the cost of the request upfront.<br>This comes with the cost of an expensive operation.|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].routes`|Routes defines how to identify the type of traffic we should handle<br>The keys are URL suffix matches, like `/v1/models`. The special `*` can be used to match anything.|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.requestHeaderModifier`|Headers to be modified in the request.|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.requestHeaderModifier.add`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.requestHeaderModifier.set`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.requestHeaderModifier.remove`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.responseHeaderModifier`|Headers to be modified in the response.|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.responseHeaderModifier.add`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.responseHeaderModifier.set`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.responseHeaderModifier.remove`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.requestRedirect`|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.requestRedirect.scheme`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.requestRedirect.authority`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.requestRedirect.authority.(any)(1)full`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.requestRedirect.authority.(any)(1)host`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.requestRedirect.authority.(any)(1)port`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.requestRedirect.path`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.requestRedirect.path.(any)(1)full`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.requestRedirect.path.(any)(1)prefix`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.requestRedirect.status`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.mcpAuthorization`|Authorization policies for MCP access.|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.mcpAuthorization.rules`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.a2a`|Mark this traffic as A2A to enable A2A processing and telemetry.|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai`|Mark this as LLM traffic to enable LLM processing.|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.request`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.request[].(1)regex`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.request[].(1)regex.action`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.request[].(1)regex.rules`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.request[].(1)regex.rules[].(any)builtin`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.request[].(1)regex.rules[].(any)pattern`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.request[].(1)webhook`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.request[].(1)webhook.target`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.request[].(1)webhook.target.(1)service`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.request[].(1)webhook.target.(1)service.name`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.request[].(1)webhook.target.(1)service.name.namespace`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.request[].(1)webhook.target.(1)service.name.hostname`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.request[].(1)webhook.target.(1)service.port`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.request[].(1)webhook.target.(1)host`|Hostname or IP address|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.request[].(1)webhook.target.(1)backend`|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.request[].(1)webhook.forwardHeaderMatches`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.request[].(1)webhook.forwardHeaderMatches[].name`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.request[].(1)webhook.forwardHeaderMatches[].value`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.request[].(1)webhook.forwardHeaderMatches[].value.(1)exact`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.request[].(1)webhook.forwardHeaderMatches[].value.(1)regex`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.request[].(1)openAIModeration`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.request[].(1)openAIModeration.model`|Model to use. Defaults to `omni-moderation-latest`|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.request[].rejection`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.request[].rejection.body`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.request[].rejection.status`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.request[].rejection.headers`|Optional headers to add, set, or remove from the rejection response|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.request[].rejection.headers.add`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.request[].rejection.headers.set`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.request[].rejection.headers.remove`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.response`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.response[].rejection`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.response[].rejection.body`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.response[].rejection.status`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.response[].rejection.headers`|Optional headers to add, set, or remove from the rejection response|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.response[].rejection.headers.add`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.response[].rejection.headers.set`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.response[].rejection.headers.remove`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.response[].kind`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.response[].kind.(1)regex`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.response[].kind.(1)regex.action`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.response[].kind.(1)regex.rules`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.response[].kind.(1)regex.rules[].(any)builtin`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.response[].kind.(1)regex.rules[].(any)pattern`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.response[].kind.(1)webhook`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.response[].kind.(1)webhook.target`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.response[].kind.(1)webhook.target.(1)service`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.response[].kind.(1)webhook.target.(1)service.name`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.response[].kind.(1)webhook.target.(1)service.name.namespace`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.response[].kind.(1)webhook.target.(1)service.name.hostname`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.response[].kind.(1)webhook.target.(1)service.port`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.response[].kind.(1)webhook.target.(1)host`|Hostname or IP address|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.response[].kind.(1)webhook.target.(1)backend`|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.response[].kind.(1)webhook.forwardHeaderMatches`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.response[].kind.(1)webhook.forwardHeaderMatches[].name`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.response[].kind.(1)webhook.forwardHeaderMatches[].value`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.response[].kind.(1)webhook.forwardHeaderMatches[].value.(1)exact`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptGuard.response[].kind.(1)webhook.forwardHeaderMatches[].value.(1)regex`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.defaults`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.overrides`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.prompts`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.prompts.append`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.prompts.append[].role`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.prompts.append[].content`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.prompts.prepend`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.prompts.prepend[].role`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.prompts.prepend[].content`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.modelAliases`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptCaching`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptCaching.cacheSystem`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptCaching.cacheMessages`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptCaching.cacheTools`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.ai.promptCaching.minTokens`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendTLS`|Send TLS to the backend.|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendTLS.cert`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendTLS.key`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendTLS.root`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendTLS.hostname`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendTLS.insecure`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendTLS.insecureHost`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendTLS.alpn`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendTLS.subjectAltNames`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendAuth`|Authenticate to the backend.|
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendAuth.(any)(1)passthrough`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendAuth.(any)(1)key`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendAuth.(any)(1)key.(any)file`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendAuth.(any)(1)gcp`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendAuth.(any)(1)aws`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendAuth.(any)(1)aws.(any)accessKeyId`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendAuth.(any)(1)aws.(any)secretAccessKey`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendAuth.(any)(1)aws.(any)region`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendAuth.(any)(1)aws.(any)sessionToken`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendAuth.(any)(1)azure`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendAuth.(any)(1)azure.(1)explicitConfig`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)clientSecret`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)clientSecret.tenant_id`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)clientSecret.client_id`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)clientSecret.client_secret`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)managedIdentity`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)managedIdentity.userAssignedIdentity`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)managedIdentity.userAssignedIdentity.(any)(1)clientId`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)managedIdentity.userAssignedIdentity.(any)(1)objectId`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)managedIdentity.userAssignedIdentity.(any)(1)resourceId`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)workloadIdentity`||
|`binds[].listeners[].routes[].backends[].(1)ai.(any)groups[].providers[].policies.backendAuth.(any)(1)azure.(1)developerImplicit`||
|`binds[].listeners[].routes[].backends[].weight`||
|`binds[].listeners[].routes[].backends[].policies`||
|`binds[].listeners[].routes[].backends[].policies.requestHeaderModifier`|Headers to be modified in the request.|
|`binds[].listeners[].routes[].backends[].policies.requestHeaderModifier.add`||
|`binds[].listeners[].routes[].backends[].policies.requestHeaderModifier.set`||
|`binds[].listeners[].routes[].backends[].policies.requestHeaderModifier.remove`||
|`binds[].listeners[].routes[].backends[].policies.responseHeaderModifier`|Headers to be modified in the response.|
|`binds[].listeners[].routes[].backends[].policies.responseHeaderModifier.add`||
|`binds[].listeners[].routes[].backends[].policies.responseHeaderModifier.set`||
|`binds[].listeners[].routes[].backends[].policies.responseHeaderModifier.remove`||
|`binds[].listeners[].routes[].backends[].policies.requestRedirect`|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].backends[].policies.requestRedirect.scheme`||
|`binds[].listeners[].routes[].backends[].policies.requestRedirect.authority`||
|`binds[].listeners[].routes[].backends[].policies.requestRedirect.authority.(any)(1)full`||
|`binds[].listeners[].routes[].backends[].policies.requestRedirect.authority.(any)(1)host`||
|`binds[].listeners[].routes[].backends[].policies.requestRedirect.authority.(any)(1)port`||
|`binds[].listeners[].routes[].backends[].policies.requestRedirect.path`||
|`binds[].listeners[].routes[].backends[].policies.requestRedirect.path.(any)(1)full`||
|`binds[].listeners[].routes[].backends[].policies.requestRedirect.path.(any)(1)prefix`||
|`binds[].listeners[].routes[].backends[].policies.requestRedirect.status`||
|`binds[].listeners[].routes[].backends[].policies.mcpAuthorization`|Authorization policies for MCP access.|
|`binds[].listeners[].routes[].backends[].policies.mcpAuthorization.rules`||
|`binds[].listeners[].routes[].backends[].policies.a2a`|Mark this traffic as A2A to enable A2A processing and telemetry.|
|`binds[].listeners[].routes[].backends[].policies.ai`|Mark this as LLM traffic to enable LLM processing.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].(1)regex`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].(1)regex.action`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].(1)regex.rules`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].(1)regex.rules[].(any)builtin`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].(1)regex.rules[].(any)pattern`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].(1)webhook`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].(1)webhook.target`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].(1)webhook.target.(1)service`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].(1)webhook.target.(1)service.name`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].(1)webhook.target.(1)service.name.namespace`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].(1)webhook.target.(1)service.name.hostname`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].(1)webhook.target.(1)service.port`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].(1)webhook.target.(1)host`|Hostname or IP address|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].(1)webhook.target.(1)backend`|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].(1)webhook.forwardHeaderMatches`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].(1)webhook.forwardHeaderMatches[].name`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].(1)webhook.forwardHeaderMatches[].value`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].(1)webhook.forwardHeaderMatches[].value.(1)exact`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].(1)webhook.forwardHeaderMatches[].value.(1)regex`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].(1)openAIModeration`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].(1)openAIModeration.model`|Model to use. Defaults to `omni-moderation-latest`|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].rejection`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].rejection.body`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].rejection.status`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].rejection.headers`|Optional headers to add, set, or remove from the rejection response|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].rejection.headers.add`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].rejection.headers.set`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].rejection.headers.remove`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].rejection`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].rejection.body`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].rejection.status`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].rejection.headers`|Optional headers to add, set, or remove from the rejection response|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].rejection.headers.add`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].rejection.headers.set`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].rejection.headers.remove`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].kind`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].kind.(1)regex`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].kind.(1)regex.action`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].kind.(1)regex.rules`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].kind.(1)regex.rules[].(any)builtin`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].kind.(1)regex.rules[].(any)pattern`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].kind.(1)webhook`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].kind.(1)webhook.target`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].kind.(1)webhook.target.(1)service`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].kind.(1)webhook.target.(1)service.name`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].kind.(1)webhook.target.(1)service.name.namespace`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].kind.(1)webhook.target.(1)service.name.hostname`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].kind.(1)webhook.target.(1)service.port`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].kind.(1)webhook.target.(1)host`|Hostname or IP address|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].kind.(1)webhook.target.(1)backend`|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].kind.(1)webhook.forwardHeaderMatches`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].kind.(1)webhook.forwardHeaderMatches[].name`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].kind.(1)webhook.forwardHeaderMatches[].value`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].kind.(1)webhook.forwardHeaderMatches[].value.(1)exact`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].kind.(1)webhook.forwardHeaderMatches[].value.(1)regex`||
|`binds[].listeners[].routes[].backends[].policies.ai.defaults`||
|`binds[].listeners[].routes[].backends[].policies.ai.overrides`||
|`binds[].listeners[].routes[].backends[].policies.ai.prompts`||
|`binds[].listeners[].routes[].backends[].policies.ai.prompts.append`||
|`binds[].listeners[].routes[].backends[].policies.ai.prompts.append[].role`||
|`binds[].listeners[].routes[].backends[].policies.ai.prompts.append[].content`||
|`binds[].listeners[].routes[].backends[].policies.ai.prompts.prepend`||
|`binds[].listeners[].routes[].backends[].policies.ai.prompts.prepend[].role`||
|`binds[].listeners[].routes[].backends[].policies.ai.prompts.prepend[].content`||
|`binds[].listeners[].routes[].backends[].policies.ai.modelAliases`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptCaching`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptCaching.cacheSystem`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptCaching.cacheMessages`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptCaching.cacheTools`||
|`binds[].listeners[].routes[].backends[].policies.ai.promptCaching.minTokens`||
|`binds[].listeners[].routes[].backends[].policies.backendTLS`|Send TLS to the backend.|
|`binds[].listeners[].routes[].backends[].policies.backendTLS.cert`||
|`binds[].listeners[].routes[].backends[].policies.backendTLS.key`||
|`binds[].listeners[].routes[].backends[].policies.backendTLS.root`||
|`binds[].listeners[].routes[].backends[].policies.backendTLS.hostname`||
|`binds[].listeners[].routes[].backends[].policies.backendTLS.insecure`||
|`binds[].listeners[].routes[].backends[].policies.backendTLS.insecureHost`||
|`binds[].listeners[].routes[].backends[].policies.backendTLS.alpn`||
|`binds[].listeners[].routes[].backends[].policies.backendTLS.subjectAltNames`||
|`binds[].listeners[].routes[].backends[].policies.backendAuth`|Authenticate to the backend.|
|`binds[].listeners[].routes[].backends[].policies.backendAuth.(any)(1)passthrough`||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.(any)(1)key`||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.(any)(1)key.(any)file`||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.(any)(1)gcp`||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.(any)(1)aws`||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.(any)(1)aws.(any)accessKeyId`||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.(any)(1)aws.(any)secretAccessKey`||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.(any)(1)aws.(any)region`||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.(any)(1)aws.(any)sessionToken`||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.(any)(1)azure`||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.(any)(1)azure.(1)explicitConfig`||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)clientSecret`||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)clientSecret.tenant_id`||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)clientSecret.client_id`||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)clientSecret.client_secret`||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)managedIdentity`||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)managedIdentity.userAssignedIdentity`||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)managedIdentity.userAssignedIdentity.(any)(1)clientId`||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)managedIdentity.userAssignedIdentity.(any)(1)objectId`||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)managedIdentity.userAssignedIdentity.(any)(1)resourceId`||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.(any)(1)azure.(1)explicitConfig.(1)workloadIdentity`||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.(any)(1)azure.(1)developerImplicit`||
|`binds[].listeners[].tcpRoutes`||
|`binds[].listeners[].tcpRoutes[].name`||
|`binds[].listeners[].tcpRoutes[].ruleName`||
|`binds[].listeners[].tcpRoutes[].hostnames`|Can be a wildcard|
|`binds[].listeners[].tcpRoutes[].policies`||
|`binds[].listeners[].tcpRoutes[].policies.backendTLS`||
|`binds[].listeners[].tcpRoutes[].policies.backendTLS.cert`||
|`binds[].listeners[].tcpRoutes[].policies.backendTLS.key`||
|`binds[].listeners[].tcpRoutes[].policies.backendTLS.root`||
|`binds[].listeners[].tcpRoutes[].policies.backendTLS.hostname`||
|`binds[].listeners[].tcpRoutes[].policies.backendTLS.insecure`||
|`binds[].listeners[].tcpRoutes[].policies.backendTLS.insecureHost`||
|`binds[].listeners[].tcpRoutes[].policies.backendTLS.alpn`||
|`binds[].listeners[].tcpRoutes[].policies.backendTLS.subjectAltNames`||
|`binds[].listeners[].tcpRoutes[].backends`||
|`binds[].listeners[].tcpRoutes[].backends[].(1)service`||
|`binds[].listeners[].tcpRoutes[].backends[].(1)service.name`||
|`binds[].listeners[].tcpRoutes[].backends[].(1)service.name.namespace`||
|`binds[].listeners[].tcpRoutes[].backends[].(1)service.name.hostname`||
|`binds[].listeners[].tcpRoutes[].backends[].(1)service.port`||
|`binds[].listeners[].tcpRoutes[].backends[].(1)host`|Hostname or IP address|
|`binds[].listeners[].tcpRoutes[].backends[].(1)backend`|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].tcpRoutes[].backends[].weight`||
|`binds[].listeners[].tcpRoutes[].backends[].policies`||
|`binds[].listeners[].tcpRoutes[].backends[].policies.backendTLS`|Send TLS to the backend.|
|`binds[].listeners[].tcpRoutes[].backends[].policies.backendTLS.cert`||
|`binds[].listeners[].tcpRoutes[].backends[].policies.backendTLS.key`||
|`binds[].listeners[].tcpRoutes[].backends[].policies.backendTLS.root`||
|`binds[].listeners[].tcpRoutes[].backends[].policies.backendTLS.hostname`||
|`binds[].listeners[].tcpRoutes[].backends[].policies.backendTLS.insecure`||
|`binds[].listeners[].tcpRoutes[].backends[].policies.backendTLS.insecureHost`||
|`binds[].listeners[].tcpRoutes[].backends[].policies.backendTLS.alpn`||
|`binds[].listeners[].tcpRoutes[].backends[].policies.backendTLS.subjectAltNames`||
|`binds[].listeners[].policies`||
|`binds[].listeners[].policies.jwtAuth`|Authenticate incoming JWT requests.|
|`binds[].listeners[].policies.jwtAuth.(any)(any)mode`||
|`binds[].listeners[].policies.jwtAuth.(any)(any)providers`||
|`binds[].listeners[].policies.jwtAuth.(any)(any)providers[].issuer`||
|`binds[].listeners[].policies.jwtAuth.(any)(any)providers[].audiences`||
|`binds[].listeners[].policies.jwtAuth.(any)(any)providers[].jwks`||
|`binds[].listeners[].policies.jwtAuth.(any)(any)providers[].jwks.(any)file`||
|`binds[].listeners[].policies.jwtAuth.(any)(any)providers[].jwks.(any)url`||
|`binds[].listeners[].policies.jwtAuth.(any)(any)mode`||
|`binds[].listeners[].policies.jwtAuth.(any)(any)issuer`||
|`binds[].listeners[].policies.jwtAuth.(any)(any)audiences`||
|`binds[].listeners[].policies.jwtAuth.(any)(any)jwks`||
|`binds[].listeners[].policies.jwtAuth.(any)(any)jwks.(any)file`||
|`binds[].listeners[].policies.jwtAuth.(any)(any)jwks.(any)url`||
|`binds[].listeners[].policies.extAuthz`|Authenticate incoming requests by calling an external authorization server.|
|`binds[].listeners[].policies.extAuthz.(any)(1)service`||
|`binds[].listeners[].policies.extAuthz.(any)(1)service.name`||
|`binds[].listeners[].policies.extAuthz.(any)(1)service.name.namespace`||
|`binds[].listeners[].policies.extAuthz.(any)(1)service.name.hostname`||
|`binds[].listeners[].policies.extAuthz.(any)(1)service.port`||
|`binds[].listeners[].policies.extAuthz.(any)(1)host`|Hostname or IP address|
|`binds[].listeners[].policies.extAuthz.(any)(1)backend`|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].policies.extAuthz.(any)context`|Additional context to send to the authorization service.<br>This maps to the `context_extensions` field of the request, and only allows static values.|
|`binds[].listeners[].policies.extAuthz.(any)metadata`|Additional metadata to send to the authorization service.<br>This maps to the `metadata_context.filter_metadata` field of the request, and allows dynamic CEL expressions.<br>If unset, by default the `envoy.filters.http.jwt_authn` key is set if the JWT policy is used as well, for compatibility.|
|`binds[].listeners[].policies.extAuthz.(any)failureMode`|Behavior when the authorization service is unavailable or returns an error|
|`binds[].listeners[].policies.extAuthz.(any)failureMode.(1)denyWithStatus`||
|`binds[].listeners[].policies.extAuthz.(any)includeRequestHeaders`|Specific headers to include in the authorization request (empty = all headers)|
|`binds[].listeners[].policies.extAuthz.(any)includeRequestBody`|Options for including the request body in the authorization request|
|`binds[].listeners[].policies.extAuthz.(any)includeRequestBody.maxRequestBytes`|Maximum size of request body to buffer (default: 8192)|
|`binds[].listeners[].policies.extAuthz.(any)includeRequestBody.allowPartialMessage`|If true, send partial body when max_request_bytes is reached|
|`binds[].listeners[].policies.extAuthz.(any)includeRequestBody.packAsBytes`|If true, pack body as raw bytes in gRPC|
|`binds[].listeners[].policies.extAuthz.(any)timeout`|Timeout for the authorization request (default: 200ms)|
|`binds[].listeners[].policies.extProc`|Extend agentgateway with an external processor|
|`binds[].listeners[].policies.extProc.(any)(1)service`||
|`binds[].listeners[].policies.extProc.(any)(1)service.name`||
|`binds[].listeners[].policies.extProc.(any)(1)service.name.namespace`||
|`binds[].listeners[].policies.extProc.(any)(1)service.name.hostname`||
|`binds[].listeners[].policies.extProc.(any)(1)service.port`||
|`binds[].listeners[].policies.extProc.(any)(1)host`|Hostname or IP address|
|`binds[].listeners[].policies.extProc.(any)(1)backend`|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].policies.extProc.(any)failureMode`||
|`binds[].listeners[].policies.transformations`|Modify requests and responses|
|`binds[].listeners[].policies.transformations.request`||
|`binds[].listeners[].policies.transformations.request.add`||
|`binds[].listeners[].policies.transformations.request.set`||
|`binds[].listeners[].policies.transformations.request.remove`||
|`binds[].listeners[].policies.transformations.request.body`||
|`binds[].listeners[].policies.transformations.response`||
|`binds[].listeners[].policies.transformations.response.add`||
|`binds[].listeners[].policies.transformations.response.set`||
|`binds[].listeners[].policies.transformations.response.remove`||
|`binds[].listeners[].policies.transformations.response.body`||
|`binds[].listeners[].policies.basicAuth`|Authenticate incoming requests using Basic Authentication with htpasswd.|
|`binds[].listeners[].policies.basicAuth.htpasswd`|.htpasswd file contents/reference|
|`binds[].listeners[].policies.basicAuth.htpasswd.(any)file`||
|`binds[].listeners[].policies.basicAuth.realm`|Realm name for the WWW-Authenticate header|
|`binds[].listeners[].policies.basicAuth.mode`|Validation mode for basic authentication|
|`binds[].listeners[].policies.apiKey`|Authenticate incoming requests using API Keys|
|`binds[].listeners[].policies.apiKey.keys`|List of API keys|
|`binds[].listeners[].policies.apiKey.keys[].key`||
|`binds[].listeners[].policies.apiKey.keys[].metadata`||
|`binds[].listeners[].policies.apiKey.mode`|Validation mode for API keys|
|`frontendPolicies`||
|`frontendPolicies.http`|Settings for handling incoming HTTP requests.|
|`frontendPolicies.http.maxBufferSize`||
|`frontendPolicies.http.http1MaxHeaders`|The maximum number of headers allowed in a request. Changing this value results in a performance<br>degradation, even if set to a lower value than the default (100)|
|`frontendPolicies.http.http1IdleTimeout`||
|`frontendPolicies.http.http2WindowSize`||
|`frontendPolicies.http.http2ConnectionWindowSize`||
|`frontendPolicies.http.http2FrameSize`||
|`frontendPolicies.http.http2KeepaliveInterval`||
|`frontendPolicies.http.http2KeepaliveTimeout`||
|`frontendPolicies.tls`|Settings for handling incoming TLS connections.|
|`frontendPolicies.tls.tlsHandshakeTimeout`||
|`frontendPolicies.tls.alpn`||
|`frontendPolicies.tcp`|Settings for handling incoming TCP connections.|
|`frontendPolicies.tcp.keepalives`||
|`frontendPolicies.tcp.keepalives.enabled`||
|`frontendPolicies.tcp.keepalives.time`||
|`frontendPolicies.tcp.keepalives.interval`||
|`frontendPolicies.tcp.keepalives.retries`||
|`frontendPolicies.accessLog`|Settings for request access logs.|
|`frontendPolicies.accessLog.filter`||
|`frontendPolicies.accessLog.add`||
|`frontendPolicies.accessLog.remove`||
|`frontendPolicies.tracing`||
|`policies`|policies defines additional policies that can be attached to various other configurations.<br>This is an advanced feature; users should typically use the inline `policies` field under route/gateway.|
|`policies[].name`||
|`policies[].target`||
|`policies[].target.(1)gateway`||
|`policies[].target.(1)listener`||
|`policies[].target.(1)route`||
|`policies[].target.(1)routeRule`||
|`policies[].target.(1)service`||
|`policies[].target.(1)backend`||
|`policies[].target.(1)subBackend`||
|`policies[].phase`|phase defines at what level the policy runs at. Gateway policies run pre-routing, while<br>Route policies apply post-routing.<br>Only a subset of policies are eligible as Gateway policies.<br>In general, normal (route level) policies should be used, except you need the policy to influence<br>routing.|
|`policies[].policy`||
|`policies[].policy.requestHeaderModifier`|Headers to be modified in the request.|
|`policies[].policy.requestHeaderModifier.add`||
|`policies[].policy.requestHeaderModifier.set`||
|`policies[].policy.requestHeaderModifier.remove`||
|`policies[].policy.responseHeaderModifier`|Headers to be modified in the response.|
|`policies[].policy.responseHeaderModifier.add`||
|`policies[].policy.responseHeaderModifier.set`||
|`policies[].policy.responseHeaderModifier.remove`||
|`policies[].policy.requestRedirect`|Directly respond to the request with a redirect.|
|`policies[].policy.requestRedirect.scheme`||
|`policies[].policy.requestRedirect.authority`||
|`policies[].policy.requestRedirect.authority.(any)(1)full`||
|`policies[].policy.requestRedirect.authority.(any)(1)host`||
|`policies[].policy.requestRedirect.authority.(any)(1)port`||
|`policies[].policy.requestRedirect.path`||
|`policies[].policy.requestRedirect.path.(any)(1)full`||
|`policies[].policy.requestRedirect.path.(any)(1)prefix`||
|`policies[].policy.requestRedirect.status`||
|`policies[].policy.urlRewrite`|Modify the URL path or authority.|
|`policies[].policy.urlRewrite.authority`||
|`policies[].policy.urlRewrite.authority.(any)(1)full`||
|`policies[].policy.urlRewrite.authority.(any)(1)host`||
|`policies[].policy.urlRewrite.authority.(any)(1)port`||
|`policies[].policy.urlRewrite.path`||
|`policies[].policy.urlRewrite.path.(any)(1)full`||
|`policies[].policy.urlRewrite.path.(any)(1)prefix`||
|`policies[].policy.requestMirror`|Mirror incoming requests to another destination.|
|`policies[].policy.requestMirror.backend`||
|`policies[].policy.requestMirror.backend.(1)service`||
|`policies[].policy.requestMirror.backend.(1)service.name`||
|`policies[].policy.requestMirror.backend.(1)service.name.namespace`||
|`policies[].policy.requestMirror.backend.(1)service.name.hostname`||
|`policies[].policy.requestMirror.backend.(1)service.port`||
|`policies[].policy.requestMirror.backend.(1)host`|Hostname or IP address|
|`policies[].policy.requestMirror.backend.(1)backend`|Explicit backend reference. Backend must be defined in the top level backends list|
|`policies[].policy.requestMirror.percentage`||
|`policies[].policy.directResponse`|Directly respond to the request with a static response.|
|`policies[].policy.directResponse.body`||
|`policies[].policy.directResponse.status`||
|`policies[].policy.cors`|Handle CORS preflight requests and append configured CORS headers to applicable requests.|
|`policies[].policy.cors.allowCredentials`||
|`policies[].policy.cors.allowHeaders`||
|`policies[].policy.cors.allowMethods`||
|`policies[].policy.cors.allowOrigins`||
|`policies[].policy.cors.exposeHeaders`||
|`policies[].policy.cors.maxAge`||
|`policies[].policy.mcpAuthorization`|Authorization policies for MCP access.|
|`policies[].policy.mcpAuthorization.rules`||
|`policies[].policy.authorization`|Authorization policies for HTTP access.|
|`policies[].policy.authorization.rules`||
|`policies[].policy.mcpAuthentication`|Authentication for MCP clients.|
|`policies[].policy.mcpAuthentication.issuer`||
|`policies[].policy.mcpAuthentication.audiences`||
|`policies[].policy.mcpAuthentication.provider`||
|`policies[].policy.mcpAuthentication.provider.(any)(1)auth0`||
|`policies[].policy.mcpAuthentication.provider.(any)(1)keycloak`||
|`policies[].policy.mcpAuthentication.resourceMetadata`||
|`policies[].policy.mcpAuthentication.jwks`||
|`policies[].policy.mcpAuthentication.jwks.(any)file`||
|`policies[].policy.mcpAuthentication.jwks.(any)url`||
|`policies[].policy.a2a`|Mark this traffic as A2A to enable A2A processing and telemetry.|
|`policies[].policy.ai`|Mark this as LLM traffic to enable LLM processing.|
|`policies[].policy.ai.promptGuard`||
|`policies[].policy.ai.promptGuard.request`||
|`policies[].policy.ai.promptGuard.request[].(1)regex`||
|`policies[].policy.ai.promptGuard.request[].(1)regex.action`||
|`policies[].policy.ai.promptGuard.request[].(1)regex.rules`||
|`policies[].policy.ai.promptGuard.request[].(1)regex.rules[].(any)builtin`||
|`policies[].policy.ai.promptGuard.request[].(1)regex.rules[].(any)pattern`||
|`policies[].policy.ai.promptGuard.request[].(1)webhook`||
|`policies[].policy.ai.promptGuard.request[].(1)webhook.target`||
|`policies[].policy.ai.promptGuard.request[].(1)webhook.target.(1)service`||
|`policies[].policy.ai.promptGuard.request[].(1)webhook.target.(1)service.name`||
|`policies[].policy.ai.promptGuard.request[].(1)webhook.target.(1)service.name.namespace`||
|`policies[].policy.ai.promptGuard.request[].(1)webhook.target.(1)service.name.hostname`||
|`policies[].policy.ai.promptGuard.request[].(1)webhook.target.(1)service.port`||
|`policies[].policy.ai.promptGuard.request[].(1)webhook.target.(1)host`|Hostname or IP address|
|`policies[].policy.ai.promptGuard.request[].(1)webhook.target.(1)backend`|Explicit backend reference. Backend must be defined in the top level backends list|
|`policies[].policy.ai.promptGuard.request[].(1)webhook.forwardHeaderMatches`||
|`policies[].policy.ai.promptGuard.request[].(1)webhook.forwardHeaderMatches[].name`||
|`policies[].policy.ai.promptGuard.request[].(1)webhook.forwardHeaderMatches[].value`||
|`policies[].policy.ai.promptGuard.request[].(1)webhook.forwardHeaderMatches[].value.(1)exact`||
|`policies[].policy.ai.promptGuard.request[].(1)webhook.forwardHeaderMatches[].value.(1)regex`||
|`policies[].policy.ai.promptGuard.request[].(1)openAIModeration`||
|`policies[].policy.ai.promptGuard.request[].(1)openAIModeration.model`|Model to use. Defaults to `omni-moderation-latest`|
|`policies[].policy.ai.promptGuard.request[].rejection`||
|`policies[].policy.ai.promptGuard.request[].rejection.body`||
|`policies[].policy.ai.promptGuard.request[].rejection.status`||
|`policies[].policy.ai.promptGuard.request[].rejection.headers`|Optional headers to add, set, or remove from the rejection response|
|`policies[].policy.ai.promptGuard.request[].rejection.headers.add`||
|`policies[].policy.ai.promptGuard.request[].rejection.headers.set`||
|`policies[].policy.ai.promptGuard.request[].rejection.headers.remove`||
|`policies[].policy.ai.promptGuard.response`||
|`policies[].policy.ai.promptGuard.response[].rejection`||
|`policies[].policy.ai.promptGuard.response[].rejection.body`||
|`policies[].policy.ai.promptGuard.response[].rejection.status`||
|`policies[].policy.ai.promptGuard.response[].rejection.headers`|Optional headers to add, set, or remove from the rejection response|
|`policies[].policy.ai.promptGuard.response[].rejection.headers.add`||
|`policies[].policy.ai.promptGuard.response[].rejection.headers.set`||
|`policies[].policy.ai.promptGuard.response[].rejection.headers.remove`||
|`policies[].policy.ai.promptGuard.response[].kind`||
|`policies[].policy.ai.promptGuard.response[].kind.(1)regex`||
|`policies[].policy.ai.promptGuard.response[].kind.(1)regex.action`||
|`policies[].policy.ai.promptGuard.response[].kind.(1)regex.rules`||
|`policies[].policy.ai.promptGuard.response[].kind.(1)regex.rules[].(any)builtin`||
|`policies[].policy.ai.promptGuard.response[].kind.(1)regex.rules[].(any)pattern`||
|`policies[].policy.ai.promptGuard.response[].kind.(1)webhook`||
|`policies[].policy.ai.promptGuard.response[].kind.(1)webhook.target`||
|`policies[].policy.ai.promptGuard.response[].kind.(1)webhook.target.(1)service`||
|`policies[].policy.ai.promptGuard.response[].kind.(1)webhook.target.(1)service.name`||
|`policies[].policy.ai.promptGuard.response[].kind.(1)webhook.target.(1)service.name.namespace`||
|`policies[].policy.ai.promptGuard.response[].kind.(1)webhook.target.(1)service.name.hostname`||
|`policies[].policy.ai.promptGuard.response[].kind.(1)webhook.target.(1)service.port`||
|`policies[].policy.ai.promptGuard.response[].kind.(1)webhook.target.(1)host`|Hostname or IP address|
|`policies[].policy.ai.promptGuard.response[].kind.(1)webhook.target.(1)backend`|Explicit backend reference. Backend must be defined in the top level backends list|
|`policies[].policy.ai.promptGuard.response[].kind.(1)webhook.forwardHeaderMatches`||
|`policies[].policy.ai.promptGuard.response[].kind.(1)webhook.forwardHeaderMatches[].name`||
|`policies[].policy.ai.promptGuard.response[].kind.(1)webhook.forwardHeaderMatches[].value`||
|`policies[].policy.ai.promptGuard.response[].kind.(1)webhook.forwardHeaderMatches[].value.(1)exact`||
|`policies[].policy.ai.promptGuard.response[].kind.(1)webhook.forwardHeaderMatches[].value.(1)regex`||
|`policies[].policy.ai.defaults`||
|`policies[].policy.ai.overrides`||
|`policies[].policy.ai.prompts`||
|`policies[].policy.ai.prompts.append`||
|`policies[].policy.ai.prompts.append[].role`||
|`policies[].policy.ai.prompts.append[].content`||
|`policies[].policy.ai.prompts.prepend`||
|`policies[].policy.ai.prompts.prepend[].role`||
|`policies[].policy.ai.prompts.prepend[].content`||
|`policies[].policy.ai.modelAliases`||
|`policies[].policy.ai.promptCaching`||
|`policies[].policy.ai.promptCaching.cacheSystem`||
|`policies[].policy.ai.promptCaching.cacheMessages`||
|`policies[].policy.ai.promptCaching.cacheTools`||
|`policies[].policy.ai.promptCaching.minTokens`||
|`policies[].policy.backendTLS`|Send TLS to the backend.|
|`policies[].policy.backendTLS.cert`||
|`policies[].policy.backendTLS.key`||
|`policies[].policy.backendTLS.root`||
|`policies[].policy.backendTLS.hostname`||
|`policies[].policy.backendTLS.insecure`||
|`policies[].policy.backendTLS.insecureHost`||
|`policies[].policy.backendTLS.alpn`||
|`policies[].policy.backendTLS.subjectAltNames`||
|`policies[].policy.backendAuth`|Authenticate to the backend.|
|`policies[].policy.backendAuth.(any)(1)passthrough`||
|`policies[].policy.backendAuth.(any)(1)key`||
|`policies[].policy.backendAuth.(any)(1)key.(any)file`||
|`policies[].policy.backendAuth.(any)(1)gcp`||
|`policies[].policy.backendAuth.(any)(1)aws`||
|`policies[].policy.backendAuth.(any)(1)aws.(any)accessKeyId`||
|`policies[].policy.backendAuth.(any)(1)aws.(any)secretAccessKey`||
|`policies[].policy.backendAuth.(any)(1)aws.(any)region`||
|`policies[].policy.backendAuth.(any)(1)aws.(any)sessionToken`||
|`policies[].policy.backendAuth.(any)(1)azure`||
|`policies[].policy.backendAuth.(any)(1)azure.(1)explicitConfig`||
|`policies[].policy.backendAuth.(any)(1)azure.(1)explicitConfig.(1)clientSecret`||
|`policies[].policy.backendAuth.(any)(1)azure.(1)explicitConfig.(1)clientSecret.tenant_id`||
|`policies[].policy.backendAuth.(any)(1)azure.(1)explicitConfig.(1)clientSecret.client_id`||
|`policies[].policy.backendAuth.(any)(1)azure.(1)explicitConfig.(1)clientSecret.client_secret`||
|`policies[].policy.backendAuth.(any)(1)azure.(1)explicitConfig.(1)managedIdentity`||
|`policies[].policy.backendAuth.(any)(1)azure.(1)explicitConfig.(1)managedIdentity.userAssignedIdentity`||
|`policies[].policy.backendAuth.(any)(1)azure.(1)explicitConfig.(1)managedIdentity.userAssignedIdentity.(any)(1)clientId`||
|`policies[].policy.backendAuth.(any)(1)azure.(1)explicitConfig.(1)managedIdentity.userAssignedIdentity.(any)(1)objectId`||
|`policies[].policy.backendAuth.(any)(1)azure.(1)explicitConfig.(1)managedIdentity.userAssignedIdentity.(any)(1)resourceId`||
|`policies[].policy.backendAuth.(any)(1)azure.(1)explicitConfig.(1)workloadIdentity`||
|`policies[].policy.backendAuth.(any)(1)azure.(1)developerImplicit`||
|`policies[].policy.localRateLimit`|Rate limit incoming requests. State is kept local.|
|`policies[].policy.localRateLimit[].maxTokens`||
|`policies[].policy.localRateLimit[].tokensPerFill`||
|`policies[].policy.localRateLimit[].fillInterval`||
|`policies[].policy.localRateLimit[].type`||
|`policies[].policy.remoteRateLimit`|Rate limit incoming requests. State is managed by a remote server.|
|`policies[].policy.remoteRateLimit.(any)(1)service`||
|`policies[].policy.remoteRateLimit.(any)(1)service.name`||
|`policies[].policy.remoteRateLimit.(any)(1)service.name.namespace`||
|`policies[].policy.remoteRateLimit.(any)(1)service.name.hostname`||
|`policies[].policy.remoteRateLimit.(any)(1)service.port`||
|`policies[].policy.remoteRateLimit.(any)(1)host`|Hostname or IP address|
|`policies[].policy.remoteRateLimit.(any)(1)backend`|Explicit backend reference. Backend must be defined in the top level backends list|
|`policies[].policy.remoteRateLimit.(any)domain`||
|`policies[].policy.remoteRateLimit.(any)descriptors`||
|`policies[].policy.remoteRateLimit.(any)descriptors[].entries`||
|`policies[].policy.remoteRateLimit.(any)descriptors[].entries[].key`||
|`policies[].policy.remoteRateLimit.(any)descriptors[].entries[].value`||
|`policies[].policy.remoteRateLimit.(any)descriptors[].type`||
|`policies[].policy.jwtAuth`|Authenticate incoming JWT requests.|
|`policies[].policy.jwtAuth.(any)(any)mode`||
|`policies[].policy.jwtAuth.(any)(any)providers`||
|`policies[].policy.jwtAuth.(any)(any)providers[].issuer`||
|`policies[].policy.jwtAuth.(any)(any)providers[].audiences`||
|`policies[].policy.jwtAuth.(any)(any)providers[].jwks`||
|`policies[].policy.jwtAuth.(any)(any)providers[].jwks.(any)file`||
|`policies[].policy.jwtAuth.(any)(any)providers[].jwks.(any)url`||
|`policies[].policy.jwtAuth.(any)(any)mode`||
|`policies[].policy.jwtAuth.(any)(any)issuer`||
|`policies[].policy.jwtAuth.(any)(any)audiences`||
|`policies[].policy.jwtAuth.(any)(any)jwks`||
|`policies[].policy.jwtAuth.(any)(any)jwks.(any)file`||
|`policies[].policy.jwtAuth.(any)(any)jwks.(any)url`||
|`policies[].policy.basicAuth`|Authenticate incoming requests using Basic Authentication with htpasswd.|
|`policies[].policy.basicAuth.htpasswd`|.htpasswd file contents/reference|
|`policies[].policy.basicAuth.htpasswd.(any)file`||
|`policies[].policy.basicAuth.realm`|Realm name for the WWW-Authenticate header|
|`policies[].policy.basicAuth.mode`|Validation mode for basic authentication|
|`policies[].policy.apiKey`|Authenticate incoming requests using API Keys|
|`policies[].policy.apiKey.keys`|List of API keys|
|`policies[].policy.apiKey.keys[].key`||
|`policies[].policy.apiKey.keys[].metadata`||
|`policies[].policy.apiKey.mode`|Validation mode for API keys|
|`policies[].policy.extAuthz`|Authenticate incoming requests by calling an external authorization server.|
|`policies[].policy.extAuthz.(any)(1)service`||
|`policies[].policy.extAuthz.(any)(1)service.name`||
|`policies[].policy.extAuthz.(any)(1)service.name.namespace`||
|`policies[].policy.extAuthz.(any)(1)service.name.hostname`||
|`policies[].policy.extAuthz.(any)(1)service.port`||
|`policies[].policy.extAuthz.(any)(1)host`|Hostname or IP address|
|`policies[].policy.extAuthz.(any)(1)backend`|Explicit backend reference. Backend must be defined in the top level backends list|
|`policies[].policy.extAuthz.(any)context`|Additional context to send to the authorization service.<br>This maps to the `context_extensions` field of the request, and only allows static values.|
|`policies[].policy.extAuthz.(any)metadata`|Additional metadata to send to the authorization service.<br>This maps to the `metadata_context.filter_metadata` field of the request, and allows dynamic CEL expressions.<br>If unset, by default the `envoy.filters.http.jwt_authn` key is set if the JWT policy is used as well, for compatibility.|
|`policies[].policy.extAuthz.(any)failureMode`|Behavior when the authorization service is unavailable or returns an error|
|`policies[].policy.extAuthz.(any)failureMode.(1)denyWithStatus`||
|`policies[].policy.extAuthz.(any)includeRequestHeaders`|Specific headers to include in the authorization request (empty = all headers)|
|`policies[].policy.extAuthz.(any)includeRequestBody`|Options for including the request body in the authorization request|
|`policies[].policy.extAuthz.(any)includeRequestBody.maxRequestBytes`|Maximum size of request body to buffer (default: 8192)|
|`policies[].policy.extAuthz.(any)includeRequestBody.allowPartialMessage`|If true, send partial body when max_request_bytes is reached|
|`policies[].policy.extAuthz.(any)includeRequestBody.packAsBytes`|If true, pack body as raw bytes in gRPC|
|`policies[].policy.extAuthz.(any)timeout`|Timeout for the authorization request (default: 200ms)|
|`policies[].policy.extProc`|Extend agentgateway with an external processor|
|`policies[].policy.extProc.(any)(1)service`||
|`policies[].policy.extProc.(any)(1)service.name`||
|`policies[].policy.extProc.(any)(1)service.name.namespace`||
|`policies[].policy.extProc.(any)(1)service.name.hostname`||
|`policies[].policy.extProc.(any)(1)service.port`||
|`policies[].policy.extProc.(any)(1)host`|Hostname or IP address|
|`policies[].policy.extProc.(any)(1)backend`|Explicit backend reference. Backend must be defined in the top level backends list|
|`policies[].policy.extProc.(any)failureMode`||
|`policies[].policy.transformations`|Modify requests and responses|
|`policies[].policy.transformations.request`||
|`policies[].policy.transformations.request.add`||
|`policies[].policy.transformations.request.set`||
|`policies[].policy.transformations.request.remove`||
|`policies[].policy.transformations.request.body`||
|`policies[].policy.transformations.response`||
|`policies[].policy.transformations.response.add`||
|`policies[].policy.transformations.response.set`||
|`policies[].policy.transformations.response.remove`||
|`policies[].policy.transformations.response.body`||
|`policies[].policy.csrf`|Handle CSRF protection by validating request origins against configured allowed origins.|
|`policies[].policy.csrf.additionalOrigins`||
|`policies[].policy.timeout`|Timeout requests that exceed the configured duration.|
|`policies[].policy.timeout.requestTimeout`||
|`policies[].policy.timeout.backendRequestTimeout`||
|`policies[].policy.retry`|Retry matching requests.|
|`policies[].policy.retry.attempts`||
|`policies[].policy.retry.backoff`||
|`policies[].policy.retry.codes`||
|`workloads`||
|`services`||
## CEL context

|Field|Description|
|-|-|
|`request`|`request` contains attributes about the incoming HTTP request|
|`request.method`|The HTTP method of the request. For example, `GET`|
|`request.uri`|The complete URI of the request. For example, `http://example.com/path`.|
|`request.host`||
|`request.scheme`||
|`request.path`|The path of the request URI. For example, `/path`.|
|`request.headers`|The headers of the request.|
|`request.body`|The body of the request. Warning: accessing the body will cause the body to be buffered.|
|`request.startTime`|The (pre-rendered) time the request started|
|`request.endTime`|The (pre-rendered) time the request completed|
|`response`|`response` contains attributes about the HTTP response|
|`response.code`|The HTTP status code of the response.|
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
|`llm.outputTokens`|The number of tokens in the output/completion.|
|`llm.totalTokens`|The total number of tokens for the request.|
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
|`source`|`source` contains attributes about the source of the request.|
|`source.address`|The IP address of the downstream connection.|
|`source.port`|The port of the downstream connection.|
|`source.identity`|The (Istio SPIFFE) identity of the downstream connection, if available.|
|`source.identity.trustDomain`|The trust domain of the identity.|
|`source.identity.namespace`|The namespace of the identity.|
|`source.identity.serviceAccount`|The service account of the identity.|
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
