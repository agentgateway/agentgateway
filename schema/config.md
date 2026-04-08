# Configuration File Schema

|Field|Type|Description|
|-|-|-|
|`config`|object||
|`config.enableIpv6`|boolean||
|`config.dns`|object|DNS resolver settings.|
|`config.dns.lookupFamily`|string|Controls which IP address families the DNS resolver will query for<br>upstream connections.<br>Accepted values: All, Auto, V4Preferred, V4Only, V6Only.<br>Defaults to Auto (IPv4-only when enableIpv6 is false, both when true).|
|`config.dns.edns0`|boolean|Whether to enable EDNS0 (Extension Mechanisms for DNS) in the resolver.<br>When `None`, the system-provided resolver setting is preserved.<br>Can also be set via the `DNS_EDNS0` environment variable.|
|`config.localXdsPath`|string|Local XDS path. If not specified, the current configuration file will be used.|
|`config.caAddress`|string||
|`config.caAuthToken`|string||
|`config.xdsAddress`|string||
|`config.xdsAuthToken`|string||
|`config.namespace`|string||
|`config.gateway`|string||
|`config.trustDomain`|string||
|`config.serviceAccount`|string||
|`config.clusterId`|string||
|`config.network`|string||
|`config.adminAddr`|string|Admin UI address in the format "ip:port"|
|`config.statsAddr`|string|Stats/metrics server address in the format "ip:port"|
|`config.readinessAddr`|string|Readiness probe server address in the format "ip:port"|
|`config.session`|object|Configuration for stateful session management|
|`config.session.key`|string|The AES-256-GCM session protection key to be used for session tokens.<br>If not set, sessions will not be encrypted.<br>For example, generated via `openssl rand -hex 32`.|
|`config.connectionTerminationDeadline`|string||
|`config.connectionMinTerminationDeadline`|string||
|`config.workerThreads`|string||
|`config.tracing`|object||
|`config.tracing.otlpEndpoint`|string||
|`config.tracing.headers`|object||
|`config.tracing.otlpProtocol`|string||
|`config.tracing.fields`|object||
|`config.tracing.fields.remove`|[]string||
|`config.tracing.fields.add`|object||
|`config.tracing.randomSampling`|string|Expression to determine the amount of *random sampling*.<br>Random sampling will initiate a new trace span if the incoming request does not have a trace already.<br>This should evaluate to either a float between 0.0-1.0 (0-100%) or true/false.<br>This defaults to 'false'.|
|`config.tracing.clientSampling`|string|Expression to determine the amount of *client sampling*.<br>Client sampling determines whether to initiate a new trace span if the incoming request does have a trace already.<br>This should evaluate to either a float between 0.0-1.0 (0-100%) or true/false.<br>This defaults to 'true'.|
|`config.tracing.path`|string|OTLP path. Default is /v1/traces|
|`config.logging`|object||
|`config.logging.filter`|string||
|`config.logging.fields`|object||
|`config.logging.fields.remove`|[]string||
|`config.logging.fields.add`|object||
|`config.logging.level`|string||
|`config.logging.format`|string||
|`config.metrics`|object||
|`config.metrics.remove`|[]string||
|`config.metrics.fields`|object||
|`config.metrics.fields.add`|object||
|`config.backend`|object||
|`config.backend.keepalives`|object||
|`config.backend.keepalives.enabled`|boolean||
|`config.backend.keepalives.time`|string||
|`config.backend.keepalives.interval`|string||
|`config.backend.keepalives.retries`|integer||
|`config.backend.connectTimeout`|string||
|`config.backend.poolIdleTimeout`|string|The maximum duration to keep an idle connection alive.|
|`config.backend.poolMaxSize`|integer|The maximum number of connections allowed in the pool, per hostname. If set, this will limit<br>the total number of connections kept alive to any given host.<br>Note: excess connections will still be created, they will just not remain idle.<br>If unset, there is no limit|
|`config.hbone`|object||
|`config.hbone.windowSize`|integer||
|`config.hbone.connectionWindowSize`|integer||
|`config.hbone.frameSize`|integer||
|`config.hbone.poolMaxStreamsPerConn`|integer||
|`config.hbone.poolUnusedReleaseTimeout`|string||
|`binds`|[]object||
|`binds[].port`|integer||
|`binds[].listeners`|[]object||
|`binds[].listeners[].name`|string||
|`binds[].listeners[].namespace`|string||
|`binds[].listeners[].hostname`|string|Can be a wildcard|
|`binds[].listeners[].protocol`|string||
|`binds[].listeners[].tls`|object||
|`binds[].listeners[].tls.cert`|string||
|`binds[].listeners[].tls.key`|string||
|`binds[].listeners[].tls.root`|string||
|`binds[].listeners[].tls.cipherSuites`|[]string|Optional cipher suite allowlist (order is preserved).|
|`binds[].listeners[].tls.minTLSVersion`|string|Minimum supported TLS version (only TLS 1.2 and 1.3 are supported).|
|`binds[].listeners[].tls.maxTLSVersion`|string|Maximum supported TLS version (only TLS 1.2 and 1.3 are supported).|
|`binds[].listeners[].routes`|[]object||
|`binds[].listeners[].routes[].name`|string||
|`binds[].listeners[].routes[].namespace`|string||
|`binds[].listeners[].routes[].ruleName`|string||
|`binds[].listeners[].routes[].hostnames`|[]string|Can be a wildcard|
|`binds[].listeners[].routes[].matches`|[]object||
|`binds[].listeners[].routes[].matches[].headers`|[]object||
|`binds[].listeners[].routes[].matches[].headers[].name`|string||
|`binds[].listeners[].routes[].matches[].headers[].value`|object|Exactly one of exact or regex may be set.|
|`binds[].listeners[].routes[].matches[].headers[].value.exact`|string||
|`binds[].listeners[].routes[].matches[].headers[].value.regex`|string||
|`binds[].listeners[].routes[].matches[].path`|object|Exactly one of exact, pathPrefix, or regex may be set.|
|`binds[].listeners[].routes[].matches[].path.exact`|string||
|`binds[].listeners[].routes[].matches[].path.pathPrefix`|string||
|`binds[].listeners[].routes[].matches[].path.regex`|string||
|`binds[].listeners[].routes[].matches[].method`|string||
|`binds[].listeners[].routes[].matches[].query`|[]object||
|`binds[].listeners[].routes[].matches[].query[].name`|string||
|`binds[].listeners[].routes[].matches[].query[].value`|object|Exactly one of exact or regex may be set.|
|`binds[].listeners[].routes[].matches[].query[].value.exact`|string||
|`binds[].listeners[].routes[].matches[].query[].value.regex`|string||
|`binds[].listeners[].routes[].policies`|object||
|`binds[].listeners[].routes[].policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`binds[].listeners[].routes[].policies.requestHeaderModifier.add`|object||
|`binds[].listeners[].routes[].policies.requestHeaderModifier.set`|object||
|`binds[].listeners[].routes[].policies.requestHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`binds[].listeners[].routes[].policies.responseHeaderModifier.add`|object||
|`binds[].listeners[].routes[].policies.responseHeaderModifier.set`|object||
|`binds[].listeners[].routes[].policies.responseHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].policies.requestRedirect.scheme`|string||
|`binds[].listeners[].routes[].policies.requestRedirect.authority`|string||
|`binds[].listeners[].routes[].policies.requestRedirect.authority.full`|string||
|`binds[].listeners[].routes[].policies.requestRedirect.authority.host`|string||
|`binds[].listeners[].routes[].policies.requestRedirect.authority.port`|integer||
|`binds[].listeners[].routes[].policies.requestRedirect.path`|object||
|`binds[].listeners[].routes[].policies.requestRedirect.path.full`|string||
|`binds[].listeners[].routes[].policies.requestRedirect.path.prefix`|string||
|`binds[].listeners[].routes[].policies.requestRedirect.status`|integer||
|`binds[].listeners[].routes[].policies.urlRewrite`|object|Modify the URL path or authority.|
|`binds[].listeners[].routes[].policies.urlRewrite.authority`|string||
|`binds[].listeners[].routes[].policies.urlRewrite.authority.full`|string||
|`binds[].listeners[].routes[].policies.urlRewrite.authority.host`|string||
|`binds[].listeners[].routes[].policies.urlRewrite.authority.port`|integer||
|`binds[].listeners[].routes[].policies.urlRewrite.path`|object||
|`binds[].listeners[].routes[].policies.urlRewrite.path.full`|string||
|`binds[].listeners[].routes[].policies.urlRewrite.path.prefix`|string||
|`binds[].listeners[].routes[].policies.requestMirror`|object|Mirror incoming requests to another destination.|
|`binds[].listeners[].routes[].policies.requestMirror.backend`|object|Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].policies.requestMirror.backend.service`|object||
|`binds[].listeners[].routes[].policies.requestMirror.backend.service.name`|object||
|`binds[].listeners[].routes[].policies.requestMirror.backend.service.name.namespace`|string||
|`binds[].listeners[].routes[].policies.requestMirror.backend.service.name.hostname`|string||
|`binds[].listeners[].routes[].policies.requestMirror.backend.service.port`|integer||
|`binds[].listeners[].routes[].policies.requestMirror.backend.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].policies.requestMirror.backend.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].policies.requestMirror.percentage`|number||
|`binds[].listeners[].routes[].policies.directResponse`|object|Directly respond to the request with a static response.|
|`binds[].listeners[].routes[].policies.directResponse.body`|array||
|`binds[].listeners[].routes[].policies.directResponse.status`|integer||
|`binds[].listeners[].routes[].policies.cors`|object|Handle CORS preflight requests and append configured CORS headers to applicable requests.|
|`binds[].listeners[].routes[].policies.cors.allowCredentials`|boolean||
|`binds[].listeners[].routes[].policies.cors.allowHeaders`|[]string||
|`binds[].listeners[].routes[].policies.cors.allowMethods`|[]string||
|`binds[].listeners[].routes[].policies.cors.allowOrigins`|[]string||
|`binds[].listeners[].routes[].policies.cors.exposeHeaders`|[]string||
|`binds[].listeners[].routes[].policies.cors.maxAge`|string||
|`binds[].listeners[].routes[].policies.mcpAuthorization`|object|Authorization policies for MCP access.|
|`binds[].listeners[].routes[].policies.mcpAuthorization.rules`|[]string||
|`binds[].listeners[].routes[].policies.authorization`|object|Authorization policies for HTTP access.|
|`binds[].listeners[].routes[].policies.authorization.rules`|[]string||
|`binds[].listeners[].routes[].policies.mcpAuthentication`|object|Authentication for MCP clients.|
|`binds[].listeners[].routes[].policies.mcpAuthentication.issuer`|string||
|`binds[].listeners[].routes[].policies.mcpAuthentication.audiences`|[]string||
|`binds[].listeners[].routes[].policies.mcpAuthentication.provider`|object||
|`binds[].listeners[].routes[].policies.mcpAuthentication.provider.auth0`|object||
|`binds[].listeners[].routes[].policies.mcpAuthentication.provider.keycloak`|object||
|`binds[].listeners[].routes[].policies.mcpAuthentication.resourceMetadata`|object||
|`binds[].listeners[].routes[].policies.mcpAuthentication.jwks`|object||
|`binds[].listeners[].routes[].policies.mcpAuthentication.jwks.file`|string||
|`binds[].listeners[].routes[].policies.mcpAuthentication.jwks.url`|string||
|`binds[].listeners[].routes[].policies.mcpAuthentication.mode`|string||
|`binds[].listeners[].routes[].policies.mcpAuthentication.jwtValidationOptions`|object|JWT validation options controlling which claims must be present in a token.<br><br>The `required_claims` set specifies which RFC 7519 registered claims must<br>exist in the token payload before validation proceeds. Only the following<br>values are recognized: `exp`, `nbf`, `aud`, `iss`, `sub`. Other registered<br>claims such as `iat` and `jti` are **not** enforced by the underlying<br>`jsonwebtoken` library and will be silently ignored.<br><br>This only enforces **presence**. Standard claims like `exp` and `nbf`<br>have their values validated independently (e.g., expiry is always checked<br>when the `exp` claim is present, regardless of this setting).<br><br>Defaults to `["exp"]`.|
|`binds[].listeners[].routes[].policies.mcpAuthentication.jwtValidationOptions.requiredClaims`|[]string|Claims that must be present in the token before validation.<br>Only "exp", "nbf", "aud", "iss", "sub" are enforced; others<br>(including "iat" and "jti") are ignored.<br>Defaults to ["exp"]. Use an empty list to require no claims.|
|`binds[].listeners[].routes[].policies.a2a`|object|Mark this traffic as A2A to enable A2A processing and telemetry.|
|`binds[].listeners[].routes[].policies.ai`|object|Mark this as LLM traffic to enable LLM processing.|
|`binds[].listeners[].routes[].policies.ai.promptGuard`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request`|[]object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].regex`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].regex.action`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].regex.rules`|[]object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].regex.rules[].builtin`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].regex.rules[].pattern`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].webhook`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].webhook.target`|object|Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].webhook.target.service`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].webhook.target.service.name`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].webhook.target.service.name.namespace`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].webhook.target.service.name.hostname`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].webhook.target.service.port`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].webhook.target.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].webhook.target.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].webhook.forwardHeaderMatches`|[]object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].webhook.forwardHeaderMatches[].name`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].webhook.forwardHeaderMatches[].value`|object|Exactly one of exact or regex may be set.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].webhook.forwardHeaderMatches[].value.exact`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].webhook.forwardHeaderMatches[].value.regex`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.model`|string|Model to use. Defaults to `omni-moderation-latest`|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.requestHeaderModifier.add`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.requestHeaderModifier.set`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.requestHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.responseHeaderModifier.add`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.responseHeaderModifier.set`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.responseHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.scheme`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.authority`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.authority.full`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.authority.host`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.authority.port`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.path`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.path.full`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.path.prefix`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.status`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request.add`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request.set`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request.remove`|[]string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request.body`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request.metadata`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response.add`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response.set`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response.remove`|[]string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response.body`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response.metadata`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS`|object|Send TLS to the backend.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.cert`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.key`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.root`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.hostname`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth`|object|Authenticate to the backend.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.passthrough`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.key`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.key.file`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.gcp`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws.accessKeyId`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws.secretAccessKey`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws.region`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws.sessionToken`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.developerImplicit`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.implicit`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.http`|object|Specify HTTP settings for the backend|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.http.version`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.http.requestTimeout`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp`|object|Specify TCP settings for the backend|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives.enabled`|boolean||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives.time`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives.interval`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives.retries`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.connectTimeout`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.connectTimeout.secs`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.connectTimeout.nanos`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.health.eviction.duration`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.health.eviction.restoreHealth`|number||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.health.eviction.consecutiveFailures`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.health.eviction.healthThreshold`|number||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service.name`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service.name.namespace`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service.name.hostname`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service.port`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails`|object|Configuration for AWS Bedrock Guardrails integration.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.guardrailIdentifier`|string|The unique identifier of the guardrail|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.guardrailVersion`|string|The version of the guardrail|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.region`|string|AWS region where the guardrail is deployed|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies`|object|Backend policies for AWS authentication (optional, defaults to implicit AWS auth)|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestHeaderModifier.add`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestHeaderModifier.set`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.responseHeaderModifier.add`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.responseHeaderModifier.set`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.responseHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.scheme`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.authority`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.authority.full`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.authority.host`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.authority.port`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.path`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.path.full`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.path.prefix`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.status`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.add`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.set`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.remove`|[]string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.body`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.metadata`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.add`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.set`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.remove`|[]string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.body`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.metadata`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS`|object|Send TLS to the backend.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.cert`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.key`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.root`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.hostname`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth`|object|Authenticate to the backend.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.passthrough`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.key`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.key.file`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.gcp`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws.accessKeyId`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws.secretAccessKey`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws.region`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws.sessionToken`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.developerImplicit`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.implicit`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.http`|object|Specify HTTP settings for the backend|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.http.version`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.http.requestTimeout`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp`|object|Specify TCP settings for the backend|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives.enabled`|boolean||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives.time`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives.interval`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives.retries`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.connectTimeout`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.connectTimeout.secs`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.connectTimeout.nanos`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction.duration`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction.restoreHealth`|number||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction.consecutiveFailures`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction.healthThreshold`|number||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.name`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.namespace`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.hostname`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.port`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor`|object|Configuration for Google Cloud Model Armor integration.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.templateId`|string|The template ID for the Model Armor configuration|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.projectId`|string|The GCP project ID|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.location`|string|The GCP region (default: us-central1)|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies`|object|Backend policies for GCP authentication (optional, defaults to implicit GCP auth)|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestHeaderModifier.add`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestHeaderModifier.set`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.responseHeaderModifier.add`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.responseHeaderModifier.set`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.responseHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.scheme`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.authority`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.authority.full`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.authority.host`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.authority.port`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.path`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.path.full`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.path.prefix`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.status`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.add`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.set`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.remove`|[]string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.body`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.metadata`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.add`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.set`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.remove`|[]string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.body`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.metadata`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS`|object|Send TLS to the backend.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.cert`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.key`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.root`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.hostname`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth`|object|Authenticate to the backend.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.passthrough`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.key`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.key.file`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.gcp`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws.accessKeyId`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws.secretAccessKey`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws.region`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws.sessionToken`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.developerImplicit`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.implicit`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.http`|object|Specify HTTP settings for the backend|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.http.version`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.http.requestTimeout`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp`|object|Specify TCP settings for the backend|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives.enabled`|boolean||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives.time`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives.interval`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives.retries`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.connectTimeout`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.connectTimeout.secs`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.connectTimeout.nanos`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.health.eviction.duration`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.health.eviction.restoreHealth`|number||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.health.eviction.consecutiveFailures`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.health.eviction.healthThreshold`|number||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service.name`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service.name.namespace`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service.name.hostname`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service.port`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].rejection`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].rejection.body`|array||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].rejection.status`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].rejection.headers`|object|Optional headers to add, set, or remove from the rejection response|
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].rejection.headers.add`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].rejection.headers.set`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.request[].rejection.headers.remove`|[]string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response`|[]object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].regex`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].regex.action`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].regex.rules`|[]object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].regex.rules[].builtin`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].regex.rules[].pattern`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].webhook`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].webhook.target`|object|Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].webhook.target.service`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].webhook.target.service.name`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].webhook.target.service.name.namespace`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].webhook.target.service.name.hostname`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].webhook.target.service.port`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].webhook.target.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].webhook.target.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].webhook.forwardHeaderMatches`|[]object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].webhook.forwardHeaderMatches[].name`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].webhook.forwardHeaderMatches[].value`|object|Exactly one of exact or regex may be set.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].webhook.forwardHeaderMatches[].value.exact`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].webhook.forwardHeaderMatches[].value.regex`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails`|object|Configuration for AWS Bedrock Guardrails integration.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.guardrailIdentifier`|string|The unique identifier of the guardrail|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.guardrailVersion`|string|The version of the guardrail|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.region`|string|AWS region where the guardrail is deployed|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies`|object|Backend policies for AWS authentication (optional, defaults to implicit AWS auth)|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestHeaderModifier.add`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestHeaderModifier.set`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.responseHeaderModifier.add`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.responseHeaderModifier.set`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.responseHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.scheme`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.authority`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.authority.full`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.authority.host`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.authority.port`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.path`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.path.full`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.path.prefix`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.status`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.add`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.set`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.remove`|[]string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.body`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.metadata`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.add`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.set`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.remove`|[]string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.body`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.metadata`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS`|object|Send TLS to the backend.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.cert`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.key`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.root`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.hostname`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth`|object|Authenticate to the backend.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.passthrough`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.key`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.key.file`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.gcp`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws.accessKeyId`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws.secretAccessKey`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws.region`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws.sessionToken`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.developerImplicit`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.implicit`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.http`|object|Specify HTTP settings for the backend|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.http.version`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.http.requestTimeout`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp`|object|Specify TCP settings for the backend|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives.enabled`|boolean||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives.time`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives.interval`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives.retries`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.connectTimeout`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.connectTimeout.secs`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.connectTimeout.nanos`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction.duration`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction.restoreHealth`|number||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction.consecutiveFailures`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction.healthThreshold`|number||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.name`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.namespace`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.hostname`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.port`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor`|object|Configuration for Google Cloud Model Armor integration.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.templateId`|string|The template ID for the Model Armor configuration|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.projectId`|string|The GCP project ID|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.location`|string|The GCP region (default: us-central1)|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies`|object|Backend policies for GCP authentication (optional, defaults to implicit GCP auth)|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestHeaderModifier.add`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestHeaderModifier.set`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.responseHeaderModifier.add`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.responseHeaderModifier.set`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.responseHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.scheme`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.authority`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.authority.full`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.authority.host`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.authority.port`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.path`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.path.full`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.path.prefix`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.status`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.add`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.set`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.remove`|[]string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.body`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.metadata`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.add`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.set`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.remove`|[]string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.body`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.metadata`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS`|object|Send TLS to the backend.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.cert`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.key`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.root`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.hostname`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth`|object|Authenticate to the backend.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.passthrough`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.key`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.key.file`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.gcp`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws.accessKeyId`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws.secretAccessKey`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws.region`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws.sessionToken`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.developerImplicit`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.implicit`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.http`|object|Specify HTTP settings for the backend|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.http.version`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.http.requestTimeout`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp`|object|Specify TCP settings for the backend|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives.enabled`|boolean||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives.time`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives.interval`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives.retries`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.connectTimeout`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.connectTimeout.secs`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.connectTimeout.nanos`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.health.eviction.duration`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.health.eviction.restoreHealth`|number||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.health.eviction.consecutiveFailures`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.health.eviction.healthThreshold`|number||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service.name`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service.name.namespace`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service.name.hostname`|string||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service.port`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].rejection`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].rejection.body`|array||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].rejection.status`|integer||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].rejection.headers`|object|Optional headers to add, set, or remove from the rejection response|
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].rejection.headers.add`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].rejection.headers.set`|object||
|`binds[].listeners[].routes[].policies.ai.promptGuard.response[].rejection.headers.remove`|[]string||
|`binds[].listeners[].routes[].policies.ai.defaults`|object||
|`binds[].listeners[].routes[].policies.ai.overrides`|object||
|`binds[].listeners[].routes[].policies.ai.transformations`|object||
|`binds[].listeners[].routes[].policies.ai.prompts`|object||
|`binds[].listeners[].routes[].policies.ai.prompts.append`|[]object||
|`binds[].listeners[].routes[].policies.ai.prompts.append[].role`|string||
|`binds[].listeners[].routes[].policies.ai.prompts.append[].content`|string||
|`binds[].listeners[].routes[].policies.ai.prompts.prepend`|[]object||
|`binds[].listeners[].routes[].policies.ai.prompts.prepend[].role`|string||
|`binds[].listeners[].routes[].policies.ai.prompts.prepend[].content`|string||
|`binds[].listeners[].routes[].policies.ai.modelAliases`|object||
|`binds[].listeners[].routes[].policies.ai.promptCaching`|object||
|`binds[].listeners[].routes[].policies.ai.promptCaching.cacheSystem`|boolean||
|`binds[].listeners[].routes[].policies.ai.promptCaching.cacheMessages`|boolean||
|`binds[].listeners[].routes[].policies.ai.promptCaching.cacheTools`|boolean||
|`binds[].listeners[].routes[].policies.ai.promptCaching.minTokens`|integer||
|`binds[].listeners[].routes[].policies.ai.routes`|object||
|`binds[].listeners[].routes[].policies.backendTLS`|object|Send TLS to the backend.|
|`binds[].listeners[].routes[].policies.backendTLS.cert`|string||
|`binds[].listeners[].routes[].policies.backendTLS.key`|string||
|`binds[].listeners[].routes[].policies.backendTLS.root`|string||
|`binds[].listeners[].routes[].policies.backendTLS.hostname`|string||
|`binds[].listeners[].routes[].policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].routes[].policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].routes[].policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].routes[].policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].routes[].policies.backendTunnel`|object|Tunnel to the backend.|
|`binds[].listeners[].routes[].policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].policies.backendTunnel.proxy.service`|object||
|`binds[].listeners[].routes[].policies.backendTunnel.proxy.service.name`|object||
|`binds[].listeners[].routes[].policies.backendTunnel.proxy.service.name.namespace`|string||
|`binds[].listeners[].routes[].policies.backendTunnel.proxy.service.name.hostname`|string||
|`binds[].listeners[].routes[].policies.backendTunnel.proxy.service.port`|integer||
|`binds[].listeners[].routes[].policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].policies.backendAuth`|object|Authenticate to the backend.|
|`binds[].listeners[].routes[].policies.backendAuth.passthrough`|object||
|`binds[].listeners[].routes[].policies.backendAuth.key`|object||
|`binds[].listeners[].routes[].policies.backendAuth.key.file`|string||
|`binds[].listeners[].routes[].policies.backendAuth.gcp`|object||
|`binds[].listeners[].routes[].policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`binds[].listeners[].routes[].policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].policies.backendAuth.aws`|object||
|`binds[].listeners[].routes[].policies.backendAuth.aws.accessKeyId`|string||
|`binds[].listeners[].routes[].policies.backendAuth.aws.secretAccessKey`|string||
|`binds[].listeners[].routes[].policies.backendAuth.aws.region`|string||
|`binds[].listeners[].routes[].policies.backendAuth.aws.sessionToken`|string||
|`binds[].listeners[].routes[].policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`binds[].listeners[].routes[].policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`binds[].listeners[].routes[].policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`binds[].listeners[].routes[].policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`binds[].listeners[].routes[].policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`binds[].listeners[].routes[].policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`binds[].listeners[].routes[].policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`binds[].listeners[].routes[].policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`binds[].listeners[].routes[].policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`binds[].listeners[].routes[].policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`binds[].listeners[].routes[].policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`binds[].listeners[].routes[].policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`binds[].listeners[].routes[].policies.backendAuth.azure.developerImplicit`|object||
|`binds[].listeners[].routes[].policies.backendAuth.azure.implicit`|object||
|`binds[].listeners[].routes[].policies.localRateLimit`|[]object|Rate limit incoming requests. State is kept local.|
|`binds[].listeners[].routes[].policies.localRateLimit[].maxTokens`|integer||
|`binds[].listeners[].routes[].policies.localRateLimit[].tokensPerFill`|integer||
|`binds[].listeners[].routes[].policies.localRateLimit[].fillInterval`|string||
|`binds[].listeners[].routes[].policies.localRateLimit[].type`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit`|object|Rate limit incoming requests. State is managed by a remote server.|
|`binds[].listeners[].routes[].policies.remoteRateLimit.service`|object||
|`binds[].listeners[].routes[].policies.remoteRateLimit.service.name`|object||
|`binds[].listeners[].routes[].policies.remoteRateLimit.service.name.namespace`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.service.name.hostname`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.service.port`|integer||
|`binds[].listeners[].routes[].policies.remoteRateLimit.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].policies.remoteRateLimit.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].policies.remoteRateLimit.domain`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies`|object|Policies to connect to the backend|
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.requestHeaderModifier.add`|object||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.requestHeaderModifier.set`|object||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.requestHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.responseHeaderModifier.add`|object||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.responseHeaderModifier.set`|object||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.responseHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.requestRedirect.scheme`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.requestRedirect.authority`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.requestRedirect.authority.full`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.requestRedirect.authority.host`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.requestRedirect.authority.port`|integer||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.requestRedirect.path`|object||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.requestRedirect.path.full`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.requestRedirect.path.prefix`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.requestRedirect.status`|integer||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.transformations.request`|object||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.transformations.request.add`|object||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.transformations.request.set`|object||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.transformations.request.remove`|[]string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.transformations.request.body`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.transformations.request.metadata`|object||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.transformations.response`|object||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.transformations.response.add`|object||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.transformations.response.set`|object||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.transformations.response.remove`|[]string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.transformations.response.body`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.transformations.response.metadata`|object||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendTLS`|object|Send TLS to the backend.|
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendTLS.cert`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendTLS.key`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendTLS.root`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendTLS.hostname`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendAuth`|object|Authenticate to the backend.|
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendAuth.passthrough`|object||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendAuth.key`|object||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendAuth.key.file`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendAuth.gcp`|object||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendAuth.aws`|object||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendAuth.aws.accessKeyId`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendAuth.aws.secretAccessKey`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendAuth.aws.region`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendAuth.aws.sessionToken`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendAuth.azure.developerImplicit`|object||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendAuth.azure.implicit`|object||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.http`|object|Specify HTTP settings for the backend|
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.http.version`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.http.requestTimeout`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.tcp`|object|Specify TCP settings for the backend|
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.tcp.keepalives`|object||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.tcp.keepalives.enabled`|boolean||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.tcp.keepalives.time`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.tcp.keepalives.interval`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.tcp.keepalives.retries`|integer||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.tcp.connectTimeout`|object||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.tcp.connectTimeout.secs`|integer||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.tcp.connectTimeout.nanos`|integer||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.health.eviction.duration`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.health.eviction.restoreHealth`|number||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.health.eviction.consecutiveFailures`|integer||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.health.eviction.healthThreshold`|number||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendTunnel.proxy.service`|object||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendTunnel.proxy.service.name`|object||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendTunnel.proxy.service.name.namespace`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendTunnel.proxy.service.name.hostname`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendTunnel.proxy.service.port`|integer||
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].policies.remoteRateLimit.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].policies.remoteRateLimit.descriptors`|[]object||
|`binds[].listeners[].routes[].policies.remoteRateLimit.descriptors[].entries`|[]object||
|`binds[].listeners[].routes[].policies.remoteRateLimit.descriptors[].entries[].key`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.descriptors[].entries[].value`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.descriptors[].type`|string||
|`binds[].listeners[].routes[].policies.remoteRateLimit.failureMode`|string|Behavior when the remote rate limit service is unavailable or returns an error.<br>Defaults to failClosed, denying requests with a 500 status on service failure.|
|`binds[].listeners[].routes[].policies.jwtAuth`|object|Authenticate incoming JWT requests.|
|`binds[].listeners[].routes[].policies.jwtAuth.mode`|string||
|`binds[].listeners[].routes[].policies.jwtAuth.providers`|[]object||
|`binds[].listeners[].routes[].policies.jwtAuth.providers[].issuer`|string||
|`binds[].listeners[].routes[].policies.jwtAuth.providers[].audiences`|[]string||
|`binds[].listeners[].routes[].policies.jwtAuth.providers[].jwks`|object||
|`binds[].listeners[].routes[].policies.jwtAuth.providers[].jwks.file`|string||
|`binds[].listeners[].routes[].policies.jwtAuth.providers[].jwks.url`|string||
|`binds[].listeners[].routes[].policies.jwtAuth.providers[].jwtValidationOptions`|object|JWT validation options controlling which claims must be present in a token.<br><br>The `required_claims` set specifies which RFC 7519 registered claims must<br>exist in the token payload before validation proceeds. Only the following<br>values are recognized: `exp`, `nbf`, `aud`, `iss`, `sub`. Other registered<br>claims such as `iat` and `jti` are **not** enforced by the underlying<br>`jsonwebtoken` library and will be silently ignored.<br><br>This only enforces **presence**. Standard claims like `exp` and `nbf`<br>have their values validated independently (e.g., expiry is always checked<br>when the `exp` claim is present, regardless of this setting).<br><br>Defaults to `["exp"]`.|
|`binds[].listeners[].routes[].policies.jwtAuth.providers[].jwtValidationOptions.requiredClaims`|[]string|Claims that must be present in the token before validation.<br>Only "exp", "nbf", "aud", "iss", "sub" are enforced; others<br>(including "iat" and "jti") are ignored.<br>Defaults to ["exp"]. Use an empty list to require no claims.|
|`binds[].listeners[].routes[].policies.jwtAuth.mode`|string||
|`binds[].listeners[].routes[].policies.jwtAuth.issuer`|string||
|`binds[].listeners[].routes[].policies.jwtAuth.audiences`|[]string||
|`binds[].listeners[].routes[].policies.jwtAuth.jwks`|object||
|`binds[].listeners[].routes[].policies.jwtAuth.jwks.file`|string||
|`binds[].listeners[].routes[].policies.jwtAuth.jwks.url`|string||
|`binds[].listeners[].routes[].policies.jwtAuth.jwtValidationOptions`|object|JWT validation options controlling which claims must be present in a token.<br><br>The `required_claims` set specifies which RFC 7519 registered claims must<br>exist in the token payload before validation proceeds. Only the following<br>values are recognized: `exp`, `nbf`, `aud`, `iss`, `sub`. Other registered<br>claims such as `iat` and `jti` are **not** enforced by the underlying<br>`jsonwebtoken` library and will be silently ignored.<br><br>This only enforces **presence**. Standard claims like `exp` and `nbf`<br>have their values validated independently (e.g., expiry is always checked<br>when the `exp` claim is present, regardless of this setting).<br><br>Defaults to `["exp"]`.|
|`binds[].listeners[].routes[].policies.jwtAuth.jwtValidationOptions.requiredClaims`|[]string|Claims that must be present in the token before validation.<br>Only "exp", "nbf", "aud", "iss", "sub" are enforced; others<br>(including "iat" and "jti") are ignored.<br>Defaults to ["exp"]. Use an empty list to require no claims.|
|`binds[].listeners[].routes[].policies.oidc`|object|Authenticate incoming browser requests with OIDC authorization code flow.|
|`binds[].listeners[].routes[].policies.oidc.issuer`|string|Issuer used for discovery and ID token validation.|
|`binds[].listeners[].routes[].policies.oidc.discovery`|object|Optional discovery document override. If omitted, discovery uses<br>`${issuer}/.well-known/openid-configuration`.|
|`binds[].listeners[].routes[].policies.oidc.discovery.file`|string||
|`binds[].listeners[].routes[].policies.oidc.discovery.url`|string||
|`binds[].listeners[].routes[].policies.oidc.authorizationEndpoint`|string|Authorization endpoint used to start the browser login flow.|
|`binds[].listeners[].routes[].policies.oidc.tokenEndpoint`|string|Token endpoint used to exchange the authorization code.|
|`binds[].listeners[].routes[].policies.oidc.tokenEndpointAuth`|string|Token endpoint client authentication method for explicit provider configuration.<br><br>Discovery mode derives this from provider metadata. Explicit mode defaults to<br>`clientSecretBasic` when omitted.|
|`binds[].listeners[].routes[].policies.oidc.jwks`|object|JWKS source used to validate returned ID tokens.|
|`binds[].listeners[].routes[].policies.oidc.jwks.file`|string||
|`binds[].listeners[].routes[].policies.oidc.jwks.url`|string||
|`binds[].listeners[].routes[].policies.oidc.clientId`|string|OAuth2 client identifier used for authorization and token exchange.|
|`binds[].listeners[].routes[].policies.oidc.clientSecret`|string|OAuth2 client secret used for token exchange.|
|`binds[].listeners[].routes[].policies.oidc.redirectURI`|string|Absolute callback URI handled by the gateway.<br>This policy always redirects unauthenticated non-callback requests back through this login<br>flow.|
|`binds[].listeners[].routes[].policies.oidc.scopes`|[]string|Additional OAuth2 scopes to request. `openid` is always included.|
|`binds[].listeners[].routes[].policies.basicAuth`|object|Authenticate incoming requests using Basic Authentication with htpasswd.|
|`binds[].listeners[].routes[].policies.basicAuth.htpasswd`|object|.htpasswd file contents/reference|
|`binds[].listeners[].routes[].policies.basicAuth.htpasswd.file`|string||
|`binds[].listeners[].routes[].policies.basicAuth.realm`|string|Realm name for the WWW-Authenticate header|
|`binds[].listeners[].routes[].policies.basicAuth.mode`|string|Validation mode for basic authentication|
|`binds[].listeners[].routes[].policies.apiKey`|object|Authenticate incoming requests using API Keys|
|`binds[].listeners[].routes[].policies.apiKey.keys`|[]object|List of API keys|
|`binds[].listeners[].routes[].policies.apiKey.keys[].key`|string||
|`binds[].listeners[].routes[].policies.apiKey.keys[].metadata`|any||
|`binds[].listeners[].routes[].policies.apiKey.mode`|string|Validation mode for API keys|
|`binds[].listeners[].routes[].policies.extAuthz`|object|Authenticate incoming requests by calling an external authorization server.|
|`binds[].listeners[].routes[].policies.extAuthz.service`|object||
|`binds[].listeners[].routes[].policies.extAuthz.service.name`|object||
|`binds[].listeners[].routes[].policies.extAuthz.service.name.namespace`|string||
|`binds[].listeners[].routes[].policies.extAuthz.service.name.hostname`|string||
|`binds[].listeners[].routes[].policies.extAuthz.service.port`|integer||
|`binds[].listeners[].routes[].policies.extAuthz.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].policies.extAuthz.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].policies.extAuthz.policies`|object|Policies to connect to the backend|
|`binds[].listeners[].routes[].policies.extAuthz.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`binds[].listeners[].routes[].policies.extAuthz.policies.requestHeaderModifier.add`|object||
|`binds[].listeners[].routes[].policies.extAuthz.policies.requestHeaderModifier.set`|object||
|`binds[].listeners[].routes[].policies.extAuthz.policies.requestHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`binds[].listeners[].routes[].policies.extAuthz.policies.responseHeaderModifier.add`|object||
|`binds[].listeners[].routes[].policies.extAuthz.policies.responseHeaderModifier.set`|object||
|`binds[].listeners[].routes[].policies.extAuthz.policies.responseHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].policies.extAuthz.policies.requestRedirect.scheme`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.requestRedirect.authority`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.requestRedirect.authority.full`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.requestRedirect.authority.host`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.requestRedirect.authority.port`|integer||
|`binds[].listeners[].routes[].policies.extAuthz.policies.requestRedirect.path`|object||
|`binds[].listeners[].routes[].policies.extAuthz.policies.requestRedirect.path.full`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.requestRedirect.path.prefix`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.requestRedirect.status`|integer||
|`binds[].listeners[].routes[].policies.extAuthz.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`binds[].listeners[].routes[].policies.extAuthz.policies.transformations.request`|object||
|`binds[].listeners[].routes[].policies.extAuthz.policies.transformations.request.add`|object||
|`binds[].listeners[].routes[].policies.extAuthz.policies.transformations.request.set`|object||
|`binds[].listeners[].routes[].policies.extAuthz.policies.transformations.request.remove`|[]string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.transformations.request.body`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.transformations.request.metadata`|object||
|`binds[].listeners[].routes[].policies.extAuthz.policies.transformations.response`|object||
|`binds[].listeners[].routes[].policies.extAuthz.policies.transformations.response.add`|object||
|`binds[].listeners[].routes[].policies.extAuthz.policies.transformations.response.set`|object||
|`binds[].listeners[].routes[].policies.extAuthz.policies.transformations.response.remove`|[]string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.transformations.response.body`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.transformations.response.metadata`|object||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendTLS`|object|Send TLS to the backend.|
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendTLS.cert`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendTLS.key`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendTLS.root`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendTLS.hostname`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendAuth`|object|Authenticate to the backend.|
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendAuth.passthrough`|object||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendAuth.key`|object||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendAuth.key.file`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendAuth.gcp`|object||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendAuth.aws`|object||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendAuth.aws.accessKeyId`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendAuth.aws.secretAccessKey`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendAuth.aws.region`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendAuth.aws.sessionToken`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendAuth.azure.developerImplicit`|object||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendAuth.azure.implicit`|object||
|`binds[].listeners[].routes[].policies.extAuthz.policies.http`|object|Specify HTTP settings for the backend|
|`binds[].listeners[].routes[].policies.extAuthz.policies.http.version`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.http.requestTimeout`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.tcp`|object|Specify TCP settings for the backend|
|`binds[].listeners[].routes[].policies.extAuthz.policies.tcp.keepalives`|object||
|`binds[].listeners[].routes[].policies.extAuthz.policies.tcp.keepalives.enabled`|boolean||
|`binds[].listeners[].routes[].policies.extAuthz.policies.tcp.keepalives.time`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.tcp.keepalives.interval`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.tcp.keepalives.retries`|integer||
|`binds[].listeners[].routes[].policies.extAuthz.policies.tcp.connectTimeout`|object||
|`binds[].listeners[].routes[].policies.extAuthz.policies.tcp.connectTimeout.secs`|integer||
|`binds[].listeners[].routes[].policies.extAuthz.policies.tcp.connectTimeout.nanos`|integer||
|`binds[].listeners[].routes[].policies.extAuthz.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`binds[].listeners[].routes[].policies.extAuthz.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`binds[].listeners[].routes[].policies.extAuthz.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`binds[].listeners[].routes[].policies.extAuthz.policies.health.eviction.duration`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.health.eviction.restoreHealth`|number||
|`binds[].listeners[].routes[].policies.extAuthz.policies.health.eviction.consecutiveFailures`|integer||
|`binds[].listeners[].routes[].policies.extAuthz.policies.health.eviction.healthThreshold`|number||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendTunnel.proxy.service`|object||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendTunnel.proxy.service.name`|object||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendTunnel.proxy.service.name.namespace`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendTunnel.proxy.service.name.hostname`|string||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendTunnel.proxy.service.port`|integer||
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].policies.extAuthz.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].policies.extAuthz.protocol`|object|The ext_authz protocol to use. Unless you need to integrate with an HTTP-only server, gRPC is recommended.<br>Exactly one of grpc or http may be set.|
|`binds[].listeners[].routes[].policies.extAuthz.protocol.grpc`|object||
|`binds[].listeners[].routes[].policies.extAuthz.protocol.grpc.context`|object|Additional context to send to the authorization service.<br>This maps to the `context_extensions` field of the request, and only allows static values.|
|`binds[].listeners[].routes[].policies.extAuthz.protocol.grpc.metadata`|object|Additional metadata to send to the authorization service.<br>This maps to the `metadata_context.filter_metadata` field of the request, and allows dynamic CEL expressions.<br>If unset, by default the `envoy.filters.http.jwt_authn` key is set if the JWT policy is used as well, for compatibility.|
|`binds[].listeners[].routes[].policies.extAuthz.protocol.http`|object||
|`binds[].listeners[].routes[].policies.extAuthz.protocol.http.path`|string||
|`binds[].listeners[].routes[].policies.extAuthz.protocol.http.redirect`|string|When using the HTTP protocol, and the server returns unauthorized, redirect to the URL resolved by<br>the provided expression rather than directly returning the error.|
|`binds[].listeners[].routes[].policies.extAuthz.protocol.http.includeResponseHeaders`|[]string|Specific headers from the authorization response will be copied into the request to the backend.|
|`binds[].listeners[].routes[].policies.extAuthz.protocol.http.addRequestHeaders`|object|Specific headers to add in the authorization request (empty = all headers), based on the expression|
|`binds[].listeners[].routes[].policies.extAuthz.protocol.http.metadata`|object|Metadata to include under the `extauthz` variable, based on the authorization response.|
|`binds[].listeners[].routes[].policies.extAuthz.failureMode`|string|Behavior when the authorization service is unavailable or returns an error|
|`binds[].listeners[].routes[].policies.extAuthz.failureMode.denyWithStatus`|integer||
|`binds[].listeners[].routes[].policies.extAuthz.includeRequestHeaders`|[]string|Specific headers to include in the authorization request.<br>If unset, the gRPC protocol sends all request headers. The HTTP protocol sends only 'Authorization'.|
|`binds[].listeners[].routes[].policies.extAuthz.includeRequestBody`|object|Options for including the request body in the authorization request|
|`binds[].listeners[].routes[].policies.extAuthz.includeRequestBody.maxRequestBytes`|integer|Maximum size of request body to buffer (default: 8192)|
|`binds[].listeners[].routes[].policies.extAuthz.includeRequestBody.allowPartialMessage`|boolean|If true, send partial body when max_request_bytes is reached|
|`binds[].listeners[].routes[].policies.extAuthz.includeRequestBody.packAsBytes`|boolean|If true, pack body as raw bytes in gRPC|
|`binds[].listeners[].routes[].policies.extProc`|object|Extend agentgateway with an external processor|
|`binds[].listeners[].routes[].policies.extProc.service`|object||
|`binds[].listeners[].routes[].policies.extProc.service.name`|object||
|`binds[].listeners[].routes[].policies.extProc.service.name.namespace`|string||
|`binds[].listeners[].routes[].policies.extProc.service.name.hostname`|string||
|`binds[].listeners[].routes[].policies.extProc.service.port`|integer||
|`binds[].listeners[].routes[].policies.extProc.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].policies.extProc.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].policies.extProc.policies`|object|Policies to connect to the backend|
|`binds[].listeners[].routes[].policies.extProc.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`binds[].listeners[].routes[].policies.extProc.policies.requestHeaderModifier.add`|object||
|`binds[].listeners[].routes[].policies.extProc.policies.requestHeaderModifier.set`|object||
|`binds[].listeners[].routes[].policies.extProc.policies.requestHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].policies.extProc.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`binds[].listeners[].routes[].policies.extProc.policies.responseHeaderModifier.add`|object||
|`binds[].listeners[].routes[].policies.extProc.policies.responseHeaderModifier.set`|object||
|`binds[].listeners[].routes[].policies.extProc.policies.responseHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].policies.extProc.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].policies.extProc.policies.requestRedirect.scheme`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.requestRedirect.authority`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.requestRedirect.authority.full`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.requestRedirect.authority.host`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.requestRedirect.authority.port`|integer||
|`binds[].listeners[].routes[].policies.extProc.policies.requestRedirect.path`|object||
|`binds[].listeners[].routes[].policies.extProc.policies.requestRedirect.path.full`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.requestRedirect.path.prefix`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.requestRedirect.status`|integer||
|`binds[].listeners[].routes[].policies.extProc.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`binds[].listeners[].routes[].policies.extProc.policies.transformations.request`|object||
|`binds[].listeners[].routes[].policies.extProc.policies.transformations.request.add`|object||
|`binds[].listeners[].routes[].policies.extProc.policies.transformations.request.set`|object||
|`binds[].listeners[].routes[].policies.extProc.policies.transformations.request.remove`|[]string||
|`binds[].listeners[].routes[].policies.extProc.policies.transformations.request.body`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.transformations.request.metadata`|object||
|`binds[].listeners[].routes[].policies.extProc.policies.transformations.response`|object||
|`binds[].listeners[].routes[].policies.extProc.policies.transformations.response.add`|object||
|`binds[].listeners[].routes[].policies.extProc.policies.transformations.response.set`|object||
|`binds[].listeners[].routes[].policies.extProc.policies.transformations.response.remove`|[]string||
|`binds[].listeners[].routes[].policies.extProc.policies.transformations.response.body`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.transformations.response.metadata`|object||
|`binds[].listeners[].routes[].policies.extProc.policies.backendTLS`|object|Send TLS to the backend.|
|`binds[].listeners[].routes[].policies.extProc.policies.backendTLS.cert`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.backendTLS.key`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.backendTLS.root`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.backendTLS.hostname`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].routes[].policies.extProc.policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].routes[].policies.extProc.policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].routes[].policies.extProc.policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].routes[].policies.extProc.policies.backendAuth`|object|Authenticate to the backend.|
|`binds[].listeners[].routes[].policies.extProc.policies.backendAuth.passthrough`|object||
|`binds[].listeners[].routes[].policies.extProc.policies.backendAuth.key`|object||
|`binds[].listeners[].routes[].policies.extProc.policies.backendAuth.key.file`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.backendAuth.gcp`|object||
|`binds[].listeners[].routes[].policies.extProc.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`binds[].listeners[].routes[].policies.extProc.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.backendAuth.aws`|object||
|`binds[].listeners[].routes[].policies.extProc.policies.backendAuth.aws.accessKeyId`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.backendAuth.aws.secretAccessKey`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.backendAuth.aws.region`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.backendAuth.aws.sessionToken`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`binds[].listeners[].routes[].policies.extProc.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`binds[].listeners[].routes[].policies.extProc.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`binds[].listeners[].routes[].policies.extProc.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`binds[].listeners[].routes[].policies.extProc.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`binds[].listeners[].routes[].policies.extProc.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`binds[].listeners[].routes[].policies.extProc.policies.backendAuth.azure.developerImplicit`|object||
|`binds[].listeners[].routes[].policies.extProc.policies.backendAuth.azure.implicit`|object||
|`binds[].listeners[].routes[].policies.extProc.policies.http`|object|Specify HTTP settings for the backend|
|`binds[].listeners[].routes[].policies.extProc.policies.http.version`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.http.requestTimeout`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.tcp`|object|Specify TCP settings for the backend|
|`binds[].listeners[].routes[].policies.extProc.policies.tcp.keepalives`|object||
|`binds[].listeners[].routes[].policies.extProc.policies.tcp.keepalives.enabled`|boolean||
|`binds[].listeners[].routes[].policies.extProc.policies.tcp.keepalives.time`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.tcp.keepalives.interval`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.tcp.keepalives.retries`|integer||
|`binds[].listeners[].routes[].policies.extProc.policies.tcp.connectTimeout`|object||
|`binds[].listeners[].routes[].policies.extProc.policies.tcp.connectTimeout.secs`|integer||
|`binds[].listeners[].routes[].policies.extProc.policies.tcp.connectTimeout.nanos`|integer||
|`binds[].listeners[].routes[].policies.extProc.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`binds[].listeners[].routes[].policies.extProc.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`binds[].listeners[].routes[].policies.extProc.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`binds[].listeners[].routes[].policies.extProc.policies.health.eviction.duration`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.health.eviction.restoreHealth`|number||
|`binds[].listeners[].routes[].policies.extProc.policies.health.eviction.consecutiveFailures`|integer||
|`binds[].listeners[].routes[].policies.extProc.policies.health.eviction.healthThreshold`|number||
|`binds[].listeners[].routes[].policies.extProc.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`binds[].listeners[].routes[].policies.extProc.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].policies.extProc.policies.backendTunnel.proxy.service`|object||
|`binds[].listeners[].routes[].policies.extProc.policies.backendTunnel.proxy.service.name`|object||
|`binds[].listeners[].routes[].policies.extProc.policies.backendTunnel.proxy.service.name.namespace`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.backendTunnel.proxy.service.name.hostname`|string||
|`binds[].listeners[].routes[].policies.extProc.policies.backendTunnel.proxy.service.port`|integer||
|`binds[].listeners[].routes[].policies.extProc.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].policies.extProc.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].policies.extProc.failureMode`|string|Behavior when the ext_proc service is unavailable or returns an error|
|`binds[].listeners[].routes[].policies.extProc.metadataContext`|object|Additional metadata to send to the external processing service.<br>Maps to the `metadata_context.filter_metadata` field in ProcessingRequest, and allows dynamic CEL expressions.|
|`binds[].listeners[].routes[].policies.extProc.requestAttributes`|object|Maps to the request `attributes` field in ProcessingRequest, and allows dynamic CEL expressions.|
|`binds[].listeners[].routes[].policies.extProc.responseAttributes`|object|Maps to the response `attributes` field in ProcessingRequest, and allows dynamic CEL expressions.|
|`binds[].listeners[].routes[].policies.transformations`|object|Modify requests and responses|
|`binds[].listeners[].routes[].policies.transformations.request`|object||
|`binds[].listeners[].routes[].policies.transformations.request.add`|object||
|`binds[].listeners[].routes[].policies.transformations.request.set`|object||
|`binds[].listeners[].routes[].policies.transformations.request.remove`|[]string||
|`binds[].listeners[].routes[].policies.transformations.request.body`|string||
|`binds[].listeners[].routes[].policies.transformations.request.metadata`|object||
|`binds[].listeners[].routes[].policies.transformations.response`|object||
|`binds[].listeners[].routes[].policies.transformations.response.add`|object||
|`binds[].listeners[].routes[].policies.transformations.response.set`|object||
|`binds[].listeners[].routes[].policies.transformations.response.remove`|[]string||
|`binds[].listeners[].routes[].policies.transformations.response.body`|string||
|`binds[].listeners[].routes[].policies.transformations.response.metadata`|object||
|`binds[].listeners[].routes[].policies.csrf`|object|Handle CSRF protection by validating request origins against configured allowed origins.|
|`binds[].listeners[].routes[].policies.csrf.additionalOrigins`|[]string||
|`binds[].listeners[].routes[].policies.timeout`|object|Timeout requests that exceed the configured duration.|
|`binds[].listeners[].routes[].policies.timeout.requestTimeout`|string||
|`binds[].listeners[].routes[].policies.timeout.backendRequestTimeout`|string||
|`binds[].listeners[].routes[].policies.retry`|object|Retry matching requests.|
|`binds[].listeners[].routes[].policies.retry.attempts`|integer||
|`binds[].listeners[].routes[].policies.retry.backoff`|string||
|`binds[].listeners[].routes[].policies.retry.codes`|[]integer||
|`binds[].listeners[].routes[].backends`|[]object||
|`binds[].listeners[].routes[].backends[].service`|object||
|`binds[].listeners[].routes[].backends[].service.name`|object||
|`binds[].listeners[].routes[].backends[].service.name.namespace`|string||
|`binds[].listeners[].routes[].backends[].service.name.hostname`|string||
|`binds[].listeners[].routes[].backends[].service.port`|integer||
|`binds[].listeners[].routes[].backends[].backend`|string||
|`binds[].listeners[].routes[].backends[].host`|string||
|`binds[].listeners[].routes[].backends[].dynamic`|object||
|`binds[].listeners[].routes[].backends[].mcp`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets`|[]object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].sse`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].sse.host`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].sse.port`|integer||
|`binds[].listeners[].routes[].backends[].mcp.targets[].sse.path`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].mcp`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].mcp.host`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].mcp.port`|integer||
|`binds[].listeners[].routes[].backends[].mcp.targets[].mcp.path`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].stdio`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].stdio.cmd`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].stdio.args`|[]string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].stdio.env`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].openapi`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].openapi.host`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].openapi.port`|integer||
|`binds[].listeners[].routes[].backends[].mcp.targets[].openapi.path`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].openapi.schema`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].openapi.schema.file`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].openapi.schema.url`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].name`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.requestHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.requestHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.requestHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.responseHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.responseHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.responseHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.requestRedirect.scheme`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.requestRedirect.authority`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.requestRedirect.authority.full`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.requestRedirect.authority.host`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.requestRedirect.authority.port`|integer||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.requestRedirect.path`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.requestRedirect.path.full`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.requestRedirect.path.prefix`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.requestRedirect.status`|integer||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.transformations.request`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.transformations.request.add`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.transformations.request.set`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.transformations.request.remove`|[]string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.transformations.request.body`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.transformations.request.metadata`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.transformations.response`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.transformations.response.add`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.transformations.response.set`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.transformations.response.remove`|[]string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.transformations.response.body`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.transformations.response.metadata`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendTLS`|object|Send TLS to the backend.|
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendTLS.cert`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendTLS.key`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendTLS.root`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendTLS.hostname`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendAuth`|object|Authenticate to the backend.|
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendAuth.passthrough`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendAuth.key`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendAuth.key.file`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendAuth.gcp`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendAuth.aws`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendAuth.aws.accessKeyId`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendAuth.aws.secretAccessKey`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendAuth.aws.region`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendAuth.aws.sessionToken`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendAuth.azure.developerImplicit`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendAuth.azure.implicit`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.http`|object|Specify HTTP settings for the backend|
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.http.version`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.http.requestTimeout`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.tcp`|object|Specify TCP settings for the backend|
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.tcp.keepalives`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.tcp.keepalives.enabled`|boolean||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.tcp.keepalives.time`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.tcp.keepalives.interval`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.tcp.keepalives.retries`|integer||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.tcp.connectTimeout`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.tcp.connectTimeout.secs`|integer||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.tcp.connectTimeout.nanos`|integer||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.health.eviction.duration`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.health.eviction.restoreHealth`|number||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.health.eviction.consecutiveFailures`|integer||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.health.eviction.healthThreshold`|number||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendTunnel.proxy.service`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendTunnel.proxy.service.name`|object||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendTunnel.proxy.service.name.namespace`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendTunnel.proxy.service.name.hostname`|string||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendTunnel.proxy.service.port`|integer||
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.mcpAuthorization`|object|Authorization policies for MCP access.|
|`binds[].listeners[].routes[].backends[].mcp.targets[].policies.mcpAuthorization.rules`|[]string||
|`binds[].listeners[].routes[].backends[].mcp.statefulMode`|string||
|`binds[].listeners[].routes[].backends[].mcp.prefixMode`|string||
|`binds[].listeners[].routes[].backends[].mcp.failureMode`|string|Behavior when one or more MCP targets fail to initialize or fail during fanout.<br>Defaults to `failClosed`.|
|`binds[].listeners[].routes[].backends[].ai`|object||
|`binds[].listeners[].routes[].backends[].ai.name`|string||
|`binds[].listeners[].routes[].backends[].ai.provider`|object|Exactly one of openAI, gemini, vertex, anthropic, bedrock, or azure may be set.|
|`binds[].listeners[].routes[].backends[].ai.provider.openAI`|object||
|`binds[].listeners[].routes[].backends[].ai.provider.openAI.model`|string||
|`binds[].listeners[].routes[].backends[].ai.provider.gemini`|object||
|`binds[].listeners[].routes[].backends[].ai.provider.gemini.model`|string||
|`binds[].listeners[].routes[].backends[].ai.provider.vertex`|object||
|`binds[].listeners[].routes[].backends[].ai.provider.vertex.model`|string||
|`binds[].listeners[].routes[].backends[].ai.provider.vertex.region`|string||
|`binds[].listeners[].routes[].backends[].ai.provider.vertex.projectId`|string||
|`binds[].listeners[].routes[].backends[].ai.provider.anthropic`|object||
|`binds[].listeners[].routes[].backends[].ai.provider.anthropic.model`|string||
|`binds[].listeners[].routes[].backends[].ai.provider.bedrock`|object||
|`binds[].listeners[].routes[].backends[].ai.provider.bedrock.model`|string||
|`binds[].listeners[].routes[].backends[].ai.provider.bedrock.region`|string||
|`binds[].listeners[].routes[].backends[].ai.provider.bedrock.guardrailIdentifier`|string||
|`binds[].listeners[].routes[].backends[].ai.provider.bedrock.guardrailVersion`|string||
|`binds[].listeners[].routes[].backends[].ai.provider.azure`|object||
|`binds[].listeners[].routes[].backends[].ai.provider.azure.model`|string||
|`binds[].listeners[].routes[].backends[].ai.provider.azure.resourceName`|string|The Azure resource name used to construct the endpoint host.|
|`binds[].listeners[].routes[].backends[].ai.provider.azure.resourceType`|string|The type of Azure endpoint. Determines the host suffix.|
|`binds[].listeners[].routes[].backends[].ai.provider.azure.apiVersion`|string||
|`binds[].listeners[].routes[].backends[].ai.provider.azure.projectName`|string|The Foundry project name, required when `resource_type` is `Foundry`.<br>Used to construct paths: `/api/projects/{project_name}/openai/v1/...`.<br>This is distinct from `resource_name` which is used for the host.|
|`binds[].listeners[].routes[].backends[].ai.hostOverride`|string|Override the upstream host for this provider.|
|`binds[].listeners[].routes[].backends[].ai.pathOverride`|string|Override the upstream path for this provider.|
|`binds[].listeners[].routes[].backends[].ai.pathPrefix`|string|Override the default base path prefix for this provider.|
|`binds[].listeners[].routes[].backends[].ai.tokenize`|boolean|Whether to tokenize on the request flow. This enables us to do more accurate rate limits,<br>since we know (part of) the cost of the request upfront.<br>This comes with the cost of an expensive operation.|
|`binds[].listeners[].routes[].backends[].ai.policies`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`binds[].listeners[].routes[].backends[].ai.policies.requestHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.requestHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.requestHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`binds[].listeners[].routes[].backends[].ai.policies.responseHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.responseHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.responseHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].backends[].ai.policies.requestRedirect.scheme`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.requestRedirect.authority`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.requestRedirect.authority.full`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.requestRedirect.authority.host`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.requestRedirect.authority.port`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.requestRedirect.path`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.requestRedirect.path.full`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.requestRedirect.path.prefix`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.requestRedirect.status`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`binds[].listeners[].routes[].backends[].ai.policies.transformations.request`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.transformations.request.add`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.transformations.request.set`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.transformations.request.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.transformations.request.body`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.transformations.request.metadata`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.transformations.response`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.transformations.response.add`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.transformations.response.set`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.transformations.response.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.transformations.response.body`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.transformations.response.metadata`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.backendTLS`|object|Send TLS to the backend.|
|`binds[].listeners[].routes[].backends[].ai.policies.backendTLS.cert`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.backendTLS.key`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.backendTLS.root`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.backendTLS.hostname`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].routes[].backends[].ai.policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].routes[].backends[].ai.policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.backendAuth`|object|Authenticate to the backend.|
|`binds[].listeners[].routes[].backends[].ai.policies.backendAuth.passthrough`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.backendAuth.key`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.backendAuth.key.file`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.backendAuth.gcp`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`binds[].listeners[].routes[].backends[].ai.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.backendAuth.aws`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.backendAuth.aws.accessKeyId`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.backendAuth.aws.secretAccessKey`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.backendAuth.aws.region`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.backendAuth.aws.sessionToken`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`binds[].listeners[].routes[].backends[].ai.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`binds[].listeners[].routes[].backends[].ai.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.backendAuth.azure.developerImplicit`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.backendAuth.azure.implicit`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.http`|object|Specify HTTP settings for the backend|
|`binds[].listeners[].routes[].backends[].ai.policies.http.version`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.http.requestTimeout`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.tcp`|object|Specify TCP settings for the backend|
|`binds[].listeners[].routes[].backends[].ai.policies.tcp.keepalives`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.tcp.keepalives.enabled`|boolean||
|`binds[].listeners[].routes[].backends[].ai.policies.tcp.keepalives.time`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.tcp.keepalives.interval`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.tcp.keepalives.retries`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.tcp.connectTimeout`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.tcp.connectTimeout.secs`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.tcp.connectTimeout.nanos`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`binds[].listeners[].routes[].backends[].ai.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`binds[].listeners[].routes[].backends[].ai.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`binds[].listeners[].routes[].backends[].ai.policies.health.eviction.duration`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.health.eviction.restoreHealth`|number||
|`binds[].listeners[].routes[].backends[].ai.policies.health.eviction.consecutiveFailures`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.health.eviction.healthThreshold`|number||
|`binds[].listeners[].routes[].backends[].ai.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`binds[].listeners[].routes[].backends[].ai.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].backends[].ai.policies.backendTunnel.proxy.service`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.backendTunnel.proxy.service.name`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.backendTunnel.proxy.service.name.namespace`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.backendTunnel.proxy.service.name.hostname`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.backendTunnel.proxy.service.port`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].backends[].ai.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].backends[].ai.policies.mcpAuthorization`|object|Authorization policies for MCP access.|
|`binds[].listeners[].routes[].backends[].ai.policies.mcpAuthorization.rules`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.a2a`|object|Mark this traffic as A2A to enable A2A processing and telemetry.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai`|object|Mark this as LLM traffic to enable LLM processing.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request`|[]object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].regex`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].regex.action`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].regex.rules`|[]object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].regex.rules[].builtin`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].regex.rules[].pattern`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].webhook`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].webhook.target`|object|Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].webhook.target.service`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].webhook.target.service.name`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].webhook.target.service.name.namespace`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].webhook.target.service.name.hostname`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].webhook.target.service.port`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].webhook.target.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].webhook.target.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].webhook.forwardHeaderMatches`|[]object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].webhook.forwardHeaderMatches[].name`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].webhook.forwardHeaderMatches[].value`|object|Exactly one of exact or regex may be set.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].webhook.forwardHeaderMatches[].value.exact`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].webhook.forwardHeaderMatches[].value.regex`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.model`|string|Model to use. Defaults to `omni-moderation-latest`|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.requestHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.requestHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.requestHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.responseHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.responseHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.responseHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.scheme`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.authority`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.authority.full`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.authority.host`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.authority.port`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.path`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.path.full`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.path.prefix`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.status`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request.add`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request.set`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request.body`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request.metadata`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response.add`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response.set`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response.body`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response.metadata`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS`|object|Send TLS to the backend.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.cert`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.key`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.root`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.hostname`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth`|object|Authenticate to the backend.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.passthrough`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.key`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.key.file`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.gcp`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws.accessKeyId`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws.secretAccessKey`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws.region`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws.sessionToken`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.developerImplicit`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.implicit`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.http`|object|Specify HTTP settings for the backend|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.http.version`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.http.requestTimeout`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.tcp`|object|Specify TCP settings for the backend|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives.enabled`|boolean||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives.time`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives.interval`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives.retries`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.tcp.connectTimeout`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.tcp.connectTimeout.secs`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.tcp.connectTimeout.nanos`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.health.eviction.duration`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.health.eviction.restoreHealth`|number||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.health.eviction.consecutiveFailures`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.health.eviction.healthThreshold`|number||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service.name`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service.name.namespace`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service.name.hostname`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service.port`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails`|object|Configuration for AWS Bedrock Guardrails integration.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.guardrailIdentifier`|string|The unique identifier of the guardrail|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.guardrailVersion`|string|The version of the guardrail|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.region`|string|AWS region where the guardrail is deployed|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies`|object|Backend policies for AWS authentication (optional, defaults to implicit AWS auth)|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.responseHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.responseHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.responseHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.scheme`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.authority`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.authority.full`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.authority.host`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.authority.port`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.path`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.path.full`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.path.prefix`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.status`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.add`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.set`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.body`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.metadata`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.add`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.set`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.body`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.metadata`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS`|object|Send TLS to the backend.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.cert`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.key`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.root`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.hostname`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth`|object|Authenticate to the backend.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.passthrough`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.key`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.key.file`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.gcp`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws.accessKeyId`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws.secretAccessKey`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws.region`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws.sessionToken`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.developerImplicit`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.implicit`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.http`|object|Specify HTTP settings for the backend|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.http.version`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.http.requestTimeout`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp`|object|Specify TCP settings for the backend|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives.enabled`|boolean||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives.time`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives.interval`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives.retries`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.connectTimeout`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.connectTimeout.secs`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.connectTimeout.nanos`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction.duration`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction.restoreHealth`|number||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction.consecutiveFailures`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction.healthThreshold`|number||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.name`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.namespace`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.hostname`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.port`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor`|object|Configuration for Google Cloud Model Armor integration.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.templateId`|string|The template ID for the Model Armor configuration|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.projectId`|string|The GCP project ID|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.location`|string|The GCP region (default: us-central1)|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies`|object|Backend policies for GCP authentication (optional, defaults to implicit GCP auth)|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.requestHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.requestHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.requestHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.responseHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.responseHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.responseHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.scheme`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.authority`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.authority.full`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.authority.host`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.authority.port`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.path`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.path.full`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.path.prefix`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.status`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.add`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.set`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.body`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.metadata`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.add`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.set`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.body`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.metadata`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS`|object|Send TLS to the backend.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.cert`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.key`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.root`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.hostname`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth`|object|Authenticate to the backend.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.passthrough`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.key`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.key.file`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.gcp`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws.accessKeyId`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws.secretAccessKey`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws.region`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws.sessionToken`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.developerImplicit`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.implicit`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.http`|object|Specify HTTP settings for the backend|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.http.version`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.http.requestTimeout`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.tcp`|object|Specify TCP settings for the backend|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives.enabled`|boolean||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives.time`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives.interval`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives.retries`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.connectTimeout`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.connectTimeout.secs`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.connectTimeout.nanos`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.health.eviction.duration`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.health.eviction.restoreHealth`|number||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.health.eviction.consecutiveFailures`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.health.eviction.healthThreshold`|number||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service.name`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service.name.namespace`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service.name.hostname`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service.port`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].rejection`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].rejection.body`|array||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].rejection.status`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].rejection.headers`|object|Optional headers to add, set, or remove from the rejection response|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].rejection.headers.add`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].rejection.headers.set`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.request[].rejection.headers.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response`|[]object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].regex`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].regex.action`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].regex.rules`|[]object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].regex.rules[].builtin`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].regex.rules[].pattern`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].webhook`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].webhook.target`|object|Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].webhook.target.service`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].webhook.target.service.name`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].webhook.target.service.name.namespace`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].webhook.target.service.name.hostname`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].webhook.target.service.port`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].webhook.target.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].webhook.target.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].webhook.forwardHeaderMatches`|[]object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].webhook.forwardHeaderMatches[].name`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].webhook.forwardHeaderMatches[].value`|object|Exactly one of exact or regex may be set.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].webhook.forwardHeaderMatches[].value.exact`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].webhook.forwardHeaderMatches[].value.regex`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails`|object|Configuration for AWS Bedrock Guardrails integration.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.guardrailIdentifier`|string|The unique identifier of the guardrail|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.guardrailVersion`|string|The version of the guardrail|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.region`|string|AWS region where the guardrail is deployed|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies`|object|Backend policies for AWS authentication (optional, defaults to implicit AWS auth)|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.responseHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.responseHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.responseHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.scheme`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.authority`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.authority.full`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.authority.host`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.authority.port`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.path`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.path.full`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.path.prefix`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.status`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.add`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.set`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.body`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.metadata`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.add`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.set`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.body`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.metadata`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS`|object|Send TLS to the backend.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.cert`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.key`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.root`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.hostname`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth`|object|Authenticate to the backend.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.passthrough`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.key`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.key.file`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.gcp`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws.accessKeyId`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws.secretAccessKey`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws.region`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws.sessionToken`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.developerImplicit`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.implicit`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.http`|object|Specify HTTP settings for the backend|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.http.version`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.http.requestTimeout`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp`|object|Specify TCP settings for the backend|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives.enabled`|boolean||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives.time`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives.interval`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives.retries`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.connectTimeout`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.connectTimeout.secs`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.connectTimeout.nanos`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction.duration`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction.restoreHealth`|number||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction.consecutiveFailures`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction.healthThreshold`|number||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.name`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.namespace`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.hostname`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.port`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor`|object|Configuration for Google Cloud Model Armor integration.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.templateId`|string|The template ID for the Model Armor configuration|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.projectId`|string|The GCP project ID|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.location`|string|The GCP region (default: us-central1)|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies`|object|Backend policies for GCP authentication (optional, defaults to implicit GCP auth)|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.requestHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.requestHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.requestHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.responseHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.responseHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.responseHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.scheme`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.authority`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.authority.full`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.authority.host`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.authority.port`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.path`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.path.full`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.path.prefix`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.status`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.add`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.set`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.body`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.metadata`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.add`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.set`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.body`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.metadata`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS`|object|Send TLS to the backend.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.cert`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.key`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.root`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.hostname`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth`|object|Authenticate to the backend.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.passthrough`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.key`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.key.file`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.gcp`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws.accessKeyId`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws.secretAccessKey`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws.region`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws.sessionToken`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.developerImplicit`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.implicit`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.http`|object|Specify HTTP settings for the backend|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.http.version`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.http.requestTimeout`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.tcp`|object|Specify TCP settings for the backend|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives.enabled`|boolean||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives.time`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives.interval`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives.retries`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.connectTimeout`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.connectTimeout.secs`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.connectTimeout.nanos`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.health.eviction.duration`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.health.eviction.restoreHealth`|number||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.health.eviction.consecutiveFailures`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.health.eviction.healthThreshold`|number||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service.name`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service.name.namespace`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service.name.hostname`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service.port`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].rejection`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].rejection.body`|array||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].rejection.status`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].rejection.headers`|object|Optional headers to add, set, or remove from the rejection response|
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].rejection.headers.add`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].rejection.headers.set`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptGuard.response[].rejection.headers.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.defaults`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.overrides`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.transformations`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.prompts`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.prompts.append`|[]object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.prompts.append[].role`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.prompts.append[].content`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.prompts.prepend`|[]object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.prompts.prepend[].role`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.prompts.prepend[].content`|string||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.modelAliases`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptCaching`|object||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptCaching.cacheSystem`|boolean||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptCaching.cacheMessages`|boolean||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptCaching.cacheTools`|boolean||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.promptCaching.minTokens`|integer||
|`binds[].listeners[].routes[].backends[].ai.policies.ai.routes`|object||
|`binds[].listeners[].routes[].backends[].ai.groups`|[]object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers`|[]object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].name`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].provider`|object|Exactly one of openAI, gemini, vertex, anthropic, bedrock, or azure may be set.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].provider.openAI`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].provider.openAI.model`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].provider.gemini`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].provider.gemini.model`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].provider.vertex`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].provider.vertex.model`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].provider.vertex.region`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].provider.vertex.projectId`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].provider.anthropic`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].provider.anthropic.model`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].provider.bedrock`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].provider.bedrock.model`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].provider.bedrock.region`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].provider.bedrock.guardrailIdentifier`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].provider.bedrock.guardrailVersion`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].provider.azure`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].provider.azure.model`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].provider.azure.resourceName`|string|The Azure resource name used to construct the endpoint host.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].provider.azure.resourceType`|string|The type of Azure endpoint. Determines the host suffix.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].provider.azure.apiVersion`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].provider.azure.projectName`|string|The Foundry project name, required when `resource_type` is `Foundry`.<br>Used to construct paths: `/api/projects/{project_name}/openai/v1/...`.<br>This is distinct from `resource_name` which is used for the host.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].hostOverride`|string|Override the upstream host for this provider.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].pathOverride`|string|Override the upstream path for this provider.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].pathPrefix`|string|Override the default base path prefix for this provider.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].tokenize`|boolean|Whether to tokenize on the request flow. This enables us to do more accurate rate limits,<br>since we know (part of) the cost of the request upfront.<br>This comes with the cost of an expensive operation.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.requestHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.requestHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.requestHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.responseHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.responseHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.responseHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.requestRedirect.scheme`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.requestRedirect.authority`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.requestRedirect.authority.full`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.requestRedirect.authority.host`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.requestRedirect.authority.port`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.requestRedirect.path`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.requestRedirect.path.full`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.requestRedirect.path.prefix`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.requestRedirect.status`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.transformations.request`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.transformations.request.add`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.transformations.request.set`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.transformations.request.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.transformations.request.body`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.transformations.request.metadata`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.transformations.response`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.transformations.response.add`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.transformations.response.set`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.transformations.response.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.transformations.response.body`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.transformations.response.metadata`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendTLS`|object|Send TLS to the backend.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendTLS.cert`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendTLS.key`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendTLS.root`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendTLS.hostname`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendAuth`|object|Authenticate to the backend.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendAuth.passthrough`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendAuth.key`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendAuth.key.file`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendAuth.gcp`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendAuth.aws`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendAuth.aws.accessKeyId`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendAuth.aws.secretAccessKey`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendAuth.aws.region`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendAuth.aws.sessionToken`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendAuth.azure.developerImplicit`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendAuth.azure.implicit`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.http`|object|Specify HTTP settings for the backend|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.http.version`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.http.requestTimeout`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.tcp`|object|Specify TCP settings for the backend|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.tcp.keepalives`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.tcp.keepalives.enabled`|boolean||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.tcp.keepalives.time`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.tcp.keepalives.interval`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.tcp.keepalives.retries`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.tcp.connectTimeout`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.tcp.connectTimeout.secs`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.tcp.connectTimeout.nanos`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.health.eviction.duration`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.health.eviction.restoreHealth`|number||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.health.eviction.consecutiveFailures`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.health.eviction.healthThreshold`|number||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendTunnel.proxy.service`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendTunnel.proxy.service.name`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendTunnel.proxy.service.name.namespace`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendTunnel.proxy.service.name.hostname`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendTunnel.proxy.service.port`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.mcpAuthorization`|object|Authorization policies for MCP access.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.mcpAuthorization.rules`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.a2a`|object|Mark this traffic as A2A to enable A2A processing and telemetry.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai`|object|Mark this as LLM traffic to enable LLM processing.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request`|[]object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].regex`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].regex.action`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].regex.rules`|[]object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].regex.rules[].builtin`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].regex.rules[].pattern`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].webhook`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].webhook.target`|object|Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].webhook.target.service`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].webhook.target.service.name`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].webhook.target.service.name.namespace`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].webhook.target.service.name.hostname`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].webhook.target.service.port`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].webhook.target.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].webhook.target.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].webhook.forwardHeaderMatches`|[]object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].webhook.forwardHeaderMatches[].name`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].webhook.forwardHeaderMatches[].value`|object|Exactly one of exact or regex may be set.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].webhook.forwardHeaderMatches[].value.exact`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].webhook.forwardHeaderMatches[].value.regex`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.model`|string|Model to use. Defaults to `omni-moderation-latest`|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.requestHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.requestHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.requestHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.responseHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.responseHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.responseHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.scheme`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.authority`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.authority.full`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.authority.host`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.authority.port`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.path`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.path.full`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.path.prefix`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.status`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request.add`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request.set`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request.body`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request.metadata`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response.add`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response.set`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response.body`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response.metadata`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS`|object|Send TLS to the backend.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.cert`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.key`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.root`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.hostname`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth`|object|Authenticate to the backend.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.passthrough`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.key`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.key.file`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.gcp`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws.accessKeyId`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws.secretAccessKey`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws.region`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws.sessionToken`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.developerImplicit`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.implicit`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.http`|object|Specify HTTP settings for the backend|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.http.version`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.http.requestTimeout`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp`|object|Specify TCP settings for the backend|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives.enabled`|boolean||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives.time`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives.interval`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives.retries`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.connectTimeout`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.connectTimeout.secs`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.connectTimeout.nanos`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.health.eviction.duration`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.health.eviction.restoreHealth`|number||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.health.eviction.consecutiveFailures`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.health.eviction.healthThreshold`|number||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service.name`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service.name.namespace`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service.name.hostname`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service.port`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails`|object|Configuration for AWS Bedrock Guardrails integration.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.guardrailIdentifier`|string|The unique identifier of the guardrail|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.guardrailVersion`|string|The version of the guardrail|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.region`|string|AWS region where the guardrail is deployed|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies`|object|Backend policies for AWS authentication (optional, defaults to implicit AWS auth)|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.responseHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.responseHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.responseHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.scheme`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.authority`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.authority.full`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.authority.host`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.authority.port`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.path`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.path.full`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.path.prefix`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.status`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.add`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.set`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.body`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.metadata`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.add`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.set`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.body`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.metadata`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS`|object|Send TLS to the backend.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.cert`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.key`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.root`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.hostname`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth`|object|Authenticate to the backend.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.passthrough`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.key`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.key.file`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.gcp`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws.accessKeyId`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws.secretAccessKey`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws.region`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws.sessionToken`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.developerImplicit`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.implicit`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.http`|object|Specify HTTP settings for the backend|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.http.version`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.http.requestTimeout`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp`|object|Specify TCP settings for the backend|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives.enabled`|boolean||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives.time`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives.interval`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives.retries`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.connectTimeout`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.connectTimeout.secs`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.connectTimeout.nanos`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction.duration`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction.restoreHealth`|number||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction.consecutiveFailures`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction.healthThreshold`|number||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.name`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.namespace`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.hostname`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.port`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor`|object|Configuration for Google Cloud Model Armor integration.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.templateId`|string|The template ID for the Model Armor configuration|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.projectId`|string|The GCP project ID|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.location`|string|The GCP region (default: us-central1)|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies`|object|Backend policies for GCP authentication (optional, defaults to implicit GCP auth)|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.responseHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.responseHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.responseHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.scheme`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.authority`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.authority.full`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.authority.host`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.authority.port`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.path`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.path.full`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.path.prefix`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.status`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.add`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.set`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.body`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.metadata`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.add`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.set`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.body`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.metadata`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS`|object|Send TLS to the backend.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.cert`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.key`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.root`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.hostname`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth`|object|Authenticate to the backend.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.passthrough`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.key`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.key.file`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.gcp`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws.accessKeyId`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws.secretAccessKey`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws.region`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws.sessionToken`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.developerImplicit`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.implicit`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.http`|object|Specify HTTP settings for the backend|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.http.version`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.http.requestTimeout`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp`|object|Specify TCP settings for the backend|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives.enabled`|boolean||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives.time`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives.interval`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives.retries`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.connectTimeout`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.connectTimeout.secs`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.connectTimeout.nanos`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.health.eviction.duration`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.health.eviction.restoreHealth`|number||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.health.eviction.consecutiveFailures`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.health.eviction.healthThreshold`|number||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service.name`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service.name.namespace`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service.name.hostname`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service.port`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].rejection`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].rejection.body`|array||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].rejection.status`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].rejection.headers`|object|Optional headers to add, set, or remove from the rejection response|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].rejection.headers.add`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].rejection.headers.set`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.request[].rejection.headers.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response`|[]object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].regex`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].regex.action`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].regex.rules`|[]object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].regex.rules[].builtin`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].regex.rules[].pattern`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].webhook`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].webhook.target`|object|Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].webhook.target.service`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].webhook.target.service.name`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].webhook.target.service.name.namespace`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].webhook.target.service.name.hostname`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].webhook.target.service.port`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].webhook.target.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].webhook.target.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].webhook.forwardHeaderMatches`|[]object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].webhook.forwardHeaderMatches[].name`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].webhook.forwardHeaderMatches[].value`|object|Exactly one of exact or regex may be set.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].webhook.forwardHeaderMatches[].value.exact`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].webhook.forwardHeaderMatches[].value.regex`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails`|object|Configuration for AWS Bedrock Guardrails integration.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.guardrailIdentifier`|string|The unique identifier of the guardrail|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.guardrailVersion`|string|The version of the guardrail|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.region`|string|AWS region where the guardrail is deployed|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies`|object|Backend policies for AWS authentication (optional, defaults to implicit AWS auth)|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.responseHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.responseHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.responseHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.scheme`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.authority`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.authority.full`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.authority.host`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.authority.port`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.path`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.path.full`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.path.prefix`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.status`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.add`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.set`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.body`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.metadata`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.add`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.set`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.body`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.metadata`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS`|object|Send TLS to the backend.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.cert`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.key`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.root`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.hostname`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth`|object|Authenticate to the backend.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.passthrough`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.key`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.key.file`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.gcp`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws.accessKeyId`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws.secretAccessKey`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws.region`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws.sessionToken`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.developerImplicit`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.implicit`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.http`|object|Specify HTTP settings for the backend|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.http.version`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.http.requestTimeout`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp`|object|Specify TCP settings for the backend|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives.enabled`|boolean||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives.time`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives.interval`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives.retries`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.connectTimeout`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.connectTimeout.secs`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.connectTimeout.nanos`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction.duration`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction.restoreHealth`|number||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction.consecutiveFailures`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction.healthThreshold`|number||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.name`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.namespace`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.hostname`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.port`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor`|object|Configuration for Google Cloud Model Armor integration.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.templateId`|string|The template ID for the Model Armor configuration|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.projectId`|string|The GCP project ID|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.location`|string|The GCP region (default: us-central1)|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies`|object|Backend policies for GCP authentication (optional, defaults to implicit GCP auth)|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.responseHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.responseHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.responseHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.scheme`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.authority`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.authority.full`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.authority.host`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.authority.port`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.path`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.path.full`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.path.prefix`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.status`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.add`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.set`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.body`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.metadata`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.add`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.set`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.body`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.metadata`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS`|object|Send TLS to the backend.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.cert`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.key`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.root`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.hostname`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth`|object|Authenticate to the backend.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.passthrough`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.key`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.key.file`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.gcp`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws.accessKeyId`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws.secretAccessKey`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws.region`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws.sessionToken`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.developerImplicit`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.implicit`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.http`|object|Specify HTTP settings for the backend|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.http.version`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.http.requestTimeout`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp`|object|Specify TCP settings for the backend|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives.enabled`|boolean||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives.time`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives.interval`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives.retries`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.connectTimeout`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.connectTimeout.secs`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.connectTimeout.nanos`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.health.eviction.duration`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.health.eviction.restoreHealth`|number||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.health.eviction.consecutiveFailures`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.health.eviction.healthThreshold`|number||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service.name`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service.name.namespace`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service.name.hostname`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service.port`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].rejection`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].rejection.body`|array||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].rejection.status`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].rejection.headers`|object|Optional headers to add, set, or remove from the rejection response|
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].rejection.headers.add`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].rejection.headers.set`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptGuard.response[].rejection.headers.remove`|[]string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.defaults`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.overrides`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.transformations`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.prompts`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.prompts.append`|[]object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.prompts.append[].role`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.prompts.append[].content`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.prompts.prepend`|[]object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.prompts.prepend[].role`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.prompts.prepend[].content`|string||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.modelAliases`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptCaching`|object||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptCaching.cacheSystem`|boolean||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptCaching.cacheMessages`|boolean||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptCaching.cacheTools`|boolean||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.promptCaching.minTokens`|integer||
|`binds[].listeners[].routes[].backends[].ai.groups[].providers[].policies.ai.routes`|object||
|`binds[].listeners[].routes[].backends[].aws`|object||
|`binds[].listeners[].routes[].backends[].aws.agentCore`|object||
|`binds[].listeners[].routes[].backends[].aws.agentCore.agentRuntimeArn`|string||
|`binds[].listeners[].routes[].backends[].aws.agentCore.qualifier`|string||
|`binds[].listeners[].routes[].backends[].weight`|integer||
|`binds[].listeners[].routes[].backends[].policies`|object||
|`binds[].listeners[].routes[].backends[].policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`binds[].listeners[].routes[].backends[].policies.requestHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].policies.requestHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].policies.requestHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`binds[].listeners[].routes[].backends[].policies.responseHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].policies.responseHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].policies.responseHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].backends[].policies.requestRedirect.scheme`|string||
|`binds[].listeners[].routes[].backends[].policies.requestRedirect.authority`|string||
|`binds[].listeners[].routes[].backends[].policies.requestRedirect.authority.full`|string||
|`binds[].listeners[].routes[].backends[].policies.requestRedirect.authority.host`|string||
|`binds[].listeners[].routes[].backends[].policies.requestRedirect.authority.port`|integer||
|`binds[].listeners[].routes[].backends[].policies.requestRedirect.path`|object||
|`binds[].listeners[].routes[].backends[].policies.requestRedirect.path.full`|string||
|`binds[].listeners[].routes[].backends[].policies.requestRedirect.path.prefix`|string||
|`binds[].listeners[].routes[].backends[].policies.requestRedirect.status`|integer||
|`binds[].listeners[].routes[].backends[].policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`binds[].listeners[].routes[].backends[].policies.transformations.request`|object||
|`binds[].listeners[].routes[].backends[].policies.transformations.request.add`|object||
|`binds[].listeners[].routes[].backends[].policies.transformations.request.set`|object||
|`binds[].listeners[].routes[].backends[].policies.transformations.request.remove`|[]string||
|`binds[].listeners[].routes[].backends[].policies.transformations.request.body`|string||
|`binds[].listeners[].routes[].backends[].policies.transformations.request.metadata`|object||
|`binds[].listeners[].routes[].backends[].policies.transformations.response`|object||
|`binds[].listeners[].routes[].backends[].policies.transformations.response.add`|object||
|`binds[].listeners[].routes[].backends[].policies.transformations.response.set`|object||
|`binds[].listeners[].routes[].backends[].policies.transformations.response.remove`|[]string||
|`binds[].listeners[].routes[].backends[].policies.transformations.response.body`|string||
|`binds[].listeners[].routes[].backends[].policies.transformations.response.metadata`|object||
|`binds[].listeners[].routes[].backends[].policies.backendTLS`|object|Send TLS to the backend.|
|`binds[].listeners[].routes[].backends[].policies.backendTLS.cert`|string||
|`binds[].listeners[].routes[].backends[].policies.backendTLS.key`|string||
|`binds[].listeners[].routes[].backends[].policies.backendTLS.root`|string||
|`binds[].listeners[].routes[].backends[].policies.backendTLS.hostname`|string||
|`binds[].listeners[].routes[].backends[].policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].routes[].backends[].policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].routes[].backends[].policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].routes[].backends[].policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].routes[].backends[].policies.backendAuth`|object|Authenticate to the backend.|
|`binds[].listeners[].routes[].backends[].policies.backendAuth.passthrough`|object||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.key`|object||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.key.file`|string||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.gcp`|object||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`binds[].listeners[].routes[].backends[].policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.aws`|object||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.aws.accessKeyId`|string||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.aws.secretAccessKey`|string||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.aws.region`|string||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.aws.sessionToken`|string||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`binds[].listeners[].routes[].backends[].policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`binds[].listeners[].routes[].backends[].policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.azure.developerImplicit`|object||
|`binds[].listeners[].routes[].backends[].policies.backendAuth.azure.implicit`|object||
|`binds[].listeners[].routes[].backends[].policies.http`|object|Specify HTTP settings for the backend|
|`binds[].listeners[].routes[].backends[].policies.http.version`|string||
|`binds[].listeners[].routes[].backends[].policies.http.requestTimeout`|string||
|`binds[].listeners[].routes[].backends[].policies.tcp`|object|Specify TCP settings for the backend|
|`binds[].listeners[].routes[].backends[].policies.tcp.keepalives`|object||
|`binds[].listeners[].routes[].backends[].policies.tcp.keepalives.enabled`|boolean||
|`binds[].listeners[].routes[].backends[].policies.tcp.keepalives.time`|string||
|`binds[].listeners[].routes[].backends[].policies.tcp.keepalives.interval`|string||
|`binds[].listeners[].routes[].backends[].policies.tcp.keepalives.retries`|integer||
|`binds[].listeners[].routes[].backends[].policies.tcp.connectTimeout`|object||
|`binds[].listeners[].routes[].backends[].policies.tcp.connectTimeout.secs`|integer||
|`binds[].listeners[].routes[].backends[].policies.tcp.connectTimeout.nanos`|integer||
|`binds[].listeners[].routes[].backends[].policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`binds[].listeners[].routes[].backends[].policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`binds[].listeners[].routes[].backends[].policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`binds[].listeners[].routes[].backends[].policies.health.eviction.duration`|string||
|`binds[].listeners[].routes[].backends[].policies.health.eviction.restoreHealth`|number||
|`binds[].listeners[].routes[].backends[].policies.health.eviction.consecutiveFailures`|integer||
|`binds[].listeners[].routes[].backends[].policies.health.eviction.healthThreshold`|number||
|`binds[].listeners[].routes[].backends[].policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`binds[].listeners[].routes[].backends[].policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].backends[].policies.backendTunnel.proxy.service`|object||
|`binds[].listeners[].routes[].backends[].policies.backendTunnel.proxy.service.name`|object||
|`binds[].listeners[].routes[].backends[].policies.backendTunnel.proxy.service.name.namespace`|string||
|`binds[].listeners[].routes[].backends[].policies.backendTunnel.proxy.service.name.hostname`|string||
|`binds[].listeners[].routes[].backends[].policies.backendTunnel.proxy.service.port`|integer||
|`binds[].listeners[].routes[].backends[].policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].backends[].policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].backends[].policies.mcpAuthorization`|object|Authorization policies for MCP access.|
|`binds[].listeners[].routes[].backends[].policies.mcpAuthorization.rules`|[]string||
|`binds[].listeners[].routes[].backends[].policies.a2a`|object|Mark this traffic as A2A to enable A2A processing and telemetry.|
|`binds[].listeners[].routes[].backends[].policies.ai`|object|Mark this as LLM traffic to enable LLM processing.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request`|[]object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].regex`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].regex.action`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].regex.rules`|[]object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].regex.rules[].builtin`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].regex.rules[].pattern`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].webhook`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].webhook.target`|object|Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].webhook.target.service`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].webhook.target.service.name`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].webhook.target.service.name.namespace`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].webhook.target.service.name.hostname`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].webhook.target.service.port`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].webhook.target.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].webhook.target.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].webhook.forwardHeaderMatches`|[]object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].webhook.forwardHeaderMatches[].name`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].webhook.forwardHeaderMatches[].value`|object|Exactly one of exact or regex may be set.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].webhook.forwardHeaderMatches[].value.exact`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].webhook.forwardHeaderMatches[].value.regex`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.model`|string|Model to use. Defaults to `omni-moderation-latest`|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.requestHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.requestHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.requestHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.responseHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.responseHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.responseHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.scheme`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.authority`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.authority.full`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.authority.host`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.authority.port`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.path`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.path.full`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.path.prefix`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.status`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request.add`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request.set`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request.remove`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request.body`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request.metadata`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response.add`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response.set`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response.remove`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response.body`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response.metadata`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS`|object|Send TLS to the backend.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.cert`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.key`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.root`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.hostname`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth`|object|Authenticate to the backend.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.passthrough`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.key`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.key.file`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.gcp`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws.accessKeyId`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws.secretAccessKey`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws.region`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws.sessionToken`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.developerImplicit`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.implicit`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.http`|object|Specify HTTP settings for the backend|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.http.version`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.http.requestTimeout`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp`|object|Specify TCP settings for the backend|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives.enabled`|boolean||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives.time`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives.interval`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives.retries`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.connectTimeout`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.connectTimeout.secs`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.connectTimeout.nanos`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.health.eviction.duration`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.health.eviction.restoreHealth`|number||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.health.eviction.consecutiveFailures`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.health.eviction.healthThreshold`|number||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service.name`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service.name.namespace`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service.name.hostname`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service.port`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails`|object|Configuration for AWS Bedrock Guardrails integration.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.guardrailIdentifier`|string|The unique identifier of the guardrail|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.guardrailVersion`|string|The version of the guardrail|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.region`|string|AWS region where the guardrail is deployed|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies`|object|Backend policies for AWS authentication (optional, defaults to implicit AWS auth)|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.responseHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.responseHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.responseHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.scheme`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.authority`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.authority.full`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.authority.host`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.authority.port`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.path`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.path.full`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.path.prefix`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.status`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.add`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.set`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.remove`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.body`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.metadata`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.add`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.set`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.remove`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.body`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.metadata`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS`|object|Send TLS to the backend.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.cert`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.key`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.root`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.hostname`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth`|object|Authenticate to the backend.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.passthrough`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.key`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.key.file`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.gcp`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws.accessKeyId`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws.secretAccessKey`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws.region`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws.sessionToken`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.developerImplicit`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.implicit`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.http`|object|Specify HTTP settings for the backend|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.http.version`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.http.requestTimeout`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp`|object|Specify TCP settings for the backend|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives.enabled`|boolean||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives.time`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives.interval`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives.retries`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.connectTimeout`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.connectTimeout.secs`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.connectTimeout.nanos`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction.duration`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction.restoreHealth`|number||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction.consecutiveFailures`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction.healthThreshold`|number||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.name`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.namespace`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.hostname`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.port`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor`|object|Configuration for Google Cloud Model Armor integration.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.templateId`|string|The template ID for the Model Armor configuration|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.projectId`|string|The GCP project ID|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.location`|string|The GCP region (default: us-central1)|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies`|object|Backend policies for GCP authentication (optional, defaults to implicit GCP auth)|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.responseHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.responseHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.responseHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.scheme`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.authority`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.authority.full`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.authority.host`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.authority.port`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.path`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.path.full`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.path.prefix`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.status`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.add`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.set`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.remove`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.body`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.metadata`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.add`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.set`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.remove`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.body`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.metadata`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS`|object|Send TLS to the backend.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.cert`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.key`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.root`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.hostname`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth`|object|Authenticate to the backend.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.passthrough`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.key`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.key.file`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.gcp`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws.accessKeyId`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws.secretAccessKey`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws.region`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws.sessionToken`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.developerImplicit`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.implicit`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.http`|object|Specify HTTP settings for the backend|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.http.version`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.http.requestTimeout`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp`|object|Specify TCP settings for the backend|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives.enabled`|boolean||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives.time`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives.interval`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives.retries`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.connectTimeout`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.connectTimeout.secs`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.connectTimeout.nanos`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.health.eviction.duration`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.health.eviction.restoreHealth`|number||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.health.eviction.consecutiveFailures`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.health.eviction.healthThreshold`|number||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service.name`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service.name.namespace`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service.name.hostname`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service.port`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].rejection`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].rejection.body`|array||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].rejection.status`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].rejection.headers`|object|Optional headers to add, set, or remove from the rejection response|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].rejection.headers.add`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].rejection.headers.set`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.request[].rejection.headers.remove`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response`|[]object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].regex`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].regex.action`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].regex.rules`|[]object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].regex.rules[].builtin`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].regex.rules[].pattern`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].webhook`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].webhook.target`|object|Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].webhook.target.service`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].webhook.target.service.name`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].webhook.target.service.name.namespace`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].webhook.target.service.name.hostname`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].webhook.target.service.port`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].webhook.target.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].webhook.target.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].webhook.forwardHeaderMatches`|[]object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].webhook.forwardHeaderMatches[].name`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].webhook.forwardHeaderMatches[].value`|object|Exactly one of exact or regex may be set.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].webhook.forwardHeaderMatches[].value.exact`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].webhook.forwardHeaderMatches[].value.regex`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails`|object|Configuration for AWS Bedrock Guardrails integration.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.guardrailIdentifier`|string|The unique identifier of the guardrail|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.guardrailVersion`|string|The version of the guardrail|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.region`|string|AWS region where the guardrail is deployed|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies`|object|Backend policies for AWS authentication (optional, defaults to implicit AWS auth)|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.responseHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.responseHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.responseHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.scheme`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.authority`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.authority.full`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.authority.host`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.authority.port`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.path`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.path.full`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.path.prefix`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.status`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.add`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.set`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.remove`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.body`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.metadata`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.add`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.set`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.remove`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.body`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.metadata`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS`|object|Send TLS to the backend.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.cert`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.key`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.root`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.hostname`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth`|object|Authenticate to the backend.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.passthrough`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.key`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.key.file`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.gcp`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws.accessKeyId`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws.secretAccessKey`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws.region`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws.sessionToken`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.developerImplicit`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.implicit`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.http`|object|Specify HTTP settings for the backend|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.http.version`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.http.requestTimeout`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp`|object|Specify TCP settings for the backend|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives.enabled`|boolean||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives.time`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives.interval`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives.retries`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.connectTimeout`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.connectTimeout.secs`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.connectTimeout.nanos`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction.duration`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction.restoreHealth`|number||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction.consecutiveFailures`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction.healthThreshold`|number||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.name`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.namespace`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.hostname`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.port`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor`|object|Configuration for Google Cloud Model Armor integration.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.templateId`|string|The template ID for the Model Armor configuration|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.projectId`|string|The GCP project ID|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.location`|string|The GCP region (default: us-central1)|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies`|object|Backend policies for GCP authentication (optional, defaults to implicit GCP auth)|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.responseHeaderModifier.add`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.responseHeaderModifier.set`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.responseHeaderModifier.remove`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.scheme`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.authority`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.authority.full`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.authority.host`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.authority.port`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.path`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.path.full`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.path.prefix`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.status`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.add`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.set`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.remove`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.body`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.metadata`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.add`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.set`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.remove`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.body`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.metadata`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS`|object|Send TLS to the backend.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.cert`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.key`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.root`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.hostname`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth`|object|Authenticate to the backend.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.passthrough`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.key`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.key.file`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.gcp`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws.accessKeyId`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws.secretAccessKey`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws.region`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws.sessionToken`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.developerImplicit`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.implicit`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.http`|object|Specify HTTP settings for the backend|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.http.version`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.http.requestTimeout`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp`|object|Specify TCP settings for the backend|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives.enabled`|boolean||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives.time`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives.interval`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives.retries`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.connectTimeout`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.connectTimeout.secs`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.connectTimeout.nanos`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.health.eviction.duration`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.health.eviction.restoreHealth`|number||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.health.eviction.consecutiveFailures`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.health.eviction.healthThreshold`|number||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service.name`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service.name.namespace`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service.name.hostname`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service.port`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].rejection`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].rejection.body`|array||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].rejection.status`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].rejection.headers`|object|Optional headers to add, set, or remove from the rejection response|
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].rejection.headers.add`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].rejection.headers.set`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptGuard.response[].rejection.headers.remove`|[]string||
|`binds[].listeners[].routes[].backends[].policies.ai.defaults`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.overrides`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.transformations`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.prompts`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.prompts.append`|[]object||
|`binds[].listeners[].routes[].backends[].policies.ai.prompts.append[].role`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.prompts.append[].content`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.prompts.prepend`|[]object||
|`binds[].listeners[].routes[].backends[].policies.ai.prompts.prepend[].role`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.prompts.prepend[].content`|string||
|`binds[].listeners[].routes[].backends[].policies.ai.modelAliases`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptCaching`|object||
|`binds[].listeners[].routes[].backends[].policies.ai.promptCaching.cacheSystem`|boolean||
|`binds[].listeners[].routes[].backends[].policies.ai.promptCaching.cacheMessages`|boolean||
|`binds[].listeners[].routes[].backends[].policies.ai.promptCaching.cacheTools`|boolean||
|`binds[].listeners[].routes[].backends[].policies.ai.promptCaching.minTokens`|integer||
|`binds[].listeners[].routes[].backends[].policies.ai.routes`|object||
|`binds[].listeners[].tcpRoutes`|[]object||
|`binds[].listeners[].tcpRoutes[].name`|string||
|`binds[].listeners[].tcpRoutes[].namespace`|string||
|`binds[].listeners[].tcpRoutes[].ruleName`|string||
|`binds[].listeners[].tcpRoutes[].hostnames`|[]string|Can be a wildcard|
|`binds[].listeners[].tcpRoutes[].policies`|object||
|`binds[].listeners[].tcpRoutes[].policies.backendTLS`|object||
|`binds[].listeners[].tcpRoutes[].policies.backendTLS.cert`|string||
|`binds[].listeners[].tcpRoutes[].policies.backendTLS.key`|string||
|`binds[].listeners[].tcpRoutes[].policies.backendTLS.root`|string||
|`binds[].listeners[].tcpRoutes[].policies.backendTLS.hostname`|string||
|`binds[].listeners[].tcpRoutes[].policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].tcpRoutes[].policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].tcpRoutes[].policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].tcpRoutes[].policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].tcpRoutes[].backends`|[]object||
|`binds[].listeners[].tcpRoutes[].backends[].service`|object||
|`binds[].listeners[].tcpRoutes[].backends[].service.name`|object||
|`binds[].listeners[].tcpRoutes[].backends[].service.name.namespace`|string||
|`binds[].listeners[].tcpRoutes[].backends[].service.name.hostname`|string||
|`binds[].listeners[].tcpRoutes[].backends[].service.port`|integer||
|`binds[].listeners[].tcpRoutes[].backends[].host`|string|Hostname or IP address|
|`binds[].listeners[].tcpRoutes[].backends[].backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].tcpRoutes[].backends[].weight`|integer||
|`binds[].listeners[].tcpRoutes[].backends[].policies`|object||
|`binds[].listeners[].tcpRoutes[].backends[].policies.backendTLS`|object|Send TLS to the backend.|
|`binds[].listeners[].tcpRoutes[].backends[].policies.backendTLS.cert`|string||
|`binds[].listeners[].tcpRoutes[].backends[].policies.backendTLS.key`|string||
|`binds[].listeners[].tcpRoutes[].backends[].policies.backendTLS.root`|string||
|`binds[].listeners[].tcpRoutes[].backends[].policies.backendTLS.hostname`|string||
|`binds[].listeners[].tcpRoutes[].backends[].policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].tcpRoutes[].backends[].policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].tcpRoutes[].backends[].policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].tcpRoutes[].backends[].policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].tcpRoutes[].backends[].policies.backendTunnel`|object|Tunnel to the backend.|
|`binds[].listeners[].tcpRoutes[].backends[].policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].tcpRoutes[].backends[].policies.backendTunnel.proxy.service`|object||
|`binds[].listeners[].tcpRoutes[].backends[].policies.backendTunnel.proxy.service.name`|object||
|`binds[].listeners[].tcpRoutes[].backends[].policies.backendTunnel.proxy.service.name.namespace`|string||
|`binds[].listeners[].tcpRoutes[].backends[].policies.backendTunnel.proxy.service.name.hostname`|string||
|`binds[].listeners[].tcpRoutes[].backends[].policies.backendTunnel.proxy.service.port`|integer||
|`binds[].listeners[].tcpRoutes[].backends[].policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`binds[].listeners[].tcpRoutes[].backends[].policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].policies`|object||
|`binds[].listeners[].policies.oidc`|object|Authenticate incoming browser requests with OIDC authorization code flow.|
|`binds[].listeners[].policies.oidc.issuer`|string|Issuer used for discovery and ID token validation.|
|`binds[].listeners[].policies.oidc.discovery`|object|Optional discovery document override. If omitted, discovery uses<br>`${issuer}/.well-known/openid-configuration`.|
|`binds[].listeners[].policies.oidc.discovery.file`|string||
|`binds[].listeners[].policies.oidc.discovery.url`|string||
|`binds[].listeners[].policies.oidc.authorizationEndpoint`|string|Authorization endpoint used to start the browser login flow.|
|`binds[].listeners[].policies.oidc.tokenEndpoint`|string|Token endpoint used to exchange the authorization code.|
|`binds[].listeners[].policies.oidc.tokenEndpointAuth`|string|Token endpoint client authentication method for explicit provider configuration.<br><br>Discovery mode derives this from provider metadata. Explicit mode defaults to<br>`clientSecretBasic` when omitted.|
|`binds[].listeners[].policies.oidc.jwks`|object|JWKS source used to validate returned ID tokens.|
|`binds[].listeners[].policies.oidc.jwks.file`|string||
|`binds[].listeners[].policies.oidc.jwks.url`|string||
|`binds[].listeners[].policies.oidc.clientId`|string|OAuth2 client identifier used for authorization and token exchange.|
|`binds[].listeners[].policies.oidc.clientSecret`|string|OAuth2 client secret used for token exchange.|
|`binds[].listeners[].policies.oidc.redirectURI`|string|Absolute callback URI handled by the gateway.<br>This policy always redirects unauthenticated non-callback requests back through this login<br>flow.|
|`binds[].listeners[].policies.oidc.scopes`|[]string|Additional OAuth2 scopes to request. `openid` is always included.|
|`binds[].listeners[].policies.jwtAuth`|object|Authenticate incoming JWT requests.|
|`binds[].listeners[].policies.jwtAuth.mode`|string||
|`binds[].listeners[].policies.jwtAuth.providers`|[]object||
|`binds[].listeners[].policies.jwtAuth.providers[].issuer`|string||
|`binds[].listeners[].policies.jwtAuth.providers[].audiences`|[]string||
|`binds[].listeners[].policies.jwtAuth.providers[].jwks`|object||
|`binds[].listeners[].policies.jwtAuth.providers[].jwks.file`|string||
|`binds[].listeners[].policies.jwtAuth.providers[].jwks.url`|string||
|`binds[].listeners[].policies.jwtAuth.providers[].jwtValidationOptions`|object|JWT validation options controlling which claims must be present in a token.<br><br>The `required_claims` set specifies which RFC 7519 registered claims must<br>exist in the token payload before validation proceeds. Only the following<br>values are recognized: `exp`, `nbf`, `aud`, `iss`, `sub`. Other registered<br>claims such as `iat` and `jti` are **not** enforced by the underlying<br>`jsonwebtoken` library and will be silently ignored.<br><br>This only enforces **presence**. Standard claims like `exp` and `nbf`<br>have their values validated independently (e.g., expiry is always checked<br>when the `exp` claim is present, regardless of this setting).<br><br>Defaults to `["exp"]`.|
|`binds[].listeners[].policies.jwtAuth.providers[].jwtValidationOptions.requiredClaims`|[]string|Claims that must be present in the token before validation.<br>Only "exp", "nbf", "aud", "iss", "sub" are enforced; others<br>(including "iat" and "jti") are ignored.<br>Defaults to ["exp"]. Use an empty list to require no claims.|
|`binds[].listeners[].policies.jwtAuth.mode`|string||
|`binds[].listeners[].policies.jwtAuth.issuer`|string||
|`binds[].listeners[].policies.jwtAuth.audiences`|[]string||
|`binds[].listeners[].policies.jwtAuth.jwks`|object||
|`binds[].listeners[].policies.jwtAuth.jwks.file`|string||
|`binds[].listeners[].policies.jwtAuth.jwks.url`|string||
|`binds[].listeners[].policies.jwtAuth.jwtValidationOptions`|object|JWT validation options controlling which claims must be present in a token.<br><br>The `required_claims` set specifies which RFC 7519 registered claims must<br>exist in the token payload before validation proceeds. Only the following<br>values are recognized: `exp`, `nbf`, `aud`, `iss`, `sub`. Other registered<br>claims such as `iat` and `jti` are **not** enforced by the underlying<br>`jsonwebtoken` library and will be silently ignored.<br><br>This only enforces **presence**. Standard claims like `exp` and `nbf`<br>have their values validated independently (e.g., expiry is always checked<br>when the `exp` claim is present, regardless of this setting).<br><br>Defaults to `["exp"]`.|
|`binds[].listeners[].policies.jwtAuth.jwtValidationOptions.requiredClaims`|[]string|Claims that must be present in the token before validation.<br>Only "exp", "nbf", "aud", "iss", "sub" are enforced; others<br>(including "iat" and "jti") are ignored.<br>Defaults to ["exp"]. Use an empty list to require no claims.|
|`binds[].listeners[].policies.extAuthz`|object|Authenticate incoming requests by calling an external authorization server.|
|`binds[].listeners[].policies.extAuthz.service`|object||
|`binds[].listeners[].policies.extAuthz.service.name`|object||
|`binds[].listeners[].policies.extAuthz.service.name.namespace`|string||
|`binds[].listeners[].policies.extAuthz.service.name.hostname`|string||
|`binds[].listeners[].policies.extAuthz.service.port`|integer||
|`binds[].listeners[].policies.extAuthz.host`|string|Hostname or IP address|
|`binds[].listeners[].policies.extAuthz.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].policies.extAuthz.policies`|object|Policies to connect to the backend|
|`binds[].listeners[].policies.extAuthz.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`binds[].listeners[].policies.extAuthz.policies.requestHeaderModifier.add`|object||
|`binds[].listeners[].policies.extAuthz.policies.requestHeaderModifier.set`|object||
|`binds[].listeners[].policies.extAuthz.policies.requestHeaderModifier.remove`|[]string||
|`binds[].listeners[].policies.extAuthz.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`binds[].listeners[].policies.extAuthz.policies.responseHeaderModifier.add`|object||
|`binds[].listeners[].policies.extAuthz.policies.responseHeaderModifier.set`|object||
|`binds[].listeners[].policies.extAuthz.policies.responseHeaderModifier.remove`|[]string||
|`binds[].listeners[].policies.extAuthz.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`binds[].listeners[].policies.extAuthz.policies.requestRedirect.scheme`|string||
|`binds[].listeners[].policies.extAuthz.policies.requestRedirect.authority`|string||
|`binds[].listeners[].policies.extAuthz.policies.requestRedirect.authority.full`|string||
|`binds[].listeners[].policies.extAuthz.policies.requestRedirect.authority.host`|string||
|`binds[].listeners[].policies.extAuthz.policies.requestRedirect.authority.port`|integer||
|`binds[].listeners[].policies.extAuthz.policies.requestRedirect.path`|object||
|`binds[].listeners[].policies.extAuthz.policies.requestRedirect.path.full`|string||
|`binds[].listeners[].policies.extAuthz.policies.requestRedirect.path.prefix`|string||
|`binds[].listeners[].policies.extAuthz.policies.requestRedirect.status`|integer||
|`binds[].listeners[].policies.extAuthz.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`binds[].listeners[].policies.extAuthz.policies.transformations.request`|object||
|`binds[].listeners[].policies.extAuthz.policies.transformations.request.add`|object||
|`binds[].listeners[].policies.extAuthz.policies.transformations.request.set`|object||
|`binds[].listeners[].policies.extAuthz.policies.transformations.request.remove`|[]string||
|`binds[].listeners[].policies.extAuthz.policies.transformations.request.body`|string||
|`binds[].listeners[].policies.extAuthz.policies.transformations.request.metadata`|object||
|`binds[].listeners[].policies.extAuthz.policies.transformations.response`|object||
|`binds[].listeners[].policies.extAuthz.policies.transformations.response.add`|object||
|`binds[].listeners[].policies.extAuthz.policies.transformations.response.set`|object||
|`binds[].listeners[].policies.extAuthz.policies.transformations.response.remove`|[]string||
|`binds[].listeners[].policies.extAuthz.policies.transformations.response.body`|string||
|`binds[].listeners[].policies.extAuthz.policies.transformations.response.metadata`|object||
|`binds[].listeners[].policies.extAuthz.policies.backendTLS`|object|Send TLS to the backend.|
|`binds[].listeners[].policies.extAuthz.policies.backendTLS.cert`|string||
|`binds[].listeners[].policies.extAuthz.policies.backendTLS.key`|string||
|`binds[].listeners[].policies.extAuthz.policies.backendTLS.root`|string||
|`binds[].listeners[].policies.extAuthz.policies.backendTLS.hostname`|string||
|`binds[].listeners[].policies.extAuthz.policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].policies.extAuthz.policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].policies.extAuthz.policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].policies.extAuthz.policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].policies.extAuthz.policies.backendAuth`|object|Authenticate to the backend.|
|`binds[].listeners[].policies.extAuthz.policies.backendAuth.passthrough`|object||
|`binds[].listeners[].policies.extAuthz.policies.backendAuth.key`|object||
|`binds[].listeners[].policies.extAuthz.policies.backendAuth.key.file`|string||
|`binds[].listeners[].policies.extAuthz.policies.backendAuth.gcp`|object||
|`binds[].listeners[].policies.extAuthz.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].policies.extAuthz.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`binds[].listeners[].policies.extAuthz.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].policies.extAuthz.policies.backendAuth.aws`|object||
|`binds[].listeners[].policies.extAuthz.policies.backendAuth.aws.accessKeyId`|string||
|`binds[].listeners[].policies.extAuthz.policies.backendAuth.aws.secretAccessKey`|string||
|`binds[].listeners[].policies.extAuthz.policies.backendAuth.aws.region`|string||
|`binds[].listeners[].policies.extAuthz.policies.backendAuth.aws.sessionToken`|string||
|`binds[].listeners[].policies.extAuthz.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`binds[].listeners[].policies.extAuthz.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`binds[].listeners[].policies.extAuthz.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`binds[].listeners[].policies.extAuthz.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`binds[].listeners[].policies.extAuthz.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`binds[].listeners[].policies.extAuthz.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`binds[].listeners[].policies.extAuthz.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`binds[].listeners[].policies.extAuthz.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`binds[].listeners[].policies.extAuthz.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`binds[].listeners[].policies.extAuthz.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`binds[].listeners[].policies.extAuthz.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`binds[].listeners[].policies.extAuthz.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`binds[].listeners[].policies.extAuthz.policies.backendAuth.azure.developerImplicit`|object||
|`binds[].listeners[].policies.extAuthz.policies.backendAuth.azure.implicit`|object||
|`binds[].listeners[].policies.extAuthz.policies.http`|object|Specify HTTP settings for the backend|
|`binds[].listeners[].policies.extAuthz.policies.http.version`|string||
|`binds[].listeners[].policies.extAuthz.policies.http.requestTimeout`|string||
|`binds[].listeners[].policies.extAuthz.policies.tcp`|object|Specify TCP settings for the backend|
|`binds[].listeners[].policies.extAuthz.policies.tcp.keepalives`|object||
|`binds[].listeners[].policies.extAuthz.policies.tcp.keepalives.enabled`|boolean||
|`binds[].listeners[].policies.extAuthz.policies.tcp.keepalives.time`|string||
|`binds[].listeners[].policies.extAuthz.policies.tcp.keepalives.interval`|string||
|`binds[].listeners[].policies.extAuthz.policies.tcp.keepalives.retries`|integer||
|`binds[].listeners[].policies.extAuthz.policies.tcp.connectTimeout`|object||
|`binds[].listeners[].policies.extAuthz.policies.tcp.connectTimeout.secs`|integer||
|`binds[].listeners[].policies.extAuthz.policies.tcp.connectTimeout.nanos`|integer||
|`binds[].listeners[].policies.extAuthz.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`binds[].listeners[].policies.extAuthz.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`binds[].listeners[].policies.extAuthz.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`binds[].listeners[].policies.extAuthz.policies.health.eviction.duration`|string||
|`binds[].listeners[].policies.extAuthz.policies.health.eviction.restoreHealth`|number||
|`binds[].listeners[].policies.extAuthz.policies.health.eviction.consecutiveFailures`|integer||
|`binds[].listeners[].policies.extAuthz.policies.health.eviction.healthThreshold`|number||
|`binds[].listeners[].policies.extAuthz.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`binds[].listeners[].policies.extAuthz.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].policies.extAuthz.policies.backendTunnel.proxy.service`|object||
|`binds[].listeners[].policies.extAuthz.policies.backendTunnel.proxy.service.name`|object||
|`binds[].listeners[].policies.extAuthz.policies.backendTunnel.proxy.service.name.namespace`|string||
|`binds[].listeners[].policies.extAuthz.policies.backendTunnel.proxy.service.name.hostname`|string||
|`binds[].listeners[].policies.extAuthz.policies.backendTunnel.proxy.service.port`|integer||
|`binds[].listeners[].policies.extAuthz.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`binds[].listeners[].policies.extAuthz.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].policies.extAuthz.protocol`|object|The ext_authz protocol to use. Unless you need to integrate with an HTTP-only server, gRPC is recommended.<br>Exactly one of grpc or http may be set.|
|`binds[].listeners[].policies.extAuthz.protocol.grpc`|object||
|`binds[].listeners[].policies.extAuthz.protocol.grpc.context`|object|Additional context to send to the authorization service.<br>This maps to the `context_extensions` field of the request, and only allows static values.|
|`binds[].listeners[].policies.extAuthz.protocol.grpc.metadata`|object|Additional metadata to send to the authorization service.<br>This maps to the `metadata_context.filter_metadata` field of the request, and allows dynamic CEL expressions.<br>If unset, by default the `envoy.filters.http.jwt_authn` key is set if the JWT policy is used as well, for compatibility.|
|`binds[].listeners[].policies.extAuthz.protocol.http`|object||
|`binds[].listeners[].policies.extAuthz.protocol.http.path`|string||
|`binds[].listeners[].policies.extAuthz.protocol.http.redirect`|string|When using the HTTP protocol, and the server returns unauthorized, redirect to the URL resolved by<br>the provided expression rather than directly returning the error.|
|`binds[].listeners[].policies.extAuthz.protocol.http.includeResponseHeaders`|[]string|Specific headers from the authorization response will be copied into the request to the backend.|
|`binds[].listeners[].policies.extAuthz.protocol.http.addRequestHeaders`|object|Specific headers to add in the authorization request (empty = all headers), based on the expression|
|`binds[].listeners[].policies.extAuthz.protocol.http.metadata`|object|Metadata to include under the `extauthz` variable, based on the authorization response.|
|`binds[].listeners[].policies.extAuthz.failureMode`|string|Behavior when the authorization service is unavailable or returns an error|
|`binds[].listeners[].policies.extAuthz.failureMode.denyWithStatus`|integer||
|`binds[].listeners[].policies.extAuthz.includeRequestHeaders`|[]string|Specific headers to include in the authorization request.<br>If unset, the gRPC protocol sends all request headers. The HTTP protocol sends only 'Authorization'.|
|`binds[].listeners[].policies.extAuthz.includeRequestBody`|object|Options for including the request body in the authorization request|
|`binds[].listeners[].policies.extAuthz.includeRequestBody.maxRequestBytes`|integer|Maximum size of request body to buffer (default: 8192)|
|`binds[].listeners[].policies.extAuthz.includeRequestBody.allowPartialMessage`|boolean|If true, send partial body when max_request_bytes is reached|
|`binds[].listeners[].policies.extAuthz.includeRequestBody.packAsBytes`|boolean|If true, pack body as raw bytes in gRPC|
|`binds[].listeners[].policies.extProc`|object|Extend agentgateway with an external processor|
|`binds[].listeners[].policies.extProc.service`|object||
|`binds[].listeners[].policies.extProc.service.name`|object||
|`binds[].listeners[].policies.extProc.service.name.namespace`|string||
|`binds[].listeners[].policies.extProc.service.name.hostname`|string||
|`binds[].listeners[].policies.extProc.service.port`|integer||
|`binds[].listeners[].policies.extProc.host`|string|Hostname or IP address|
|`binds[].listeners[].policies.extProc.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].policies.extProc.policies`|object|Policies to connect to the backend|
|`binds[].listeners[].policies.extProc.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`binds[].listeners[].policies.extProc.policies.requestHeaderModifier.add`|object||
|`binds[].listeners[].policies.extProc.policies.requestHeaderModifier.set`|object||
|`binds[].listeners[].policies.extProc.policies.requestHeaderModifier.remove`|[]string||
|`binds[].listeners[].policies.extProc.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`binds[].listeners[].policies.extProc.policies.responseHeaderModifier.add`|object||
|`binds[].listeners[].policies.extProc.policies.responseHeaderModifier.set`|object||
|`binds[].listeners[].policies.extProc.policies.responseHeaderModifier.remove`|[]string||
|`binds[].listeners[].policies.extProc.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`binds[].listeners[].policies.extProc.policies.requestRedirect.scheme`|string||
|`binds[].listeners[].policies.extProc.policies.requestRedirect.authority`|string||
|`binds[].listeners[].policies.extProc.policies.requestRedirect.authority.full`|string||
|`binds[].listeners[].policies.extProc.policies.requestRedirect.authority.host`|string||
|`binds[].listeners[].policies.extProc.policies.requestRedirect.authority.port`|integer||
|`binds[].listeners[].policies.extProc.policies.requestRedirect.path`|object||
|`binds[].listeners[].policies.extProc.policies.requestRedirect.path.full`|string||
|`binds[].listeners[].policies.extProc.policies.requestRedirect.path.prefix`|string||
|`binds[].listeners[].policies.extProc.policies.requestRedirect.status`|integer||
|`binds[].listeners[].policies.extProc.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`binds[].listeners[].policies.extProc.policies.transformations.request`|object||
|`binds[].listeners[].policies.extProc.policies.transformations.request.add`|object||
|`binds[].listeners[].policies.extProc.policies.transformations.request.set`|object||
|`binds[].listeners[].policies.extProc.policies.transformations.request.remove`|[]string||
|`binds[].listeners[].policies.extProc.policies.transformations.request.body`|string||
|`binds[].listeners[].policies.extProc.policies.transformations.request.metadata`|object||
|`binds[].listeners[].policies.extProc.policies.transformations.response`|object||
|`binds[].listeners[].policies.extProc.policies.transformations.response.add`|object||
|`binds[].listeners[].policies.extProc.policies.transformations.response.set`|object||
|`binds[].listeners[].policies.extProc.policies.transformations.response.remove`|[]string||
|`binds[].listeners[].policies.extProc.policies.transformations.response.body`|string||
|`binds[].listeners[].policies.extProc.policies.transformations.response.metadata`|object||
|`binds[].listeners[].policies.extProc.policies.backendTLS`|object|Send TLS to the backend.|
|`binds[].listeners[].policies.extProc.policies.backendTLS.cert`|string||
|`binds[].listeners[].policies.extProc.policies.backendTLS.key`|string||
|`binds[].listeners[].policies.extProc.policies.backendTLS.root`|string||
|`binds[].listeners[].policies.extProc.policies.backendTLS.hostname`|string||
|`binds[].listeners[].policies.extProc.policies.backendTLS.insecure`|boolean||
|`binds[].listeners[].policies.extProc.policies.backendTLS.insecureHost`|boolean||
|`binds[].listeners[].policies.extProc.policies.backendTLS.alpn`|[]string||
|`binds[].listeners[].policies.extProc.policies.backendTLS.subjectAltNames`|[]string||
|`binds[].listeners[].policies.extProc.policies.backendAuth`|object|Authenticate to the backend.|
|`binds[].listeners[].policies.extProc.policies.backendAuth.passthrough`|object||
|`binds[].listeners[].policies.extProc.policies.backendAuth.key`|object||
|`binds[].listeners[].policies.extProc.policies.backendAuth.key.file`|string||
|`binds[].listeners[].policies.extProc.policies.backendAuth.gcp`|object||
|`binds[].listeners[].policies.extProc.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].policies.extProc.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`binds[].listeners[].policies.extProc.policies.backendAuth.gcp.type`|string||
|`binds[].listeners[].policies.extProc.policies.backendAuth.aws`|object||
|`binds[].listeners[].policies.extProc.policies.backendAuth.aws.accessKeyId`|string||
|`binds[].listeners[].policies.extProc.policies.backendAuth.aws.secretAccessKey`|string||
|`binds[].listeners[].policies.extProc.policies.backendAuth.aws.region`|string||
|`binds[].listeners[].policies.extProc.policies.backendAuth.aws.sessionToken`|string||
|`binds[].listeners[].policies.extProc.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`binds[].listeners[].policies.extProc.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`binds[].listeners[].policies.extProc.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`binds[].listeners[].policies.extProc.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`binds[].listeners[].policies.extProc.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`binds[].listeners[].policies.extProc.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`binds[].listeners[].policies.extProc.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`binds[].listeners[].policies.extProc.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`binds[].listeners[].policies.extProc.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`binds[].listeners[].policies.extProc.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`binds[].listeners[].policies.extProc.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`binds[].listeners[].policies.extProc.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`binds[].listeners[].policies.extProc.policies.backendAuth.azure.developerImplicit`|object||
|`binds[].listeners[].policies.extProc.policies.backendAuth.azure.implicit`|object||
|`binds[].listeners[].policies.extProc.policies.http`|object|Specify HTTP settings for the backend|
|`binds[].listeners[].policies.extProc.policies.http.version`|string||
|`binds[].listeners[].policies.extProc.policies.http.requestTimeout`|string||
|`binds[].listeners[].policies.extProc.policies.tcp`|object|Specify TCP settings for the backend|
|`binds[].listeners[].policies.extProc.policies.tcp.keepalives`|object||
|`binds[].listeners[].policies.extProc.policies.tcp.keepalives.enabled`|boolean||
|`binds[].listeners[].policies.extProc.policies.tcp.keepalives.time`|string||
|`binds[].listeners[].policies.extProc.policies.tcp.keepalives.interval`|string||
|`binds[].listeners[].policies.extProc.policies.tcp.keepalives.retries`|integer||
|`binds[].listeners[].policies.extProc.policies.tcp.connectTimeout`|object||
|`binds[].listeners[].policies.extProc.policies.tcp.connectTimeout.secs`|integer||
|`binds[].listeners[].policies.extProc.policies.tcp.connectTimeout.nanos`|integer||
|`binds[].listeners[].policies.extProc.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`binds[].listeners[].policies.extProc.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`binds[].listeners[].policies.extProc.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`binds[].listeners[].policies.extProc.policies.health.eviction.duration`|string||
|`binds[].listeners[].policies.extProc.policies.health.eviction.restoreHealth`|number||
|`binds[].listeners[].policies.extProc.policies.health.eviction.consecutiveFailures`|integer||
|`binds[].listeners[].policies.extProc.policies.health.eviction.healthThreshold`|number||
|`binds[].listeners[].policies.extProc.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`binds[].listeners[].policies.extProc.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`binds[].listeners[].policies.extProc.policies.backendTunnel.proxy.service`|object||
|`binds[].listeners[].policies.extProc.policies.backendTunnel.proxy.service.name`|object||
|`binds[].listeners[].policies.extProc.policies.backendTunnel.proxy.service.name.namespace`|string||
|`binds[].listeners[].policies.extProc.policies.backendTunnel.proxy.service.name.hostname`|string||
|`binds[].listeners[].policies.extProc.policies.backendTunnel.proxy.service.port`|integer||
|`binds[].listeners[].policies.extProc.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`binds[].listeners[].policies.extProc.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`binds[].listeners[].policies.extProc.failureMode`|string|Behavior when the ext_proc service is unavailable or returns an error|
|`binds[].listeners[].policies.extProc.metadataContext`|object|Additional metadata to send to the external processing service.<br>Maps to the `metadata_context.filter_metadata` field in ProcessingRequest, and allows dynamic CEL expressions.|
|`binds[].listeners[].policies.extProc.requestAttributes`|object|Maps to the request `attributes` field in ProcessingRequest, and allows dynamic CEL expressions.|
|`binds[].listeners[].policies.extProc.responseAttributes`|object|Maps to the response `attributes` field in ProcessingRequest, and allows dynamic CEL expressions.|
|`binds[].listeners[].policies.transformations`|object|Modify requests and responses|
|`binds[].listeners[].policies.transformations.request`|object||
|`binds[].listeners[].policies.transformations.request.add`|object||
|`binds[].listeners[].policies.transformations.request.set`|object||
|`binds[].listeners[].policies.transformations.request.remove`|[]string||
|`binds[].listeners[].policies.transformations.request.body`|string||
|`binds[].listeners[].policies.transformations.request.metadata`|object||
|`binds[].listeners[].policies.transformations.response`|object||
|`binds[].listeners[].policies.transformations.response.add`|object||
|`binds[].listeners[].policies.transformations.response.set`|object||
|`binds[].listeners[].policies.transformations.response.remove`|[]string||
|`binds[].listeners[].policies.transformations.response.body`|string||
|`binds[].listeners[].policies.transformations.response.metadata`|object||
|`binds[].listeners[].policies.basicAuth`|object|Authenticate incoming requests using Basic Authentication with htpasswd.|
|`binds[].listeners[].policies.basicAuth.htpasswd`|object|.htpasswd file contents/reference|
|`binds[].listeners[].policies.basicAuth.htpasswd.file`|string||
|`binds[].listeners[].policies.basicAuth.realm`|string|Realm name for the WWW-Authenticate header|
|`binds[].listeners[].policies.basicAuth.mode`|string|Validation mode for basic authentication|
|`binds[].listeners[].policies.apiKey`|object|Authenticate incoming requests using API Keys|
|`binds[].listeners[].policies.apiKey.keys`|[]object|List of API keys|
|`binds[].listeners[].policies.apiKey.keys[].key`|string||
|`binds[].listeners[].policies.apiKey.keys[].metadata`|any||
|`binds[].listeners[].policies.apiKey.mode`|string|Validation mode for API keys|
|`binds[].tunnelProtocol`|string||
|`frontendPolicies`|object||
|`frontendPolicies.http`|object|Settings for handling incoming HTTP requests.|
|`frontendPolicies.http.maxBufferSize`|integer||
|`frontendPolicies.http.http1MaxHeaders`|integer|The maximum number of headers allowed in a request. Changing this value results in a performance<br>degradation, even if set to a lower value than the default (100)|
|`frontendPolicies.http.http1IdleTimeout`|string||
|`frontendPolicies.http.http2WindowSize`|integer||
|`frontendPolicies.http.http2ConnectionWindowSize`|integer||
|`frontendPolicies.http.http2FrameSize`|integer||
|`frontendPolicies.http.http2KeepaliveInterval`|string||
|`frontendPolicies.http.http2KeepaliveTimeout`|string||
|`frontendPolicies.tls`|object|Settings for handling incoming TLS connections.|
|`frontendPolicies.tls.handshakeTimeout`|string||
|`frontendPolicies.tls.alpn`|array||
|`frontendPolicies.tls.minVersion`|string||
|`frontendPolicies.tls.maxVersion`|string||
|`frontendPolicies.tls.cipherSuites`|[]string||
|`frontendPolicies.tcp`|object|Settings for handling incoming TCP connections.|
|`frontendPolicies.tcp.keepalives`|object||
|`frontendPolicies.tcp.keepalives.enabled`|boolean||
|`frontendPolicies.tcp.keepalives.time`|string||
|`frontendPolicies.tcp.keepalives.interval`|string||
|`frontendPolicies.tcp.keepalives.retries`|integer||
|`frontendPolicies.networkAuthorization`|object|CEL authorization for downstream network connections.|
|`frontendPolicies.networkAuthorization.rules`|[]string||
|`frontendPolicies.accessLog`|object|Settings for request access logs.|
|`frontendPolicies.accessLog.filter`|string||
|`frontendPolicies.accessLog.add`|object||
|`frontendPolicies.accessLog.remove`|[]string||
|`frontendPolicies.accessLog.otlp`|object||
|`frontendPolicies.accessLog.otlp.service`|object||
|`frontendPolicies.accessLog.otlp.service.name`|object||
|`frontendPolicies.accessLog.otlp.service.name.namespace`|string||
|`frontendPolicies.accessLog.otlp.service.name.hostname`|string||
|`frontendPolicies.accessLog.otlp.service.port`|integer||
|`frontendPolicies.accessLog.otlp.host`|string|Hostname or IP address|
|`frontendPolicies.accessLog.otlp.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`frontendPolicies.accessLog.otlp.policies`|object||
|`frontendPolicies.accessLog.otlp.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`frontendPolicies.accessLog.otlp.policies.requestHeaderModifier.add`|object||
|`frontendPolicies.accessLog.otlp.policies.requestHeaderModifier.set`|object||
|`frontendPolicies.accessLog.otlp.policies.requestHeaderModifier.remove`|[]string||
|`frontendPolicies.accessLog.otlp.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`frontendPolicies.accessLog.otlp.policies.responseHeaderModifier.add`|object||
|`frontendPolicies.accessLog.otlp.policies.responseHeaderModifier.set`|object||
|`frontendPolicies.accessLog.otlp.policies.responseHeaderModifier.remove`|[]string||
|`frontendPolicies.accessLog.otlp.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`frontendPolicies.accessLog.otlp.policies.requestRedirect.scheme`|string||
|`frontendPolicies.accessLog.otlp.policies.requestRedirect.authority`|string||
|`frontendPolicies.accessLog.otlp.policies.requestRedirect.authority.full`|string||
|`frontendPolicies.accessLog.otlp.policies.requestRedirect.authority.host`|string||
|`frontendPolicies.accessLog.otlp.policies.requestRedirect.authority.port`|integer||
|`frontendPolicies.accessLog.otlp.policies.requestRedirect.path`|object||
|`frontendPolicies.accessLog.otlp.policies.requestRedirect.path.full`|string||
|`frontendPolicies.accessLog.otlp.policies.requestRedirect.path.prefix`|string||
|`frontendPolicies.accessLog.otlp.policies.requestRedirect.status`|integer||
|`frontendPolicies.accessLog.otlp.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`frontendPolicies.accessLog.otlp.policies.transformations.request`|object||
|`frontendPolicies.accessLog.otlp.policies.transformations.request.add`|object||
|`frontendPolicies.accessLog.otlp.policies.transformations.request.set`|object||
|`frontendPolicies.accessLog.otlp.policies.transformations.request.remove`|[]string||
|`frontendPolicies.accessLog.otlp.policies.transformations.request.body`|string||
|`frontendPolicies.accessLog.otlp.policies.transformations.request.metadata`|object||
|`frontendPolicies.accessLog.otlp.policies.transformations.response`|object||
|`frontendPolicies.accessLog.otlp.policies.transformations.response.add`|object||
|`frontendPolicies.accessLog.otlp.policies.transformations.response.set`|object||
|`frontendPolicies.accessLog.otlp.policies.transformations.response.remove`|[]string||
|`frontendPolicies.accessLog.otlp.policies.transformations.response.body`|string||
|`frontendPolicies.accessLog.otlp.policies.transformations.response.metadata`|object||
|`frontendPolicies.accessLog.otlp.policies.backendTLS`|object|Send TLS to the backend.|
|`frontendPolicies.accessLog.otlp.policies.backendTLS.cert`|string||
|`frontendPolicies.accessLog.otlp.policies.backendTLS.key`|string||
|`frontendPolicies.accessLog.otlp.policies.backendTLS.root`|string||
|`frontendPolicies.accessLog.otlp.policies.backendTLS.hostname`|string||
|`frontendPolicies.accessLog.otlp.policies.backendTLS.insecure`|boolean||
|`frontendPolicies.accessLog.otlp.policies.backendTLS.insecureHost`|boolean||
|`frontendPolicies.accessLog.otlp.policies.backendTLS.alpn`|[]string||
|`frontendPolicies.accessLog.otlp.policies.backendTLS.subjectAltNames`|[]string||
|`frontendPolicies.accessLog.otlp.policies.backendAuth`|object|Authenticate to the backend.|
|`frontendPolicies.accessLog.otlp.policies.backendAuth.passthrough`|object||
|`frontendPolicies.accessLog.otlp.policies.backendAuth.key`|object||
|`frontendPolicies.accessLog.otlp.policies.backendAuth.key.file`|string||
|`frontendPolicies.accessLog.otlp.policies.backendAuth.gcp`|object||
|`frontendPolicies.accessLog.otlp.policies.backendAuth.gcp.type`|string||
|`frontendPolicies.accessLog.otlp.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`frontendPolicies.accessLog.otlp.policies.backendAuth.gcp.type`|string||
|`frontendPolicies.accessLog.otlp.policies.backendAuth.aws`|object||
|`frontendPolicies.accessLog.otlp.policies.backendAuth.aws.accessKeyId`|string||
|`frontendPolicies.accessLog.otlp.policies.backendAuth.aws.secretAccessKey`|string||
|`frontendPolicies.accessLog.otlp.policies.backendAuth.aws.region`|string||
|`frontendPolicies.accessLog.otlp.policies.backendAuth.aws.sessionToken`|string||
|`frontendPolicies.accessLog.otlp.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`frontendPolicies.accessLog.otlp.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`frontendPolicies.accessLog.otlp.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`frontendPolicies.accessLog.otlp.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`frontendPolicies.accessLog.otlp.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`frontendPolicies.accessLog.otlp.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`frontendPolicies.accessLog.otlp.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`frontendPolicies.accessLog.otlp.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`frontendPolicies.accessLog.otlp.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`frontendPolicies.accessLog.otlp.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`frontendPolicies.accessLog.otlp.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`frontendPolicies.accessLog.otlp.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`frontendPolicies.accessLog.otlp.policies.backendAuth.azure.developerImplicit`|object||
|`frontendPolicies.accessLog.otlp.policies.backendAuth.azure.implicit`|object||
|`frontendPolicies.accessLog.otlp.policies.http`|object|Specify HTTP settings for the backend|
|`frontendPolicies.accessLog.otlp.policies.http.version`|string||
|`frontendPolicies.accessLog.otlp.policies.http.requestTimeout`|string||
|`frontendPolicies.accessLog.otlp.policies.tcp`|object|Specify TCP settings for the backend|
|`frontendPolicies.accessLog.otlp.policies.tcp.keepalives`|object||
|`frontendPolicies.accessLog.otlp.policies.tcp.keepalives.enabled`|boolean||
|`frontendPolicies.accessLog.otlp.policies.tcp.keepalives.time`|string||
|`frontendPolicies.accessLog.otlp.policies.tcp.keepalives.interval`|string||
|`frontendPolicies.accessLog.otlp.policies.tcp.keepalives.retries`|integer||
|`frontendPolicies.accessLog.otlp.policies.tcp.connectTimeout`|object||
|`frontendPolicies.accessLog.otlp.policies.tcp.connectTimeout.secs`|integer||
|`frontendPolicies.accessLog.otlp.policies.tcp.connectTimeout.nanos`|integer||
|`frontendPolicies.accessLog.otlp.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`frontendPolicies.accessLog.otlp.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`frontendPolicies.accessLog.otlp.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`frontendPolicies.accessLog.otlp.policies.health.eviction.duration`|string||
|`frontendPolicies.accessLog.otlp.policies.health.eviction.restoreHealth`|number||
|`frontendPolicies.accessLog.otlp.policies.health.eviction.consecutiveFailures`|integer||
|`frontendPolicies.accessLog.otlp.policies.health.eviction.healthThreshold`|number||
|`frontendPolicies.accessLog.otlp.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`frontendPolicies.accessLog.otlp.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`frontendPolicies.accessLog.otlp.policies.backendTunnel.proxy.service`|object||
|`frontendPolicies.accessLog.otlp.policies.backendTunnel.proxy.service.name`|object||
|`frontendPolicies.accessLog.otlp.policies.backendTunnel.proxy.service.name.namespace`|string||
|`frontendPolicies.accessLog.otlp.policies.backendTunnel.proxy.service.name.hostname`|string||
|`frontendPolicies.accessLog.otlp.policies.backendTunnel.proxy.service.port`|integer||
|`frontendPolicies.accessLog.otlp.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`frontendPolicies.accessLog.otlp.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`frontendPolicies.accessLog.otlp.protocol`|string||
|`frontendPolicies.accessLog.otlp.path`|string||
|`frontendPolicies.tracing`|object||
|`frontendPolicies.tracing.service`|object||
|`frontendPolicies.tracing.service.name`|object||
|`frontendPolicies.tracing.service.name.namespace`|string||
|`frontendPolicies.tracing.service.name.hostname`|string||
|`frontendPolicies.tracing.service.port`|integer||
|`frontendPolicies.tracing.host`|string|Hostname or IP address|
|`frontendPolicies.tracing.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`frontendPolicies.tracing.policies`|object|Policies to connect to the backend|
|`frontendPolicies.tracing.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`frontendPolicies.tracing.policies.requestHeaderModifier.add`|object||
|`frontendPolicies.tracing.policies.requestHeaderModifier.set`|object||
|`frontendPolicies.tracing.policies.requestHeaderModifier.remove`|[]string||
|`frontendPolicies.tracing.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`frontendPolicies.tracing.policies.responseHeaderModifier.add`|object||
|`frontendPolicies.tracing.policies.responseHeaderModifier.set`|object||
|`frontendPolicies.tracing.policies.responseHeaderModifier.remove`|[]string||
|`frontendPolicies.tracing.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`frontendPolicies.tracing.policies.requestRedirect.scheme`|string||
|`frontendPolicies.tracing.policies.requestRedirect.authority`|string||
|`frontendPolicies.tracing.policies.requestRedirect.authority.full`|string||
|`frontendPolicies.tracing.policies.requestRedirect.authority.host`|string||
|`frontendPolicies.tracing.policies.requestRedirect.authority.port`|integer||
|`frontendPolicies.tracing.policies.requestRedirect.path`|object||
|`frontendPolicies.tracing.policies.requestRedirect.path.full`|string||
|`frontendPolicies.tracing.policies.requestRedirect.path.prefix`|string||
|`frontendPolicies.tracing.policies.requestRedirect.status`|integer||
|`frontendPolicies.tracing.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`frontendPolicies.tracing.policies.transformations.request`|object||
|`frontendPolicies.tracing.policies.transformations.request.add`|object||
|`frontendPolicies.tracing.policies.transformations.request.set`|object||
|`frontendPolicies.tracing.policies.transformations.request.remove`|[]string||
|`frontendPolicies.tracing.policies.transformations.request.body`|string||
|`frontendPolicies.tracing.policies.transformations.request.metadata`|object||
|`frontendPolicies.tracing.policies.transformations.response`|object||
|`frontendPolicies.tracing.policies.transformations.response.add`|object||
|`frontendPolicies.tracing.policies.transformations.response.set`|object||
|`frontendPolicies.tracing.policies.transformations.response.remove`|[]string||
|`frontendPolicies.tracing.policies.transformations.response.body`|string||
|`frontendPolicies.tracing.policies.transformations.response.metadata`|object||
|`frontendPolicies.tracing.policies.backendTLS`|object|Send TLS to the backend.|
|`frontendPolicies.tracing.policies.backendTLS.cert`|string||
|`frontendPolicies.tracing.policies.backendTLS.key`|string||
|`frontendPolicies.tracing.policies.backendTLS.root`|string||
|`frontendPolicies.tracing.policies.backendTLS.hostname`|string||
|`frontendPolicies.tracing.policies.backendTLS.insecure`|boolean||
|`frontendPolicies.tracing.policies.backendTLS.insecureHost`|boolean||
|`frontendPolicies.tracing.policies.backendTLS.alpn`|[]string||
|`frontendPolicies.tracing.policies.backendTLS.subjectAltNames`|[]string||
|`frontendPolicies.tracing.policies.backendAuth`|object|Authenticate to the backend.|
|`frontendPolicies.tracing.policies.backendAuth.passthrough`|object||
|`frontendPolicies.tracing.policies.backendAuth.key`|object||
|`frontendPolicies.tracing.policies.backendAuth.key.file`|string||
|`frontendPolicies.tracing.policies.backendAuth.gcp`|object||
|`frontendPolicies.tracing.policies.backendAuth.gcp.type`|string||
|`frontendPolicies.tracing.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`frontendPolicies.tracing.policies.backendAuth.gcp.type`|string||
|`frontendPolicies.tracing.policies.backendAuth.aws`|object||
|`frontendPolicies.tracing.policies.backendAuth.aws.accessKeyId`|string||
|`frontendPolicies.tracing.policies.backendAuth.aws.secretAccessKey`|string||
|`frontendPolicies.tracing.policies.backendAuth.aws.region`|string||
|`frontendPolicies.tracing.policies.backendAuth.aws.sessionToken`|string||
|`frontendPolicies.tracing.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`frontendPolicies.tracing.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`frontendPolicies.tracing.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`frontendPolicies.tracing.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`frontendPolicies.tracing.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`frontendPolicies.tracing.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`frontendPolicies.tracing.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`frontendPolicies.tracing.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`frontendPolicies.tracing.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`frontendPolicies.tracing.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`frontendPolicies.tracing.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`frontendPolicies.tracing.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`frontendPolicies.tracing.policies.backendAuth.azure.developerImplicit`|object||
|`frontendPolicies.tracing.policies.backendAuth.azure.implicit`|object||
|`frontendPolicies.tracing.policies.http`|object|Specify HTTP settings for the backend|
|`frontendPolicies.tracing.policies.http.version`|string||
|`frontendPolicies.tracing.policies.http.requestTimeout`|string||
|`frontendPolicies.tracing.policies.tcp`|object|Specify TCP settings for the backend|
|`frontendPolicies.tracing.policies.tcp.keepalives`|object||
|`frontendPolicies.tracing.policies.tcp.keepalives.enabled`|boolean||
|`frontendPolicies.tracing.policies.tcp.keepalives.time`|string||
|`frontendPolicies.tracing.policies.tcp.keepalives.interval`|string||
|`frontendPolicies.tracing.policies.tcp.keepalives.retries`|integer||
|`frontendPolicies.tracing.policies.tcp.connectTimeout`|object||
|`frontendPolicies.tracing.policies.tcp.connectTimeout.secs`|integer||
|`frontendPolicies.tracing.policies.tcp.connectTimeout.nanos`|integer||
|`frontendPolicies.tracing.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`frontendPolicies.tracing.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`frontendPolicies.tracing.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`frontendPolicies.tracing.policies.health.eviction.duration`|string||
|`frontendPolicies.tracing.policies.health.eviction.restoreHealth`|number||
|`frontendPolicies.tracing.policies.health.eviction.consecutiveFailures`|integer||
|`frontendPolicies.tracing.policies.health.eviction.healthThreshold`|number||
|`frontendPolicies.tracing.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`frontendPolicies.tracing.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`frontendPolicies.tracing.policies.backendTunnel.proxy.service`|object||
|`frontendPolicies.tracing.policies.backendTunnel.proxy.service.name`|object||
|`frontendPolicies.tracing.policies.backendTunnel.proxy.service.name.namespace`|string||
|`frontendPolicies.tracing.policies.backendTunnel.proxy.service.name.hostname`|string||
|`frontendPolicies.tracing.policies.backendTunnel.proxy.service.port`|integer||
|`frontendPolicies.tracing.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`frontendPolicies.tracing.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`frontendPolicies.tracing.attributes`|object|Span attributes to add, keyed by attribute name.|
|`frontendPolicies.tracing.resources`|object|Resource attributes to add to the tracer provider (OTel `Resource`).<br>This can be used to set things like `service.name` dynamically.|
|`frontendPolicies.tracing.remove`|[]string|Attribute keys to remove from the emitted span attributes.<br><br>This is applied before `attributes` are evaluated/added, so it can be used to drop<br>default attributes or avoid duplication.|
|`frontendPolicies.tracing.randomSampling`|string|Optional per-policy override for random sampling. If set, overrides global config for<br>requests that use this frontend policy.|
|`frontendPolicies.tracing.clientSampling`|string|Optional per-policy override for client sampling. If set, overrides global config for<br>requests that use this frontend policy.|
|`frontendPolicies.tracing.path`|string||
|`frontendPolicies.tracing.protocol`|string||
|`policies`|[]object|policies defines additional policies that can be attached to various other configurations.<br>This is an advanced feature; users should typically use the inline `policies` field under route/gateway.|
|`policies[].name`|object||
|`policies[].name.name`|string||
|`policies[].name.namespace`|string||
|`policies[].target`|object|Exactly one of gateway, route, or backend may be set.|
|`policies[].target.gateway`|object||
|`policies[].target.gateway.gatewayName`|string||
|`policies[].target.gateway.gatewayNamespace`|string||
|`policies[].target.gateway.listenerName`|string||
|`policies[].target.route`|object||
|`policies[].target.route.name`|string||
|`policies[].target.route.namespace`|string||
|`policies[].target.route.ruleName`|string||
|`policies[].target.route.kind`|string||
|`policies[].target.backend`|object|Exactly one of backend or service may be set.|
|`policies[].target.backend.backend`|object||
|`policies[].target.backend.backend.name`|string||
|`policies[].target.backend.backend.namespace`|string||
|`policies[].target.backend.backend.section`|string||
|`policies[].target.backend.service`|object||
|`policies[].target.backend.service.hostname`|string||
|`policies[].target.backend.service.namespace`|string||
|`policies[].target.backend.service.port`|integer||
|`policies[].phase`|string|phase defines at what level the policy runs at. Gateway policies run pre-routing, while<br>Route policies apply post-routing.<br>Only a subset of policies are eligible as Gateway policies.<br>In general, normal (route level) policies should be used, except you need the policy to influence<br>routing.|
|`policies[].policy`|object||
|`policies[].policy.requestHeaderModifier`|object|Headers to be modified in the request.|
|`policies[].policy.requestHeaderModifier.add`|object||
|`policies[].policy.requestHeaderModifier.set`|object||
|`policies[].policy.requestHeaderModifier.remove`|[]string||
|`policies[].policy.responseHeaderModifier`|object|Headers to be modified in the response.|
|`policies[].policy.responseHeaderModifier.add`|object||
|`policies[].policy.responseHeaderModifier.set`|object||
|`policies[].policy.responseHeaderModifier.remove`|[]string||
|`policies[].policy.requestRedirect`|object|Directly respond to the request with a redirect.|
|`policies[].policy.requestRedirect.scheme`|string||
|`policies[].policy.requestRedirect.authority`|string||
|`policies[].policy.requestRedirect.authority.full`|string||
|`policies[].policy.requestRedirect.authority.host`|string||
|`policies[].policy.requestRedirect.authority.port`|integer||
|`policies[].policy.requestRedirect.path`|object||
|`policies[].policy.requestRedirect.path.full`|string||
|`policies[].policy.requestRedirect.path.prefix`|string||
|`policies[].policy.requestRedirect.status`|integer||
|`policies[].policy.urlRewrite`|object|Modify the URL path or authority.|
|`policies[].policy.urlRewrite.authority`|string||
|`policies[].policy.urlRewrite.authority.full`|string||
|`policies[].policy.urlRewrite.authority.host`|string||
|`policies[].policy.urlRewrite.authority.port`|integer||
|`policies[].policy.urlRewrite.path`|object||
|`policies[].policy.urlRewrite.path.full`|string||
|`policies[].policy.urlRewrite.path.prefix`|string||
|`policies[].policy.requestMirror`|object|Mirror incoming requests to another destination.|
|`policies[].policy.requestMirror.backend`|object|Exactly one of service, host, or backend may be set.|
|`policies[].policy.requestMirror.backend.service`|object||
|`policies[].policy.requestMirror.backend.service.name`|object||
|`policies[].policy.requestMirror.backend.service.name.namespace`|string||
|`policies[].policy.requestMirror.backend.service.name.hostname`|string||
|`policies[].policy.requestMirror.backend.service.port`|integer||
|`policies[].policy.requestMirror.backend.host`|string|Hostname or IP address|
|`policies[].policy.requestMirror.backend.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`policies[].policy.requestMirror.percentage`|number||
|`policies[].policy.directResponse`|object|Directly respond to the request with a static response.|
|`policies[].policy.directResponse.body`|array||
|`policies[].policy.directResponse.status`|integer||
|`policies[].policy.cors`|object|Handle CORS preflight requests and append configured CORS headers to applicable requests.|
|`policies[].policy.cors.allowCredentials`|boolean||
|`policies[].policy.cors.allowHeaders`|[]string||
|`policies[].policy.cors.allowMethods`|[]string||
|`policies[].policy.cors.allowOrigins`|[]string||
|`policies[].policy.cors.exposeHeaders`|[]string||
|`policies[].policy.cors.maxAge`|string||
|`policies[].policy.mcpAuthorization`|object|Authorization policies for MCP access.|
|`policies[].policy.mcpAuthorization.rules`|[]string||
|`policies[].policy.authorization`|object|Authorization policies for HTTP access.|
|`policies[].policy.authorization.rules`|[]string||
|`policies[].policy.mcpAuthentication`|object|Authentication for MCP clients.|
|`policies[].policy.mcpAuthentication.issuer`|string||
|`policies[].policy.mcpAuthentication.audiences`|[]string||
|`policies[].policy.mcpAuthentication.provider`|object||
|`policies[].policy.mcpAuthentication.provider.auth0`|object||
|`policies[].policy.mcpAuthentication.provider.keycloak`|object||
|`policies[].policy.mcpAuthentication.resourceMetadata`|object||
|`policies[].policy.mcpAuthentication.jwks`|object||
|`policies[].policy.mcpAuthentication.jwks.file`|string||
|`policies[].policy.mcpAuthentication.jwks.url`|string||
|`policies[].policy.mcpAuthentication.mode`|string||
|`policies[].policy.mcpAuthentication.jwtValidationOptions`|object|JWT validation options controlling which claims must be present in a token.<br><br>The `required_claims` set specifies which RFC 7519 registered claims must<br>exist in the token payload before validation proceeds. Only the following<br>values are recognized: `exp`, `nbf`, `aud`, `iss`, `sub`. Other registered<br>claims such as `iat` and `jti` are **not** enforced by the underlying<br>`jsonwebtoken` library and will be silently ignored.<br><br>This only enforces **presence**. Standard claims like `exp` and `nbf`<br>have their values validated independently (e.g., expiry is always checked<br>when the `exp` claim is present, regardless of this setting).<br><br>Defaults to `["exp"]`.|
|`policies[].policy.mcpAuthentication.jwtValidationOptions.requiredClaims`|[]string|Claims that must be present in the token before validation.<br>Only "exp", "nbf", "aud", "iss", "sub" are enforced; others<br>(including "iat" and "jti") are ignored.<br>Defaults to ["exp"]. Use an empty list to require no claims.|
|`policies[].policy.a2a`|object|Mark this traffic as A2A to enable A2A processing and telemetry.|
|`policies[].policy.ai`|object|Mark this as LLM traffic to enable LLM processing.|
|`policies[].policy.ai.promptGuard`|object||
|`policies[].policy.ai.promptGuard.request`|[]object||
|`policies[].policy.ai.promptGuard.request[].regex`|object||
|`policies[].policy.ai.promptGuard.request[].regex.action`|string||
|`policies[].policy.ai.promptGuard.request[].regex.rules`|[]object||
|`policies[].policy.ai.promptGuard.request[].regex.rules[].builtin`|string||
|`policies[].policy.ai.promptGuard.request[].regex.rules[].pattern`|string||
|`policies[].policy.ai.promptGuard.request[].webhook`|object||
|`policies[].policy.ai.promptGuard.request[].webhook.target`|object|Exactly one of service, host, or backend may be set.|
|`policies[].policy.ai.promptGuard.request[].webhook.target.service`|object||
|`policies[].policy.ai.promptGuard.request[].webhook.target.service.name`|object||
|`policies[].policy.ai.promptGuard.request[].webhook.target.service.name.namespace`|string||
|`policies[].policy.ai.promptGuard.request[].webhook.target.service.name.hostname`|string||
|`policies[].policy.ai.promptGuard.request[].webhook.target.service.port`|integer||
|`policies[].policy.ai.promptGuard.request[].webhook.target.host`|string|Hostname or IP address|
|`policies[].policy.ai.promptGuard.request[].webhook.target.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`policies[].policy.ai.promptGuard.request[].webhook.forwardHeaderMatches`|[]object||
|`policies[].policy.ai.promptGuard.request[].webhook.forwardHeaderMatches[].name`|string||
|`policies[].policy.ai.promptGuard.request[].webhook.forwardHeaderMatches[].value`|object|Exactly one of exact or regex may be set.|
|`policies[].policy.ai.promptGuard.request[].webhook.forwardHeaderMatches[].value.exact`|string||
|`policies[].policy.ai.promptGuard.request[].webhook.forwardHeaderMatches[].value.regex`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration`|object||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.model`|string|Model to use. Defaults to `omni-moderation-latest`|
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies`|object||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.requestHeaderModifier.add`|object||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.requestHeaderModifier.set`|object||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.requestHeaderModifier.remove`|[]string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.responseHeaderModifier.add`|object||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.responseHeaderModifier.set`|object||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.responseHeaderModifier.remove`|[]string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.scheme`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.authority`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.authority.full`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.authority.host`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.authority.port`|integer||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.path`|object||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.path.full`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.path.prefix`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.status`|integer||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.transformations.request`|object||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.transformations.request.add`|object||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.transformations.request.set`|object||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.transformations.request.remove`|[]string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.transformations.request.body`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.transformations.request.metadata`|object||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.transformations.response`|object||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.transformations.response.add`|object||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.transformations.response.set`|object||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.transformations.response.remove`|[]string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.transformations.response.body`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.transformations.response.metadata`|object||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendTLS`|object|Send TLS to the backend.|
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendTLS.cert`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendTLS.key`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendTLS.root`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendTLS.hostname`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendTLS.insecure`|boolean||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendTLS.insecureHost`|boolean||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendTLS.alpn`|[]string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendTLS.subjectAltNames`|[]string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendAuth`|object|Authenticate to the backend.|
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendAuth.passthrough`|object||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendAuth.key`|object||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendAuth.key.file`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendAuth.gcp`|object||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendAuth.gcp.type`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendAuth.gcp.type`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws`|object||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws.accessKeyId`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws.secretAccessKey`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws.region`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws.sessionToken`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.developerImplicit`|object||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.implicit`|object||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.http`|object|Specify HTTP settings for the backend|
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.http.version`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.http.requestTimeout`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.tcp`|object|Specify TCP settings for the backend|
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives`|object||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives.enabled`|boolean||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives.time`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives.interval`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives.retries`|integer||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.tcp.connectTimeout`|object||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.tcp.connectTimeout.secs`|integer||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.tcp.connectTimeout.nanos`|integer||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.health.eviction.duration`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.health.eviction.restoreHealth`|number||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.health.eviction.consecutiveFailures`|integer||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.health.eviction.healthThreshold`|number||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service`|object||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service.name`|object||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service.name.namespace`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service.name.hostname`|string||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service.port`|integer||
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`policies[].policy.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails`|object|Configuration for AWS Bedrock Guardrails integration.|
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.guardrailIdentifier`|string|The unique identifier of the guardrail|
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.guardrailVersion`|string|The version of the guardrail|
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.region`|string|AWS region where the guardrail is deployed|
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies`|object|Backend policies for AWS authentication (optional, defaults to implicit AWS auth)|
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.requestHeaderModifier.add`|object||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.requestHeaderModifier.set`|object||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.requestHeaderModifier.remove`|[]string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.responseHeaderModifier.add`|object||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.responseHeaderModifier.set`|object||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.responseHeaderModifier.remove`|[]string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.scheme`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.authority`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.authority.full`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.authority.host`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.authority.port`|integer||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.path`|object||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.path.full`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.path.prefix`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.status`|integer||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request`|object||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.add`|object||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.set`|object||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.remove`|[]string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.body`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.metadata`|object||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response`|object||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.add`|object||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.set`|object||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.remove`|[]string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.body`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.metadata`|object||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS`|object|Send TLS to the backend.|
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.cert`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.key`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.root`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.hostname`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.insecure`|boolean||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.insecureHost`|boolean||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.alpn`|[]string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.subjectAltNames`|[]string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth`|object|Authenticate to the backend.|
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.passthrough`|object||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.key`|object||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.key.file`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.gcp`|object||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws`|object||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws.accessKeyId`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws.secretAccessKey`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws.region`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws.sessionToken`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.developerImplicit`|object||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.implicit`|object||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.http`|object|Specify HTTP settings for the backend|
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.http.version`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.http.requestTimeout`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.tcp`|object|Specify TCP settings for the backend|
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives`|object||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives.enabled`|boolean||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives.time`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives.interval`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives.retries`|integer||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.connectTimeout`|object||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.connectTimeout.secs`|integer||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.connectTimeout.nanos`|integer||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction.duration`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction.restoreHealth`|number||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction.consecutiveFailures`|integer||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction.healthThreshold`|number||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service`|object||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.name`|object||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.namespace`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.hostname`|string||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.port`|integer||
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`policies[].policy.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`policies[].policy.ai.promptGuard.request[].googleModelArmor`|object|Configuration for Google Cloud Model Armor integration.|
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.templateId`|string|The template ID for the Model Armor configuration|
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.projectId`|string|The GCP project ID|
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.location`|string|The GCP region (default: us-central1)|
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies`|object|Backend policies for GCP authentication (optional, defaults to implicit GCP auth)|
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.requestHeaderModifier.add`|object||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.requestHeaderModifier.set`|object||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.requestHeaderModifier.remove`|[]string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.responseHeaderModifier.add`|object||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.responseHeaderModifier.set`|object||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.responseHeaderModifier.remove`|[]string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.scheme`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.authority`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.authority.full`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.authority.host`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.authority.port`|integer||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.path`|object||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.path.full`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.path.prefix`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.status`|integer||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.transformations.request`|object||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.add`|object||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.set`|object||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.remove`|[]string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.body`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.metadata`|object||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.transformations.response`|object||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.add`|object||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.set`|object||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.remove`|[]string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.body`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.metadata`|object||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendTLS`|object|Send TLS to the backend.|
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.cert`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.key`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.root`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.hostname`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.insecure`|boolean||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.insecureHost`|boolean||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.alpn`|[]string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.subjectAltNames`|[]string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendAuth`|object|Authenticate to the backend.|
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.passthrough`|object||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.key`|object||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.key.file`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.gcp`|object||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws`|object||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws.accessKeyId`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws.secretAccessKey`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws.region`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws.sessionToken`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.developerImplicit`|object||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.implicit`|object||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.http`|object|Specify HTTP settings for the backend|
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.http.version`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.http.requestTimeout`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.tcp`|object|Specify TCP settings for the backend|
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives`|object||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives.enabled`|boolean||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives.time`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives.interval`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives.retries`|integer||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.tcp.connectTimeout`|object||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.tcp.connectTimeout.secs`|integer||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.tcp.connectTimeout.nanos`|integer||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.health.eviction.duration`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.health.eviction.restoreHealth`|number||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.health.eviction.consecutiveFailures`|integer||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.health.eviction.healthThreshold`|number||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service`|object||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service.name`|object||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service.name.namespace`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service.name.hostname`|string||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service.port`|integer||
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`policies[].policy.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`policies[].policy.ai.promptGuard.request[].rejection`|object||
|`policies[].policy.ai.promptGuard.request[].rejection.body`|array||
|`policies[].policy.ai.promptGuard.request[].rejection.status`|integer||
|`policies[].policy.ai.promptGuard.request[].rejection.headers`|object|Optional headers to add, set, or remove from the rejection response|
|`policies[].policy.ai.promptGuard.request[].rejection.headers.add`|object||
|`policies[].policy.ai.promptGuard.request[].rejection.headers.set`|object||
|`policies[].policy.ai.promptGuard.request[].rejection.headers.remove`|[]string||
|`policies[].policy.ai.promptGuard.response`|[]object||
|`policies[].policy.ai.promptGuard.response[].regex`|object||
|`policies[].policy.ai.promptGuard.response[].regex.action`|string||
|`policies[].policy.ai.promptGuard.response[].regex.rules`|[]object||
|`policies[].policy.ai.promptGuard.response[].regex.rules[].builtin`|string||
|`policies[].policy.ai.promptGuard.response[].regex.rules[].pattern`|string||
|`policies[].policy.ai.promptGuard.response[].webhook`|object||
|`policies[].policy.ai.promptGuard.response[].webhook.target`|object|Exactly one of service, host, or backend may be set.|
|`policies[].policy.ai.promptGuard.response[].webhook.target.service`|object||
|`policies[].policy.ai.promptGuard.response[].webhook.target.service.name`|object||
|`policies[].policy.ai.promptGuard.response[].webhook.target.service.name.namespace`|string||
|`policies[].policy.ai.promptGuard.response[].webhook.target.service.name.hostname`|string||
|`policies[].policy.ai.promptGuard.response[].webhook.target.service.port`|integer||
|`policies[].policy.ai.promptGuard.response[].webhook.target.host`|string|Hostname or IP address|
|`policies[].policy.ai.promptGuard.response[].webhook.target.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`policies[].policy.ai.promptGuard.response[].webhook.forwardHeaderMatches`|[]object||
|`policies[].policy.ai.promptGuard.response[].webhook.forwardHeaderMatches[].name`|string||
|`policies[].policy.ai.promptGuard.response[].webhook.forwardHeaderMatches[].value`|object|Exactly one of exact or regex may be set.|
|`policies[].policy.ai.promptGuard.response[].webhook.forwardHeaderMatches[].value.exact`|string||
|`policies[].policy.ai.promptGuard.response[].webhook.forwardHeaderMatches[].value.regex`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails`|object|Configuration for AWS Bedrock Guardrails integration.|
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.guardrailIdentifier`|string|The unique identifier of the guardrail|
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.guardrailVersion`|string|The version of the guardrail|
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.region`|string|AWS region where the guardrail is deployed|
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies`|object|Backend policies for AWS authentication (optional, defaults to implicit AWS auth)|
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.requestHeaderModifier.add`|object||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.requestHeaderModifier.set`|object||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.requestHeaderModifier.remove`|[]string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.responseHeaderModifier.add`|object||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.responseHeaderModifier.set`|object||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.responseHeaderModifier.remove`|[]string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.scheme`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.authority`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.authority.full`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.authority.host`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.authority.port`|integer||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.path`|object||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.path.full`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.path.prefix`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.status`|integer||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request`|object||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.add`|object||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.set`|object||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.remove`|[]string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.body`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.metadata`|object||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response`|object||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.add`|object||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.set`|object||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.remove`|[]string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.body`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.metadata`|object||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS`|object|Send TLS to the backend.|
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.cert`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.key`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.root`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.hostname`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.insecure`|boolean||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.insecureHost`|boolean||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.alpn`|[]string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.subjectAltNames`|[]string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth`|object|Authenticate to the backend.|
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.passthrough`|object||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.key`|object||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.key.file`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.gcp`|object||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws`|object||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws.accessKeyId`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws.secretAccessKey`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws.region`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws.sessionToken`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.developerImplicit`|object||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.implicit`|object||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.http`|object|Specify HTTP settings for the backend|
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.http.version`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.http.requestTimeout`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.tcp`|object|Specify TCP settings for the backend|
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives`|object||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives.enabled`|boolean||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives.time`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives.interval`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives.retries`|integer||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.connectTimeout`|object||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.connectTimeout.secs`|integer||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.connectTimeout.nanos`|integer||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction.duration`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction.restoreHealth`|number||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction.consecutiveFailures`|integer||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction.healthThreshold`|number||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service`|object||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.name`|object||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.namespace`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.hostname`|string||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.port`|integer||
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`policies[].policy.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`policies[].policy.ai.promptGuard.response[].googleModelArmor`|object|Configuration for Google Cloud Model Armor integration.|
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.templateId`|string|The template ID for the Model Armor configuration|
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.projectId`|string|The GCP project ID|
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.location`|string|The GCP region (default: us-central1)|
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies`|object|Backend policies for GCP authentication (optional, defaults to implicit GCP auth)|
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.requestHeaderModifier.add`|object||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.requestHeaderModifier.set`|object||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.requestHeaderModifier.remove`|[]string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.responseHeaderModifier.add`|object||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.responseHeaderModifier.set`|object||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.responseHeaderModifier.remove`|[]string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.scheme`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.authority`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.authority.full`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.authority.host`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.authority.port`|integer||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.path`|object||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.path.full`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.path.prefix`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.status`|integer||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.transformations.request`|object||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.add`|object||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.set`|object||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.remove`|[]string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.body`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.metadata`|object||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.transformations.response`|object||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.add`|object||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.set`|object||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.remove`|[]string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.body`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.metadata`|object||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendTLS`|object|Send TLS to the backend.|
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.cert`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.key`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.root`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.hostname`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.insecure`|boolean||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.insecureHost`|boolean||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.alpn`|[]string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.subjectAltNames`|[]string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendAuth`|object|Authenticate to the backend.|
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.passthrough`|object||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.key`|object||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.key.file`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.gcp`|object||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws`|object||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws.accessKeyId`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws.secretAccessKey`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws.region`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws.sessionToken`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.developerImplicit`|object||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.implicit`|object||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.http`|object|Specify HTTP settings for the backend|
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.http.version`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.http.requestTimeout`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.tcp`|object|Specify TCP settings for the backend|
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives`|object||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives.enabled`|boolean||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives.time`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives.interval`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives.retries`|integer||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.tcp.connectTimeout`|object||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.tcp.connectTimeout.secs`|integer||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.tcp.connectTimeout.nanos`|integer||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.health.eviction.duration`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.health.eviction.restoreHealth`|number||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.health.eviction.consecutiveFailures`|integer||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.health.eviction.healthThreshold`|number||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service`|object||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service.name`|object||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service.name.namespace`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service.name.hostname`|string||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service.port`|integer||
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`policies[].policy.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`policies[].policy.ai.promptGuard.response[].rejection`|object||
|`policies[].policy.ai.promptGuard.response[].rejection.body`|array||
|`policies[].policy.ai.promptGuard.response[].rejection.status`|integer||
|`policies[].policy.ai.promptGuard.response[].rejection.headers`|object|Optional headers to add, set, or remove from the rejection response|
|`policies[].policy.ai.promptGuard.response[].rejection.headers.add`|object||
|`policies[].policy.ai.promptGuard.response[].rejection.headers.set`|object||
|`policies[].policy.ai.promptGuard.response[].rejection.headers.remove`|[]string||
|`policies[].policy.ai.defaults`|object||
|`policies[].policy.ai.overrides`|object||
|`policies[].policy.ai.transformations`|object||
|`policies[].policy.ai.prompts`|object||
|`policies[].policy.ai.prompts.append`|[]object||
|`policies[].policy.ai.prompts.append[].role`|string||
|`policies[].policy.ai.prompts.append[].content`|string||
|`policies[].policy.ai.prompts.prepend`|[]object||
|`policies[].policy.ai.prompts.prepend[].role`|string||
|`policies[].policy.ai.prompts.prepend[].content`|string||
|`policies[].policy.ai.modelAliases`|object||
|`policies[].policy.ai.promptCaching`|object||
|`policies[].policy.ai.promptCaching.cacheSystem`|boolean||
|`policies[].policy.ai.promptCaching.cacheMessages`|boolean||
|`policies[].policy.ai.promptCaching.cacheTools`|boolean||
|`policies[].policy.ai.promptCaching.minTokens`|integer||
|`policies[].policy.ai.routes`|object||
|`policies[].policy.backendTLS`|object|Send TLS to the backend.|
|`policies[].policy.backendTLS.cert`|string||
|`policies[].policy.backendTLS.key`|string||
|`policies[].policy.backendTLS.root`|string||
|`policies[].policy.backendTLS.hostname`|string||
|`policies[].policy.backendTLS.insecure`|boolean||
|`policies[].policy.backendTLS.insecureHost`|boolean||
|`policies[].policy.backendTLS.alpn`|[]string||
|`policies[].policy.backendTLS.subjectAltNames`|[]string||
|`policies[].policy.backendTunnel`|object|Tunnel to the backend.|
|`policies[].policy.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`policies[].policy.backendTunnel.proxy.service`|object||
|`policies[].policy.backendTunnel.proxy.service.name`|object||
|`policies[].policy.backendTunnel.proxy.service.name.namespace`|string||
|`policies[].policy.backendTunnel.proxy.service.name.hostname`|string||
|`policies[].policy.backendTunnel.proxy.service.port`|integer||
|`policies[].policy.backendTunnel.proxy.host`|string|Hostname or IP address|
|`policies[].policy.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`policies[].policy.backendAuth`|object|Authenticate to the backend.|
|`policies[].policy.backendAuth.passthrough`|object||
|`policies[].policy.backendAuth.key`|object||
|`policies[].policy.backendAuth.key.file`|string||
|`policies[].policy.backendAuth.gcp`|object||
|`policies[].policy.backendAuth.gcp.type`|string||
|`policies[].policy.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`policies[].policy.backendAuth.gcp.type`|string||
|`policies[].policy.backendAuth.aws`|object||
|`policies[].policy.backendAuth.aws.accessKeyId`|string||
|`policies[].policy.backendAuth.aws.secretAccessKey`|string||
|`policies[].policy.backendAuth.aws.region`|string||
|`policies[].policy.backendAuth.aws.sessionToken`|string||
|`policies[].policy.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`policies[].policy.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`policies[].policy.backendAuth.azure.explicitConfig.clientSecret`|object||
|`policies[].policy.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`policies[].policy.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`policies[].policy.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`policies[].policy.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`policies[].policy.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`policies[].policy.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`policies[].policy.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`policies[].policy.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`policies[].policy.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`policies[].policy.backendAuth.azure.developerImplicit`|object||
|`policies[].policy.backendAuth.azure.implicit`|object||
|`policies[].policy.localRateLimit`|[]object|Rate limit incoming requests. State is kept local.|
|`policies[].policy.localRateLimit[].maxTokens`|integer||
|`policies[].policy.localRateLimit[].tokensPerFill`|integer||
|`policies[].policy.localRateLimit[].fillInterval`|string||
|`policies[].policy.localRateLimit[].type`|string||
|`policies[].policy.remoteRateLimit`|object|Rate limit incoming requests. State is managed by a remote server.|
|`policies[].policy.remoteRateLimit.service`|object||
|`policies[].policy.remoteRateLimit.service.name`|object||
|`policies[].policy.remoteRateLimit.service.name.namespace`|string||
|`policies[].policy.remoteRateLimit.service.name.hostname`|string||
|`policies[].policy.remoteRateLimit.service.port`|integer||
|`policies[].policy.remoteRateLimit.host`|string|Hostname or IP address|
|`policies[].policy.remoteRateLimit.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`policies[].policy.remoteRateLimit.domain`|string||
|`policies[].policy.remoteRateLimit.policies`|object|Policies to connect to the backend|
|`policies[].policy.remoteRateLimit.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`policies[].policy.remoteRateLimit.policies.requestHeaderModifier.add`|object||
|`policies[].policy.remoteRateLimit.policies.requestHeaderModifier.set`|object||
|`policies[].policy.remoteRateLimit.policies.requestHeaderModifier.remove`|[]string||
|`policies[].policy.remoteRateLimit.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`policies[].policy.remoteRateLimit.policies.responseHeaderModifier.add`|object||
|`policies[].policy.remoteRateLimit.policies.responseHeaderModifier.set`|object||
|`policies[].policy.remoteRateLimit.policies.responseHeaderModifier.remove`|[]string||
|`policies[].policy.remoteRateLimit.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`policies[].policy.remoteRateLimit.policies.requestRedirect.scheme`|string||
|`policies[].policy.remoteRateLimit.policies.requestRedirect.authority`|string||
|`policies[].policy.remoteRateLimit.policies.requestRedirect.authority.full`|string||
|`policies[].policy.remoteRateLimit.policies.requestRedirect.authority.host`|string||
|`policies[].policy.remoteRateLimit.policies.requestRedirect.authority.port`|integer||
|`policies[].policy.remoteRateLimit.policies.requestRedirect.path`|object||
|`policies[].policy.remoteRateLimit.policies.requestRedirect.path.full`|string||
|`policies[].policy.remoteRateLimit.policies.requestRedirect.path.prefix`|string||
|`policies[].policy.remoteRateLimit.policies.requestRedirect.status`|integer||
|`policies[].policy.remoteRateLimit.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`policies[].policy.remoteRateLimit.policies.transformations.request`|object||
|`policies[].policy.remoteRateLimit.policies.transformations.request.add`|object||
|`policies[].policy.remoteRateLimit.policies.transformations.request.set`|object||
|`policies[].policy.remoteRateLimit.policies.transformations.request.remove`|[]string||
|`policies[].policy.remoteRateLimit.policies.transformations.request.body`|string||
|`policies[].policy.remoteRateLimit.policies.transformations.request.metadata`|object||
|`policies[].policy.remoteRateLimit.policies.transformations.response`|object||
|`policies[].policy.remoteRateLimit.policies.transformations.response.add`|object||
|`policies[].policy.remoteRateLimit.policies.transformations.response.set`|object||
|`policies[].policy.remoteRateLimit.policies.transformations.response.remove`|[]string||
|`policies[].policy.remoteRateLimit.policies.transformations.response.body`|string||
|`policies[].policy.remoteRateLimit.policies.transformations.response.metadata`|object||
|`policies[].policy.remoteRateLimit.policies.backendTLS`|object|Send TLS to the backend.|
|`policies[].policy.remoteRateLimit.policies.backendTLS.cert`|string||
|`policies[].policy.remoteRateLimit.policies.backendTLS.key`|string||
|`policies[].policy.remoteRateLimit.policies.backendTLS.root`|string||
|`policies[].policy.remoteRateLimit.policies.backendTLS.hostname`|string||
|`policies[].policy.remoteRateLimit.policies.backendTLS.insecure`|boolean||
|`policies[].policy.remoteRateLimit.policies.backendTLS.insecureHost`|boolean||
|`policies[].policy.remoteRateLimit.policies.backendTLS.alpn`|[]string||
|`policies[].policy.remoteRateLimit.policies.backendTLS.subjectAltNames`|[]string||
|`policies[].policy.remoteRateLimit.policies.backendAuth`|object|Authenticate to the backend.|
|`policies[].policy.remoteRateLimit.policies.backendAuth.passthrough`|object||
|`policies[].policy.remoteRateLimit.policies.backendAuth.key`|object||
|`policies[].policy.remoteRateLimit.policies.backendAuth.key.file`|string||
|`policies[].policy.remoteRateLimit.policies.backendAuth.gcp`|object||
|`policies[].policy.remoteRateLimit.policies.backendAuth.gcp.type`|string||
|`policies[].policy.remoteRateLimit.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`policies[].policy.remoteRateLimit.policies.backendAuth.gcp.type`|string||
|`policies[].policy.remoteRateLimit.policies.backendAuth.aws`|object||
|`policies[].policy.remoteRateLimit.policies.backendAuth.aws.accessKeyId`|string||
|`policies[].policy.remoteRateLimit.policies.backendAuth.aws.secretAccessKey`|string||
|`policies[].policy.remoteRateLimit.policies.backendAuth.aws.region`|string||
|`policies[].policy.remoteRateLimit.policies.backendAuth.aws.sessionToken`|string||
|`policies[].policy.remoteRateLimit.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`policies[].policy.remoteRateLimit.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`policies[].policy.remoteRateLimit.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`policies[].policy.remoteRateLimit.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`policies[].policy.remoteRateLimit.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`policies[].policy.remoteRateLimit.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`policies[].policy.remoteRateLimit.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`policies[].policy.remoteRateLimit.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`policies[].policy.remoteRateLimit.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`policies[].policy.remoteRateLimit.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`policies[].policy.remoteRateLimit.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`policies[].policy.remoteRateLimit.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`policies[].policy.remoteRateLimit.policies.backendAuth.azure.developerImplicit`|object||
|`policies[].policy.remoteRateLimit.policies.backendAuth.azure.implicit`|object||
|`policies[].policy.remoteRateLimit.policies.http`|object|Specify HTTP settings for the backend|
|`policies[].policy.remoteRateLimit.policies.http.version`|string||
|`policies[].policy.remoteRateLimit.policies.http.requestTimeout`|string||
|`policies[].policy.remoteRateLimit.policies.tcp`|object|Specify TCP settings for the backend|
|`policies[].policy.remoteRateLimit.policies.tcp.keepalives`|object||
|`policies[].policy.remoteRateLimit.policies.tcp.keepalives.enabled`|boolean||
|`policies[].policy.remoteRateLimit.policies.tcp.keepalives.time`|string||
|`policies[].policy.remoteRateLimit.policies.tcp.keepalives.interval`|string||
|`policies[].policy.remoteRateLimit.policies.tcp.keepalives.retries`|integer||
|`policies[].policy.remoteRateLimit.policies.tcp.connectTimeout`|object||
|`policies[].policy.remoteRateLimit.policies.tcp.connectTimeout.secs`|integer||
|`policies[].policy.remoteRateLimit.policies.tcp.connectTimeout.nanos`|integer||
|`policies[].policy.remoteRateLimit.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`policies[].policy.remoteRateLimit.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`policies[].policy.remoteRateLimit.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`policies[].policy.remoteRateLimit.policies.health.eviction.duration`|string||
|`policies[].policy.remoteRateLimit.policies.health.eviction.restoreHealth`|number||
|`policies[].policy.remoteRateLimit.policies.health.eviction.consecutiveFailures`|integer||
|`policies[].policy.remoteRateLimit.policies.health.eviction.healthThreshold`|number||
|`policies[].policy.remoteRateLimit.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`policies[].policy.remoteRateLimit.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`policies[].policy.remoteRateLimit.policies.backendTunnel.proxy.service`|object||
|`policies[].policy.remoteRateLimit.policies.backendTunnel.proxy.service.name`|object||
|`policies[].policy.remoteRateLimit.policies.backendTunnel.proxy.service.name.namespace`|string||
|`policies[].policy.remoteRateLimit.policies.backendTunnel.proxy.service.name.hostname`|string||
|`policies[].policy.remoteRateLimit.policies.backendTunnel.proxy.service.port`|integer||
|`policies[].policy.remoteRateLimit.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`policies[].policy.remoteRateLimit.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`policies[].policy.remoteRateLimit.descriptors`|[]object||
|`policies[].policy.remoteRateLimit.descriptors[].entries`|[]object||
|`policies[].policy.remoteRateLimit.descriptors[].entries[].key`|string||
|`policies[].policy.remoteRateLimit.descriptors[].entries[].value`|string||
|`policies[].policy.remoteRateLimit.descriptors[].type`|string||
|`policies[].policy.remoteRateLimit.failureMode`|string|Behavior when the remote rate limit service is unavailable or returns an error.<br>Defaults to failClosed, denying requests with a 500 status on service failure.|
|`policies[].policy.jwtAuth`|object|Authenticate incoming JWT requests.|
|`policies[].policy.jwtAuth.mode`|string||
|`policies[].policy.jwtAuth.providers`|[]object||
|`policies[].policy.jwtAuth.providers[].issuer`|string||
|`policies[].policy.jwtAuth.providers[].audiences`|[]string||
|`policies[].policy.jwtAuth.providers[].jwks`|object||
|`policies[].policy.jwtAuth.providers[].jwks.file`|string||
|`policies[].policy.jwtAuth.providers[].jwks.url`|string||
|`policies[].policy.jwtAuth.providers[].jwtValidationOptions`|object|JWT validation options controlling which claims must be present in a token.<br><br>The `required_claims` set specifies which RFC 7519 registered claims must<br>exist in the token payload before validation proceeds. Only the following<br>values are recognized: `exp`, `nbf`, `aud`, `iss`, `sub`. Other registered<br>claims such as `iat` and `jti` are **not** enforced by the underlying<br>`jsonwebtoken` library and will be silently ignored.<br><br>This only enforces **presence**. Standard claims like `exp` and `nbf`<br>have their values validated independently (e.g., expiry is always checked<br>when the `exp` claim is present, regardless of this setting).<br><br>Defaults to `["exp"]`.|
|`policies[].policy.jwtAuth.providers[].jwtValidationOptions.requiredClaims`|[]string|Claims that must be present in the token before validation.<br>Only "exp", "nbf", "aud", "iss", "sub" are enforced; others<br>(including "iat" and "jti") are ignored.<br>Defaults to ["exp"]. Use an empty list to require no claims.|
|`policies[].policy.jwtAuth.mode`|string||
|`policies[].policy.jwtAuth.issuer`|string||
|`policies[].policy.jwtAuth.audiences`|[]string||
|`policies[].policy.jwtAuth.jwks`|object||
|`policies[].policy.jwtAuth.jwks.file`|string||
|`policies[].policy.jwtAuth.jwks.url`|string||
|`policies[].policy.jwtAuth.jwtValidationOptions`|object|JWT validation options controlling which claims must be present in a token.<br><br>The `required_claims` set specifies which RFC 7519 registered claims must<br>exist in the token payload before validation proceeds. Only the following<br>values are recognized: `exp`, `nbf`, `aud`, `iss`, `sub`. Other registered<br>claims such as `iat` and `jti` are **not** enforced by the underlying<br>`jsonwebtoken` library and will be silently ignored.<br><br>This only enforces **presence**. Standard claims like `exp` and `nbf`<br>have their values validated independently (e.g., expiry is always checked<br>when the `exp` claim is present, regardless of this setting).<br><br>Defaults to `["exp"]`.|
|`policies[].policy.jwtAuth.jwtValidationOptions.requiredClaims`|[]string|Claims that must be present in the token before validation.<br>Only "exp", "nbf", "aud", "iss", "sub" are enforced; others<br>(including "iat" and "jti") are ignored.<br>Defaults to ["exp"]. Use an empty list to require no claims.|
|`policies[].policy.oidc`|object|Authenticate incoming browser requests with OIDC authorization code flow.|
|`policies[].policy.oidc.issuer`|string|Issuer used for discovery and ID token validation.|
|`policies[].policy.oidc.discovery`|object|Optional discovery document override. If omitted, discovery uses<br>`${issuer}/.well-known/openid-configuration`.|
|`policies[].policy.oidc.discovery.file`|string||
|`policies[].policy.oidc.discovery.url`|string||
|`policies[].policy.oidc.authorizationEndpoint`|string|Authorization endpoint used to start the browser login flow.|
|`policies[].policy.oidc.tokenEndpoint`|string|Token endpoint used to exchange the authorization code.|
|`policies[].policy.oidc.tokenEndpointAuth`|string|Token endpoint client authentication method for explicit provider configuration.<br><br>Discovery mode derives this from provider metadata. Explicit mode defaults to<br>`clientSecretBasic` when omitted.|
|`policies[].policy.oidc.jwks`|object|JWKS source used to validate returned ID tokens.|
|`policies[].policy.oidc.jwks.file`|string||
|`policies[].policy.oidc.jwks.url`|string||
|`policies[].policy.oidc.clientId`|string|OAuth2 client identifier used for authorization and token exchange.|
|`policies[].policy.oidc.clientSecret`|string|OAuth2 client secret used for token exchange.|
|`policies[].policy.oidc.redirectURI`|string|Absolute callback URI handled by the gateway.<br>This policy always redirects unauthenticated non-callback requests back through this login<br>flow.|
|`policies[].policy.oidc.scopes`|[]string|Additional OAuth2 scopes to request. `openid` is always included.|
|`policies[].policy.basicAuth`|object|Authenticate incoming requests using Basic Authentication with htpasswd.|
|`policies[].policy.basicAuth.htpasswd`|object|.htpasswd file contents/reference|
|`policies[].policy.basicAuth.htpasswd.file`|string||
|`policies[].policy.basicAuth.realm`|string|Realm name for the WWW-Authenticate header|
|`policies[].policy.basicAuth.mode`|string|Validation mode for basic authentication|
|`policies[].policy.apiKey`|object|Authenticate incoming requests using API Keys|
|`policies[].policy.apiKey.keys`|[]object|List of API keys|
|`policies[].policy.apiKey.keys[].key`|string||
|`policies[].policy.apiKey.keys[].metadata`|any||
|`policies[].policy.apiKey.mode`|string|Validation mode for API keys|
|`policies[].policy.extAuthz`|object|Authenticate incoming requests by calling an external authorization server.|
|`policies[].policy.extAuthz.service`|object||
|`policies[].policy.extAuthz.service.name`|object||
|`policies[].policy.extAuthz.service.name.namespace`|string||
|`policies[].policy.extAuthz.service.name.hostname`|string||
|`policies[].policy.extAuthz.service.port`|integer||
|`policies[].policy.extAuthz.host`|string|Hostname or IP address|
|`policies[].policy.extAuthz.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`policies[].policy.extAuthz.policies`|object|Policies to connect to the backend|
|`policies[].policy.extAuthz.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`policies[].policy.extAuthz.policies.requestHeaderModifier.add`|object||
|`policies[].policy.extAuthz.policies.requestHeaderModifier.set`|object||
|`policies[].policy.extAuthz.policies.requestHeaderModifier.remove`|[]string||
|`policies[].policy.extAuthz.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`policies[].policy.extAuthz.policies.responseHeaderModifier.add`|object||
|`policies[].policy.extAuthz.policies.responseHeaderModifier.set`|object||
|`policies[].policy.extAuthz.policies.responseHeaderModifier.remove`|[]string||
|`policies[].policy.extAuthz.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`policies[].policy.extAuthz.policies.requestRedirect.scheme`|string||
|`policies[].policy.extAuthz.policies.requestRedirect.authority`|string||
|`policies[].policy.extAuthz.policies.requestRedirect.authority.full`|string||
|`policies[].policy.extAuthz.policies.requestRedirect.authority.host`|string||
|`policies[].policy.extAuthz.policies.requestRedirect.authority.port`|integer||
|`policies[].policy.extAuthz.policies.requestRedirect.path`|object||
|`policies[].policy.extAuthz.policies.requestRedirect.path.full`|string||
|`policies[].policy.extAuthz.policies.requestRedirect.path.prefix`|string||
|`policies[].policy.extAuthz.policies.requestRedirect.status`|integer||
|`policies[].policy.extAuthz.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`policies[].policy.extAuthz.policies.transformations.request`|object||
|`policies[].policy.extAuthz.policies.transformations.request.add`|object||
|`policies[].policy.extAuthz.policies.transformations.request.set`|object||
|`policies[].policy.extAuthz.policies.transformations.request.remove`|[]string||
|`policies[].policy.extAuthz.policies.transformations.request.body`|string||
|`policies[].policy.extAuthz.policies.transformations.request.metadata`|object||
|`policies[].policy.extAuthz.policies.transformations.response`|object||
|`policies[].policy.extAuthz.policies.transformations.response.add`|object||
|`policies[].policy.extAuthz.policies.transformations.response.set`|object||
|`policies[].policy.extAuthz.policies.transformations.response.remove`|[]string||
|`policies[].policy.extAuthz.policies.transformations.response.body`|string||
|`policies[].policy.extAuthz.policies.transformations.response.metadata`|object||
|`policies[].policy.extAuthz.policies.backendTLS`|object|Send TLS to the backend.|
|`policies[].policy.extAuthz.policies.backendTLS.cert`|string||
|`policies[].policy.extAuthz.policies.backendTLS.key`|string||
|`policies[].policy.extAuthz.policies.backendTLS.root`|string||
|`policies[].policy.extAuthz.policies.backendTLS.hostname`|string||
|`policies[].policy.extAuthz.policies.backendTLS.insecure`|boolean||
|`policies[].policy.extAuthz.policies.backendTLS.insecureHost`|boolean||
|`policies[].policy.extAuthz.policies.backendTLS.alpn`|[]string||
|`policies[].policy.extAuthz.policies.backendTLS.subjectAltNames`|[]string||
|`policies[].policy.extAuthz.policies.backendAuth`|object|Authenticate to the backend.|
|`policies[].policy.extAuthz.policies.backendAuth.passthrough`|object||
|`policies[].policy.extAuthz.policies.backendAuth.key`|object||
|`policies[].policy.extAuthz.policies.backendAuth.key.file`|string||
|`policies[].policy.extAuthz.policies.backendAuth.gcp`|object||
|`policies[].policy.extAuthz.policies.backendAuth.gcp.type`|string||
|`policies[].policy.extAuthz.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`policies[].policy.extAuthz.policies.backendAuth.gcp.type`|string||
|`policies[].policy.extAuthz.policies.backendAuth.aws`|object||
|`policies[].policy.extAuthz.policies.backendAuth.aws.accessKeyId`|string||
|`policies[].policy.extAuthz.policies.backendAuth.aws.secretAccessKey`|string||
|`policies[].policy.extAuthz.policies.backendAuth.aws.region`|string||
|`policies[].policy.extAuthz.policies.backendAuth.aws.sessionToken`|string||
|`policies[].policy.extAuthz.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`policies[].policy.extAuthz.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`policies[].policy.extAuthz.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`policies[].policy.extAuthz.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`policies[].policy.extAuthz.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`policies[].policy.extAuthz.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`policies[].policy.extAuthz.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`policies[].policy.extAuthz.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`policies[].policy.extAuthz.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`policies[].policy.extAuthz.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`policies[].policy.extAuthz.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`policies[].policy.extAuthz.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`policies[].policy.extAuthz.policies.backendAuth.azure.developerImplicit`|object||
|`policies[].policy.extAuthz.policies.backendAuth.azure.implicit`|object||
|`policies[].policy.extAuthz.policies.http`|object|Specify HTTP settings for the backend|
|`policies[].policy.extAuthz.policies.http.version`|string||
|`policies[].policy.extAuthz.policies.http.requestTimeout`|string||
|`policies[].policy.extAuthz.policies.tcp`|object|Specify TCP settings for the backend|
|`policies[].policy.extAuthz.policies.tcp.keepalives`|object||
|`policies[].policy.extAuthz.policies.tcp.keepalives.enabled`|boolean||
|`policies[].policy.extAuthz.policies.tcp.keepalives.time`|string||
|`policies[].policy.extAuthz.policies.tcp.keepalives.interval`|string||
|`policies[].policy.extAuthz.policies.tcp.keepalives.retries`|integer||
|`policies[].policy.extAuthz.policies.tcp.connectTimeout`|object||
|`policies[].policy.extAuthz.policies.tcp.connectTimeout.secs`|integer||
|`policies[].policy.extAuthz.policies.tcp.connectTimeout.nanos`|integer||
|`policies[].policy.extAuthz.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`policies[].policy.extAuthz.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`policies[].policy.extAuthz.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`policies[].policy.extAuthz.policies.health.eviction.duration`|string||
|`policies[].policy.extAuthz.policies.health.eviction.restoreHealth`|number||
|`policies[].policy.extAuthz.policies.health.eviction.consecutiveFailures`|integer||
|`policies[].policy.extAuthz.policies.health.eviction.healthThreshold`|number||
|`policies[].policy.extAuthz.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`policies[].policy.extAuthz.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`policies[].policy.extAuthz.policies.backendTunnel.proxy.service`|object||
|`policies[].policy.extAuthz.policies.backendTunnel.proxy.service.name`|object||
|`policies[].policy.extAuthz.policies.backendTunnel.proxy.service.name.namespace`|string||
|`policies[].policy.extAuthz.policies.backendTunnel.proxy.service.name.hostname`|string||
|`policies[].policy.extAuthz.policies.backendTunnel.proxy.service.port`|integer||
|`policies[].policy.extAuthz.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`policies[].policy.extAuthz.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`policies[].policy.extAuthz.protocol`|object|The ext_authz protocol to use. Unless you need to integrate with an HTTP-only server, gRPC is recommended.<br>Exactly one of grpc or http may be set.|
|`policies[].policy.extAuthz.protocol.grpc`|object||
|`policies[].policy.extAuthz.protocol.grpc.context`|object|Additional context to send to the authorization service.<br>This maps to the `context_extensions` field of the request, and only allows static values.|
|`policies[].policy.extAuthz.protocol.grpc.metadata`|object|Additional metadata to send to the authorization service.<br>This maps to the `metadata_context.filter_metadata` field of the request, and allows dynamic CEL expressions.<br>If unset, by default the `envoy.filters.http.jwt_authn` key is set if the JWT policy is used as well, for compatibility.|
|`policies[].policy.extAuthz.protocol.http`|object||
|`policies[].policy.extAuthz.protocol.http.path`|string||
|`policies[].policy.extAuthz.protocol.http.redirect`|string|When using the HTTP protocol, and the server returns unauthorized, redirect to the URL resolved by<br>the provided expression rather than directly returning the error.|
|`policies[].policy.extAuthz.protocol.http.includeResponseHeaders`|[]string|Specific headers from the authorization response will be copied into the request to the backend.|
|`policies[].policy.extAuthz.protocol.http.addRequestHeaders`|object|Specific headers to add in the authorization request (empty = all headers), based on the expression|
|`policies[].policy.extAuthz.protocol.http.metadata`|object|Metadata to include under the `extauthz` variable, based on the authorization response.|
|`policies[].policy.extAuthz.failureMode`|string|Behavior when the authorization service is unavailable or returns an error|
|`policies[].policy.extAuthz.failureMode.denyWithStatus`|integer||
|`policies[].policy.extAuthz.includeRequestHeaders`|[]string|Specific headers to include in the authorization request.<br>If unset, the gRPC protocol sends all request headers. The HTTP protocol sends only 'Authorization'.|
|`policies[].policy.extAuthz.includeRequestBody`|object|Options for including the request body in the authorization request|
|`policies[].policy.extAuthz.includeRequestBody.maxRequestBytes`|integer|Maximum size of request body to buffer (default: 8192)|
|`policies[].policy.extAuthz.includeRequestBody.allowPartialMessage`|boolean|If true, send partial body when max_request_bytes is reached|
|`policies[].policy.extAuthz.includeRequestBody.packAsBytes`|boolean|If true, pack body as raw bytes in gRPC|
|`policies[].policy.extProc`|object|Extend agentgateway with an external processor|
|`policies[].policy.extProc.service`|object||
|`policies[].policy.extProc.service.name`|object||
|`policies[].policy.extProc.service.name.namespace`|string||
|`policies[].policy.extProc.service.name.hostname`|string||
|`policies[].policy.extProc.service.port`|integer||
|`policies[].policy.extProc.host`|string|Hostname or IP address|
|`policies[].policy.extProc.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`policies[].policy.extProc.policies`|object|Policies to connect to the backend|
|`policies[].policy.extProc.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`policies[].policy.extProc.policies.requestHeaderModifier.add`|object||
|`policies[].policy.extProc.policies.requestHeaderModifier.set`|object||
|`policies[].policy.extProc.policies.requestHeaderModifier.remove`|[]string||
|`policies[].policy.extProc.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`policies[].policy.extProc.policies.responseHeaderModifier.add`|object||
|`policies[].policy.extProc.policies.responseHeaderModifier.set`|object||
|`policies[].policy.extProc.policies.responseHeaderModifier.remove`|[]string||
|`policies[].policy.extProc.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`policies[].policy.extProc.policies.requestRedirect.scheme`|string||
|`policies[].policy.extProc.policies.requestRedirect.authority`|string||
|`policies[].policy.extProc.policies.requestRedirect.authority.full`|string||
|`policies[].policy.extProc.policies.requestRedirect.authority.host`|string||
|`policies[].policy.extProc.policies.requestRedirect.authority.port`|integer||
|`policies[].policy.extProc.policies.requestRedirect.path`|object||
|`policies[].policy.extProc.policies.requestRedirect.path.full`|string||
|`policies[].policy.extProc.policies.requestRedirect.path.prefix`|string||
|`policies[].policy.extProc.policies.requestRedirect.status`|integer||
|`policies[].policy.extProc.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`policies[].policy.extProc.policies.transformations.request`|object||
|`policies[].policy.extProc.policies.transformations.request.add`|object||
|`policies[].policy.extProc.policies.transformations.request.set`|object||
|`policies[].policy.extProc.policies.transformations.request.remove`|[]string||
|`policies[].policy.extProc.policies.transformations.request.body`|string||
|`policies[].policy.extProc.policies.transformations.request.metadata`|object||
|`policies[].policy.extProc.policies.transformations.response`|object||
|`policies[].policy.extProc.policies.transformations.response.add`|object||
|`policies[].policy.extProc.policies.transformations.response.set`|object||
|`policies[].policy.extProc.policies.transformations.response.remove`|[]string||
|`policies[].policy.extProc.policies.transformations.response.body`|string||
|`policies[].policy.extProc.policies.transformations.response.metadata`|object||
|`policies[].policy.extProc.policies.backendTLS`|object|Send TLS to the backend.|
|`policies[].policy.extProc.policies.backendTLS.cert`|string||
|`policies[].policy.extProc.policies.backendTLS.key`|string||
|`policies[].policy.extProc.policies.backendTLS.root`|string||
|`policies[].policy.extProc.policies.backendTLS.hostname`|string||
|`policies[].policy.extProc.policies.backendTLS.insecure`|boolean||
|`policies[].policy.extProc.policies.backendTLS.insecureHost`|boolean||
|`policies[].policy.extProc.policies.backendTLS.alpn`|[]string||
|`policies[].policy.extProc.policies.backendTLS.subjectAltNames`|[]string||
|`policies[].policy.extProc.policies.backendAuth`|object|Authenticate to the backend.|
|`policies[].policy.extProc.policies.backendAuth.passthrough`|object||
|`policies[].policy.extProc.policies.backendAuth.key`|object||
|`policies[].policy.extProc.policies.backendAuth.key.file`|string||
|`policies[].policy.extProc.policies.backendAuth.gcp`|object||
|`policies[].policy.extProc.policies.backendAuth.gcp.type`|string||
|`policies[].policy.extProc.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`policies[].policy.extProc.policies.backendAuth.gcp.type`|string||
|`policies[].policy.extProc.policies.backendAuth.aws`|object||
|`policies[].policy.extProc.policies.backendAuth.aws.accessKeyId`|string||
|`policies[].policy.extProc.policies.backendAuth.aws.secretAccessKey`|string||
|`policies[].policy.extProc.policies.backendAuth.aws.region`|string||
|`policies[].policy.extProc.policies.backendAuth.aws.sessionToken`|string||
|`policies[].policy.extProc.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`policies[].policy.extProc.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`policies[].policy.extProc.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`policies[].policy.extProc.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`policies[].policy.extProc.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`policies[].policy.extProc.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`policies[].policy.extProc.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`policies[].policy.extProc.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`policies[].policy.extProc.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`policies[].policy.extProc.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`policies[].policy.extProc.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`policies[].policy.extProc.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`policies[].policy.extProc.policies.backendAuth.azure.developerImplicit`|object||
|`policies[].policy.extProc.policies.backendAuth.azure.implicit`|object||
|`policies[].policy.extProc.policies.http`|object|Specify HTTP settings for the backend|
|`policies[].policy.extProc.policies.http.version`|string||
|`policies[].policy.extProc.policies.http.requestTimeout`|string||
|`policies[].policy.extProc.policies.tcp`|object|Specify TCP settings for the backend|
|`policies[].policy.extProc.policies.tcp.keepalives`|object||
|`policies[].policy.extProc.policies.tcp.keepalives.enabled`|boolean||
|`policies[].policy.extProc.policies.tcp.keepalives.time`|string||
|`policies[].policy.extProc.policies.tcp.keepalives.interval`|string||
|`policies[].policy.extProc.policies.tcp.keepalives.retries`|integer||
|`policies[].policy.extProc.policies.tcp.connectTimeout`|object||
|`policies[].policy.extProc.policies.tcp.connectTimeout.secs`|integer||
|`policies[].policy.extProc.policies.tcp.connectTimeout.nanos`|integer||
|`policies[].policy.extProc.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`policies[].policy.extProc.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`policies[].policy.extProc.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`policies[].policy.extProc.policies.health.eviction.duration`|string||
|`policies[].policy.extProc.policies.health.eviction.restoreHealth`|number||
|`policies[].policy.extProc.policies.health.eviction.consecutiveFailures`|integer||
|`policies[].policy.extProc.policies.health.eviction.healthThreshold`|number||
|`policies[].policy.extProc.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`policies[].policy.extProc.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`policies[].policy.extProc.policies.backendTunnel.proxy.service`|object||
|`policies[].policy.extProc.policies.backendTunnel.proxy.service.name`|object||
|`policies[].policy.extProc.policies.backendTunnel.proxy.service.name.namespace`|string||
|`policies[].policy.extProc.policies.backendTunnel.proxy.service.name.hostname`|string||
|`policies[].policy.extProc.policies.backendTunnel.proxy.service.port`|integer||
|`policies[].policy.extProc.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`policies[].policy.extProc.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`policies[].policy.extProc.failureMode`|string|Behavior when the ext_proc service is unavailable or returns an error|
|`policies[].policy.extProc.metadataContext`|object|Additional metadata to send to the external processing service.<br>Maps to the `metadata_context.filter_metadata` field in ProcessingRequest, and allows dynamic CEL expressions.|
|`policies[].policy.extProc.requestAttributes`|object|Maps to the request `attributes` field in ProcessingRequest, and allows dynamic CEL expressions.|
|`policies[].policy.extProc.responseAttributes`|object|Maps to the response `attributes` field in ProcessingRequest, and allows dynamic CEL expressions.|
|`policies[].policy.transformations`|object|Modify requests and responses|
|`policies[].policy.transformations.request`|object||
|`policies[].policy.transformations.request.add`|object||
|`policies[].policy.transformations.request.set`|object||
|`policies[].policy.transformations.request.remove`|[]string||
|`policies[].policy.transformations.request.body`|string||
|`policies[].policy.transformations.request.metadata`|object||
|`policies[].policy.transformations.response`|object||
|`policies[].policy.transformations.response.add`|object||
|`policies[].policy.transformations.response.set`|object||
|`policies[].policy.transformations.response.remove`|[]string||
|`policies[].policy.transformations.response.body`|string||
|`policies[].policy.transformations.response.metadata`|object||
|`policies[].policy.csrf`|object|Handle CSRF protection by validating request origins against configured allowed origins.|
|`policies[].policy.csrf.additionalOrigins`|[]string||
|`policies[].policy.timeout`|object|Timeout requests that exceed the configured duration.|
|`policies[].policy.timeout.requestTimeout`|string||
|`policies[].policy.timeout.backendRequestTimeout`|string||
|`policies[].policy.retry`|object|Retry matching requests.|
|`policies[].policy.retry.attempts`|integer||
|`policies[].policy.retry.backoff`|string||
|`policies[].policy.retry.codes`|[]integer||
|`workloads`|any||
|`services`|any||
|`backends`|[]object||
|`backends[].name`|string||
|`backends[].host`|string||
|`backends[].policies`|object||
|`backends[].policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`backends[].policies.requestHeaderModifier.add`|object||
|`backends[].policies.requestHeaderModifier.set`|object||
|`backends[].policies.requestHeaderModifier.remove`|[]string||
|`backends[].policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`backends[].policies.responseHeaderModifier.add`|object||
|`backends[].policies.responseHeaderModifier.set`|object||
|`backends[].policies.responseHeaderModifier.remove`|[]string||
|`backends[].policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`backends[].policies.requestRedirect.scheme`|string||
|`backends[].policies.requestRedirect.authority`|string||
|`backends[].policies.requestRedirect.authority.full`|string||
|`backends[].policies.requestRedirect.authority.host`|string||
|`backends[].policies.requestRedirect.authority.port`|integer||
|`backends[].policies.requestRedirect.path`|object||
|`backends[].policies.requestRedirect.path.full`|string||
|`backends[].policies.requestRedirect.path.prefix`|string||
|`backends[].policies.requestRedirect.status`|integer||
|`backends[].policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`backends[].policies.transformations.request`|object||
|`backends[].policies.transformations.request.add`|object||
|`backends[].policies.transformations.request.set`|object||
|`backends[].policies.transformations.request.remove`|[]string||
|`backends[].policies.transformations.request.body`|string||
|`backends[].policies.transformations.request.metadata`|object||
|`backends[].policies.transformations.response`|object||
|`backends[].policies.transformations.response.add`|object||
|`backends[].policies.transformations.response.set`|object||
|`backends[].policies.transformations.response.remove`|[]string||
|`backends[].policies.transformations.response.body`|string||
|`backends[].policies.transformations.response.metadata`|object||
|`backends[].policies.backendTLS`|object|Send TLS to the backend.|
|`backends[].policies.backendTLS.cert`|string||
|`backends[].policies.backendTLS.key`|string||
|`backends[].policies.backendTLS.root`|string||
|`backends[].policies.backendTLS.hostname`|string||
|`backends[].policies.backendTLS.insecure`|boolean||
|`backends[].policies.backendTLS.insecureHost`|boolean||
|`backends[].policies.backendTLS.alpn`|[]string||
|`backends[].policies.backendTLS.subjectAltNames`|[]string||
|`backends[].policies.backendAuth`|object|Authenticate to the backend.|
|`backends[].policies.backendAuth.passthrough`|object||
|`backends[].policies.backendAuth.key`|object||
|`backends[].policies.backendAuth.key.file`|string||
|`backends[].policies.backendAuth.gcp`|object||
|`backends[].policies.backendAuth.gcp.type`|string||
|`backends[].policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`backends[].policies.backendAuth.gcp.type`|string||
|`backends[].policies.backendAuth.aws`|object||
|`backends[].policies.backendAuth.aws.accessKeyId`|string||
|`backends[].policies.backendAuth.aws.secretAccessKey`|string||
|`backends[].policies.backendAuth.aws.region`|string||
|`backends[].policies.backendAuth.aws.sessionToken`|string||
|`backends[].policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`backends[].policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`backends[].policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`backends[].policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`backends[].policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`backends[].policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`backends[].policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`backends[].policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`backends[].policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`backends[].policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`backends[].policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`backends[].policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`backends[].policies.backendAuth.azure.developerImplicit`|object||
|`backends[].policies.backendAuth.azure.implicit`|object||
|`backends[].policies.http`|object|Specify HTTP settings for the backend|
|`backends[].policies.http.version`|string||
|`backends[].policies.http.requestTimeout`|string||
|`backends[].policies.tcp`|object|Specify TCP settings for the backend|
|`backends[].policies.tcp.keepalives`|object||
|`backends[].policies.tcp.keepalives.enabled`|boolean||
|`backends[].policies.tcp.keepalives.time`|string||
|`backends[].policies.tcp.keepalives.interval`|string||
|`backends[].policies.tcp.keepalives.retries`|integer||
|`backends[].policies.tcp.connectTimeout`|object||
|`backends[].policies.tcp.connectTimeout.secs`|integer||
|`backends[].policies.tcp.connectTimeout.nanos`|integer||
|`backends[].policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`backends[].policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`backends[].policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`backends[].policies.health.eviction.duration`|string||
|`backends[].policies.health.eviction.restoreHealth`|number||
|`backends[].policies.health.eviction.consecutiveFailures`|integer||
|`backends[].policies.health.eviction.healthThreshold`|number||
|`backends[].policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`backends[].policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`backends[].policies.backendTunnel.proxy.service`|object||
|`backends[].policies.backendTunnel.proxy.service.name`|object||
|`backends[].policies.backendTunnel.proxy.service.name.namespace`|string||
|`backends[].policies.backendTunnel.proxy.service.name.hostname`|string||
|`backends[].policies.backendTunnel.proxy.service.port`|integer||
|`backends[].policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`backends[].policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`backends[].policies.mcpAuthorization`|object|Authorization policies for MCP access.|
|`backends[].policies.mcpAuthorization.rules`|[]string||
|`backends[].policies.a2a`|object|Mark this traffic as A2A to enable A2A processing and telemetry.|
|`backends[].policies.ai`|object|Mark this as LLM traffic to enable LLM processing.|
|`backends[].policies.ai.promptGuard`|object||
|`backends[].policies.ai.promptGuard.request`|[]object||
|`backends[].policies.ai.promptGuard.request[].regex`|object||
|`backends[].policies.ai.promptGuard.request[].regex.action`|string||
|`backends[].policies.ai.promptGuard.request[].regex.rules`|[]object||
|`backends[].policies.ai.promptGuard.request[].regex.rules[].builtin`|string||
|`backends[].policies.ai.promptGuard.request[].regex.rules[].pattern`|string||
|`backends[].policies.ai.promptGuard.request[].webhook`|object||
|`backends[].policies.ai.promptGuard.request[].webhook.target`|object|Exactly one of service, host, or backend may be set.|
|`backends[].policies.ai.promptGuard.request[].webhook.target.service`|object||
|`backends[].policies.ai.promptGuard.request[].webhook.target.service.name`|object||
|`backends[].policies.ai.promptGuard.request[].webhook.target.service.name.namespace`|string||
|`backends[].policies.ai.promptGuard.request[].webhook.target.service.name.hostname`|string||
|`backends[].policies.ai.promptGuard.request[].webhook.target.service.port`|integer||
|`backends[].policies.ai.promptGuard.request[].webhook.target.host`|string|Hostname or IP address|
|`backends[].policies.ai.promptGuard.request[].webhook.target.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`backends[].policies.ai.promptGuard.request[].webhook.forwardHeaderMatches`|[]object||
|`backends[].policies.ai.promptGuard.request[].webhook.forwardHeaderMatches[].name`|string||
|`backends[].policies.ai.promptGuard.request[].webhook.forwardHeaderMatches[].value`|object|Exactly one of exact or regex may be set.|
|`backends[].policies.ai.promptGuard.request[].webhook.forwardHeaderMatches[].value.exact`|string||
|`backends[].policies.ai.promptGuard.request[].webhook.forwardHeaderMatches[].value.regex`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration`|object||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.model`|string|Model to use. Defaults to `omni-moderation-latest`|
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies`|object||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.requestHeaderModifier.add`|object||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.requestHeaderModifier.set`|object||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.requestHeaderModifier.remove`|[]string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.responseHeaderModifier.add`|object||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.responseHeaderModifier.set`|object||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.responseHeaderModifier.remove`|[]string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.scheme`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.authority`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.authority.full`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.authority.host`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.authority.port`|integer||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.path`|object||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.path.full`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.path.prefix`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.status`|integer||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request`|object||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request.add`|object||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request.set`|object||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request.remove`|[]string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request.body`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request.metadata`|object||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response`|object||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response.add`|object||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response.set`|object||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response.remove`|[]string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response.body`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response.metadata`|object||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS`|object|Send TLS to the backend.|
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.cert`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.key`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.root`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.hostname`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.insecure`|boolean||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.insecureHost`|boolean||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.alpn`|[]string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.subjectAltNames`|[]string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth`|object|Authenticate to the backend.|
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.passthrough`|object||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.key`|object||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.key.file`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.gcp`|object||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.gcp.type`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.gcp.type`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws`|object||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws.accessKeyId`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws.secretAccessKey`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws.region`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws.sessionToken`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.developerImplicit`|object||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.implicit`|object||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.http`|object|Specify HTTP settings for the backend|
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.http.version`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.http.requestTimeout`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp`|object|Specify TCP settings for the backend|
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives`|object||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives.enabled`|boolean||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives.time`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives.interval`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives.retries`|integer||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.connectTimeout`|object||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.connectTimeout.secs`|integer||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.tcp.connectTimeout.nanos`|integer||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.health.eviction.duration`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.health.eviction.restoreHealth`|number||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.health.eviction.consecutiveFailures`|integer||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.health.eviction.healthThreshold`|number||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service`|object||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service.name`|object||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service.name.namespace`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service.name.hostname`|string||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service.port`|integer||
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`backends[].policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails`|object|Configuration for AWS Bedrock Guardrails integration.|
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.guardrailIdentifier`|string|The unique identifier of the guardrail|
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.guardrailVersion`|string|The version of the guardrail|
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.region`|string|AWS region where the guardrail is deployed|
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies`|object|Backend policies for AWS authentication (optional, defaults to implicit AWS auth)|
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestHeaderModifier.add`|object||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestHeaderModifier.set`|object||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestHeaderModifier.remove`|[]string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.responseHeaderModifier.add`|object||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.responseHeaderModifier.set`|object||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.responseHeaderModifier.remove`|[]string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.scheme`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.authority`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.authority.full`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.authority.host`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.authority.port`|integer||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.path`|object||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.path.full`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.path.prefix`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.status`|integer||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request`|object||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.add`|object||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.set`|object||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.remove`|[]string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.body`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.metadata`|object||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response`|object||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.add`|object||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.set`|object||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.remove`|[]string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.body`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.metadata`|object||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS`|object|Send TLS to the backend.|
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.cert`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.key`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.root`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.hostname`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.insecure`|boolean||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.insecureHost`|boolean||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.alpn`|[]string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.subjectAltNames`|[]string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth`|object|Authenticate to the backend.|
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.passthrough`|object||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.key`|object||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.key.file`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.gcp`|object||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws`|object||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws.accessKeyId`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws.secretAccessKey`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws.region`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws.sessionToken`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.developerImplicit`|object||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.implicit`|object||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.http`|object|Specify HTTP settings for the backend|
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.http.version`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.http.requestTimeout`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp`|object|Specify TCP settings for the backend|
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives`|object||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives.enabled`|boolean||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives.time`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives.interval`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives.retries`|integer||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.connectTimeout`|object||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.connectTimeout.secs`|integer||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.connectTimeout.nanos`|integer||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction.duration`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction.restoreHealth`|number||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction.consecutiveFailures`|integer||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction.healthThreshold`|number||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service`|object||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.name`|object||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.namespace`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.hostname`|string||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.port`|integer||
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`backends[].policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`backends[].policies.ai.promptGuard.request[].googleModelArmor`|object|Configuration for Google Cloud Model Armor integration.|
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.templateId`|string|The template ID for the Model Armor configuration|
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.projectId`|string|The GCP project ID|
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.location`|string|The GCP region (default: us-central1)|
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies`|object|Backend policies for GCP authentication (optional, defaults to implicit GCP auth)|
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestHeaderModifier.add`|object||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestHeaderModifier.set`|object||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestHeaderModifier.remove`|[]string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.responseHeaderModifier.add`|object||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.responseHeaderModifier.set`|object||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.responseHeaderModifier.remove`|[]string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.scheme`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.authority`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.authority.full`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.authority.host`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.authority.port`|integer||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.path`|object||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.path.full`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.path.prefix`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.status`|integer||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request`|object||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.add`|object||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.set`|object||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.remove`|[]string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.body`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.metadata`|object||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response`|object||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.add`|object||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.set`|object||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.remove`|[]string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.body`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.metadata`|object||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS`|object|Send TLS to the backend.|
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.cert`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.key`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.root`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.hostname`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.insecure`|boolean||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.insecureHost`|boolean||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.alpn`|[]string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.subjectAltNames`|[]string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth`|object|Authenticate to the backend.|
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.passthrough`|object||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.key`|object||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.key.file`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.gcp`|object||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws`|object||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws.accessKeyId`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws.secretAccessKey`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws.region`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws.sessionToken`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.developerImplicit`|object||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.implicit`|object||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.http`|object|Specify HTTP settings for the backend|
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.http.version`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.http.requestTimeout`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp`|object|Specify TCP settings for the backend|
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives`|object||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives.enabled`|boolean||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives.time`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives.interval`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives.retries`|integer||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.connectTimeout`|object||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.connectTimeout.secs`|integer||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.connectTimeout.nanos`|integer||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.health.eviction.duration`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.health.eviction.restoreHealth`|number||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.health.eviction.consecutiveFailures`|integer||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.health.eviction.healthThreshold`|number||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service`|object||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service.name`|object||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service.name.namespace`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service.name.hostname`|string||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service.port`|integer||
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`backends[].policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`backends[].policies.ai.promptGuard.request[].rejection`|object||
|`backends[].policies.ai.promptGuard.request[].rejection.body`|array||
|`backends[].policies.ai.promptGuard.request[].rejection.status`|integer||
|`backends[].policies.ai.promptGuard.request[].rejection.headers`|object|Optional headers to add, set, or remove from the rejection response|
|`backends[].policies.ai.promptGuard.request[].rejection.headers.add`|object||
|`backends[].policies.ai.promptGuard.request[].rejection.headers.set`|object||
|`backends[].policies.ai.promptGuard.request[].rejection.headers.remove`|[]string||
|`backends[].policies.ai.promptGuard.response`|[]object||
|`backends[].policies.ai.promptGuard.response[].regex`|object||
|`backends[].policies.ai.promptGuard.response[].regex.action`|string||
|`backends[].policies.ai.promptGuard.response[].regex.rules`|[]object||
|`backends[].policies.ai.promptGuard.response[].regex.rules[].builtin`|string||
|`backends[].policies.ai.promptGuard.response[].regex.rules[].pattern`|string||
|`backends[].policies.ai.promptGuard.response[].webhook`|object||
|`backends[].policies.ai.promptGuard.response[].webhook.target`|object|Exactly one of service, host, or backend may be set.|
|`backends[].policies.ai.promptGuard.response[].webhook.target.service`|object||
|`backends[].policies.ai.promptGuard.response[].webhook.target.service.name`|object||
|`backends[].policies.ai.promptGuard.response[].webhook.target.service.name.namespace`|string||
|`backends[].policies.ai.promptGuard.response[].webhook.target.service.name.hostname`|string||
|`backends[].policies.ai.promptGuard.response[].webhook.target.service.port`|integer||
|`backends[].policies.ai.promptGuard.response[].webhook.target.host`|string|Hostname or IP address|
|`backends[].policies.ai.promptGuard.response[].webhook.target.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`backends[].policies.ai.promptGuard.response[].webhook.forwardHeaderMatches`|[]object||
|`backends[].policies.ai.promptGuard.response[].webhook.forwardHeaderMatches[].name`|string||
|`backends[].policies.ai.promptGuard.response[].webhook.forwardHeaderMatches[].value`|object|Exactly one of exact or regex may be set.|
|`backends[].policies.ai.promptGuard.response[].webhook.forwardHeaderMatches[].value.exact`|string||
|`backends[].policies.ai.promptGuard.response[].webhook.forwardHeaderMatches[].value.regex`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails`|object|Configuration for AWS Bedrock Guardrails integration.|
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.guardrailIdentifier`|string|The unique identifier of the guardrail|
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.guardrailVersion`|string|The version of the guardrail|
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.region`|string|AWS region where the guardrail is deployed|
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies`|object|Backend policies for AWS authentication (optional, defaults to implicit AWS auth)|
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestHeaderModifier.add`|object||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestHeaderModifier.set`|object||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestHeaderModifier.remove`|[]string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.responseHeaderModifier.add`|object||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.responseHeaderModifier.set`|object||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.responseHeaderModifier.remove`|[]string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.scheme`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.authority`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.authority.full`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.authority.host`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.authority.port`|integer||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.path`|object||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.path.full`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.path.prefix`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.status`|integer||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request`|object||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.add`|object||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.set`|object||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.remove`|[]string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.body`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.metadata`|object||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response`|object||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.add`|object||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.set`|object||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.remove`|[]string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.body`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.metadata`|object||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS`|object|Send TLS to the backend.|
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.cert`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.key`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.root`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.hostname`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.insecure`|boolean||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.insecureHost`|boolean||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.alpn`|[]string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.subjectAltNames`|[]string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth`|object|Authenticate to the backend.|
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.passthrough`|object||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.key`|object||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.key.file`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.gcp`|object||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws`|object||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws.accessKeyId`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws.secretAccessKey`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws.region`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws.sessionToken`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.developerImplicit`|object||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.implicit`|object||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.http`|object|Specify HTTP settings for the backend|
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.http.version`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.http.requestTimeout`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp`|object|Specify TCP settings for the backend|
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives`|object||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives.enabled`|boolean||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives.time`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives.interval`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives.retries`|integer||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.connectTimeout`|object||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.connectTimeout.secs`|integer||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.connectTimeout.nanos`|integer||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction.duration`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction.restoreHealth`|number||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction.consecutiveFailures`|integer||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction.healthThreshold`|number||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service`|object||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.name`|object||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.namespace`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.hostname`|string||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.port`|integer||
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`backends[].policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`backends[].policies.ai.promptGuard.response[].googleModelArmor`|object|Configuration for Google Cloud Model Armor integration.|
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.templateId`|string|The template ID for the Model Armor configuration|
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.projectId`|string|The GCP project ID|
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.location`|string|The GCP region (default: us-central1)|
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies`|object|Backend policies for GCP authentication (optional, defaults to implicit GCP auth)|
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestHeaderModifier.add`|object||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestHeaderModifier.set`|object||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestHeaderModifier.remove`|[]string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.responseHeaderModifier.add`|object||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.responseHeaderModifier.set`|object||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.responseHeaderModifier.remove`|[]string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.scheme`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.authority`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.authority.full`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.authority.host`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.authority.port`|integer||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.path`|object||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.path.full`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.path.prefix`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.status`|integer||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request`|object||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.add`|object||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.set`|object||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.remove`|[]string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.body`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.metadata`|object||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response`|object||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.add`|object||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.set`|object||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.remove`|[]string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.body`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.metadata`|object||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS`|object|Send TLS to the backend.|
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.cert`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.key`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.root`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.hostname`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.insecure`|boolean||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.insecureHost`|boolean||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.alpn`|[]string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.subjectAltNames`|[]string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth`|object|Authenticate to the backend.|
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.passthrough`|object||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.key`|object||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.key.file`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.gcp`|object||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws`|object||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws.accessKeyId`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws.secretAccessKey`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws.region`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws.sessionToken`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.developerImplicit`|object||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.implicit`|object||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.http`|object|Specify HTTP settings for the backend|
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.http.version`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.http.requestTimeout`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp`|object|Specify TCP settings for the backend|
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives`|object||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives.enabled`|boolean||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives.time`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives.interval`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives.retries`|integer||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.connectTimeout`|object||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.connectTimeout.secs`|integer||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.connectTimeout.nanos`|integer||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.health.eviction.duration`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.health.eviction.restoreHealth`|number||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.health.eviction.consecutiveFailures`|integer||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.health.eviction.healthThreshold`|number||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service`|object||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service.name`|object||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service.name.namespace`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service.name.hostname`|string||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service.port`|integer||
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`backends[].policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`backends[].policies.ai.promptGuard.response[].rejection`|object||
|`backends[].policies.ai.promptGuard.response[].rejection.body`|array||
|`backends[].policies.ai.promptGuard.response[].rejection.status`|integer||
|`backends[].policies.ai.promptGuard.response[].rejection.headers`|object|Optional headers to add, set, or remove from the rejection response|
|`backends[].policies.ai.promptGuard.response[].rejection.headers.add`|object||
|`backends[].policies.ai.promptGuard.response[].rejection.headers.set`|object||
|`backends[].policies.ai.promptGuard.response[].rejection.headers.remove`|[]string||
|`backends[].policies.ai.defaults`|object||
|`backends[].policies.ai.overrides`|object||
|`backends[].policies.ai.transformations`|object||
|`backends[].policies.ai.prompts`|object||
|`backends[].policies.ai.prompts.append`|[]object||
|`backends[].policies.ai.prompts.append[].role`|string||
|`backends[].policies.ai.prompts.append[].content`|string||
|`backends[].policies.ai.prompts.prepend`|[]object||
|`backends[].policies.ai.prompts.prepend[].role`|string||
|`backends[].policies.ai.prompts.prepend[].content`|string||
|`backends[].policies.ai.modelAliases`|object||
|`backends[].policies.ai.promptCaching`|object||
|`backends[].policies.ai.promptCaching.cacheSystem`|boolean||
|`backends[].policies.ai.promptCaching.cacheMessages`|boolean||
|`backends[].policies.ai.promptCaching.cacheTools`|boolean||
|`backends[].policies.ai.promptCaching.minTokens`|integer||
|`backends[].policies.ai.routes`|object||
|`llm`|object||
|`llm.port`|integer||
|`llm.models`|[]object|models defines the set of models that can be served by this gateway. The model name refers to the<br>model in the users request that is matched; the model sent to the actual LLM can be overridden<br>on a per-model basis.|
|`llm.models[].name`|string|name is the name of the model we are matching from a users request. If params.model is set, that<br>will be used in the request to the LLM provider. If not, the incoming model is used.|
|`llm.models[].params`|object|params customizes parameters for the outgoing request|
|`llm.models[].params.model`|string|The model to send to the provider.<br>If unset, the same model will be used from the request.|
|`llm.models[].params.apiKey`|object|An API key to attach to the request.<br>If unset this will be automatically detected from the environment.|
|`llm.models[].params.apiKey.file`|string||
|`llm.models[].params.awsRegion`|string||
|`llm.models[].params.vertexRegion`|string||
|`llm.models[].params.vertexProject`|string||
|`llm.models[].params.azureResourceName`|string|For Azure: the resource name of the deployment|
|`llm.models[].params.azureResourceType`|string|For Azure: the type of Azure endpoint (openAI or foundry)|
|`llm.models[].params.azureApiVersion`|string|For Azure: the API version to use|
|`llm.models[].params.azureProjectName`|string|For Azure: the Foundry project name (required for foundry resource type)|
|`llm.models[].params.hostOverride`|string|Override the upstream host for this provider.|
|`llm.models[].params.pathOverride`|string|Override the upstream path for this provider.|
|`llm.models[].params.pathPrefix`|string|Override the default base path prefix for this provider.|
|`llm.models[].params.tokenize`|boolean|Whether to tokenize the request before forwarding it upstream.|
|`llm.models[].provider`|string|provider of the LLM we are connecting too|
|`llm.models[].defaults`|object|defaults allows setting default values for the request. If these are not present in the request body, they will be set.<br>To override even when set, use `overrides`.|
|`llm.models[].overrides`|object|overrides allows setting values for the request, overriding any existing values|
|`llm.models[].transformation`|object|transformation allows setting values from CEL expressions for the request, overriding any existing values.|
|`llm.models[].requestHeaders`|object|requestHeaders modifies headers in requests to the LLM provider.|
|`llm.models[].requestHeaders.add`|object||
|`llm.models[].requestHeaders.set`|object||
|`llm.models[].requestHeaders.remove`|[]string||
|`llm.models[].responseHeaders`|object|responseHeaders modifies headers in responses from the LLM provider.|
|`llm.models[].responseHeaders.add`|object||
|`llm.models[].responseHeaders.set`|object||
|`llm.models[].responseHeaders.remove`|[]string||
|`llm.models[].backendTLS`|object|backendTLS configures TLS when connecting to the LLM provider.|
|`llm.models[].backendTLS.cert`|string||
|`llm.models[].backendTLS.key`|string||
|`llm.models[].backendTLS.root`|string||
|`llm.models[].backendTLS.hostname`|string||
|`llm.models[].backendTLS.insecure`|boolean||
|`llm.models[].backendTLS.insecureHost`|boolean||
|`llm.models[].backendTLS.alpn`|[]string||
|`llm.models[].backendTLS.subjectAltNames`|[]string||
|`llm.models[].health`|object|health configures outlier detection for this model backend.|
|`llm.models[].health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`llm.models[].health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`llm.models[].health.eviction.duration`|string||
|`llm.models[].health.eviction.restoreHealth`|number||
|`llm.models[].health.eviction.consecutiveFailures`|integer||
|`llm.models[].health.eviction.healthThreshold`|number||
|`llm.models[].backendTunnel`|object|backendTunnel configures tunneling when connecting to the LLM provider.|
|`llm.models[].backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`llm.models[].backendTunnel.proxy.service`|object||
|`llm.models[].backendTunnel.proxy.service.name`|object||
|`llm.models[].backendTunnel.proxy.service.name.namespace`|string||
|`llm.models[].backendTunnel.proxy.service.name.hostname`|string||
|`llm.models[].backendTunnel.proxy.service.port`|integer||
|`llm.models[].backendTunnel.proxy.host`|string|Hostname or IP address|
|`llm.models[].backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`llm.models[].guardrails`|object|guardrails to apply to the request or response|
|`llm.models[].guardrails.request`|[]object||
|`llm.models[].guardrails.request[].regex`|object||
|`llm.models[].guardrails.request[].regex.action`|string||
|`llm.models[].guardrails.request[].regex.rules`|[]object||
|`llm.models[].guardrails.request[].regex.rules[].builtin`|string||
|`llm.models[].guardrails.request[].regex.rules[].pattern`|string||
|`llm.models[].guardrails.request[].webhook`|object||
|`llm.models[].guardrails.request[].webhook.target`|object|Exactly one of service, host, or backend may be set.|
|`llm.models[].guardrails.request[].webhook.target.service`|object||
|`llm.models[].guardrails.request[].webhook.target.service.name`|object||
|`llm.models[].guardrails.request[].webhook.target.service.name.namespace`|string||
|`llm.models[].guardrails.request[].webhook.target.service.name.hostname`|string||
|`llm.models[].guardrails.request[].webhook.target.service.port`|integer||
|`llm.models[].guardrails.request[].webhook.target.host`|string|Hostname or IP address|
|`llm.models[].guardrails.request[].webhook.target.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`llm.models[].guardrails.request[].webhook.forwardHeaderMatches`|[]object||
|`llm.models[].guardrails.request[].webhook.forwardHeaderMatches[].name`|string||
|`llm.models[].guardrails.request[].webhook.forwardHeaderMatches[].value`|object|Exactly one of exact or regex may be set.|
|`llm.models[].guardrails.request[].webhook.forwardHeaderMatches[].value.exact`|string||
|`llm.models[].guardrails.request[].webhook.forwardHeaderMatches[].value.regex`|string||
|`llm.models[].guardrails.request[].openAIModeration`|object||
|`llm.models[].guardrails.request[].openAIModeration.model`|string|Model to use. Defaults to `omni-moderation-latest`|
|`llm.models[].guardrails.request[].openAIModeration.policies`|object||
|`llm.models[].guardrails.request[].openAIModeration.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`llm.models[].guardrails.request[].openAIModeration.policies.requestHeaderModifier.add`|object||
|`llm.models[].guardrails.request[].openAIModeration.policies.requestHeaderModifier.set`|object||
|`llm.models[].guardrails.request[].openAIModeration.policies.requestHeaderModifier.remove`|[]string||
|`llm.models[].guardrails.request[].openAIModeration.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`llm.models[].guardrails.request[].openAIModeration.policies.responseHeaderModifier.add`|object||
|`llm.models[].guardrails.request[].openAIModeration.policies.responseHeaderModifier.set`|object||
|`llm.models[].guardrails.request[].openAIModeration.policies.responseHeaderModifier.remove`|[]string||
|`llm.models[].guardrails.request[].openAIModeration.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`llm.models[].guardrails.request[].openAIModeration.policies.requestRedirect.scheme`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.requestRedirect.authority`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.requestRedirect.authority.full`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.requestRedirect.authority.host`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.requestRedirect.authority.port`|integer||
|`llm.models[].guardrails.request[].openAIModeration.policies.requestRedirect.path`|object||
|`llm.models[].guardrails.request[].openAIModeration.policies.requestRedirect.path.full`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.requestRedirect.path.prefix`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.requestRedirect.status`|integer||
|`llm.models[].guardrails.request[].openAIModeration.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`llm.models[].guardrails.request[].openAIModeration.policies.transformations.request`|object||
|`llm.models[].guardrails.request[].openAIModeration.policies.transformations.request.add`|object||
|`llm.models[].guardrails.request[].openAIModeration.policies.transformations.request.set`|object||
|`llm.models[].guardrails.request[].openAIModeration.policies.transformations.request.remove`|[]string||
|`llm.models[].guardrails.request[].openAIModeration.policies.transformations.request.body`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.transformations.request.metadata`|object||
|`llm.models[].guardrails.request[].openAIModeration.policies.transformations.response`|object||
|`llm.models[].guardrails.request[].openAIModeration.policies.transformations.response.add`|object||
|`llm.models[].guardrails.request[].openAIModeration.policies.transformations.response.set`|object||
|`llm.models[].guardrails.request[].openAIModeration.policies.transformations.response.remove`|[]string||
|`llm.models[].guardrails.request[].openAIModeration.policies.transformations.response.body`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.transformations.response.metadata`|object||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendTLS`|object|Send TLS to the backend.|
|`llm.models[].guardrails.request[].openAIModeration.policies.backendTLS.cert`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendTLS.key`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendTLS.root`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendTLS.hostname`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendTLS.insecure`|boolean||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendTLS.insecureHost`|boolean||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendTLS.alpn`|[]string||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendTLS.subjectAltNames`|[]string||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendAuth`|object|Authenticate to the backend.|
|`llm.models[].guardrails.request[].openAIModeration.policies.backendAuth.passthrough`|object||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendAuth.key`|object||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendAuth.key.file`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendAuth.gcp`|object||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendAuth.gcp.type`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`llm.models[].guardrails.request[].openAIModeration.policies.backendAuth.gcp.type`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendAuth.aws`|object||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendAuth.aws.accessKeyId`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendAuth.aws.secretAccessKey`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendAuth.aws.region`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendAuth.aws.sessionToken`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`llm.models[].guardrails.request[].openAIModeration.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`llm.models[].guardrails.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendAuth.azure.developerImplicit`|object||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendAuth.azure.implicit`|object||
|`llm.models[].guardrails.request[].openAIModeration.policies.http`|object|Specify HTTP settings for the backend|
|`llm.models[].guardrails.request[].openAIModeration.policies.http.version`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.http.requestTimeout`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.tcp`|object|Specify TCP settings for the backend|
|`llm.models[].guardrails.request[].openAIModeration.policies.tcp.keepalives`|object||
|`llm.models[].guardrails.request[].openAIModeration.policies.tcp.keepalives.enabled`|boolean||
|`llm.models[].guardrails.request[].openAIModeration.policies.tcp.keepalives.time`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.tcp.keepalives.interval`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.tcp.keepalives.retries`|integer||
|`llm.models[].guardrails.request[].openAIModeration.policies.tcp.connectTimeout`|object||
|`llm.models[].guardrails.request[].openAIModeration.policies.tcp.connectTimeout.secs`|integer||
|`llm.models[].guardrails.request[].openAIModeration.policies.tcp.connectTimeout.nanos`|integer||
|`llm.models[].guardrails.request[].openAIModeration.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`llm.models[].guardrails.request[].openAIModeration.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`llm.models[].guardrails.request[].openAIModeration.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`llm.models[].guardrails.request[].openAIModeration.policies.health.eviction.duration`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.health.eviction.restoreHealth`|number||
|`llm.models[].guardrails.request[].openAIModeration.policies.health.eviction.consecutiveFailures`|integer||
|`llm.models[].guardrails.request[].openAIModeration.policies.health.eviction.healthThreshold`|number||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`llm.models[].guardrails.request[].openAIModeration.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`llm.models[].guardrails.request[].openAIModeration.policies.backendTunnel.proxy.service`|object||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendTunnel.proxy.service.name`|object||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendTunnel.proxy.service.name.namespace`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendTunnel.proxy.service.name.hostname`|string||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendTunnel.proxy.service.port`|integer||
|`llm.models[].guardrails.request[].openAIModeration.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`llm.models[].guardrails.request[].openAIModeration.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`llm.models[].guardrails.request[].bedrockGuardrails`|object|Configuration for AWS Bedrock Guardrails integration.|
|`llm.models[].guardrails.request[].bedrockGuardrails.guardrailIdentifier`|string|The unique identifier of the guardrail|
|`llm.models[].guardrails.request[].bedrockGuardrails.guardrailVersion`|string|The version of the guardrail|
|`llm.models[].guardrails.request[].bedrockGuardrails.region`|string|AWS region where the guardrail is deployed|
|`llm.models[].guardrails.request[].bedrockGuardrails.policies`|object|Backend policies for AWS authentication (optional, defaults to implicit AWS auth)|
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.requestHeaderModifier.add`|object||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.requestHeaderModifier.set`|object||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.requestHeaderModifier.remove`|[]string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.responseHeaderModifier.add`|object||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.responseHeaderModifier.set`|object||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.responseHeaderModifier.remove`|[]string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.requestRedirect.scheme`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.requestRedirect.authority`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.requestRedirect.authority.full`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.requestRedirect.authority.host`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.requestRedirect.authority.port`|integer||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.requestRedirect.path`|object||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.requestRedirect.path.full`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.requestRedirect.path.prefix`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.requestRedirect.status`|integer||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.transformations.request`|object||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.transformations.request.add`|object||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.transformations.request.set`|object||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.transformations.request.remove`|[]string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.transformations.request.body`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.transformations.request.metadata`|object||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.transformations.response`|object||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.transformations.response.add`|object||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.transformations.response.set`|object||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.transformations.response.remove`|[]string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.transformations.response.body`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.transformations.response.metadata`|object||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendTLS`|object|Send TLS to the backend.|
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendTLS.cert`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendTLS.key`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendTLS.root`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendTLS.hostname`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendTLS.insecure`|boolean||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendTLS.insecureHost`|boolean||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendTLS.alpn`|[]string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendTLS.subjectAltNames`|[]string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendAuth`|object|Authenticate to the backend.|
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendAuth.passthrough`|object||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendAuth.key`|object||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendAuth.key.file`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendAuth.gcp`|object||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendAuth.aws`|object||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendAuth.aws.accessKeyId`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendAuth.aws.secretAccessKey`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendAuth.aws.region`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendAuth.aws.sessionToken`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendAuth.azure.developerImplicit`|object||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendAuth.azure.implicit`|object||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.http`|object|Specify HTTP settings for the backend|
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.http.version`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.http.requestTimeout`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.tcp`|object|Specify TCP settings for the backend|
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.tcp.keepalives`|object||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.tcp.keepalives.enabled`|boolean||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.tcp.keepalives.time`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.tcp.keepalives.interval`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.tcp.keepalives.retries`|integer||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.tcp.connectTimeout`|object||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.tcp.connectTimeout.secs`|integer||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.tcp.connectTimeout.nanos`|integer||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.health.eviction.duration`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.health.eviction.restoreHealth`|number||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.health.eviction.consecutiveFailures`|integer||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.health.eviction.healthThreshold`|number||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendTunnel.proxy.service`|object||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.name`|object||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.namespace`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.hostname`|string||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.port`|integer||
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`llm.models[].guardrails.request[].bedrockGuardrails.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`llm.models[].guardrails.request[].googleModelArmor`|object|Configuration for Google Cloud Model Armor integration.|
|`llm.models[].guardrails.request[].googleModelArmor.templateId`|string|The template ID for the Model Armor configuration|
|`llm.models[].guardrails.request[].googleModelArmor.projectId`|string|The GCP project ID|
|`llm.models[].guardrails.request[].googleModelArmor.location`|string|The GCP region (default: us-central1)|
|`llm.models[].guardrails.request[].googleModelArmor.policies`|object|Backend policies for GCP authentication (optional, defaults to implicit GCP auth)|
|`llm.models[].guardrails.request[].googleModelArmor.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`llm.models[].guardrails.request[].googleModelArmor.policies.requestHeaderModifier.add`|object||
|`llm.models[].guardrails.request[].googleModelArmor.policies.requestHeaderModifier.set`|object||
|`llm.models[].guardrails.request[].googleModelArmor.policies.requestHeaderModifier.remove`|[]string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`llm.models[].guardrails.request[].googleModelArmor.policies.responseHeaderModifier.add`|object||
|`llm.models[].guardrails.request[].googleModelArmor.policies.responseHeaderModifier.set`|object||
|`llm.models[].guardrails.request[].googleModelArmor.policies.responseHeaderModifier.remove`|[]string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`llm.models[].guardrails.request[].googleModelArmor.policies.requestRedirect.scheme`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.requestRedirect.authority`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.requestRedirect.authority.full`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.requestRedirect.authority.host`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.requestRedirect.authority.port`|integer||
|`llm.models[].guardrails.request[].googleModelArmor.policies.requestRedirect.path`|object||
|`llm.models[].guardrails.request[].googleModelArmor.policies.requestRedirect.path.full`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.requestRedirect.path.prefix`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.requestRedirect.status`|integer||
|`llm.models[].guardrails.request[].googleModelArmor.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`llm.models[].guardrails.request[].googleModelArmor.policies.transformations.request`|object||
|`llm.models[].guardrails.request[].googleModelArmor.policies.transformations.request.add`|object||
|`llm.models[].guardrails.request[].googleModelArmor.policies.transformations.request.set`|object||
|`llm.models[].guardrails.request[].googleModelArmor.policies.transformations.request.remove`|[]string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.transformations.request.body`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.transformations.request.metadata`|object||
|`llm.models[].guardrails.request[].googleModelArmor.policies.transformations.response`|object||
|`llm.models[].guardrails.request[].googleModelArmor.policies.transformations.response.add`|object||
|`llm.models[].guardrails.request[].googleModelArmor.policies.transformations.response.set`|object||
|`llm.models[].guardrails.request[].googleModelArmor.policies.transformations.response.remove`|[]string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.transformations.response.body`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.transformations.response.metadata`|object||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendTLS`|object|Send TLS to the backend.|
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendTLS.cert`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendTLS.key`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendTLS.root`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendTLS.hostname`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendTLS.insecure`|boolean||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendTLS.insecureHost`|boolean||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendTLS.alpn`|[]string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendTLS.subjectAltNames`|[]string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendAuth`|object|Authenticate to the backend.|
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendAuth.passthrough`|object||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendAuth.key`|object||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendAuth.key.file`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendAuth.gcp`|object||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendAuth.aws`|object||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendAuth.aws.accessKeyId`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendAuth.aws.secretAccessKey`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendAuth.aws.region`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendAuth.aws.sessionToken`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendAuth.azure.developerImplicit`|object||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendAuth.azure.implicit`|object||
|`llm.models[].guardrails.request[].googleModelArmor.policies.http`|object|Specify HTTP settings for the backend|
|`llm.models[].guardrails.request[].googleModelArmor.policies.http.version`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.http.requestTimeout`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.tcp`|object|Specify TCP settings for the backend|
|`llm.models[].guardrails.request[].googleModelArmor.policies.tcp.keepalives`|object||
|`llm.models[].guardrails.request[].googleModelArmor.policies.tcp.keepalives.enabled`|boolean||
|`llm.models[].guardrails.request[].googleModelArmor.policies.tcp.keepalives.time`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.tcp.keepalives.interval`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.tcp.keepalives.retries`|integer||
|`llm.models[].guardrails.request[].googleModelArmor.policies.tcp.connectTimeout`|object||
|`llm.models[].guardrails.request[].googleModelArmor.policies.tcp.connectTimeout.secs`|integer||
|`llm.models[].guardrails.request[].googleModelArmor.policies.tcp.connectTimeout.nanos`|integer||
|`llm.models[].guardrails.request[].googleModelArmor.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`llm.models[].guardrails.request[].googleModelArmor.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`llm.models[].guardrails.request[].googleModelArmor.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`llm.models[].guardrails.request[].googleModelArmor.policies.health.eviction.duration`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.health.eviction.restoreHealth`|number||
|`llm.models[].guardrails.request[].googleModelArmor.policies.health.eviction.consecutiveFailures`|integer||
|`llm.models[].guardrails.request[].googleModelArmor.policies.health.eviction.healthThreshold`|number||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendTunnel.proxy.service`|object||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendTunnel.proxy.service.name`|object||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendTunnel.proxy.service.name.namespace`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendTunnel.proxy.service.name.hostname`|string||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendTunnel.proxy.service.port`|integer||
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`llm.models[].guardrails.request[].googleModelArmor.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`llm.models[].guardrails.request[].rejection`|object||
|`llm.models[].guardrails.request[].rejection.body`|array||
|`llm.models[].guardrails.request[].rejection.status`|integer||
|`llm.models[].guardrails.request[].rejection.headers`|object|Optional headers to add, set, or remove from the rejection response|
|`llm.models[].guardrails.request[].rejection.headers.add`|object||
|`llm.models[].guardrails.request[].rejection.headers.set`|object||
|`llm.models[].guardrails.request[].rejection.headers.remove`|[]string||
|`llm.models[].guardrails.response`|[]object||
|`llm.models[].guardrails.response[].regex`|object||
|`llm.models[].guardrails.response[].regex.action`|string||
|`llm.models[].guardrails.response[].regex.rules`|[]object||
|`llm.models[].guardrails.response[].regex.rules[].builtin`|string||
|`llm.models[].guardrails.response[].regex.rules[].pattern`|string||
|`llm.models[].guardrails.response[].webhook`|object||
|`llm.models[].guardrails.response[].webhook.target`|object|Exactly one of service, host, or backend may be set.|
|`llm.models[].guardrails.response[].webhook.target.service`|object||
|`llm.models[].guardrails.response[].webhook.target.service.name`|object||
|`llm.models[].guardrails.response[].webhook.target.service.name.namespace`|string||
|`llm.models[].guardrails.response[].webhook.target.service.name.hostname`|string||
|`llm.models[].guardrails.response[].webhook.target.service.port`|integer||
|`llm.models[].guardrails.response[].webhook.target.host`|string|Hostname or IP address|
|`llm.models[].guardrails.response[].webhook.target.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`llm.models[].guardrails.response[].webhook.forwardHeaderMatches`|[]object||
|`llm.models[].guardrails.response[].webhook.forwardHeaderMatches[].name`|string||
|`llm.models[].guardrails.response[].webhook.forwardHeaderMatches[].value`|object|Exactly one of exact or regex may be set.|
|`llm.models[].guardrails.response[].webhook.forwardHeaderMatches[].value.exact`|string||
|`llm.models[].guardrails.response[].webhook.forwardHeaderMatches[].value.regex`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails`|object|Configuration for AWS Bedrock Guardrails integration.|
|`llm.models[].guardrails.response[].bedrockGuardrails.guardrailIdentifier`|string|The unique identifier of the guardrail|
|`llm.models[].guardrails.response[].bedrockGuardrails.guardrailVersion`|string|The version of the guardrail|
|`llm.models[].guardrails.response[].bedrockGuardrails.region`|string|AWS region where the guardrail is deployed|
|`llm.models[].guardrails.response[].bedrockGuardrails.policies`|object|Backend policies for AWS authentication (optional, defaults to implicit AWS auth)|
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.requestHeaderModifier.add`|object||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.requestHeaderModifier.set`|object||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.requestHeaderModifier.remove`|[]string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.responseHeaderModifier.add`|object||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.responseHeaderModifier.set`|object||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.responseHeaderModifier.remove`|[]string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.requestRedirect.scheme`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.requestRedirect.authority`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.requestRedirect.authority.full`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.requestRedirect.authority.host`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.requestRedirect.authority.port`|integer||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.requestRedirect.path`|object||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.requestRedirect.path.full`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.requestRedirect.path.prefix`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.requestRedirect.status`|integer||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.transformations.request`|object||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.transformations.request.add`|object||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.transformations.request.set`|object||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.transformations.request.remove`|[]string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.transformations.request.body`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.transformations.request.metadata`|object||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.transformations.response`|object||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.transformations.response.add`|object||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.transformations.response.set`|object||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.transformations.response.remove`|[]string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.transformations.response.body`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.transformations.response.metadata`|object||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendTLS`|object|Send TLS to the backend.|
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendTLS.cert`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendTLS.key`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendTLS.root`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendTLS.hostname`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendTLS.insecure`|boolean||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendTLS.insecureHost`|boolean||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendTLS.alpn`|[]string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendTLS.subjectAltNames`|[]string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendAuth`|object|Authenticate to the backend.|
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendAuth.passthrough`|object||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendAuth.key`|object||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendAuth.key.file`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendAuth.gcp`|object||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendAuth.aws`|object||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendAuth.aws.accessKeyId`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendAuth.aws.secretAccessKey`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendAuth.aws.region`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendAuth.aws.sessionToken`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendAuth.azure.developerImplicit`|object||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendAuth.azure.implicit`|object||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.http`|object|Specify HTTP settings for the backend|
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.http.version`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.http.requestTimeout`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.tcp`|object|Specify TCP settings for the backend|
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.tcp.keepalives`|object||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.tcp.keepalives.enabled`|boolean||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.tcp.keepalives.time`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.tcp.keepalives.interval`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.tcp.keepalives.retries`|integer||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.tcp.connectTimeout`|object||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.tcp.connectTimeout.secs`|integer||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.tcp.connectTimeout.nanos`|integer||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.health.eviction.duration`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.health.eviction.restoreHealth`|number||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.health.eviction.consecutiveFailures`|integer||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.health.eviction.healthThreshold`|number||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendTunnel.proxy.service`|object||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.name`|object||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.namespace`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.hostname`|string||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.port`|integer||
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`llm.models[].guardrails.response[].bedrockGuardrails.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`llm.models[].guardrails.response[].googleModelArmor`|object|Configuration for Google Cloud Model Armor integration.|
|`llm.models[].guardrails.response[].googleModelArmor.templateId`|string|The template ID for the Model Armor configuration|
|`llm.models[].guardrails.response[].googleModelArmor.projectId`|string|The GCP project ID|
|`llm.models[].guardrails.response[].googleModelArmor.location`|string|The GCP region (default: us-central1)|
|`llm.models[].guardrails.response[].googleModelArmor.policies`|object|Backend policies for GCP authentication (optional, defaults to implicit GCP auth)|
|`llm.models[].guardrails.response[].googleModelArmor.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`llm.models[].guardrails.response[].googleModelArmor.policies.requestHeaderModifier.add`|object||
|`llm.models[].guardrails.response[].googleModelArmor.policies.requestHeaderModifier.set`|object||
|`llm.models[].guardrails.response[].googleModelArmor.policies.requestHeaderModifier.remove`|[]string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`llm.models[].guardrails.response[].googleModelArmor.policies.responseHeaderModifier.add`|object||
|`llm.models[].guardrails.response[].googleModelArmor.policies.responseHeaderModifier.set`|object||
|`llm.models[].guardrails.response[].googleModelArmor.policies.responseHeaderModifier.remove`|[]string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`llm.models[].guardrails.response[].googleModelArmor.policies.requestRedirect.scheme`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.requestRedirect.authority`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.requestRedirect.authority.full`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.requestRedirect.authority.host`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.requestRedirect.authority.port`|integer||
|`llm.models[].guardrails.response[].googleModelArmor.policies.requestRedirect.path`|object||
|`llm.models[].guardrails.response[].googleModelArmor.policies.requestRedirect.path.full`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.requestRedirect.path.prefix`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.requestRedirect.status`|integer||
|`llm.models[].guardrails.response[].googleModelArmor.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`llm.models[].guardrails.response[].googleModelArmor.policies.transformations.request`|object||
|`llm.models[].guardrails.response[].googleModelArmor.policies.transformations.request.add`|object||
|`llm.models[].guardrails.response[].googleModelArmor.policies.transformations.request.set`|object||
|`llm.models[].guardrails.response[].googleModelArmor.policies.transformations.request.remove`|[]string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.transformations.request.body`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.transformations.request.metadata`|object||
|`llm.models[].guardrails.response[].googleModelArmor.policies.transformations.response`|object||
|`llm.models[].guardrails.response[].googleModelArmor.policies.transformations.response.add`|object||
|`llm.models[].guardrails.response[].googleModelArmor.policies.transformations.response.set`|object||
|`llm.models[].guardrails.response[].googleModelArmor.policies.transformations.response.remove`|[]string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.transformations.response.body`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.transformations.response.metadata`|object||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendTLS`|object|Send TLS to the backend.|
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendTLS.cert`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendTLS.key`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendTLS.root`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendTLS.hostname`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendTLS.insecure`|boolean||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendTLS.insecureHost`|boolean||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendTLS.alpn`|[]string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendTLS.subjectAltNames`|[]string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendAuth`|object|Authenticate to the backend.|
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendAuth.passthrough`|object||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendAuth.key`|object||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendAuth.key.file`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendAuth.gcp`|object||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendAuth.aws`|object||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendAuth.aws.accessKeyId`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendAuth.aws.secretAccessKey`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendAuth.aws.region`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendAuth.aws.sessionToken`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendAuth.azure.developerImplicit`|object||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendAuth.azure.implicit`|object||
|`llm.models[].guardrails.response[].googleModelArmor.policies.http`|object|Specify HTTP settings for the backend|
|`llm.models[].guardrails.response[].googleModelArmor.policies.http.version`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.http.requestTimeout`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.tcp`|object|Specify TCP settings for the backend|
|`llm.models[].guardrails.response[].googleModelArmor.policies.tcp.keepalives`|object||
|`llm.models[].guardrails.response[].googleModelArmor.policies.tcp.keepalives.enabled`|boolean||
|`llm.models[].guardrails.response[].googleModelArmor.policies.tcp.keepalives.time`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.tcp.keepalives.interval`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.tcp.keepalives.retries`|integer||
|`llm.models[].guardrails.response[].googleModelArmor.policies.tcp.connectTimeout`|object||
|`llm.models[].guardrails.response[].googleModelArmor.policies.tcp.connectTimeout.secs`|integer||
|`llm.models[].guardrails.response[].googleModelArmor.policies.tcp.connectTimeout.nanos`|integer||
|`llm.models[].guardrails.response[].googleModelArmor.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`llm.models[].guardrails.response[].googleModelArmor.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`llm.models[].guardrails.response[].googleModelArmor.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`llm.models[].guardrails.response[].googleModelArmor.policies.health.eviction.duration`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.health.eviction.restoreHealth`|number||
|`llm.models[].guardrails.response[].googleModelArmor.policies.health.eviction.consecutiveFailures`|integer||
|`llm.models[].guardrails.response[].googleModelArmor.policies.health.eviction.healthThreshold`|number||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendTunnel.proxy.service`|object||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendTunnel.proxy.service.name`|object||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendTunnel.proxy.service.name.namespace`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendTunnel.proxy.service.name.hostname`|string||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendTunnel.proxy.service.port`|integer||
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`llm.models[].guardrails.response[].googleModelArmor.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`llm.models[].guardrails.response[].rejection`|object||
|`llm.models[].guardrails.response[].rejection.body`|array||
|`llm.models[].guardrails.response[].rejection.status`|integer||
|`llm.models[].guardrails.response[].rejection.headers`|object|Optional headers to add, set, or remove from the rejection response|
|`llm.models[].guardrails.response[].rejection.headers.add`|object||
|`llm.models[].guardrails.response[].rejection.headers.set`|object||
|`llm.models[].guardrails.response[].rejection.headers.remove`|[]string||
|`llm.models[].matches`|[]object|matches specifies the conditions under which this model should be used in addition to matching the model name.|
|`llm.models[].matches[].headers`|[]object||
|`llm.models[].matches[].headers[].name`|string||
|`llm.models[].matches[].headers[].value`|object|Exactly one of exact or regex may be set.|
|`llm.models[].matches[].headers[].value.exact`|string||
|`llm.models[].matches[].headers[].value.regex`|string||
|`llm.policies`|object|policies defines policies for handling incoming requests, before a model is selected|
|`llm.policies.oidc`|object|Authenticate incoming browser requests with OIDC authorization code flow.|
|`llm.policies.oidc.issuer`|string|Issuer used for discovery and ID token validation.|
|`llm.policies.oidc.discovery`|object|Optional discovery document override. If omitted, discovery uses<br>`${issuer}/.well-known/openid-configuration`.|
|`llm.policies.oidc.discovery.file`|string||
|`llm.policies.oidc.discovery.url`|string||
|`llm.policies.oidc.authorizationEndpoint`|string|Authorization endpoint used to start the browser login flow.|
|`llm.policies.oidc.tokenEndpoint`|string|Token endpoint used to exchange the authorization code.|
|`llm.policies.oidc.tokenEndpointAuth`|string|Token endpoint client authentication method for explicit provider configuration.<br><br>Discovery mode derives this from provider metadata. Explicit mode defaults to<br>`clientSecretBasic` when omitted.|
|`llm.policies.oidc.jwks`|object|JWKS source used to validate returned ID tokens.|
|`llm.policies.oidc.jwks.file`|string||
|`llm.policies.oidc.jwks.url`|string||
|`llm.policies.oidc.clientId`|string|OAuth2 client identifier used for authorization and token exchange.|
|`llm.policies.oidc.clientSecret`|string|OAuth2 client secret used for token exchange.|
|`llm.policies.oidc.redirectURI`|string|Absolute callback URI handled by the gateway.<br>This policy always redirects unauthenticated non-callback requests back through this login<br>flow.|
|`llm.policies.oidc.scopes`|[]string|Additional OAuth2 scopes to request. `openid` is always included.|
|`llm.policies.jwtAuth`|object|Authenticate incoming JWT requests.|
|`llm.policies.jwtAuth.mode`|string||
|`llm.policies.jwtAuth.providers`|[]object||
|`llm.policies.jwtAuth.providers[].issuer`|string||
|`llm.policies.jwtAuth.providers[].audiences`|[]string||
|`llm.policies.jwtAuth.providers[].jwks`|object||
|`llm.policies.jwtAuth.providers[].jwks.file`|string||
|`llm.policies.jwtAuth.providers[].jwks.url`|string||
|`llm.policies.jwtAuth.providers[].jwtValidationOptions`|object|JWT validation options controlling which claims must be present in a token.<br><br>The `required_claims` set specifies which RFC 7519 registered claims must<br>exist in the token payload before validation proceeds. Only the following<br>values are recognized: `exp`, `nbf`, `aud`, `iss`, `sub`. Other registered<br>claims such as `iat` and `jti` are **not** enforced by the underlying<br>`jsonwebtoken` library and will be silently ignored.<br><br>This only enforces **presence**. Standard claims like `exp` and `nbf`<br>have their values validated independently (e.g., expiry is always checked<br>when the `exp` claim is present, regardless of this setting).<br><br>Defaults to `["exp"]`.|
|`llm.policies.jwtAuth.providers[].jwtValidationOptions.requiredClaims`|[]string|Claims that must be present in the token before validation.<br>Only "exp", "nbf", "aud", "iss", "sub" are enforced; others<br>(including "iat" and "jti") are ignored.<br>Defaults to ["exp"]. Use an empty list to require no claims.|
|`llm.policies.jwtAuth.mode`|string||
|`llm.policies.jwtAuth.issuer`|string||
|`llm.policies.jwtAuth.audiences`|[]string||
|`llm.policies.jwtAuth.jwks`|object||
|`llm.policies.jwtAuth.jwks.file`|string||
|`llm.policies.jwtAuth.jwks.url`|string||
|`llm.policies.jwtAuth.jwtValidationOptions`|object|JWT validation options controlling which claims must be present in a token.<br><br>The `required_claims` set specifies which RFC 7519 registered claims must<br>exist in the token payload before validation proceeds. Only the following<br>values are recognized: `exp`, `nbf`, `aud`, `iss`, `sub`. Other registered<br>claims such as `iat` and `jti` are **not** enforced by the underlying<br>`jsonwebtoken` library and will be silently ignored.<br><br>This only enforces **presence**. Standard claims like `exp` and `nbf`<br>have their values validated independently (e.g., expiry is always checked<br>when the `exp` claim is present, regardless of this setting).<br><br>Defaults to `["exp"]`.|
|`llm.policies.jwtAuth.jwtValidationOptions.requiredClaims`|[]string|Claims that must be present in the token before validation.<br>Only "exp", "nbf", "aud", "iss", "sub" are enforced; others<br>(including "iat" and "jti") are ignored.<br>Defaults to ["exp"]. Use an empty list to require no claims.|
|`llm.policies.extAuthz`|object|Authenticate incoming requests by calling an external authorization server.|
|`llm.policies.extAuthz.service`|object||
|`llm.policies.extAuthz.service.name`|object||
|`llm.policies.extAuthz.service.name.namespace`|string||
|`llm.policies.extAuthz.service.name.hostname`|string||
|`llm.policies.extAuthz.service.port`|integer||
|`llm.policies.extAuthz.host`|string|Hostname or IP address|
|`llm.policies.extAuthz.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`llm.policies.extAuthz.policies`|object|Policies to connect to the backend|
|`llm.policies.extAuthz.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`llm.policies.extAuthz.policies.requestHeaderModifier.add`|object||
|`llm.policies.extAuthz.policies.requestHeaderModifier.set`|object||
|`llm.policies.extAuthz.policies.requestHeaderModifier.remove`|[]string||
|`llm.policies.extAuthz.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`llm.policies.extAuthz.policies.responseHeaderModifier.add`|object||
|`llm.policies.extAuthz.policies.responseHeaderModifier.set`|object||
|`llm.policies.extAuthz.policies.responseHeaderModifier.remove`|[]string||
|`llm.policies.extAuthz.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`llm.policies.extAuthz.policies.requestRedirect.scheme`|string||
|`llm.policies.extAuthz.policies.requestRedirect.authority`|string||
|`llm.policies.extAuthz.policies.requestRedirect.authority.full`|string||
|`llm.policies.extAuthz.policies.requestRedirect.authority.host`|string||
|`llm.policies.extAuthz.policies.requestRedirect.authority.port`|integer||
|`llm.policies.extAuthz.policies.requestRedirect.path`|object||
|`llm.policies.extAuthz.policies.requestRedirect.path.full`|string||
|`llm.policies.extAuthz.policies.requestRedirect.path.prefix`|string||
|`llm.policies.extAuthz.policies.requestRedirect.status`|integer||
|`llm.policies.extAuthz.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`llm.policies.extAuthz.policies.transformations.request`|object||
|`llm.policies.extAuthz.policies.transformations.request.add`|object||
|`llm.policies.extAuthz.policies.transformations.request.set`|object||
|`llm.policies.extAuthz.policies.transformations.request.remove`|[]string||
|`llm.policies.extAuthz.policies.transformations.request.body`|string||
|`llm.policies.extAuthz.policies.transformations.request.metadata`|object||
|`llm.policies.extAuthz.policies.transformations.response`|object||
|`llm.policies.extAuthz.policies.transformations.response.add`|object||
|`llm.policies.extAuthz.policies.transformations.response.set`|object||
|`llm.policies.extAuthz.policies.transformations.response.remove`|[]string||
|`llm.policies.extAuthz.policies.transformations.response.body`|string||
|`llm.policies.extAuthz.policies.transformations.response.metadata`|object||
|`llm.policies.extAuthz.policies.backendTLS`|object|Send TLS to the backend.|
|`llm.policies.extAuthz.policies.backendTLS.cert`|string||
|`llm.policies.extAuthz.policies.backendTLS.key`|string||
|`llm.policies.extAuthz.policies.backendTLS.root`|string||
|`llm.policies.extAuthz.policies.backendTLS.hostname`|string||
|`llm.policies.extAuthz.policies.backendTLS.insecure`|boolean||
|`llm.policies.extAuthz.policies.backendTLS.insecureHost`|boolean||
|`llm.policies.extAuthz.policies.backendTLS.alpn`|[]string||
|`llm.policies.extAuthz.policies.backendTLS.subjectAltNames`|[]string||
|`llm.policies.extAuthz.policies.backendAuth`|object|Authenticate to the backend.|
|`llm.policies.extAuthz.policies.backendAuth.passthrough`|object||
|`llm.policies.extAuthz.policies.backendAuth.key`|object||
|`llm.policies.extAuthz.policies.backendAuth.key.file`|string||
|`llm.policies.extAuthz.policies.backendAuth.gcp`|object||
|`llm.policies.extAuthz.policies.backendAuth.gcp.type`|string||
|`llm.policies.extAuthz.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`llm.policies.extAuthz.policies.backendAuth.gcp.type`|string||
|`llm.policies.extAuthz.policies.backendAuth.aws`|object||
|`llm.policies.extAuthz.policies.backendAuth.aws.accessKeyId`|string||
|`llm.policies.extAuthz.policies.backendAuth.aws.secretAccessKey`|string||
|`llm.policies.extAuthz.policies.backendAuth.aws.region`|string||
|`llm.policies.extAuthz.policies.backendAuth.aws.sessionToken`|string||
|`llm.policies.extAuthz.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`llm.policies.extAuthz.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`llm.policies.extAuthz.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`llm.policies.extAuthz.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`llm.policies.extAuthz.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`llm.policies.extAuthz.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`llm.policies.extAuthz.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`llm.policies.extAuthz.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`llm.policies.extAuthz.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`llm.policies.extAuthz.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`llm.policies.extAuthz.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`llm.policies.extAuthz.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`llm.policies.extAuthz.policies.backendAuth.azure.developerImplicit`|object||
|`llm.policies.extAuthz.policies.backendAuth.azure.implicit`|object||
|`llm.policies.extAuthz.policies.http`|object|Specify HTTP settings for the backend|
|`llm.policies.extAuthz.policies.http.version`|string||
|`llm.policies.extAuthz.policies.http.requestTimeout`|string||
|`llm.policies.extAuthz.policies.tcp`|object|Specify TCP settings for the backend|
|`llm.policies.extAuthz.policies.tcp.keepalives`|object||
|`llm.policies.extAuthz.policies.tcp.keepalives.enabled`|boolean||
|`llm.policies.extAuthz.policies.tcp.keepalives.time`|string||
|`llm.policies.extAuthz.policies.tcp.keepalives.interval`|string||
|`llm.policies.extAuthz.policies.tcp.keepalives.retries`|integer||
|`llm.policies.extAuthz.policies.tcp.connectTimeout`|object||
|`llm.policies.extAuthz.policies.tcp.connectTimeout.secs`|integer||
|`llm.policies.extAuthz.policies.tcp.connectTimeout.nanos`|integer||
|`llm.policies.extAuthz.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`llm.policies.extAuthz.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`llm.policies.extAuthz.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`llm.policies.extAuthz.policies.health.eviction.duration`|string||
|`llm.policies.extAuthz.policies.health.eviction.restoreHealth`|number||
|`llm.policies.extAuthz.policies.health.eviction.consecutiveFailures`|integer||
|`llm.policies.extAuthz.policies.health.eviction.healthThreshold`|number||
|`llm.policies.extAuthz.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`llm.policies.extAuthz.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`llm.policies.extAuthz.policies.backendTunnel.proxy.service`|object||
|`llm.policies.extAuthz.policies.backendTunnel.proxy.service.name`|object||
|`llm.policies.extAuthz.policies.backendTunnel.proxy.service.name.namespace`|string||
|`llm.policies.extAuthz.policies.backendTunnel.proxy.service.name.hostname`|string||
|`llm.policies.extAuthz.policies.backendTunnel.proxy.service.port`|integer||
|`llm.policies.extAuthz.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`llm.policies.extAuthz.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`llm.policies.extAuthz.protocol`|object|The ext_authz protocol to use. Unless you need to integrate with an HTTP-only server, gRPC is recommended.<br>Exactly one of grpc or http may be set.|
|`llm.policies.extAuthz.protocol.grpc`|object||
|`llm.policies.extAuthz.protocol.grpc.context`|object|Additional context to send to the authorization service.<br>This maps to the `context_extensions` field of the request, and only allows static values.|
|`llm.policies.extAuthz.protocol.grpc.metadata`|object|Additional metadata to send to the authorization service.<br>This maps to the `metadata_context.filter_metadata` field of the request, and allows dynamic CEL expressions.<br>If unset, by default the `envoy.filters.http.jwt_authn` key is set if the JWT policy is used as well, for compatibility.|
|`llm.policies.extAuthz.protocol.http`|object||
|`llm.policies.extAuthz.protocol.http.path`|string||
|`llm.policies.extAuthz.protocol.http.redirect`|string|When using the HTTP protocol, and the server returns unauthorized, redirect to the URL resolved by<br>the provided expression rather than directly returning the error.|
|`llm.policies.extAuthz.protocol.http.includeResponseHeaders`|[]string|Specific headers from the authorization response will be copied into the request to the backend.|
|`llm.policies.extAuthz.protocol.http.addRequestHeaders`|object|Specific headers to add in the authorization request (empty = all headers), based on the expression|
|`llm.policies.extAuthz.protocol.http.metadata`|object|Metadata to include under the `extauthz` variable, based on the authorization response.|
|`llm.policies.extAuthz.failureMode`|string|Behavior when the authorization service is unavailable or returns an error|
|`llm.policies.extAuthz.failureMode.denyWithStatus`|integer||
|`llm.policies.extAuthz.includeRequestHeaders`|[]string|Specific headers to include in the authorization request.<br>If unset, the gRPC protocol sends all request headers. The HTTP protocol sends only 'Authorization'.|
|`llm.policies.extAuthz.includeRequestBody`|object|Options for including the request body in the authorization request|
|`llm.policies.extAuthz.includeRequestBody.maxRequestBytes`|integer|Maximum size of request body to buffer (default: 8192)|
|`llm.policies.extAuthz.includeRequestBody.allowPartialMessage`|boolean|If true, send partial body when max_request_bytes is reached|
|`llm.policies.extAuthz.includeRequestBody.packAsBytes`|boolean|If true, pack body as raw bytes in gRPC|
|`llm.policies.extProc`|object|Extend agentgateway with an external processor|
|`llm.policies.extProc.service`|object||
|`llm.policies.extProc.service.name`|object||
|`llm.policies.extProc.service.name.namespace`|string||
|`llm.policies.extProc.service.name.hostname`|string||
|`llm.policies.extProc.service.port`|integer||
|`llm.policies.extProc.host`|string|Hostname or IP address|
|`llm.policies.extProc.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`llm.policies.extProc.policies`|object|Policies to connect to the backend|
|`llm.policies.extProc.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`llm.policies.extProc.policies.requestHeaderModifier.add`|object||
|`llm.policies.extProc.policies.requestHeaderModifier.set`|object||
|`llm.policies.extProc.policies.requestHeaderModifier.remove`|[]string||
|`llm.policies.extProc.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`llm.policies.extProc.policies.responseHeaderModifier.add`|object||
|`llm.policies.extProc.policies.responseHeaderModifier.set`|object||
|`llm.policies.extProc.policies.responseHeaderModifier.remove`|[]string||
|`llm.policies.extProc.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`llm.policies.extProc.policies.requestRedirect.scheme`|string||
|`llm.policies.extProc.policies.requestRedirect.authority`|string||
|`llm.policies.extProc.policies.requestRedirect.authority.full`|string||
|`llm.policies.extProc.policies.requestRedirect.authority.host`|string||
|`llm.policies.extProc.policies.requestRedirect.authority.port`|integer||
|`llm.policies.extProc.policies.requestRedirect.path`|object||
|`llm.policies.extProc.policies.requestRedirect.path.full`|string||
|`llm.policies.extProc.policies.requestRedirect.path.prefix`|string||
|`llm.policies.extProc.policies.requestRedirect.status`|integer||
|`llm.policies.extProc.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`llm.policies.extProc.policies.transformations.request`|object||
|`llm.policies.extProc.policies.transformations.request.add`|object||
|`llm.policies.extProc.policies.transformations.request.set`|object||
|`llm.policies.extProc.policies.transformations.request.remove`|[]string||
|`llm.policies.extProc.policies.transformations.request.body`|string||
|`llm.policies.extProc.policies.transformations.request.metadata`|object||
|`llm.policies.extProc.policies.transformations.response`|object||
|`llm.policies.extProc.policies.transformations.response.add`|object||
|`llm.policies.extProc.policies.transformations.response.set`|object||
|`llm.policies.extProc.policies.transformations.response.remove`|[]string||
|`llm.policies.extProc.policies.transformations.response.body`|string||
|`llm.policies.extProc.policies.transformations.response.metadata`|object||
|`llm.policies.extProc.policies.backendTLS`|object|Send TLS to the backend.|
|`llm.policies.extProc.policies.backendTLS.cert`|string||
|`llm.policies.extProc.policies.backendTLS.key`|string||
|`llm.policies.extProc.policies.backendTLS.root`|string||
|`llm.policies.extProc.policies.backendTLS.hostname`|string||
|`llm.policies.extProc.policies.backendTLS.insecure`|boolean||
|`llm.policies.extProc.policies.backendTLS.insecureHost`|boolean||
|`llm.policies.extProc.policies.backendTLS.alpn`|[]string||
|`llm.policies.extProc.policies.backendTLS.subjectAltNames`|[]string||
|`llm.policies.extProc.policies.backendAuth`|object|Authenticate to the backend.|
|`llm.policies.extProc.policies.backendAuth.passthrough`|object||
|`llm.policies.extProc.policies.backendAuth.key`|object||
|`llm.policies.extProc.policies.backendAuth.key.file`|string||
|`llm.policies.extProc.policies.backendAuth.gcp`|object||
|`llm.policies.extProc.policies.backendAuth.gcp.type`|string||
|`llm.policies.extProc.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`llm.policies.extProc.policies.backendAuth.gcp.type`|string||
|`llm.policies.extProc.policies.backendAuth.aws`|object||
|`llm.policies.extProc.policies.backendAuth.aws.accessKeyId`|string||
|`llm.policies.extProc.policies.backendAuth.aws.secretAccessKey`|string||
|`llm.policies.extProc.policies.backendAuth.aws.region`|string||
|`llm.policies.extProc.policies.backendAuth.aws.sessionToken`|string||
|`llm.policies.extProc.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`llm.policies.extProc.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`llm.policies.extProc.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`llm.policies.extProc.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`llm.policies.extProc.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`llm.policies.extProc.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`llm.policies.extProc.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`llm.policies.extProc.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`llm.policies.extProc.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`llm.policies.extProc.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`llm.policies.extProc.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`llm.policies.extProc.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`llm.policies.extProc.policies.backendAuth.azure.developerImplicit`|object||
|`llm.policies.extProc.policies.backendAuth.azure.implicit`|object||
|`llm.policies.extProc.policies.http`|object|Specify HTTP settings for the backend|
|`llm.policies.extProc.policies.http.version`|string||
|`llm.policies.extProc.policies.http.requestTimeout`|string||
|`llm.policies.extProc.policies.tcp`|object|Specify TCP settings for the backend|
|`llm.policies.extProc.policies.tcp.keepalives`|object||
|`llm.policies.extProc.policies.tcp.keepalives.enabled`|boolean||
|`llm.policies.extProc.policies.tcp.keepalives.time`|string||
|`llm.policies.extProc.policies.tcp.keepalives.interval`|string||
|`llm.policies.extProc.policies.tcp.keepalives.retries`|integer||
|`llm.policies.extProc.policies.tcp.connectTimeout`|object||
|`llm.policies.extProc.policies.tcp.connectTimeout.secs`|integer||
|`llm.policies.extProc.policies.tcp.connectTimeout.nanos`|integer||
|`llm.policies.extProc.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`llm.policies.extProc.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`llm.policies.extProc.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`llm.policies.extProc.policies.health.eviction.duration`|string||
|`llm.policies.extProc.policies.health.eviction.restoreHealth`|number||
|`llm.policies.extProc.policies.health.eviction.consecutiveFailures`|integer||
|`llm.policies.extProc.policies.health.eviction.healthThreshold`|number||
|`llm.policies.extProc.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`llm.policies.extProc.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`llm.policies.extProc.policies.backendTunnel.proxy.service`|object||
|`llm.policies.extProc.policies.backendTunnel.proxy.service.name`|object||
|`llm.policies.extProc.policies.backendTunnel.proxy.service.name.namespace`|string||
|`llm.policies.extProc.policies.backendTunnel.proxy.service.name.hostname`|string||
|`llm.policies.extProc.policies.backendTunnel.proxy.service.port`|integer||
|`llm.policies.extProc.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`llm.policies.extProc.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`llm.policies.extProc.failureMode`|string|Behavior when the ext_proc service is unavailable or returns an error|
|`llm.policies.extProc.metadataContext`|object|Additional metadata to send to the external processing service.<br>Maps to the `metadata_context.filter_metadata` field in ProcessingRequest, and allows dynamic CEL expressions.|
|`llm.policies.extProc.requestAttributes`|object|Maps to the request `attributes` field in ProcessingRequest, and allows dynamic CEL expressions.|
|`llm.policies.extProc.responseAttributes`|object|Maps to the response `attributes` field in ProcessingRequest, and allows dynamic CEL expressions.|
|`llm.policies.transformations`|object|Modify requests and responses|
|`llm.policies.transformations.request`|object||
|`llm.policies.transformations.request.add`|object||
|`llm.policies.transformations.request.set`|object||
|`llm.policies.transformations.request.remove`|[]string||
|`llm.policies.transformations.request.body`|string||
|`llm.policies.transformations.request.metadata`|object||
|`llm.policies.transformations.response`|object||
|`llm.policies.transformations.response.add`|object||
|`llm.policies.transformations.response.set`|object||
|`llm.policies.transformations.response.remove`|[]string||
|`llm.policies.transformations.response.body`|string||
|`llm.policies.transformations.response.metadata`|object||
|`llm.policies.basicAuth`|object|Authenticate incoming requests using Basic Authentication with htpasswd.|
|`llm.policies.basicAuth.htpasswd`|object|.htpasswd file contents/reference|
|`llm.policies.basicAuth.htpasswd.file`|string||
|`llm.policies.basicAuth.realm`|string|Realm name for the WWW-Authenticate header|
|`llm.policies.basicAuth.mode`|string|Validation mode for basic authentication|
|`llm.policies.apiKey`|object|Authenticate incoming requests using API Keys|
|`llm.policies.apiKey.keys`|[]object|List of API keys|
|`llm.policies.apiKey.keys[].key`|string||
|`llm.policies.apiKey.keys[].metadata`|any||
|`llm.policies.apiKey.mode`|string|Validation mode for API keys|
|`llm.policies.authorization`|object|Authorization policies for HTTP access.|
|`llm.policies.authorization.rules`|[]string||
|`mcp`|object||
|`mcp.port`|integer||
|`mcp.targets`|[]object||
|`mcp.targets[].sse`|object||
|`mcp.targets[].sse.host`|string||
|`mcp.targets[].sse.port`|integer||
|`mcp.targets[].sse.path`|string||
|`mcp.targets[].mcp`|object||
|`mcp.targets[].mcp.host`|string||
|`mcp.targets[].mcp.port`|integer||
|`mcp.targets[].mcp.path`|string||
|`mcp.targets[].stdio`|object||
|`mcp.targets[].stdio.cmd`|string||
|`mcp.targets[].stdio.args`|[]string||
|`mcp.targets[].stdio.env`|object||
|`mcp.targets[].openapi`|object||
|`mcp.targets[].openapi.host`|string||
|`mcp.targets[].openapi.port`|integer||
|`mcp.targets[].openapi.path`|string||
|`mcp.targets[].openapi.schema`|object||
|`mcp.targets[].openapi.schema.file`|string||
|`mcp.targets[].openapi.schema.url`|string||
|`mcp.targets[].name`|string||
|`mcp.targets[].policies`|object||
|`mcp.targets[].policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`mcp.targets[].policies.requestHeaderModifier.add`|object||
|`mcp.targets[].policies.requestHeaderModifier.set`|object||
|`mcp.targets[].policies.requestHeaderModifier.remove`|[]string||
|`mcp.targets[].policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`mcp.targets[].policies.responseHeaderModifier.add`|object||
|`mcp.targets[].policies.responseHeaderModifier.set`|object||
|`mcp.targets[].policies.responseHeaderModifier.remove`|[]string||
|`mcp.targets[].policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`mcp.targets[].policies.requestRedirect.scheme`|string||
|`mcp.targets[].policies.requestRedirect.authority`|string||
|`mcp.targets[].policies.requestRedirect.authority.full`|string||
|`mcp.targets[].policies.requestRedirect.authority.host`|string||
|`mcp.targets[].policies.requestRedirect.authority.port`|integer||
|`mcp.targets[].policies.requestRedirect.path`|object||
|`mcp.targets[].policies.requestRedirect.path.full`|string||
|`mcp.targets[].policies.requestRedirect.path.prefix`|string||
|`mcp.targets[].policies.requestRedirect.status`|integer||
|`mcp.targets[].policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`mcp.targets[].policies.transformations.request`|object||
|`mcp.targets[].policies.transformations.request.add`|object||
|`mcp.targets[].policies.transformations.request.set`|object||
|`mcp.targets[].policies.transformations.request.remove`|[]string||
|`mcp.targets[].policies.transformations.request.body`|string||
|`mcp.targets[].policies.transformations.request.metadata`|object||
|`mcp.targets[].policies.transformations.response`|object||
|`mcp.targets[].policies.transformations.response.add`|object||
|`mcp.targets[].policies.transformations.response.set`|object||
|`mcp.targets[].policies.transformations.response.remove`|[]string||
|`mcp.targets[].policies.transformations.response.body`|string||
|`mcp.targets[].policies.transformations.response.metadata`|object||
|`mcp.targets[].policies.backendTLS`|object|Send TLS to the backend.|
|`mcp.targets[].policies.backendTLS.cert`|string||
|`mcp.targets[].policies.backendTLS.key`|string||
|`mcp.targets[].policies.backendTLS.root`|string||
|`mcp.targets[].policies.backendTLS.hostname`|string||
|`mcp.targets[].policies.backendTLS.insecure`|boolean||
|`mcp.targets[].policies.backendTLS.insecureHost`|boolean||
|`mcp.targets[].policies.backendTLS.alpn`|[]string||
|`mcp.targets[].policies.backendTLS.subjectAltNames`|[]string||
|`mcp.targets[].policies.backendAuth`|object|Authenticate to the backend.|
|`mcp.targets[].policies.backendAuth.passthrough`|object||
|`mcp.targets[].policies.backendAuth.key`|object||
|`mcp.targets[].policies.backendAuth.key.file`|string||
|`mcp.targets[].policies.backendAuth.gcp`|object||
|`mcp.targets[].policies.backendAuth.gcp.type`|string||
|`mcp.targets[].policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`mcp.targets[].policies.backendAuth.gcp.type`|string||
|`mcp.targets[].policies.backendAuth.aws`|object||
|`mcp.targets[].policies.backendAuth.aws.accessKeyId`|string||
|`mcp.targets[].policies.backendAuth.aws.secretAccessKey`|string||
|`mcp.targets[].policies.backendAuth.aws.region`|string||
|`mcp.targets[].policies.backendAuth.aws.sessionToken`|string||
|`mcp.targets[].policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`mcp.targets[].policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`mcp.targets[].policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`mcp.targets[].policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`mcp.targets[].policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`mcp.targets[].policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`mcp.targets[].policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`mcp.targets[].policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`mcp.targets[].policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`mcp.targets[].policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`mcp.targets[].policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`mcp.targets[].policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`mcp.targets[].policies.backendAuth.azure.developerImplicit`|object||
|`mcp.targets[].policies.backendAuth.azure.implicit`|object||
|`mcp.targets[].policies.http`|object|Specify HTTP settings for the backend|
|`mcp.targets[].policies.http.version`|string||
|`mcp.targets[].policies.http.requestTimeout`|string||
|`mcp.targets[].policies.tcp`|object|Specify TCP settings for the backend|
|`mcp.targets[].policies.tcp.keepalives`|object||
|`mcp.targets[].policies.tcp.keepalives.enabled`|boolean||
|`mcp.targets[].policies.tcp.keepalives.time`|string||
|`mcp.targets[].policies.tcp.keepalives.interval`|string||
|`mcp.targets[].policies.tcp.keepalives.retries`|integer||
|`mcp.targets[].policies.tcp.connectTimeout`|object||
|`mcp.targets[].policies.tcp.connectTimeout.secs`|integer||
|`mcp.targets[].policies.tcp.connectTimeout.nanos`|integer||
|`mcp.targets[].policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`mcp.targets[].policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`mcp.targets[].policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`mcp.targets[].policies.health.eviction.duration`|string||
|`mcp.targets[].policies.health.eviction.restoreHealth`|number||
|`mcp.targets[].policies.health.eviction.consecutiveFailures`|integer||
|`mcp.targets[].policies.health.eviction.healthThreshold`|number||
|`mcp.targets[].policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`mcp.targets[].policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`mcp.targets[].policies.backendTunnel.proxy.service`|object||
|`mcp.targets[].policies.backendTunnel.proxy.service.name`|object||
|`mcp.targets[].policies.backendTunnel.proxy.service.name.namespace`|string||
|`mcp.targets[].policies.backendTunnel.proxy.service.name.hostname`|string||
|`mcp.targets[].policies.backendTunnel.proxy.service.port`|integer||
|`mcp.targets[].policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`mcp.targets[].policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`mcp.targets[].policies.mcpAuthorization`|object|Authorization policies for MCP access.|
|`mcp.targets[].policies.mcpAuthorization.rules`|[]string||
|`mcp.statefulMode`|string||
|`mcp.prefixMode`|string||
|`mcp.failureMode`|string|Behavior when one or more MCP targets fail to initialize or fail during fanout.<br>Defaults to `failClosed`.|
|`mcp.policies`|object||
|`mcp.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`mcp.policies.requestHeaderModifier.add`|object||
|`mcp.policies.requestHeaderModifier.set`|object||
|`mcp.policies.requestHeaderModifier.remove`|[]string||
|`mcp.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`mcp.policies.responseHeaderModifier.add`|object||
|`mcp.policies.responseHeaderModifier.set`|object||
|`mcp.policies.responseHeaderModifier.remove`|[]string||
|`mcp.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`mcp.policies.requestRedirect.scheme`|string||
|`mcp.policies.requestRedirect.authority`|string||
|`mcp.policies.requestRedirect.authority.full`|string||
|`mcp.policies.requestRedirect.authority.host`|string||
|`mcp.policies.requestRedirect.authority.port`|integer||
|`mcp.policies.requestRedirect.path`|object||
|`mcp.policies.requestRedirect.path.full`|string||
|`mcp.policies.requestRedirect.path.prefix`|string||
|`mcp.policies.requestRedirect.status`|integer||
|`mcp.policies.urlRewrite`|object|Modify the URL path or authority.|
|`mcp.policies.urlRewrite.authority`|string||
|`mcp.policies.urlRewrite.authority.full`|string||
|`mcp.policies.urlRewrite.authority.host`|string||
|`mcp.policies.urlRewrite.authority.port`|integer||
|`mcp.policies.urlRewrite.path`|object||
|`mcp.policies.urlRewrite.path.full`|string||
|`mcp.policies.urlRewrite.path.prefix`|string||
|`mcp.policies.requestMirror`|object|Mirror incoming requests to another destination.|
|`mcp.policies.requestMirror.backend`|object|Exactly one of service, host, or backend may be set.|
|`mcp.policies.requestMirror.backend.service`|object||
|`mcp.policies.requestMirror.backend.service.name`|object||
|`mcp.policies.requestMirror.backend.service.name.namespace`|string||
|`mcp.policies.requestMirror.backend.service.name.hostname`|string||
|`mcp.policies.requestMirror.backend.service.port`|integer||
|`mcp.policies.requestMirror.backend.host`|string|Hostname or IP address|
|`mcp.policies.requestMirror.backend.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`mcp.policies.requestMirror.percentage`|number||
|`mcp.policies.directResponse`|object|Directly respond to the request with a static response.|
|`mcp.policies.directResponse.body`|array||
|`mcp.policies.directResponse.status`|integer||
|`mcp.policies.cors`|object|Handle CORS preflight requests and append configured CORS headers to applicable requests.|
|`mcp.policies.cors.allowCredentials`|boolean||
|`mcp.policies.cors.allowHeaders`|[]string||
|`mcp.policies.cors.allowMethods`|[]string||
|`mcp.policies.cors.allowOrigins`|[]string||
|`mcp.policies.cors.exposeHeaders`|[]string||
|`mcp.policies.cors.maxAge`|string||
|`mcp.policies.mcpAuthorization`|object|Authorization policies for MCP access.|
|`mcp.policies.mcpAuthorization.rules`|[]string||
|`mcp.policies.authorization`|object|Authorization policies for HTTP access.|
|`mcp.policies.authorization.rules`|[]string||
|`mcp.policies.mcpAuthentication`|object|Authentication for MCP clients.|
|`mcp.policies.mcpAuthentication.issuer`|string||
|`mcp.policies.mcpAuthentication.audiences`|[]string||
|`mcp.policies.mcpAuthentication.provider`|object||
|`mcp.policies.mcpAuthentication.provider.auth0`|object||
|`mcp.policies.mcpAuthentication.provider.keycloak`|object||
|`mcp.policies.mcpAuthentication.resourceMetadata`|object||
|`mcp.policies.mcpAuthentication.jwks`|object||
|`mcp.policies.mcpAuthentication.jwks.file`|string||
|`mcp.policies.mcpAuthentication.jwks.url`|string||
|`mcp.policies.mcpAuthentication.mode`|string||
|`mcp.policies.mcpAuthentication.jwtValidationOptions`|object|JWT validation options controlling which claims must be present in a token.<br><br>The `required_claims` set specifies which RFC 7519 registered claims must<br>exist in the token payload before validation proceeds. Only the following<br>values are recognized: `exp`, `nbf`, `aud`, `iss`, `sub`. Other registered<br>claims such as `iat` and `jti` are **not** enforced by the underlying<br>`jsonwebtoken` library and will be silently ignored.<br><br>This only enforces **presence**. Standard claims like `exp` and `nbf`<br>have their values validated independently (e.g., expiry is always checked<br>when the `exp` claim is present, regardless of this setting).<br><br>Defaults to `["exp"]`.|
|`mcp.policies.mcpAuthentication.jwtValidationOptions.requiredClaims`|[]string|Claims that must be present in the token before validation.<br>Only "exp", "nbf", "aud", "iss", "sub" are enforced; others<br>(including "iat" and "jti") are ignored.<br>Defaults to ["exp"]. Use an empty list to require no claims.|
|`mcp.policies.a2a`|object|Mark this traffic as A2A to enable A2A processing and telemetry.|
|`mcp.policies.ai`|object|Mark this as LLM traffic to enable LLM processing.|
|`mcp.policies.ai.promptGuard`|object||
|`mcp.policies.ai.promptGuard.request`|[]object||
|`mcp.policies.ai.promptGuard.request[].regex`|object||
|`mcp.policies.ai.promptGuard.request[].regex.action`|string||
|`mcp.policies.ai.promptGuard.request[].regex.rules`|[]object||
|`mcp.policies.ai.promptGuard.request[].regex.rules[].builtin`|string||
|`mcp.policies.ai.promptGuard.request[].regex.rules[].pattern`|string||
|`mcp.policies.ai.promptGuard.request[].webhook`|object||
|`mcp.policies.ai.promptGuard.request[].webhook.target`|object|Exactly one of service, host, or backend may be set.|
|`mcp.policies.ai.promptGuard.request[].webhook.target.service`|object||
|`mcp.policies.ai.promptGuard.request[].webhook.target.service.name`|object||
|`mcp.policies.ai.promptGuard.request[].webhook.target.service.name.namespace`|string||
|`mcp.policies.ai.promptGuard.request[].webhook.target.service.name.hostname`|string||
|`mcp.policies.ai.promptGuard.request[].webhook.target.service.port`|integer||
|`mcp.policies.ai.promptGuard.request[].webhook.target.host`|string|Hostname or IP address|
|`mcp.policies.ai.promptGuard.request[].webhook.target.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`mcp.policies.ai.promptGuard.request[].webhook.forwardHeaderMatches`|[]object||
|`mcp.policies.ai.promptGuard.request[].webhook.forwardHeaderMatches[].name`|string||
|`mcp.policies.ai.promptGuard.request[].webhook.forwardHeaderMatches[].value`|object|Exactly one of exact or regex may be set.|
|`mcp.policies.ai.promptGuard.request[].webhook.forwardHeaderMatches[].value.exact`|string||
|`mcp.policies.ai.promptGuard.request[].webhook.forwardHeaderMatches[].value.regex`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration`|object||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.model`|string|Model to use. Defaults to `omni-moderation-latest`|
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies`|object||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.requestHeaderModifier.add`|object||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.requestHeaderModifier.set`|object||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.requestHeaderModifier.remove`|[]string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.responseHeaderModifier.add`|object||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.responseHeaderModifier.set`|object||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.responseHeaderModifier.remove`|[]string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.scheme`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.authority`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.authority.full`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.authority.host`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.authority.port`|integer||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.path`|object||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.path.full`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.path.prefix`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.requestRedirect.status`|integer||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request`|object||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request.add`|object||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request.set`|object||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request.remove`|[]string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request.body`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.transformations.request.metadata`|object||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response`|object||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response.add`|object||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response.set`|object||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response.remove`|[]string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response.body`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.transformations.response.metadata`|object||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS`|object|Send TLS to the backend.|
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.cert`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.key`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.root`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.hostname`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.insecure`|boolean||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.insecureHost`|boolean||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.alpn`|[]string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendTLS.subjectAltNames`|[]string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth`|object|Authenticate to the backend.|
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.passthrough`|object||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.key`|object||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.key.file`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.gcp`|object||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.gcp.type`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.gcp.type`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws`|object||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws.accessKeyId`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws.secretAccessKey`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws.region`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.aws.sessionToken`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.developerImplicit`|object||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendAuth.azure.implicit`|object||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.http`|object|Specify HTTP settings for the backend|
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.http.version`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.http.requestTimeout`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.tcp`|object|Specify TCP settings for the backend|
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives`|object||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives.enabled`|boolean||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives.time`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives.interval`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.tcp.keepalives.retries`|integer||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.tcp.connectTimeout`|object||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.tcp.connectTimeout.secs`|integer||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.tcp.connectTimeout.nanos`|integer||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.health.eviction.duration`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.health.eviction.restoreHealth`|number||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.health.eviction.consecutiveFailures`|integer||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.health.eviction.healthThreshold`|number||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service`|object||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service.name`|object||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service.name.namespace`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service.name.hostname`|string||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.service.port`|integer||
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`mcp.policies.ai.promptGuard.request[].openAIModeration.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails`|object|Configuration for AWS Bedrock Guardrails integration.|
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.guardrailIdentifier`|string|The unique identifier of the guardrail|
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.guardrailVersion`|string|The version of the guardrail|
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.region`|string|AWS region where the guardrail is deployed|
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies`|object|Backend policies for AWS authentication (optional, defaults to implicit AWS auth)|
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestHeaderModifier.add`|object||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestHeaderModifier.set`|object||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestHeaderModifier.remove`|[]string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.responseHeaderModifier.add`|object||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.responseHeaderModifier.set`|object||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.responseHeaderModifier.remove`|[]string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.scheme`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.authority`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.authority.full`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.authority.host`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.authority.port`|integer||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.path`|object||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.path.full`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.path.prefix`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.requestRedirect.status`|integer||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request`|object||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.add`|object||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.set`|object||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.remove`|[]string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.body`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.request.metadata`|object||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response`|object||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.add`|object||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.set`|object||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.remove`|[]string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.body`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.transformations.response.metadata`|object||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS`|object|Send TLS to the backend.|
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.cert`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.key`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.root`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.hostname`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.insecure`|boolean||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.insecureHost`|boolean||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.alpn`|[]string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTLS.subjectAltNames`|[]string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth`|object|Authenticate to the backend.|
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.passthrough`|object||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.key`|object||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.key.file`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.gcp`|object||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws`|object||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws.accessKeyId`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws.secretAccessKey`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws.region`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.aws.sessionToken`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.developerImplicit`|object||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendAuth.azure.implicit`|object||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.http`|object|Specify HTTP settings for the backend|
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.http.version`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.http.requestTimeout`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp`|object|Specify TCP settings for the backend|
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives`|object||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives.enabled`|boolean||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives.time`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives.interval`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.keepalives.retries`|integer||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.connectTimeout`|object||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.connectTimeout.secs`|integer||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.tcp.connectTimeout.nanos`|integer||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction.duration`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction.restoreHealth`|number||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction.consecutiveFailures`|integer||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.health.eviction.healthThreshold`|number||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service`|object||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.name`|object||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.namespace`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.hostname`|string||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.service.port`|integer||
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`mcp.policies.ai.promptGuard.request[].bedrockGuardrails.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`mcp.policies.ai.promptGuard.request[].googleModelArmor`|object|Configuration for Google Cloud Model Armor integration.|
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.templateId`|string|The template ID for the Model Armor configuration|
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.projectId`|string|The GCP project ID|
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.location`|string|The GCP region (default: us-central1)|
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies`|object|Backend policies for GCP authentication (optional, defaults to implicit GCP auth)|
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.requestHeaderModifier.add`|object||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.requestHeaderModifier.set`|object||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.requestHeaderModifier.remove`|[]string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.responseHeaderModifier.add`|object||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.responseHeaderModifier.set`|object||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.responseHeaderModifier.remove`|[]string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.scheme`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.authority`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.authority.full`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.authority.host`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.authority.port`|integer||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.path`|object||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.path.full`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.path.prefix`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.requestRedirect.status`|integer||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request`|object||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.add`|object||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.set`|object||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.remove`|[]string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.body`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.request.metadata`|object||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response`|object||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.add`|object||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.set`|object||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.remove`|[]string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.body`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.transformations.response.metadata`|object||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS`|object|Send TLS to the backend.|
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.cert`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.key`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.root`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.hostname`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.insecure`|boolean||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.insecureHost`|boolean||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.alpn`|[]string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTLS.subjectAltNames`|[]string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth`|object|Authenticate to the backend.|
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.passthrough`|object||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.key`|object||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.key.file`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.gcp`|object||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws`|object||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws.accessKeyId`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws.secretAccessKey`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws.region`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.aws.sessionToken`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.developerImplicit`|object||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendAuth.azure.implicit`|object||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.http`|object|Specify HTTP settings for the backend|
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.http.version`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.http.requestTimeout`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.tcp`|object|Specify TCP settings for the backend|
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives`|object||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives.enabled`|boolean||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives.time`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives.interval`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.keepalives.retries`|integer||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.connectTimeout`|object||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.connectTimeout.secs`|integer||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.tcp.connectTimeout.nanos`|integer||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.health.eviction.duration`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.health.eviction.restoreHealth`|number||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.health.eviction.consecutiveFailures`|integer||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.health.eviction.healthThreshold`|number||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service`|object||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service.name`|object||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service.name.namespace`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service.name.hostname`|string||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.service.port`|integer||
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`mcp.policies.ai.promptGuard.request[].googleModelArmor.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`mcp.policies.ai.promptGuard.request[].rejection`|object||
|`mcp.policies.ai.promptGuard.request[].rejection.body`|array||
|`mcp.policies.ai.promptGuard.request[].rejection.status`|integer||
|`mcp.policies.ai.promptGuard.request[].rejection.headers`|object|Optional headers to add, set, or remove from the rejection response|
|`mcp.policies.ai.promptGuard.request[].rejection.headers.add`|object||
|`mcp.policies.ai.promptGuard.request[].rejection.headers.set`|object||
|`mcp.policies.ai.promptGuard.request[].rejection.headers.remove`|[]string||
|`mcp.policies.ai.promptGuard.response`|[]object||
|`mcp.policies.ai.promptGuard.response[].regex`|object||
|`mcp.policies.ai.promptGuard.response[].regex.action`|string||
|`mcp.policies.ai.promptGuard.response[].regex.rules`|[]object||
|`mcp.policies.ai.promptGuard.response[].regex.rules[].builtin`|string||
|`mcp.policies.ai.promptGuard.response[].regex.rules[].pattern`|string||
|`mcp.policies.ai.promptGuard.response[].webhook`|object||
|`mcp.policies.ai.promptGuard.response[].webhook.target`|object|Exactly one of service, host, or backend may be set.|
|`mcp.policies.ai.promptGuard.response[].webhook.target.service`|object||
|`mcp.policies.ai.promptGuard.response[].webhook.target.service.name`|object||
|`mcp.policies.ai.promptGuard.response[].webhook.target.service.name.namespace`|string||
|`mcp.policies.ai.promptGuard.response[].webhook.target.service.name.hostname`|string||
|`mcp.policies.ai.promptGuard.response[].webhook.target.service.port`|integer||
|`mcp.policies.ai.promptGuard.response[].webhook.target.host`|string|Hostname or IP address|
|`mcp.policies.ai.promptGuard.response[].webhook.target.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`mcp.policies.ai.promptGuard.response[].webhook.forwardHeaderMatches`|[]object||
|`mcp.policies.ai.promptGuard.response[].webhook.forwardHeaderMatches[].name`|string||
|`mcp.policies.ai.promptGuard.response[].webhook.forwardHeaderMatches[].value`|object|Exactly one of exact or regex may be set.|
|`mcp.policies.ai.promptGuard.response[].webhook.forwardHeaderMatches[].value.exact`|string||
|`mcp.policies.ai.promptGuard.response[].webhook.forwardHeaderMatches[].value.regex`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails`|object|Configuration for AWS Bedrock Guardrails integration.|
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.guardrailIdentifier`|string|The unique identifier of the guardrail|
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.guardrailVersion`|string|The version of the guardrail|
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.region`|string|AWS region where the guardrail is deployed|
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies`|object|Backend policies for AWS authentication (optional, defaults to implicit AWS auth)|
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestHeaderModifier.add`|object||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestHeaderModifier.set`|object||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestHeaderModifier.remove`|[]string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.responseHeaderModifier.add`|object||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.responseHeaderModifier.set`|object||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.responseHeaderModifier.remove`|[]string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.scheme`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.authority`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.authority.full`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.authority.host`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.authority.port`|integer||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.path`|object||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.path.full`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.path.prefix`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.requestRedirect.status`|integer||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request`|object||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.add`|object||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.set`|object||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.remove`|[]string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.body`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.request.metadata`|object||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response`|object||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.add`|object||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.set`|object||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.remove`|[]string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.body`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.transformations.response.metadata`|object||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS`|object|Send TLS to the backend.|
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.cert`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.key`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.root`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.hostname`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.insecure`|boolean||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.insecureHost`|boolean||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.alpn`|[]string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTLS.subjectAltNames`|[]string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth`|object|Authenticate to the backend.|
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.passthrough`|object||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.key`|object||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.key.file`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.gcp`|object||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.gcp.type`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws`|object||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws.accessKeyId`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws.secretAccessKey`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws.region`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.aws.sessionToken`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.developerImplicit`|object||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendAuth.azure.implicit`|object||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.http`|object|Specify HTTP settings for the backend|
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.http.version`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.http.requestTimeout`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp`|object|Specify TCP settings for the backend|
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives`|object||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives.enabled`|boolean||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives.time`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives.interval`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.keepalives.retries`|integer||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.connectTimeout`|object||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.connectTimeout.secs`|integer||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.tcp.connectTimeout.nanos`|integer||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction.duration`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction.restoreHealth`|number||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction.consecutiveFailures`|integer||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.health.eviction.healthThreshold`|number||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service`|object||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.name`|object||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.namespace`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.name.hostname`|string||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.service.port`|integer||
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`mcp.policies.ai.promptGuard.response[].bedrockGuardrails.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`mcp.policies.ai.promptGuard.response[].googleModelArmor`|object|Configuration for Google Cloud Model Armor integration.|
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.templateId`|string|The template ID for the Model Armor configuration|
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.projectId`|string|The GCP project ID|
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.location`|string|The GCP region (default: us-central1)|
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies`|object|Backend policies for GCP authentication (optional, defaults to implicit GCP auth)|
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.requestHeaderModifier.add`|object||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.requestHeaderModifier.set`|object||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.requestHeaderModifier.remove`|[]string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.responseHeaderModifier.add`|object||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.responseHeaderModifier.set`|object||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.responseHeaderModifier.remove`|[]string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.scheme`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.authority`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.authority.full`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.authority.host`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.authority.port`|integer||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.path`|object||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.path.full`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.path.prefix`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.requestRedirect.status`|integer||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request`|object||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.add`|object||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.set`|object||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.remove`|[]string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.body`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.request.metadata`|object||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response`|object||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.add`|object||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.set`|object||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.remove`|[]string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.body`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.transformations.response.metadata`|object||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS`|object|Send TLS to the backend.|
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.cert`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.key`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.root`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.hostname`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.insecure`|boolean||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.insecureHost`|boolean||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.alpn`|[]string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTLS.subjectAltNames`|[]string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth`|object|Authenticate to the backend.|
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.passthrough`|object||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.key`|object||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.key.file`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.gcp`|object||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.gcp.type`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws`|object||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws.accessKeyId`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws.secretAccessKey`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws.region`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.aws.sessionToken`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.developerImplicit`|object||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendAuth.azure.implicit`|object||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.http`|object|Specify HTTP settings for the backend|
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.http.version`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.http.requestTimeout`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.tcp`|object|Specify TCP settings for the backend|
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives`|object||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives.enabled`|boolean||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives.time`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives.interval`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.keepalives.retries`|integer||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.connectTimeout`|object||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.connectTimeout.secs`|integer||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.tcp.connectTimeout.nanos`|integer||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.health.eviction.duration`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.health.eviction.restoreHealth`|number||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.health.eviction.consecutiveFailures`|integer||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.health.eviction.healthThreshold`|number||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service`|object||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service.name`|object||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service.name.namespace`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service.name.hostname`|string||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.service.port`|integer||
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`mcp.policies.ai.promptGuard.response[].googleModelArmor.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`mcp.policies.ai.promptGuard.response[].rejection`|object||
|`mcp.policies.ai.promptGuard.response[].rejection.body`|array||
|`mcp.policies.ai.promptGuard.response[].rejection.status`|integer||
|`mcp.policies.ai.promptGuard.response[].rejection.headers`|object|Optional headers to add, set, or remove from the rejection response|
|`mcp.policies.ai.promptGuard.response[].rejection.headers.add`|object||
|`mcp.policies.ai.promptGuard.response[].rejection.headers.set`|object||
|`mcp.policies.ai.promptGuard.response[].rejection.headers.remove`|[]string||
|`mcp.policies.ai.defaults`|object||
|`mcp.policies.ai.overrides`|object||
|`mcp.policies.ai.transformations`|object||
|`mcp.policies.ai.prompts`|object||
|`mcp.policies.ai.prompts.append`|[]object||
|`mcp.policies.ai.prompts.append[].role`|string||
|`mcp.policies.ai.prompts.append[].content`|string||
|`mcp.policies.ai.prompts.prepend`|[]object||
|`mcp.policies.ai.prompts.prepend[].role`|string||
|`mcp.policies.ai.prompts.prepend[].content`|string||
|`mcp.policies.ai.modelAliases`|object||
|`mcp.policies.ai.promptCaching`|object||
|`mcp.policies.ai.promptCaching.cacheSystem`|boolean||
|`mcp.policies.ai.promptCaching.cacheMessages`|boolean||
|`mcp.policies.ai.promptCaching.cacheTools`|boolean||
|`mcp.policies.ai.promptCaching.minTokens`|integer||
|`mcp.policies.ai.routes`|object||
|`mcp.policies.backendTLS`|object|Send TLS to the backend.|
|`mcp.policies.backendTLS.cert`|string||
|`mcp.policies.backendTLS.key`|string||
|`mcp.policies.backendTLS.root`|string||
|`mcp.policies.backendTLS.hostname`|string||
|`mcp.policies.backendTLS.insecure`|boolean||
|`mcp.policies.backendTLS.insecureHost`|boolean||
|`mcp.policies.backendTLS.alpn`|[]string||
|`mcp.policies.backendTLS.subjectAltNames`|[]string||
|`mcp.policies.backendTunnel`|object|Tunnel to the backend.|
|`mcp.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`mcp.policies.backendTunnel.proxy.service`|object||
|`mcp.policies.backendTunnel.proxy.service.name`|object||
|`mcp.policies.backendTunnel.proxy.service.name.namespace`|string||
|`mcp.policies.backendTunnel.proxy.service.name.hostname`|string||
|`mcp.policies.backendTunnel.proxy.service.port`|integer||
|`mcp.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`mcp.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`mcp.policies.backendAuth`|object|Authenticate to the backend.|
|`mcp.policies.backendAuth.passthrough`|object||
|`mcp.policies.backendAuth.key`|object||
|`mcp.policies.backendAuth.key.file`|string||
|`mcp.policies.backendAuth.gcp`|object||
|`mcp.policies.backendAuth.gcp.type`|string||
|`mcp.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`mcp.policies.backendAuth.gcp.type`|string||
|`mcp.policies.backendAuth.aws`|object||
|`mcp.policies.backendAuth.aws.accessKeyId`|string||
|`mcp.policies.backendAuth.aws.secretAccessKey`|string||
|`mcp.policies.backendAuth.aws.region`|string||
|`mcp.policies.backendAuth.aws.sessionToken`|string||
|`mcp.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`mcp.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`mcp.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`mcp.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`mcp.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`mcp.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`mcp.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`mcp.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`mcp.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`mcp.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`mcp.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`mcp.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`mcp.policies.backendAuth.azure.developerImplicit`|object||
|`mcp.policies.backendAuth.azure.implicit`|object||
|`mcp.policies.localRateLimit`|[]object|Rate limit incoming requests. State is kept local.|
|`mcp.policies.localRateLimit[].maxTokens`|integer||
|`mcp.policies.localRateLimit[].tokensPerFill`|integer||
|`mcp.policies.localRateLimit[].fillInterval`|string||
|`mcp.policies.localRateLimit[].type`|string||
|`mcp.policies.remoteRateLimit`|object|Rate limit incoming requests. State is managed by a remote server.|
|`mcp.policies.remoteRateLimit.service`|object||
|`mcp.policies.remoteRateLimit.service.name`|object||
|`mcp.policies.remoteRateLimit.service.name.namespace`|string||
|`mcp.policies.remoteRateLimit.service.name.hostname`|string||
|`mcp.policies.remoteRateLimit.service.port`|integer||
|`mcp.policies.remoteRateLimit.host`|string|Hostname or IP address|
|`mcp.policies.remoteRateLimit.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`mcp.policies.remoteRateLimit.domain`|string||
|`mcp.policies.remoteRateLimit.policies`|object|Policies to connect to the backend|
|`mcp.policies.remoteRateLimit.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`mcp.policies.remoteRateLimit.policies.requestHeaderModifier.add`|object||
|`mcp.policies.remoteRateLimit.policies.requestHeaderModifier.set`|object||
|`mcp.policies.remoteRateLimit.policies.requestHeaderModifier.remove`|[]string||
|`mcp.policies.remoteRateLimit.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`mcp.policies.remoteRateLimit.policies.responseHeaderModifier.add`|object||
|`mcp.policies.remoteRateLimit.policies.responseHeaderModifier.set`|object||
|`mcp.policies.remoteRateLimit.policies.responseHeaderModifier.remove`|[]string||
|`mcp.policies.remoteRateLimit.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`mcp.policies.remoteRateLimit.policies.requestRedirect.scheme`|string||
|`mcp.policies.remoteRateLimit.policies.requestRedirect.authority`|string||
|`mcp.policies.remoteRateLimit.policies.requestRedirect.authority.full`|string||
|`mcp.policies.remoteRateLimit.policies.requestRedirect.authority.host`|string||
|`mcp.policies.remoteRateLimit.policies.requestRedirect.authority.port`|integer||
|`mcp.policies.remoteRateLimit.policies.requestRedirect.path`|object||
|`mcp.policies.remoteRateLimit.policies.requestRedirect.path.full`|string||
|`mcp.policies.remoteRateLimit.policies.requestRedirect.path.prefix`|string||
|`mcp.policies.remoteRateLimit.policies.requestRedirect.status`|integer||
|`mcp.policies.remoteRateLimit.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`mcp.policies.remoteRateLimit.policies.transformations.request`|object||
|`mcp.policies.remoteRateLimit.policies.transformations.request.add`|object||
|`mcp.policies.remoteRateLimit.policies.transformations.request.set`|object||
|`mcp.policies.remoteRateLimit.policies.transformations.request.remove`|[]string||
|`mcp.policies.remoteRateLimit.policies.transformations.request.body`|string||
|`mcp.policies.remoteRateLimit.policies.transformations.request.metadata`|object||
|`mcp.policies.remoteRateLimit.policies.transformations.response`|object||
|`mcp.policies.remoteRateLimit.policies.transformations.response.add`|object||
|`mcp.policies.remoteRateLimit.policies.transformations.response.set`|object||
|`mcp.policies.remoteRateLimit.policies.transformations.response.remove`|[]string||
|`mcp.policies.remoteRateLimit.policies.transformations.response.body`|string||
|`mcp.policies.remoteRateLimit.policies.transformations.response.metadata`|object||
|`mcp.policies.remoteRateLimit.policies.backendTLS`|object|Send TLS to the backend.|
|`mcp.policies.remoteRateLimit.policies.backendTLS.cert`|string||
|`mcp.policies.remoteRateLimit.policies.backendTLS.key`|string||
|`mcp.policies.remoteRateLimit.policies.backendTLS.root`|string||
|`mcp.policies.remoteRateLimit.policies.backendTLS.hostname`|string||
|`mcp.policies.remoteRateLimit.policies.backendTLS.insecure`|boolean||
|`mcp.policies.remoteRateLimit.policies.backendTLS.insecureHost`|boolean||
|`mcp.policies.remoteRateLimit.policies.backendTLS.alpn`|[]string||
|`mcp.policies.remoteRateLimit.policies.backendTLS.subjectAltNames`|[]string||
|`mcp.policies.remoteRateLimit.policies.backendAuth`|object|Authenticate to the backend.|
|`mcp.policies.remoteRateLimit.policies.backendAuth.passthrough`|object||
|`mcp.policies.remoteRateLimit.policies.backendAuth.key`|object||
|`mcp.policies.remoteRateLimit.policies.backendAuth.key.file`|string||
|`mcp.policies.remoteRateLimit.policies.backendAuth.gcp`|object||
|`mcp.policies.remoteRateLimit.policies.backendAuth.gcp.type`|string||
|`mcp.policies.remoteRateLimit.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`mcp.policies.remoteRateLimit.policies.backendAuth.gcp.type`|string||
|`mcp.policies.remoteRateLimit.policies.backendAuth.aws`|object||
|`mcp.policies.remoteRateLimit.policies.backendAuth.aws.accessKeyId`|string||
|`mcp.policies.remoteRateLimit.policies.backendAuth.aws.secretAccessKey`|string||
|`mcp.policies.remoteRateLimit.policies.backendAuth.aws.region`|string||
|`mcp.policies.remoteRateLimit.policies.backendAuth.aws.sessionToken`|string||
|`mcp.policies.remoteRateLimit.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`mcp.policies.remoteRateLimit.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`mcp.policies.remoteRateLimit.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`mcp.policies.remoteRateLimit.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`mcp.policies.remoteRateLimit.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`mcp.policies.remoteRateLimit.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`mcp.policies.remoteRateLimit.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`mcp.policies.remoteRateLimit.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`mcp.policies.remoteRateLimit.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`mcp.policies.remoteRateLimit.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`mcp.policies.remoteRateLimit.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`mcp.policies.remoteRateLimit.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`mcp.policies.remoteRateLimit.policies.backendAuth.azure.developerImplicit`|object||
|`mcp.policies.remoteRateLimit.policies.backendAuth.azure.implicit`|object||
|`mcp.policies.remoteRateLimit.policies.http`|object|Specify HTTP settings for the backend|
|`mcp.policies.remoteRateLimit.policies.http.version`|string||
|`mcp.policies.remoteRateLimit.policies.http.requestTimeout`|string||
|`mcp.policies.remoteRateLimit.policies.tcp`|object|Specify TCP settings for the backend|
|`mcp.policies.remoteRateLimit.policies.tcp.keepalives`|object||
|`mcp.policies.remoteRateLimit.policies.tcp.keepalives.enabled`|boolean||
|`mcp.policies.remoteRateLimit.policies.tcp.keepalives.time`|string||
|`mcp.policies.remoteRateLimit.policies.tcp.keepalives.interval`|string||
|`mcp.policies.remoteRateLimit.policies.tcp.keepalives.retries`|integer||
|`mcp.policies.remoteRateLimit.policies.tcp.connectTimeout`|object||
|`mcp.policies.remoteRateLimit.policies.tcp.connectTimeout.secs`|integer||
|`mcp.policies.remoteRateLimit.policies.tcp.connectTimeout.nanos`|integer||
|`mcp.policies.remoteRateLimit.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`mcp.policies.remoteRateLimit.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`mcp.policies.remoteRateLimit.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`mcp.policies.remoteRateLimit.policies.health.eviction.duration`|string||
|`mcp.policies.remoteRateLimit.policies.health.eviction.restoreHealth`|number||
|`mcp.policies.remoteRateLimit.policies.health.eviction.consecutiveFailures`|integer||
|`mcp.policies.remoteRateLimit.policies.health.eviction.healthThreshold`|number||
|`mcp.policies.remoteRateLimit.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`mcp.policies.remoteRateLimit.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`mcp.policies.remoteRateLimit.policies.backendTunnel.proxy.service`|object||
|`mcp.policies.remoteRateLimit.policies.backendTunnel.proxy.service.name`|object||
|`mcp.policies.remoteRateLimit.policies.backendTunnel.proxy.service.name.namespace`|string||
|`mcp.policies.remoteRateLimit.policies.backendTunnel.proxy.service.name.hostname`|string||
|`mcp.policies.remoteRateLimit.policies.backendTunnel.proxy.service.port`|integer||
|`mcp.policies.remoteRateLimit.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`mcp.policies.remoteRateLimit.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`mcp.policies.remoteRateLimit.descriptors`|[]object||
|`mcp.policies.remoteRateLimit.descriptors[].entries`|[]object||
|`mcp.policies.remoteRateLimit.descriptors[].entries[].key`|string||
|`mcp.policies.remoteRateLimit.descriptors[].entries[].value`|string||
|`mcp.policies.remoteRateLimit.descriptors[].type`|string||
|`mcp.policies.remoteRateLimit.failureMode`|string|Behavior when the remote rate limit service is unavailable or returns an error.<br>Defaults to failClosed, denying requests with a 500 status on service failure.|
|`mcp.policies.jwtAuth`|object|Authenticate incoming JWT requests.|
|`mcp.policies.jwtAuth.mode`|string||
|`mcp.policies.jwtAuth.providers`|[]object||
|`mcp.policies.jwtAuth.providers[].issuer`|string||
|`mcp.policies.jwtAuth.providers[].audiences`|[]string||
|`mcp.policies.jwtAuth.providers[].jwks`|object||
|`mcp.policies.jwtAuth.providers[].jwks.file`|string||
|`mcp.policies.jwtAuth.providers[].jwks.url`|string||
|`mcp.policies.jwtAuth.providers[].jwtValidationOptions`|object|JWT validation options controlling which claims must be present in a token.<br><br>The `required_claims` set specifies which RFC 7519 registered claims must<br>exist in the token payload before validation proceeds. Only the following<br>values are recognized: `exp`, `nbf`, `aud`, `iss`, `sub`. Other registered<br>claims such as `iat` and `jti` are **not** enforced by the underlying<br>`jsonwebtoken` library and will be silently ignored.<br><br>This only enforces **presence**. Standard claims like `exp` and `nbf`<br>have their values validated independently (e.g., expiry is always checked<br>when the `exp` claim is present, regardless of this setting).<br><br>Defaults to `["exp"]`.|
|`mcp.policies.jwtAuth.providers[].jwtValidationOptions.requiredClaims`|[]string|Claims that must be present in the token before validation.<br>Only "exp", "nbf", "aud", "iss", "sub" are enforced; others<br>(including "iat" and "jti") are ignored.<br>Defaults to ["exp"]. Use an empty list to require no claims.|
|`mcp.policies.jwtAuth.mode`|string||
|`mcp.policies.jwtAuth.issuer`|string||
|`mcp.policies.jwtAuth.audiences`|[]string||
|`mcp.policies.jwtAuth.jwks`|object||
|`mcp.policies.jwtAuth.jwks.file`|string||
|`mcp.policies.jwtAuth.jwks.url`|string||
|`mcp.policies.jwtAuth.jwtValidationOptions`|object|JWT validation options controlling which claims must be present in a token.<br><br>The `required_claims` set specifies which RFC 7519 registered claims must<br>exist in the token payload before validation proceeds. Only the following<br>values are recognized: `exp`, `nbf`, `aud`, `iss`, `sub`. Other registered<br>claims such as `iat` and `jti` are **not** enforced by the underlying<br>`jsonwebtoken` library and will be silently ignored.<br><br>This only enforces **presence**. Standard claims like `exp` and `nbf`<br>have their values validated independently (e.g., expiry is always checked<br>when the `exp` claim is present, regardless of this setting).<br><br>Defaults to `["exp"]`.|
|`mcp.policies.jwtAuth.jwtValidationOptions.requiredClaims`|[]string|Claims that must be present in the token before validation.<br>Only "exp", "nbf", "aud", "iss", "sub" are enforced; others<br>(including "iat" and "jti") are ignored.<br>Defaults to ["exp"]. Use an empty list to require no claims.|
|`mcp.policies.oidc`|object|Authenticate incoming browser requests with OIDC authorization code flow.|
|`mcp.policies.oidc.issuer`|string|Issuer used for discovery and ID token validation.|
|`mcp.policies.oidc.discovery`|object|Optional discovery document override. If omitted, discovery uses<br>`${issuer}/.well-known/openid-configuration`.|
|`mcp.policies.oidc.discovery.file`|string||
|`mcp.policies.oidc.discovery.url`|string||
|`mcp.policies.oidc.authorizationEndpoint`|string|Authorization endpoint used to start the browser login flow.|
|`mcp.policies.oidc.tokenEndpoint`|string|Token endpoint used to exchange the authorization code.|
|`mcp.policies.oidc.tokenEndpointAuth`|string|Token endpoint client authentication method for explicit provider configuration.<br><br>Discovery mode derives this from provider metadata. Explicit mode defaults to<br>`clientSecretBasic` when omitted.|
|`mcp.policies.oidc.jwks`|object|JWKS source used to validate returned ID tokens.|
|`mcp.policies.oidc.jwks.file`|string||
|`mcp.policies.oidc.jwks.url`|string||
|`mcp.policies.oidc.clientId`|string|OAuth2 client identifier used for authorization and token exchange.|
|`mcp.policies.oidc.clientSecret`|string|OAuth2 client secret used for token exchange.|
|`mcp.policies.oidc.redirectURI`|string|Absolute callback URI handled by the gateway.<br>This policy always redirects unauthenticated non-callback requests back through this login<br>flow.|
|`mcp.policies.oidc.scopes`|[]string|Additional OAuth2 scopes to request. `openid` is always included.|
|`mcp.policies.basicAuth`|object|Authenticate incoming requests using Basic Authentication with htpasswd.|
|`mcp.policies.basicAuth.htpasswd`|object|.htpasswd file contents/reference|
|`mcp.policies.basicAuth.htpasswd.file`|string||
|`mcp.policies.basicAuth.realm`|string|Realm name for the WWW-Authenticate header|
|`mcp.policies.basicAuth.mode`|string|Validation mode for basic authentication|
|`mcp.policies.apiKey`|object|Authenticate incoming requests using API Keys|
|`mcp.policies.apiKey.keys`|[]object|List of API keys|
|`mcp.policies.apiKey.keys[].key`|string||
|`mcp.policies.apiKey.keys[].metadata`|any||
|`mcp.policies.apiKey.mode`|string|Validation mode for API keys|
|`mcp.policies.extAuthz`|object|Authenticate incoming requests by calling an external authorization server.|
|`mcp.policies.extAuthz.service`|object||
|`mcp.policies.extAuthz.service.name`|object||
|`mcp.policies.extAuthz.service.name.namespace`|string||
|`mcp.policies.extAuthz.service.name.hostname`|string||
|`mcp.policies.extAuthz.service.port`|integer||
|`mcp.policies.extAuthz.host`|string|Hostname or IP address|
|`mcp.policies.extAuthz.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`mcp.policies.extAuthz.policies`|object|Policies to connect to the backend|
|`mcp.policies.extAuthz.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`mcp.policies.extAuthz.policies.requestHeaderModifier.add`|object||
|`mcp.policies.extAuthz.policies.requestHeaderModifier.set`|object||
|`mcp.policies.extAuthz.policies.requestHeaderModifier.remove`|[]string||
|`mcp.policies.extAuthz.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`mcp.policies.extAuthz.policies.responseHeaderModifier.add`|object||
|`mcp.policies.extAuthz.policies.responseHeaderModifier.set`|object||
|`mcp.policies.extAuthz.policies.responseHeaderModifier.remove`|[]string||
|`mcp.policies.extAuthz.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`mcp.policies.extAuthz.policies.requestRedirect.scheme`|string||
|`mcp.policies.extAuthz.policies.requestRedirect.authority`|string||
|`mcp.policies.extAuthz.policies.requestRedirect.authority.full`|string||
|`mcp.policies.extAuthz.policies.requestRedirect.authority.host`|string||
|`mcp.policies.extAuthz.policies.requestRedirect.authority.port`|integer||
|`mcp.policies.extAuthz.policies.requestRedirect.path`|object||
|`mcp.policies.extAuthz.policies.requestRedirect.path.full`|string||
|`mcp.policies.extAuthz.policies.requestRedirect.path.prefix`|string||
|`mcp.policies.extAuthz.policies.requestRedirect.status`|integer||
|`mcp.policies.extAuthz.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`mcp.policies.extAuthz.policies.transformations.request`|object||
|`mcp.policies.extAuthz.policies.transformations.request.add`|object||
|`mcp.policies.extAuthz.policies.transformations.request.set`|object||
|`mcp.policies.extAuthz.policies.transformations.request.remove`|[]string||
|`mcp.policies.extAuthz.policies.transformations.request.body`|string||
|`mcp.policies.extAuthz.policies.transformations.request.metadata`|object||
|`mcp.policies.extAuthz.policies.transformations.response`|object||
|`mcp.policies.extAuthz.policies.transformations.response.add`|object||
|`mcp.policies.extAuthz.policies.transformations.response.set`|object||
|`mcp.policies.extAuthz.policies.transformations.response.remove`|[]string||
|`mcp.policies.extAuthz.policies.transformations.response.body`|string||
|`mcp.policies.extAuthz.policies.transformations.response.metadata`|object||
|`mcp.policies.extAuthz.policies.backendTLS`|object|Send TLS to the backend.|
|`mcp.policies.extAuthz.policies.backendTLS.cert`|string||
|`mcp.policies.extAuthz.policies.backendTLS.key`|string||
|`mcp.policies.extAuthz.policies.backendTLS.root`|string||
|`mcp.policies.extAuthz.policies.backendTLS.hostname`|string||
|`mcp.policies.extAuthz.policies.backendTLS.insecure`|boolean||
|`mcp.policies.extAuthz.policies.backendTLS.insecureHost`|boolean||
|`mcp.policies.extAuthz.policies.backendTLS.alpn`|[]string||
|`mcp.policies.extAuthz.policies.backendTLS.subjectAltNames`|[]string||
|`mcp.policies.extAuthz.policies.backendAuth`|object|Authenticate to the backend.|
|`mcp.policies.extAuthz.policies.backendAuth.passthrough`|object||
|`mcp.policies.extAuthz.policies.backendAuth.key`|object||
|`mcp.policies.extAuthz.policies.backendAuth.key.file`|string||
|`mcp.policies.extAuthz.policies.backendAuth.gcp`|object||
|`mcp.policies.extAuthz.policies.backendAuth.gcp.type`|string||
|`mcp.policies.extAuthz.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`mcp.policies.extAuthz.policies.backendAuth.gcp.type`|string||
|`mcp.policies.extAuthz.policies.backendAuth.aws`|object||
|`mcp.policies.extAuthz.policies.backendAuth.aws.accessKeyId`|string||
|`mcp.policies.extAuthz.policies.backendAuth.aws.secretAccessKey`|string||
|`mcp.policies.extAuthz.policies.backendAuth.aws.region`|string||
|`mcp.policies.extAuthz.policies.backendAuth.aws.sessionToken`|string||
|`mcp.policies.extAuthz.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`mcp.policies.extAuthz.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`mcp.policies.extAuthz.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`mcp.policies.extAuthz.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`mcp.policies.extAuthz.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`mcp.policies.extAuthz.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`mcp.policies.extAuthz.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`mcp.policies.extAuthz.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`mcp.policies.extAuthz.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`mcp.policies.extAuthz.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`mcp.policies.extAuthz.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`mcp.policies.extAuthz.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`mcp.policies.extAuthz.policies.backendAuth.azure.developerImplicit`|object||
|`mcp.policies.extAuthz.policies.backendAuth.azure.implicit`|object||
|`mcp.policies.extAuthz.policies.http`|object|Specify HTTP settings for the backend|
|`mcp.policies.extAuthz.policies.http.version`|string||
|`mcp.policies.extAuthz.policies.http.requestTimeout`|string||
|`mcp.policies.extAuthz.policies.tcp`|object|Specify TCP settings for the backend|
|`mcp.policies.extAuthz.policies.tcp.keepalives`|object||
|`mcp.policies.extAuthz.policies.tcp.keepalives.enabled`|boolean||
|`mcp.policies.extAuthz.policies.tcp.keepalives.time`|string||
|`mcp.policies.extAuthz.policies.tcp.keepalives.interval`|string||
|`mcp.policies.extAuthz.policies.tcp.keepalives.retries`|integer||
|`mcp.policies.extAuthz.policies.tcp.connectTimeout`|object||
|`mcp.policies.extAuthz.policies.tcp.connectTimeout.secs`|integer||
|`mcp.policies.extAuthz.policies.tcp.connectTimeout.nanos`|integer||
|`mcp.policies.extAuthz.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`mcp.policies.extAuthz.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`mcp.policies.extAuthz.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`mcp.policies.extAuthz.policies.health.eviction.duration`|string||
|`mcp.policies.extAuthz.policies.health.eviction.restoreHealth`|number||
|`mcp.policies.extAuthz.policies.health.eviction.consecutiveFailures`|integer||
|`mcp.policies.extAuthz.policies.health.eviction.healthThreshold`|number||
|`mcp.policies.extAuthz.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`mcp.policies.extAuthz.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`mcp.policies.extAuthz.policies.backendTunnel.proxy.service`|object||
|`mcp.policies.extAuthz.policies.backendTunnel.proxy.service.name`|object||
|`mcp.policies.extAuthz.policies.backendTunnel.proxy.service.name.namespace`|string||
|`mcp.policies.extAuthz.policies.backendTunnel.proxy.service.name.hostname`|string||
|`mcp.policies.extAuthz.policies.backendTunnel.proxy.service.port`|integer||
|`mcp.policies.extAuthz.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`mcp.policies.extAuthz.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`mcp.policies.extAuthz.protocol`|object|The ext_authz protocol to use. Unless you need to integrate with an HTTP-only server, gRPC is recommended.<br>Exactly one of grpc or http may be set.|
|`mcp.policies.extAuthz.protocol.grpc`|object||
|`mcp.policies.extAuthz.protocol.grpc.context`|object|Additional context to send to the authorization service.<br>This maps to the `context_extensions` field of the request, and only allows static values.|
|`mcp.policies.extAuthz.protocol.grpc.metadata`|object|Additional metadata to send to the authorization service.<br>This maps to the `metadata_context.filter_metadata` field of the request, and allows dynamic CEL expressions.<br>If unset, by default the `envoy.filters.http.jwt_authn` key is set if the JWT policy is used as well, for compatibility.|
|`mcp.policies.extAuthz.protocol.http`|object||
|`mcp.policies.extAuthz.protocol.http.path`|string||
|`mcp.policies.extAuthz.protocol.http.redirect`|string|When using the HTTP protocol, and the server returns unauthorized, redirect to the URL resolved by<br>the provided expression rather than directly returning the error.|
|`mcp.policies.extAuthz.protocol.http.includeResponseHeaders`|[]string|Specific headers from the authorization response will be copied into the request to the backend.|
|`mcp.policies.extAuthz.protocol.http.addRequestHeaders`|object|Specific headers to add in the authorization request (empty = all headers), based on the expression|
|`mcp.policies.extAuthz.protocol.http.metadata`|object|Metadata to include under the `extauthz` variable, based on the authorization response.|
|`mcp.policies.extAuthz.failureMode`|string|Behavior when the authorization service is unavailable or returns an error|
|`mcp.policies.extAuthz.failureMode.denyWithStatus`|integer||
|`mcp.policies.extAuthz.includeRequestHeaders`|[]string|Specific headers to include in the authorization request.<br>If unset, the gRPC protocol sends all request headers. The HTTP protocol sends only 'Authorization'.|
|`mcp.policies.extAuthz.includeRequestBody`|object|Options for including the request body in the authorization request|
|`mcp.policies.extAuthz.includeRequestBody.maxRequestBytes`|integer|Maximum size of request body to buffer (default: 8192)|
|`mcp.policies.extAuthz.includeRequestBody.allowPartialMessage`|boolean|If true, send partial body when max_request_bytes is reached|
|`mcp.policies.extAuthz.includeRequestBody.packAsBytes`|boolean|If true, pack body as raw bytes in gRPC|
|`mcp.policies.extProc`|object|Extend agentgateway with an external processor|
|`mcp.policies.extProc.service`|object||
|`mcp.policies.extProc.service.name`|object||
|`mcp.policies.extProc.service.name.namespace`|string||
|`mcp.policies.extProc.service.name.hostname`|string||
|`mcp.policies.extProc.service.port`|integer||
|`mcp.policies.extProc.host`|string|Hostname or IP address|
|`mcp.policies.extProc.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`mcp.policies.extProc.policies`|object|Policies to connect to the backend|
|`mcp.policies.extProc.policies.requestHeaderModifier`|object|Headers to be modified in the request.|
|`mcp.policies.extProc.policies.requestHeaderModifier.add`|object||
|`mcp.policies.extProc.policies.requestHeaderModifier.set`|object||
|`mcp.policies.extProc.policies.requestHeaderModifier.remove`|[]string||
|`mcp.policies.extProc.policies.responseHeaderModifier`|object|Headers to be modified in the response.|
|`mcp.policies.extProc.policies.responseHeaderModifier.add`|object||
|`mcp.policies.extProc.policies.responseHeaderModifier.set`|object||
|`mcp.policies.extProc.policies.responseHeaderModifier.remove`|[]string||
|`mcp.policies.extProc.policies.requestRedirect`|object|Directly respond to the request with a redirect.|
|`mcp.policies.extProc.policies.requestRedirect.scheme`|string||
|`mcp.policies.extProc.policies.requestRedirect.authority`|string||
|`mcp.policies.extProc.policies.requestRedirect.authority.full`|string||
|`mcp.policies.extProc.policies.requestRedirect.authority.host`|string||
|`mcp.policies.extProc.policies.requestRedirect.authority.port`|integer||
|`mcp.policies.extProc.policies.requestRedirect.path`|object||
|`mcp.policies.extProc.policies.requestRedirect.path.full`|string||
|`mcp.policies.extProc.policies.requestRedirect.path.prefix`|string||
|`mcp.policies.extProc.policies.requestRedirect.status`|integer||
|`mcp.policies.extProc.policies.transformations`|object|Modify requests and responses sent to and from the backend.|
|`mcp.policies.extProc.policies.transformations.request`|object||
|`mcp.policies.extProc.policies.transformations.request.add`|object||
|`mcp.policies.extProc.policies.transformations.request.set`|object||
|`mcp.policies.extProc.policies.transformations.request.remove`|[]string||
|`mcp.policies.extProc.policies.transformations.request.body`|string||
|`mcp.policies.extProc.policies.transformations.request.metadata`|object||
|`mcp.policies.extProc.policies.transformations.response`|object||
|`mcp.policies.extProc.policies.transformations.response.add`|object||
|`mcp.policies.extProc.policies.transformations.response.set`|object||
|`mcp.policies.extProc.policies.transformations.response.remove`|[]string||
|`mcp.policies.extProc.policies.transformations.response.body`|string||
|`mcp.policies.extProc.policies.transformations.response.metadata`|object||
|`mcp.policies.extProc.policies.backendTLS`|object|Send TLS to the backend.|
|`mcp.policies.extProc.policies.backendTLS.cert`|string||
|`mcp.policies.extProc.policies.backendTLS.key`|string||
|`mcp.policies.extProc.policies.backendTLS.root`|string||
|`mcp.policies.extProc.policies.backendTLS.hostname`|string||
|`mcp.policies.extProc.policies.backendTLS.insecure`|boolean||
|`mcp.policies.extProc.policies.backendTLS.insecureHost`|boolean||
|`mcp.policies.extProc.policies.backendTLS.alpn`|[]string||
|`mcp.policies.extProc.policies.backendTLS.subjectAltNames`|[]string||
|`mcp.policies.extProc.policies.backendAuth`|object|Authenticate to the backend.|
|`mcp.policies.extProc.policies.backendAuth.passthrough`|object||
|`mcp.policies.extProc.policies.backendAuth.key`|object||
|`mcp.policies.extProc.policies.backendAuth.key.file`|string||
|`mcp.policies.extProc.policies.backendAuth.gcp`|object||
|`mcp.policies.extProc.policies.backendAuth.gcp.type`|string||
|`mcp.policies.extProc.policies.backendAuth.gcp.audience`|string|Audience for the token. If not set, the destination host will be used.|
|`mcp.policies.extProc.policies.backendAuth.gcp.type`|string||
|`mcp.policies.extProc.policies.backendAuth.aws`|object||
|`mcp.policies.extProc.policies.backendAuth.aws.accessKeyId`|string||
|`mcp.policies.extProc.policies.backendAuth.aws.secretAccessKey`|string||
|`mcp.policies.extProc.policies.backendAuth.aws.region`|string||
|`mcp.policies.extProc.policies.backendAuth.aws.sessionToken`|string||
|`mcp.policies.extProc.policies.backendAuth.azure`|object|Exactly one of explicitConfig, developerImplicit, or implicit may be set.|
|`mcp.policies.extProc.policies.backendAuth.azure.explicitConfig`|object|Exactly one of clientSecret, managedIdentity, or workloadIdentity may be set.|
|`mcp.policies.extProc.policies.backendAuth.azure.explicitConfig.clientSecret`|object||
|`mcp.policies.extProc.policies.backendAuth.azure.explicitConfig.clientSecret.tenant_id`|string||
|`mcp.policies.extProc.policies.backendAuth.azure.explicitConfig.clientSecret.client_id`|string||
|`mcp.policies.extProc.policies.backendAuth.azure.explicitConfig.clientSecret.client_secret`|string||
|`mcp.policies.extProc.policies.backendAuth.azure.explicitConfig.managedIdentity`|object||
|`mcp.policies.extProc.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity`|object||
|`mcp.policies.extProc.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.clientId`|string||
|`mcp.policies.extProc.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.objectId`|string||
|`mcp.policies.extProc.policies.backendAuth.azure.explicitConfig.managedIdentity.userAssignedIdentity.resourceId`|string||
|`mcp.policies.extProc.policies.backendAuth.azure.explicitConfig.workloadIdentity`|object||
|`mcp.policies.extProc.policies.backendAuth.azure.developerImplicit`|object||
|`mcp.policies.extProc.policies.backendAuth.azure.implicit`|object||
|`mcp.policies.extProc.policies.http`|object|Specify HTTP settings for the backend|
|`mcp.policies.extProc.policies.http.version`|string||
|`mcp.policies.extProc.policies.http.requestTimeout`|string||
|`mcp.policies.extProc.policies.tcp`|object|Specify TCP settings for the backend|
|`mcp.policies.extProc.policies.tcp.keepalives`|object||
|`mcp.policies.extProc.policies.tcp.keepalives.enabled`|boolean||
|`mcp.policies.extProc.policies.tcp.keepalives.time`|string||
|`mcp.policies.extProc.policies.tcp.keepalives.interval`|string||
|`mcp.policies.extProc.policies.tcp.keepalives.retries`|integer||
|`mcp.policies.extProc.policies.tcp.connectTimeout`|object||
|`mcp.policies.extProc.policies.tcp.connectTimeout.secs`|integer||
|`mcp.policies.extProc.policies.tcp.connectTimeout.nanos`|integer||
|`mcp.policies.extProc.policies.health`|object|Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.|
|`mcp.policies.extProc.policies.health.unhealthyExpression`|string|CEL expression; `true` means unhealthy (evict). E.g. `response.code >= 500`.<br>When unset, any 5xx or connection failure is treated as unhealthy.|
|`mcp.policies.extProc.policies.health.eviction`|object|Local/config eviction sub-policy with duration as string; mirrors `Eviction`.|
|`mcp.policies.extProc.policies.health.eviction.duration`|string||
|`mcp.policies.extProc.policies.health.eviction.restoreHealth`|number||
|`mcp.policies.extProc.policies.health.eviction.consecutiveFailures`|integer||
|`mcp.policies.extProc.policies.health.eviction.healthThreshold`|number||
|`mcp.policies.extProc.policies.backendTunnel`|object|Specify a tunnel to use when connecting to the backend|
|`mcp.policies.extProc.policies.backendTunnel.proxy`|object|Reference to the proxy address<br>Exactly one of service, host, or backend may be set.|
|`mcp.policies.extProc.policies.backendTunnel.proxy.service`|object||
|`mcp.policies.extProc.policies.backendTunnel.proxy.service.name`|object||
|`mcp.policies.extProc.policies.backendTunnel.proxy.service.name.namespace`|string||
|`mcp.policies.extProc.policies.backendTunnel.proxy.service.name.hostname`|string||
|`mcp.policies.extProc.policies.backendTunnel.proxy.service.port`|integer||
|`mcp.policies.extProc.policies.backendTunnel.proxy.host`|string|Hostname or IP address|
|`mcp.policies.extProc.policies.backendTunnel.proxy.backend`|string|Explicit backend reference. Backend must be defined in the top level backends list|
|`mcp.policies.extProc.failureMode`|string|Behavior when the ext_proc service is unavailable or returns an error|
|`mcp.policies.extProc.metadataContext`|object|Additional metadata to send to the external processing service.<br>Maps to the `metadata_context.filter_metadata` field in ProcessingRequest, and allows dynamic CEL expressions.|
|`mcp.policies.extProc.requestAttributes`|object|Maps to the request `attributes` field in ProcessingRequest, and allows dynamic CEL expressions.|
|`mcp.policies.extProc.responseAttributes`|object|Maps to the response `attributes` field in ProcessingRequest, and allows dynamic CEL expressions.|
|`mcp.policies.transformations`|object|Modify requests and responses|
|`mcp.policies.transformations.request`|object||
|`mcp.policies.transformations.request.add`|object||
|`mcp.policies.transformations.request.set`|object||
|`mcp.policies.transformations.request.remove`|[]string||
|`mcp.policies.transformations.request.body`|string||
|`mcp.policies.transformations.request.metadata`|object||
|`mcp.policies.transformations.response`|object||
|`mcp.policies.transformations.response.add`|object||
|`mcp.policies.transformations.response.set`|object||
|`mcp.policies.transformations.response.remove`|[]string||
|`mcp.policies.transformations.response.body`|string||
|`mcp.policies.transformations.response.metadata`|object||
|`mcp.policies.csrf`|object|Handle CSRF protection by validating request origins against configured allowed origins.|
|`mcp.policies.csrf.additionalOrigins`|[]string||
|`mcp.policies.timeout`|object|Timeout requests that exceed the configured duration.|
|`mcp.policies.timeout.requestTimeout`|string||
|`mcp.policies.timeout.backendRequestTimeout`|string||
|`mcp.policies.retry`|object|Retry matching requests.|
|`mcp.policies.retry.attempts`|integer||
|`mcp.policies.retry.backoff`|string||
|`mcp.policies.retry.codes`|[]integer||
