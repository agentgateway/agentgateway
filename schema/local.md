|Field|Column|
|-|-|
|`binds`||
|`binds[].listeners`||
|`binds[].listeners[].gatewayName`||
|`binds[].listeners[].hostname`|Can be a wildcard|
|`binds[].listeners[].name`||
|`binds[].listeners[].protocol`||
|`binds[].listeners[].routes`||
|`binds[].listeners[].routes[].backends`||
|`binds[].listeners[].routes[].backends[].weight`||
|`binds[].listeners[].routes[].hostnames`|Can be a wildcard|
|`binds[].listeners[].routes[].matches`||
|`binds[].listeners[].routes[].matches[].headers`||
|`binds[].listeners[].routes[].matches[].headers[].name`||
|`binds[].listeners[].routes[].matches[].headers[].value`||
|`binds[].listeners[].routes[].matches[].method`||
|`binds[].listeners[].routes[].matches[].path`||
|`binds[].listeners[].routes[].matches[].query`||
|`binds[].listeners[].routes[].matches[].query[].name`||
|`binds[].listeners[].routes[].matches[].query[].value`||
|`binds[].listeners[].routes[].name`||
|`binds[].listeners[].routes[].policies`||
|`binds[].listeners[].routes[].policies.a2a`|Mark this traffic as A2A to enable A2A processing and telemetry.|
|`binds[].listeners[].routes[].policies.ai`|Mark this as LLM traffic to enable LLM processing.|
|`binds[].listeners[].routes[].policies.backendAuth`|Authenticate to the backend.|
|`binds[].listeners[].routes[].policies.backendTLS`|Send TLS to the backend.|
|`binds[].listeners[].routes[].policies.backendTLS.cert`||
|`binds[].listeners[].routes[].policies.backendTLS.insecure`||
|`binds[].listeners[].routes[].policies.backendTLS.insecureHost`||
|`binds[].listeners[].routes[].policies.backendTLS.key`||
|`binds[].listeners[].routes[].policies.backendTLS.root`||
|`binds[].listeners[].routes[].policies.cors`|Handle CORS preflight requests and append configured CORS headers to applicable requests.|
|`binds[].listeners[].routes[].policies.cors.allowCredentials`||
|`binds[].listeners[].routes[].policies.cors.allowHeaders`||
|`binds[].listeners[].routes[].policies.cors.allowMethods`||
|`binds[].listeners[].routes[].policies.cors.allowOrigins`||
|`binds[].listeners[].routes[].policies.cors.exposeHeaders`||
|`binds[].listeners[].routes[].policies.cors.maxAge`||
|`binds[].listeners[].routes[].policies.directResponse`|Directly respond to the request with a static response.|
|`binds[].listeners[].routes[].policies.directResponse.body`||
|`binds[].listeners[].routes[].policies.directResponse.status`||
|`binds[].listeners[].routes[].policies.extAuthz`|Authenticate incoming requests by calling an external authorization server.|
|`binds[].listeners[].routes[].policies.jwtAuth`|Authenticate incoming JWT requests.|
|`binds[].listeners[].routes[].policies.jwtAuth.audiences`||
|`binds[].listeners[].routes[].policies.jwtAuth.issuer`||
|`binds[].listeners[].routes[].policies.jwtAuth.jwks`||
|`binds[].listeners[].routes[].policies.localRateLimit`|Rate limit incoming requests. State is kept local.|
|`binds[].listeners[].routes[].policies.mcpAuthentication`|Authentication for MCP clients.|
|`binds[].listeners[].routes[].policies.mcpAuthentication.audience`||
|`binds[].listeners[].routes[].policies.mcpAuthentication.issuer`||
|`binds[].listeners[].routes[].policies.mcpAuthentication.provider`||
|`binds[].listeners[].routes[].policies.mcpAuthentication.scopes`||
|`binds[].listeners[].routes[].policies.mcpAuthorization`|Authorization policies for MCP access.|
|`binds[].listeners[].routes[].policies.mcpAuthorization.rules`||
|`binds[].listeners[].routes[].policies.remoteRateLimit`|Rate limit incoming requests. State is managed by a remote server.|
|`binds[].listeners[].routes[].policies.requestHeaderModifier`|Headers to be modified in the request.|
|`binds[].listeners[].routes[].policies.requestHeaderModifier.add`||
|`binds[].listeners[].routes[].policies.requestHeaderModifier.remove`||
|`binds[].listeners[].routes[].policies.requestHeaderModifier.set`||
|`binds[].listeners[].routes[].policies.requestMirror`|Mirror incoming requests to another destination.|
|`binds[].listeners[].routes[].policies.requestMirror.backend`||
|`binds[].listeners[].routes[].policies.requestMirror.percentage`||
|`binds[].listeners[].routes[].policies.requestRedirect`|Directly respond to the request with a redirect.|
|`binds[].listeners[].routes[].policies.requestRedirect.authority`||
|`binds[].listeners[].routes[].policies.requestRedirect.path`||
|`binds[].listeners[].routes[].policies.requestRedirect.scheme`||
|`binds[].listeners[].routes[].policies.requestRedirect.status`||
|`binds[].listeners[].routes[].policies.responseHeaderModifier`|Headers to be modified in the response.|
|`binds[].listeners[].routes[].policies.responseHeaderModifier.add`||
|`binds[].listeners[].routes[].policies.responseHeaderModifier.remove`||
|`binds[].listeners[].routes[].policies.responseHeaderModifier.set`||
|`binds[].listeners[].routes[].policies.retry`|Retry matching requests.|
|`binds[].listeners[].routes[].policies.retry.attempts`||
|`binds[].listeners[].routes[].policies.retry.backoff`||
|`binds[].listeners[].routes[].policies.retry.codes`||
|`binds[].listeners[].routes[].policies.timeout`|Timeout requests that exceed the configured duration.|
|`binds[].listeners[].routes[].policies.timeout.backendRequestTimeout`||
|`binds[].listeners[].routes[].policies.timeout.requestTimeout`||
|`binds[].listeners[].routes[].policies.transformations`|Modify requests and responses|
|`binds[].listeners[].routes[].policies.transformations.request`||
|`binds[].listeners[].routes[].policies.transformations.request.add`||
|`binds[].listeners[].routes[].policies.transformations.request.body`||
|`binds[].listeners[].routes[].policies.transformations.request.remove`||
|`binds[].listeners[].routes[].policies.transformations.request.set`||
|`binds[].listeners[].routes[].policies.transformations.response`||
|`binds[].listeners[].routes[].policies.transformations.response.add`||
|`binds[].listeners[].routes[].policies.transformations.response.body`||
|`binds[].listeners[].routes[].policies.transformations.response.remove`||
|`binds[].listeners[].routes[].policies.transformations.response.set`||
|`binds[].listeners[].routes[].policies.urlRewrite`|Modify the URL path or authority.|
|`binds[].listeners[].routes[].policies.urlRewrite.authority`||
|`binds[].listeners[].routes[].policies.urlRewrite.path`||
|`binds[].listeners[].routes[].ruleName`||
|`binds[].listeners[].tcpRoutes`||
|`binds[].listeners[].tcpRoutes[].backends`||
|`binds[].listeners[].tcpRoutes[].backends[].backend`||
|`binds[].listeners[].tcpRoutes[].backends[].weight`||
|`binds[].listeners[].tcpRoutes[].hostnames`|Can be a wildcard|
|`binds[].listeners[].tcpRoutes[].name`||
|`binds[].listeners[].tcpRoutes[].policies`||
|`binds[].listeners[].tcpRoutes[].policies.backendTls`||
|`binds[].listeners[].tcpRoutes[].policies.backendTls.cert`||
|`binds[].listeners[].tcpRoutes[].policies.backendTls.insecure`||
|`binds[].listeners[].tcpRoutes[].policies.backendTls.insecureHost`||
|`binds[].listeners[].tcpRoutes[].policies.backendTls.key`||
|`binds[].listeners[].tcpRoutes[].policies.backendTls.root`||
|`binds[].listeners[].tcpRoutes[].ruleName`||
|`binds[].listeners[].tls`||
|`binds[].listeners[].tls.cert`||
|`binds[].listeners[].tls.key`||
|`binds[].port`||
|`config`||
|`services`||
|`workloads`||
