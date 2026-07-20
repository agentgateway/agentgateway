import type en from "./en";

type LocaleShape<T> = {
  readonly [Key in keyof T]: T[Key] extends string
    ? string
    : LocaleShape<T[Key]>;
};

const zhCN = {
  translation: {
    common: {
      apply: "应用",
      auto: "自动",
      cancel: "取消",
      close: "关闭",
      confirm: "确认",
      copied: "已复制",
      discardChanges: "放弃更改",
      noMatches: "没有匹配项",
      noOptions: "没有可选项",
      noMatchesCustomValues: "没有匹配项。可以使用自定义值。",
      noValuesFound: "未找到值。",
      notAvailable: "不适用",
      save: "保存",
      search: "搜索{{label}}",
      searchPlaceholder: "搜索{{label}}…",
      select: "选择",
      showOptions: "显示{{label}}选项",
      viewDiff: "查看差异",
    },
    dateRange: {
      apply: "应用",
      cancel: "取消",
      from: "开始时间",
      interval: "间隔",
      last1Hour: "最近 1 小时",
      last12Hours: "最近 12 小时",
      last24Hours: "最近 24 小时",
      last7Days: "最近 7 天",
      last14Days: "最近 14 天",
      last30Days: "最近 30 天",
      quickRanges: "快捷时间范围",
      to: "结束时间",
      second_one: "{{count}} 秒",
      second_other: "{{count}} 秒",
      minute_one: "{{count}} 分钟",
      minute_other: "{{count}} 分钟",
      hour_one: "{{count}} 小时",
      hour_other: "{{count}} 小时",
      day_one: "{{count}} 天",
      day_other: "{{count}} 天",
      invalidDate: "无效日期",
    },
    drawer: {
      discardUnsavedChanges: "放弃未保存的更改？",
      unsavedChangesMessage: "你的更改尚未保存，关闭后将丢失。",
    },
    schema: {
      array: "数组",
      help: "帮助",
      object: "object",
      oneOf: "以下之一：",
      required: "必填",
      value: "值",
    },
    shell: {
      clientSetup: "客户端设置",
      documentation: "文档",
      feedback: "反馈",
      gatewayOverview: "网关概览",
      llmConfiguration: "LLM 配置",
      mcpConfiguration: "MCP 配置",
      policyTools: "策略工具",
      primaryNavigation: "主导航",
      projectLinks: "项目链接",
      toggleTheme: "切换主题",
      trafficConfiguration: "流量配置",
    },
    language: {
      english: "English",
      select: "选择语言",
      simplifiedChinese: "简体中文",
    },
    nav: {
      analytics: "分析",
      celPlayground: "CEL 演练场",
      chatPlayground: "聊天演练场",
      clientSetup: "客户端设置",
      costs: "成本",
      gateway: "网关",
      gateways: "网关",
      getStarted: "开始使用",
      guardrails: "防护规则",
      home: "首页",
      keys: "虚拟 API 密钥",
      listeners: "监听器",
      llm: "LLM",
      logs: "日志",
      mcp: "MCP",
      models: "模型",
      policies: "策略",
      providers: "提供商",
      rawConfiguration: "原始配置",
      routes: "路由",
      servers: "服务器",
      settings: "设置",
      toolPlayground: "工具演练场",
      tools: "工具",
      traffic: "流量",
    },
    copy: {
      loadingLogs: "正在加载日志",
      noLogEntries: "没有日志记录",
      yModelcontextprotocolServerFilesystemTmp:
        "-y @modelcontextprotocol/server-filesystem /tmp",
      valueValueValue: ":{{value}} · {{value}} · {{value}}",
      thisCannotBeUndone: "？此操作无法撤销。",
      trafficCanNoLongerBeSentToThisTarget: "？无法再将流量发送到该目标。",
      trafficMatchingThisRouteWillNoLongerReachItsBackends:
        "？匹配该路由的流量将不再到达其后端。",
      considerMovingListenerOwnershipTo: "。考虑将监听器所有权移至",
      oauth2Auth: "“/oauth2/auth”",
      value: "{{value}}",
      valueAllValueListeners: "{{value}}（所有 {{value}} 监听器）",
      valueAllListeners: "{{value}}（所有监听器）",
      valueValueConfigured: "已配置 {{value}} 个{{value}}。",
      valueBinds_one: "{{count}} 个绑定",
      valueBinds_other: "{{count}} 个绑定",
      valueByValue: "{{value}}（按{{value}}）",
      valueConfiguredServers_one: "{{count}} 个已配置服务器",
      valueConfiguredServers_other: "{{count}} 个已配置服务器",
      valueEnabled: "{{value}} 已启用",
      valueGateways_one: "{{count}} 个网关",
      valueGateways_other: "{{count}} 个网关",
      valueListenerValueMixHttpAndTcpRoutes:
        "{{value}} 监听器{{value}} 混合 HTTP 和 TCP 路由",
      valueListeners_one: "{{count}} 个监听器",
      valueListeners_other: "{{count}} 个监听器",
      valueModels_one: "{{count}} 个模型",
      valueModels_other: "{{count}} 个模型",
      valueOf3Enabled: "已启用 {{value}} / 3 项",
      valuePathOverride: "{{value}} 路径覆盖",
      valuePolicies_one: "{{count}} 项策略",
      valuePolicies_other: "{{count}} 项策略",
      valuePrioritiesValueTargets: "{{value}} 个优先级，{{value}} 个目标",
      valueQuickKeys: "{{value}} 快捷键",
      valueRoutes_one: "{{count}} 条路由",
      valueRoutes_other: "{{count}} 条路由",
      valueRows_one: "{{count}} 行",
      valueRows_other: "{{count}} 行",
      valueRules_one: "{{count}} 条规则",
      valueRules_other: "{{count}} 条规则",
      valueRulesWithFallback_one: "{{count}} 条规则，含回退",
      valueRulesWithFallback_other: "{{count}} 条规则，含回退",
      valueSharedProviders_one: "{{count}} 个共享提供商",
      valueSharedProviders_other: "{{count}} 个共享提供商",
      valueTokensValueCalls: "{{value}} 个令牌 / {{value}} 次调用",
      valueTotal: "{{value}}总计",
      valueVirtualModels_one: "{{count}} 个虚拟模型",
      valueVirtualModels_other: "{{count}} 个虚拟模型",
      valueWarningValue_one: "{{count}} 条警告",
      valueWarningValue_other: "{{count}} 条警告",
      valueWeightedTargets_one: "{{count}} 个加权目标",
      valueWeightedTargets_other: "{{count}} 个加权目标",
      valueValue: "{{value}}/{{value}}",
      audienceParametersNamingTheTargetServicesAtTheAuthorizationServer:
        "`audience` 参数在授权服务器上命名目标服务。",
      clientIdParameterIdentifyingTheGatewayAtTheAuthorizationServer:
        "`client_id` 参数标识授权服务器上的网关。",
      clientIdClientSecretSentInTheHttpBasicAuthorizationHeaderRfc6749231:
        "`client_id`/`client_secret` 在 HTTP 基本授权请求头中发送（RFC 6749 §2.3.1）。",
      clientIdClientSecretSentInTheRequestFormBody:
        "`client_id`/`client_secret` 在请求表单正文中发送。",
      privateKeyJwtClientAssertionRfc7523:
        "`privateKeyJwt` 客户端断言 (RFC 7523)。",
      requestedTokenTypeParameterWhenUnsetTheFormFieldIsOmittedAndADeclaredResponseTyp_46odee:
        "`requested_token_type` 参数。未设置时，表单字段将被省略\n声明的响应类型预计为 access_token。",
      resourceParametersNamingTheProtectedResourceApis:
        "`resource` 命名受保护资源 API 的参数。",
      resourceParametersWithTheTargetServiceUris:
        "带有目标服务 URI 的 `resource` 参数。",
      scopeValuesForTheRequestedTokenSentSpaceDelimited:
        "所请求令牌的 `scope` 值，以空格分隔发送。",
      text200Ok: "200 好",
      text400BadRequest: "400 错误请求",
      text401Unauthorized: "401 未经授权",
      text403Forbidden: "403 禁止访问",
      text404NotFound: "404 未找到",
      text429RateLimited: "429 请求受限",
      text500ServerError: "500 服务器错误",
      aCustomProviderSAdvertisedUpstreamWireFormatUnlikeInputFormatThisDescribesWhatTh_fgckra:
        "自定义提供商声明的上游线路格式。\n\n与 `InputFormat` 不同，此处描述的是后端接受的格式，而不是客户端发送的格式。与 `RouteType` 不同，它仅适用于可以转换或透传的 LLM 负载端点；models、passthrough 和 detect 等通用路由没有 `ProviderFormat`。",
      aSourceOfModelCostCatalogData: "模型成本目录数据的来源。",
      aValidTokenIssuedByAConfiguredIssuerMustBePresentThisIsTheDefaultOption:
        "必须存在由配置的签发者颁发的有效令牌。\n这是默认选项。",
      absoluteCallbackUriHandledByTheGatewayThisPolicyAlwaysRedirectsUnauthenticatedNo_1lp94cf:
        "由网关处理的绝对回调 URI。\n此策略始终将未经身份验证的非回调请求重定向到此登录流程。",
      acceptAnyRequestHeaderInBrowserPreflightChecks:
        "接受浏览器预检检查中的任何请求头",
      acceptConnectionsWithOrWithoutAProxyProtocolHeader:
        "接受带有或不带有 PROXY 协议请求头的连接。",
      acceptProxyProtocolV1OrV2: "接受 PROXY 协议 v1 或 v2。",
      acceptProxyProtocolV1: "接受 PROXY 协议 v1。",
      acceptProxyProtocolV2: "接受 PROXY 协议 v2。",
      acceptedTokenAudiencesMatchedAgainstTheJwtAudClaimWhenSet:
        "接受的令牌受众，在设置时与 JWT `aud` 声明进行匹配。",
      acceptedTokenAudiencesMatchedAgainstTheJwtAudClaim:
        "接受的令牌受众，与 JWT `aud` 声明相匹配。",
      access: "访问",
      accessLogFieldNamesToRemove: "要删除的访问日志字段名称。",
      accessLogFieldsToAddComputedFromCelExpressions:
        "访问要添加的日志字段，根据 CEL 表达式计算。",
      action: "操作",
      actionToTakeWhenARegexRuleMatches: "正则表达式规则匹配时要采取的操作。",
      actions: "操作",
      activeRuntimeResourcesFromTheGatewayDump:
        "来自网关转储的活动运行时资源。",
      adc: "ADC",
      adcCompatibleGoogleCredentialJsonIfNotSetAmbientCredentialsAreUsed:
        "与 ADC 兼容的 Google 凭证 JSON。如果未设置，则使用环境凭据。",
      add: "添加",
      addValueGuard: "添加 {{value}} 防护",
      addAccessControlAllowCredentialsTrueOnAllowedCorsResponses:
        "在允许的 CORS 响应上添加 `Access-Control-Allow-Credentials: true`。",
      addACelExpressionToStartAuthorizingRequests:
        "添加 CEL 表达式以开始授权请求。",
      addAGatewayBeforeAttachingRoutes: "在附加路由之前添加网关。",
      addAGatewayBeforeExposingTheUi: "在公开 UI 之前添加网关。",
      addAGatewayBeforeHttpTrafficCanBeServed:
        "请先添加网关，再提供 HTTP 流量服务。",
      addAListenerBeforeHttpOrTcpTrafficCanBeServed:
        "请先添加监听器，再提供 HTTP 或 TCP 流量服务。",
      addAListenerToStartMatchingTrafficOnThisPort:
        "添加监听器以开始匹配此端口上的流量。",
      addAModelBeforeLlmTrafficCanBeServed:
        "请先添加模型，再提供 LLM 流量服务。",
      addANameBeforeCreatingThisVirtualApiKey:
        "在创建此虚拟 API 密钥之前添加名称。",
      addANamedGatewayBeforeAttachingLlmMcpUiOrRoutes:
        "在附加 LLM、MCP、UI 或路由之前添加命名网关。",
      addAProviderWhenMultipleModelsShouldShareTheSameCredentialsOrUpstreamConnectionSettings:
        "当多个模型应共享相同的凭据或上游连接设置时添加提供商。",
      addARemotePolicyProcessorToInspectMcpRequestsAndResponses:
        "添加远程策略处理器来检查 MCP 请求和响应。",
      addATargetSoTheGatewayCanExposeMcpTraffic:
        "添加目标，以便网关可以公开 MCP 流量。",
      addAnMcpTargetBeforeToolsAreAvailable: "请先添加 MCP 目标，再使用工具。",
      addBackend: "添加后端",
      addBind: "添加绑定",
      addCacheMarkersToChatMessagesWhenSupportedByTheProvider:
        "当提供商支持时，将缓存标记添加到聊天消息中。",
      addCacheMarkersToSystemPromptsWhenSupportedByTheProvider:
        "当提供商支持时，将缓存标记添加到系统提示中。",
      addCacheMarkersToToolDefinitionsWhenSupportedByTheProvider:
        "当提供商支持时，将缓存标记添加到工具定义中。",
      addCurrentOrigin: "添加当前源",
      addDescriptor: "添加描述符",
      addEntry: "添加条目",
      addFallback: "添加后备",
      addFallbackGroup: "添加后备组",
      addGateway: "添加网关",
      addGuard: "添加防护规则",
      addHeader: "添加请求头",
      addHeaders: "添加标题",
      addListener: "添加监听器",
      addMatch: "添加匹配条件",
      addModel: "添加模型",
      addModelCost: "添加模型成本",
      addModelUsingProvider: "使用提供商添加模型",
      addPattern: "添加模式",
      addProcessor: "添加处理器",
      addProvider: "添加提供商",
      addQuery: "添加查询条件",
      addRequestHeaders: "添加请求头",
      addRoute: "添加路由",
      addRule: "添加规则",
      addServer: "添加服务器",
      addTarget: "添加目标",
      addVirtualModel: "添加虚拟模型",
      additionalMetadataToSendToTheExternalProcessingServiceMapsToTheMetadataContextFi_d3ztkj:
        "要发送到外部处理服务的附加元数据。\n映射到ProcessingRequest 中的`metadata_context.filter_metadata` 字段，并允许动态CEL 表达式。",
      additionalOauth2ScopesToRequestOpenidIsAlwaysIncluded:
        "要请求的其他 OAuth2 范围。 `openid` 始终包含在内。",
      additionalScopes: "附加作用域",
      additionalSubjectAlternativeNamesAcceptedForTheBackendCertificate:
        "后端证书接受的其他主题备用名称。",
      additionalTrustedOriginsAllowedToSendStateChangingRequests:
        "允许发送状态更改请求的其他受信任源。",
      addsGenAiPromptAndGenAiCompletionAttributesToAccessLogs:
        "添加 `gen_ai.prompt` 和 `gen_ai.completion` 属性以访问日志。",
      adminUiAddressInTheFormatIpPortLocalhostPortUnixPathToSocketOrOff:
        "管理 UI 地址，格式为“ip:port”、“localhost:port”、“unix:/path/to/socket”或“off”",
      advanced: "高级",
      agentgateway: "代理网关",
      agentgatewayHome: "代理网关首页",
      agentgatewayIsAGatewayThatCanRouteSecureAndObserveLlmMcpAndTraditionalApiTraffic_sbsjep:
        "Agentgateway 是一个可以路由、保护和观察 LLM、MCP 和传统 API 流量的网关。选择要启用的一项或多项功能，然后继续。",
      agentgatewayRoutesRequestsByMatchingAnIncomingModelNameAndThenSendingItToTheConf_w5k7w1:
        "Agentgateway 通过匹配传入模型名称来路由请求，然后将其发送到配置的模型。传出模型可以从传入模型传递、进行转换或者是静态模型。",
      agwSkAutoGenerate: "agw_sk_*****（自动生成）",
      ai: "人工智能",
      all: "全部",
      allValue: "全部{{value}}",
      allow: "允许",
      allowAllRequestHeaders: "允许所有请求头",
      allowCredentials: "允许凭据",
      allowModeOverride: "允许模式覆盖",
      allowPartialMessage: "允许部分消息",
      allowRequestsWhenTheRateLimitServiceIsUnavailable:
        "当速率限制服务不可用时允许请求。",
      allowTheRequestThroughWhenTheRateLimitServiceIsUnavailable:
        "当速率限制服务不可用时允许请求通过。",
      allowTheRequestThroughWhenTheWebhookGuardrailIsUnavailable:
        "当 webhook 防护规则不可用时允许请求通过。",
      allowTheRequestWhenTheAuthorizationServiceCannotMakeADecision:
        "当授权服务无法做出决定时允许请求。",
      allowTheRequestWhenThisCelExpressionIsTrue:
        "当此 CEL 表达式为 true 时允许请求。",
      allowTrafficWhenTheProcessorIsUnavailable: "当处理器不可用时允许流量。",
      allowDenyFilterOverRequestHeadersMirroringExtAuthzEmptyAllowedForwardsEveryHeade_17m99zk:
        "允许/拒绝对请求头进行过滤，镜像 ext_authz: 空 `allowed`\n转发每个请求头以及所有伪请求头（`:authority`、`:method`，...）；\n非空 `allowed` 仅转发列出的名称。 `disallowed` 始终\n获胜。请求头名称匹配不区分大小写；伪请求头完全匹配。",
      allowedHeaders: "允许的请求头",
      allowedMethods: "允许的方法",
      allowedOrigins: "允许的来源",
      allowlistOnlyMethodsListedHereRunThroughThisProcessorAtTheConfiguredPhaseKeysMay_1ppmyo1:
        "白名单：只有此处列出的方法通过此处理器运行\n配置阶段。键可以是精确的 (`tools/call`)、前缀 (`tools/*`)、\n或后缀 (`*/list`) 通配符，或 `*` 对于所有方法。方法匹配\n没有密钥绕过该处理器；请参阅 [`phase::resolve`] 了解匹配优先级。",
      alpnProtocolsAdvertisedToDownstreamClients:
        "向下游客户端通告的 ALPN 协议。",
      alpnProtocolsToOfferToTheBackend: "提供给后端的 ALPN 协议。",
      always: "总是",
      alwaysPrefixExposedToolNamesWithTheTargetName:
        "始终使用目标名称作为公开工具名称的前缀。",
      ambient: "环境",
      anApiKeyToAttachToTheRequestIfUnsetThisWillBeAutomaticallyDetectedFromTheEnvironment:
        "附加到请求的 API 密钥。\n如果未设置，则会自动从环境中检测到。",
      anAwsStsSessionTagPassedToAssumeRoleForCostAttributionExactlyOneOfValueAndExpressionMustBeSet:
        "传递到 AssumeRole 进行成本归因的 AWS STS 会话标签。\n必须恰好设置 `value` 和 `expression` 之一。",
      analytics: "分析",
      analyticsApiError: "分析 API 错误",
      analyzeApiVersion: "分析API版本",
      analyzeLlmTrafficByModelUserAndProvider:
        "按模型、用户和提供商分析 LLM 流量。",
      analyzeTextConfigurationForDetectingHarmfulContentCategoriesHateSelfHarmSexualVi_zwlwnr:
        "分析文本配置以检测有害内容类别\n（仇恨、自残、性、暴力）和黑名单匹配。",
      andForwardTheModelAsIs: "并按原样转发模型。",
      andHasNo: "并且没有",
      andSave: "并保存。",
      andSearchFor: "并搜索",
      andSendTo: "并发送至",
      andSetItTo: "并将其设置为",
      andSetTheAdvancedProxyUrl: "并设置高级代理 URL。",
      andStripThe: "并剥离",
      anotherVirtualKeyAlreadyUsesThisNameTheKeyWillStillBeCreatedWithAUniqueMetadataId:
        "另一个虚拟键已使用该名称。密钥仍将使用唯一的元数据 ID 创建。",
      anthropicV1Messages: "Anthropic /v1/messages",
      anthropicV1MessagesCountTokens: "Anthropic /v1/messages/count_tokens",
      anyStatus: "任何状态",
      apiKey: "API 密钥",
      apiKeyValueToAccept: "要接受的 API 密钥值。",
      apiKeys: "API 密钥",
      apiKeysThatAreAcceptedByThisPolicy: "本策略接受的 API 密钥。",
      apiVersionToUseDefault20240215Preview:
        "要使用的 API 版本（默认值：“2024-02-15-preview”）",
      apiVersionToUseDefault20240901:
        "要使用的 API 版本（默认值：“2024-09-01”）",
      apis: "API",
      apply: "应用",
      applyAuthorization: "申请授权",
      applyChanges: "应用更改",
      applyCors: "应用 CORS",
      applyMcpCors: "应用 MCP CORS",
      applyPromptAndResponseGuardrailsToAllLlmModels:
        "将提示和响应防护规则应用于所有 LLM 模型。",
      applyPromptGuardsToStreamingResponsesAndRealtimeWebsocketMessages:
        "将提示防护应用于流响应和实时 Websocket 消息。",
      applyRegexBasedMaskingOrRejectionRules:
        "应用基于正则表达式的屏蔽或拒绝规则。",
      arguments: "论点",
      argumentsJson: "参数 JSON",
      argumentsMustBeAJsonObject: "参数必须是 JSON 对象。",
      asACustomModelThenTestFrom: "作为自定义模型，然后进行测试",
      ask: "询问",
      askATestQuestion: "问一个测试问题...",
      atLeastOneMatchGroupMustMatchWithinAGroupEveryHeaderConditionMustMatch:
        "至少有一个比赛组必须匹配。在组内，每个请求头条件都必须匹配。",
      attachARouteToAGateway: "将路由附加到网关。",
      attachHttpAndTcpRoutesToTrafficGateways:
        "将 HTTP 和 TCP 路由附加到流量网关。",
      attributeKeysToRemoveFromTheEmittedSpanAttributesThisIsAppliedBeforeAttributesAr_1mndxj6:
        "要从发出的跨度属性中删除的属性键。\n\n这是在评估/添加 `attributes` 之前应用的，因此可以用于删除\n默认属性或避免重复。",
      attributes: "属性",
      audienceForTheTokenIfNotSetTheDestinationHostWillBeUsed:
        "令牌的受众。如果未设置，将使用目标主机。",
      audiences: "受众",
      audioIn: "音频输入",
      audioOut: "音频输出",
      auth: "授权",
      authConfiguresAuthenticationWhenConnectingToTheLlmProvider:
        "auth 配置连接到 LLM 提供商时的身份验证。",
      authenticateBrowserRequestsWithOidcAuthorizationCodeFlow:
        "使用 OIDC 授权码流程验证浏览器请求。",
      authenticateIncomingRequestsWithApiKeys: "使用 API 密钥验证传入请求。",
      authenticateIncomingRequestsWithBasicAuthCredentialsFromAnHtpasswdUserDatabase:
        "使用 htpasswd 用户数据库中的基本身份验证凭据对传入请求进行身份验证。",
      authenticateIncomingRequestsWithJwtBearerTokens:
        "使用 JWT Bearer 令牌验证入站请求。",
      authenticateMcpClients: "验证 MCP 客户端。",
      authenticateToAzureServices: "对 Azure 服务进行身份验证。",
      authenticateToGitHubCopilot: "向 GitHub Copilot 进行身份验证。",
      authenticateToGoogleCloudServices: "向 Google Cloud 服务进行身份验证。",
      authentication: "身份验证",
      authenticationCredentialsSentToTheBackend: "身份验证凭据发送到后端。",
      authenticationCredentialsSentToThisBackend:
        "发送到此后端的身份验证凭据。",
      authorization: "授权",
      authorizationBehavior: "授权行为",
      authorizationConfiguresHttpAuthorizationRulesForRequestsToThisModel:
        "授权为对此模型的请求配置 HTTP 授权规则。",
      authorizationEndpoint: "授权端点",
      authorizationEndpointUsedToStartTheBrowserLoginFlow:
        "用于启动浏览器登录流程的授权端点。",
      authorizationHeader: "授权请求头",
      authorizationResponseHeadersToCopyIntoTheBackendRequest:
        "授权响应请求头复制到后端请求中。",
      authorizationRulesForIncomingHttpRequests: "传入 HTTP 请求的授权规则。",
      authorizationRulesForMcpRequests: "MCP 请求的授权规则。",
      authorizeIncomingRequestsAfterThisBackendIsSelected:
        "选择此后端后授权传入请求。",
      authorizeIncomingRequestsByCallingAnExternalAuthorizationServiceAfterThisBackendIsSelected:
        "选择此后端后，通过调用外部授权服务来授权传入请求。",
      authorizeIncomingRequestsByCallingAnExternalAuthorizationService:
        "通过调用外部授权服务来授权传入请求。",
      automaticallyChooseBasedOnTheEnableIpv6SettingWhenIpv6IsEnabledThisBehavesLikeV4_nsr4ii:
        "根据 `enable_ipv6` 设置自动选择。当 IPv6 是\n启用此行为类似于 `V4Preferred`；否则为 `V4Only`。",
      automaticallyDetectAuthenticationMethodBasedOnEnvironmentUsesWorkloadIdentityOnK_y198si:
        "根据环境自动检测认证方法。\n在 K8s 上使用工作负载身份、Azure VM 上的托管身份或本地开发人员工具。",
      awsAccessKeyId: "AWS 访问密钥 ID",
      awsCredentials: "AWS 凭证",
      awsIamRoleArnToAssume: "要承担的 AWS IAM 角色 ARN。",
      awsRegion: "AWS 区域",
      awsRegionWhereTheGuardrailIsDeployed: "部署防护规则的AWS区域",
      awsSecretAccessKey: "AWS 秘密访问密钥",
      awsSigV4SigningServiceNameForExampleBedrockBedrockAgentcoreOrExecuteApi:
        "AWS SigV4 签名服务名称（例如“bedrock”、“bedrock-agentcore”或“execute-api”）。",
      azureAiFoundryProjectEndpointResourceNameServicesAiAzureComRequiresProjectNameTo_bpdjpb:
        "Azure AI Foundry（项目）端点：`{resourceName}.services.ai.azure.com`\n需要 `project_name` 来构建类似 `/api/projects/{project}/openai/v1/...` 的路径",
      azureApiVersion: "Azure API 版本",
      azureContentSafety: "Azure 内容安全",
      azureCredentials: "Azure 凭据",
      azureOpenAiServiceEndpointResourceNameOpenaiAzureCom:
        "Azure OpenAI 服务端点：`{resourceName}.openai.azure.com`",
      azureProjectName: "Azure 项目名称",
      azureResourceName: "Azure 资源名称",
      azureResourceType: "Azure 资源类型",
      backToHome: "返回首页",
      backend: "后端",
      backendHostUrlForGuardrailChecks: "用于防护规则检查的后端主机 URL。",
      backendPolicies: "后端策略",
      backendPoliciesForAwsAuthenticationOptionalDefaultsToImplicitAwsAuth:
        "AWS 身份验证的后端策略（可选，默认为隐式 AWS 身份验证）",
      backendPoliciesForAzureAuthenticationOptionalDefaultsToImplicitAzureAuth:
        "Azure 身份验证的后端策略（可选，默认为隐式 Azure 身份验证）",
      backendPoliciesForGcpAuthenticationOptionalDefaultsToImplicitGcpAuth:
        "GCP 身份验证的后端策略（可选，默认为隐式 GCP 身份验证）",
      backendPoliciesPreserved: "保留后端策略",
      backendPoliciesUsedWhenCallingTheModerationProvider:
        "调用审核提供商时使用的后端策略。",
      backendPoliciesUsedWhenConnectingToTheService:
        "连接到服务时使用的后端策略。",
      backendReference: "后端引用",
      backendThatReceivesGuardrailWebhookRequests:
        "接收 Guardrail Webhook 请求的后端。",
      backendThatReceivesMirroredRequestCopies: "接收镜像请求副本的后端。",
      backendYaml: "后端 YAML",
      backends: "后端",
      backends_i9thuc: "后端",
      backendsDefinesExplicitBackendsThatCanBeReferencedByRoutesAndPoliciesTypicallyIn_1a5i8ts:
        "backends 定义了可以被路由和策略引用的显式后端。\n通常，内联后端用于路由/策略，但这允许重复使用相同的后端\n跨越不同的配置。",
      backendTunnelConfiguresTunnelingWhenConnectingToTheLlmProvider:
        "backendTunnel 在连接到 LLM 提供商时配置隧道。",
      baseCostCatalogRefreshedValueModelsFromValueProviders:
        "基本成本目录已刷新：来自 {{value}} 提供商的 {{value}} 模型。",
      baseUrl: "基础 URL",
      baseUrlForTheUpstreamProviderExpandsToHostOverridePathPrefixAndTlsForHttpsUrls:
        "上游提供商的基本 URL。对于 https URL，扩展为 hostOverride、pathPrefix 和 tls。",
      basicAuth: "基本授权",
      bearerToken: "Bearer 令牌",
      bedrockGuardrails: "Bedrock 防护规则",
      behaviorWhenOneOrMoreMcpTargetsFailToInitializeOrFailDuringFanoutDefaultsToFailClosed:
        "当一个或多个 MCP 目标无法初始化或在扇出期间失败时的行为。\n默认为 `failClosed`。",
      behaviorWhenTheAuthorizationServiceIsUnavailableOrReturnsAnError:
        "授权服务不可用或返回错误时的行为。",
      behaviorWhenTheExternalProcessingServiceIsUnavailableOrReturnsAnError:
        "外部处理服务不可用或返回错误时的行为。",
      behaviorWhenTheProcessorIsUnavailableOrReturnsAnError:
        "处理器不可用或返回错误时的行为。",
      behaviorWhenTheRemoteRateLimitServiceIsUnavailableOrReturnsAnErrorDefaultsToFail_1bpcema:
        "远程速率限制服务不可用或返回错误时的行为。\n默认为failClosed，拒绝服务失败时状态为 500 的请求。",
      behaviorWhenTheWebhookIsUnreachableOrReturnsAnErrorDefaultsToFailClosed:
        "Webhook 无法访问或返回错误时的行为。\n默认为 `failClosed`。",
      bind: "绑定",
      bindPort: "绑定端口",
      bindPortThisListenerIsAttachedTo: "绑定此监听器所附加的端口。",
      bindThisSurfaceOnItsOwnListenerPort:
        "将此表面绑定到其自己的监听器端口上。",
      bindsDefinesTheLowLevelApiForConfiguringTheProxyEachBindRepresentsASinglePortThe_96e01v:
        "binds 定义了用于配置代理的低级 API。\n每个绑定代表代理侦听的单个端口以及全套配置\n该端口的（监听器、路由、后端）。\n已弃用；建议改用 `gateways` 和 `routes`。",
      blocklistNamesToCheckAgainst: "要检查的黑名单名称",
      blocklists: "阻止列表",
      bodyExpression: "正文表达式",
      bodyOptions: "正文选项",
      breakdown: "明细",
      browserAccessIsNotAllowed: "不允许浏览器访问",
      browserBasedOidcAuthenticationPolicyExplicitModeIsStillOidcItSuppliesProviderMet_1en29xp:
        "基于浏览器的 OIDC 身份验证策略。\n\n显式模式仍然是 OIDC：它手动提供提供商元数据，而不是使用发现。\n未经身份验证的非回调请求始终重定向到提供商登录流程。路由\n需要非重定向身份验证行为应使用不同的身份验证策略。",
      bufferAndSendTheBodyUpToTheConfiguredLimit:
        "缓冲并发送正文，直至达到配置的限制。",
      bufferAndSendTheFullBodyToTheExternalProcessingService:
        "缓冲并将完整正文发送到外部处理服务。",
      bufferIncomingRequestBodiesBeforeForwarding:
        "在转发之前缓冲传入的请求正文。",
      bufferRequestAndResponseBodies: "缓冲请求和响应主体。",
      bufferTheFullBodyBeforeSendingItToTheProcessor:
        "在将整个正文发送到处理器之前对其进行缓冲。",
      bufferUpstreamResponseBodiesBeforeSendingThemToTheClient:
        "在将上游响应正文发送到客户端之前对其进行缓冲。",
      buffered: "缓冲的",
      bufferedPartial: "缓冲部分",
      builtInDetectors: "内置检测器",
      builtInPatternName: "内置模式名称。",
      caSin: "加州新",
      cacheAuthorizationResultsUsingCelExpressionsAsTheCacheKeyWarningTheSafetyOfThisF_1hqhg1t:
        "使用CEL表达式作为缓存键来缓存授权结果。\n警告：该功能的安全性取决于缓存键准确捕获字段\n服务器运行。例如，如果您根据请求头 A 返回不同的结果，但仅\n缓存请求头 B，用户可能会得到不正确的缓存命中。",
      cacheRead: "缓存读取",
      cacheWrite: "缓存写入",
      callAWebhookToEvaluateThePrompt: "调用 Webhook 来评估提示。",
      callAWebhookToEvaluateTheResponse: "调用 Webhook 来评估响应。",
      callAnHttpAuthorizationService: "调用 HTTP 授权服务。",
      callTheAuthorizationServiceUsingHttp: "使用 HTTP 调用授权服务。",
      callTheAuthorizationServiceUsingTheGRpcAuthorizationProtocol:
        "使用gRPC授权协议调用授权服务。",
      callTool: "调用工具",
      callingValueMcpValue: "正在调用 {{value}} MCP {{value}}",
      calls: "通话",
      canBeAWildcard: "可以是通配符",
      canadianSocialInsuranceNumberPattern: "加拿大社会保险号码模式。",
      cancel: "取消",
      catalogSources: "目录来源",
      celAuthorizationForDownstreamNetworkConnections:
        "用于下游网络连接的 CEL 授权。",
      celAuthorizationRulesForDownstreamNetworkConnections:
        "用于下游网络连接的 CEL 授权规则。",
      celAuthorizationRulesForMcpToolsPromptsAndResources:
        "MCP 工具、提示和资源的 CEL 授权规则。",
      celAuthorizationRulesToEvaluateForARequest:
        "用于评估请求的 CEL 授权规则。",
      celError: "CEL 错误",
      celExpression: "CEL 表达式",
      celExpressionEvaluatedAgainstEachRequestToProduceTheTagValueForExampleJwtSubOrRe_1jdxqht:
        '针对每个请求计算 CEL 表达式以生成标签值，例如\n例如 `jwt.sub` 或 `request.headers["x-app"]`。如果表达式不\n在请求时生成有效的标签值，则请求被拒绝。',
      celExpressionEvaluatedAgainstEachResponseToDecideWhetherToRetryAResponseIsRetrie_qrheq5:
        "针对每个响应评估 CEL 表达式以决定是否重试。回应\n当其状态代码为 `codes` *或*此表达式计算结果为 `true` 时重试。",
      celExpressionEvaluatedAgainstTheRequestBeforeAnyAttemptWhenFalseRetriesAreDisabl_sapdox:
        '在任何尝试之前根据请求评估 CEL 表达式；当`false`时，\n重试被禁用（仅进行初始尝试），例如`request.method == "GET"`。\n重试需要将请求正文缓冲在内存中以便重播，因此我们可以跳过\n当已知请求不可重试时（例如流式传输或 websockets），该成本。',
      celExpressionThatComputesARedirectUrlWhenAuthorizationFailsWhenTheAuthorizationS_vhwf5d:
        "授权失败时计算重定向 URL 的 CEL 表达式。\n当授权服务返回未授权时，此重定向而不是直接返回错误。",
      celExpressionThatComputesAReplacementBody: "计算替换体的 CEL 表达式。",
      celExpressionThatComputesTheAuthorizationRequestBodyStringsAndBytesAreUsedDirect_1etgvrf:
        "计算授权请求正文的 CEL 表达式。\n直接使用字符串和字节；其他值是 JSON 编码的。\n如果设置，这将取代转发传入的请求正文。",
      celExpressionThatComputesTheAuthorizationRequestPath:
        "计算授权请求路径的 CEL 表达式。",
      celExpressionThatComputesTheResponseBody: "计算响应正文的 CEL 表达式。",
      celExpressionThatDecidesWhetherARequestIsExportedOverOtlp:
        "决定是否通过 OTLP 导出请求的 CEL 表达式。",
      celExpressionThatDecidesWhetherARequestIsLogged:
        "决定是否记录请求的 CEL 表达式。",
      celExpressionThatReturnsHowLongCachedAuthorizationResultsAreReusedTheExpressionI_kb9kvi:
        "返回缓存授权结果重用时间的 CEL 表达式。\n应用授权响应后计算表达式\n到请求，并且必须返回持续时间或时间戳。",
      celExpressionUsedToComputeTheDescriptorEntryValue:
        "CEL 表达式用于计算描述符条目值。",
      celExpressionUsedToPopulateTheAgentgatewayGroupRequestLogAttribute:
        "用于填充 `agentgateway.group` 请求日志属性的 CEL 表达式。",
      celExpressionUsedToPopulateTheAgentgatewayUserRequestLogAttribute:
        "用于填充 `agentgateway.user` 请求日志属性的 CEL 表达式。",
      celExpressionUsedToPopulateTheAgentgatewayGroupRequestLogAttribute_n5btzx:
        "用于填充 agentgateway.group 请求日志属性的 CEL 表达式。",
      celExpressionUsedToPopulateTheAgentgatewayUserRequestLogAttribute_r3ojz7:
        "用于填充 agentgateway.user 请求日志属性的 CEL 表达式。",
      celExpressionWhereTrueMarksTheBackendResponseAsUnhealthyWhenUnsetAny5xxResponseO_19ajrab:
        "CEL 表达式，其中 `true` 将后端响应标记为不正常。\n未设置时，任何 5xx 响应或连接失败都将被视为不健康。",
      celExpressionsEvaluatedPerRequestAndSentToTheProcessorAsMetadata:
        "CEL 表达式根据请求进行评估并作为元数据发送到处理器。",
      celExpressionsSentAsAttributesToTheProcessor:
        "CEL 表达式作为属性发送到处理器。",
      celExpressionsThatMakeUpTheCacheKeyEmptyKeysAreAcceptedButDoNotProduceCacheHits:
        "组成缓存键的 CEL 表达式。接受空键，但不会产生缓存命中。",
      celPlayground: "CEL 演练场",
      celReference: "CEL 参考",
      certificate: "证书",
      certificateSourceModeStaticModeUsesCertKeyAsTheLeafCertificateDynamicCaModeUsesC_1dwhpmp:
        "证书来源模式。静态模式使用cert/key作为叶证书；动态CA\n模式使用 cert/key 作为按需 SNI 叶证书颁发的 CA。",
      chatPlayground: "聊天演练场",
      chooseFailureBehaviorAndWhichRequestResponsePhasesAreSent:
        "选择失败行为以及发送哪些请求/响应阶段。",
      chooseHowLlmTrafficIsExposed: "选择如何公开 LLM 流量。",
      chooseHowMcpIsExposed: "选择 MCP 的暴露方式。",
      chooseHowTheGatewayBehavesWhenARequestHasNoTokenOrATokenCannotBeVerified:
        "选择当请求没有令牌或令牌无法验证时网关的行为方式。",
      chooseProtocolAndFailOpenFailClosedBehavior:
        "选择协议和故障开放/故障关闭行为。",
      chooseSessionToolPrefixAndFailureBehavior:
        "选择会话、工具前缀和失败行为。",
      cipherSuitesAllowedForDownstreamTls: "密码套件允许下游 TLS。",
      claimRequirementsToEnforceAfterTheTokenSignatureIsVerified:
        "验证令牌签名后强制执行的声明要求。",
      claimsThatMustBePresentInTheTokenBeforeValidationOnlyExpNbfAudIssSubAreEnforcedO_ux04jc:
        "验证之前令牌中必须存在的声明。\n仅强制执行“exp”、“nbf”、“aud”、“iss”、“sub”；其他\n（包括“iat”和“jti”）将被忽略。\n默认为[“exp”]。使用空列表不需要任何声明。",
      claudeCode: "Claude Code",
      claudeDesktop: "Claude Desktop",
      claudeSubscriptionKeyDetected: "检测到 Claude 订阅密钥",
      clear: "清除",
      clearAuthorization: "清除授权",
      clearEnvironment: "清除环境",
      clearFilters: "清除筛选条件",
      client: "客户端",
      clientAuthenticationUsedWhenCallingTheTokenEndpoint:
        "调用令牌端点时使用的客户端身份验证。",
      clientAuthenticationUsedWhenCallingTheTokenEndpointWhenUnsetNoClientAuthenticationFieldsAreSent:
        "调用令牌端点时使用的客户端身份验证。\n未设置时，不会发送任何客户端身份验证字段。",
      clientCertificateFileToPresentToTheBackend:
        "要呈现给后端的客户端证书文件。",
      clientId: "客户端 ID",
      clientIdOptional: "客户端 ID（可选）",
      clientRequested: "客户端请求",
      clientSecret: "客户端密钥",
      clientSecretBasic: "客户端密钥 Basic",
      clientSecretPost: "客户端密钥 POST",
      clientSetup: "客户端设置",
      close: "关闭",
      codexCli: "法典 CLI",
      cohereV2RerankDocumentReranking: "Cohere /v2/rerank（文档重新排名）",
      commaSeparatedListOfAdditionalSpiffeTrustDomainsAcceptedOnInboundHboneConnection_ib2a3q:
        "入站 HBONE 接受的其他 SPIFFE 信任域的逗号分隔列表\n连接。本地 trust_domain 始终隐式包含在内。",
      commaSeparatedNames: "逗号分隔的名称。",
      command: "命令",
      condition: "条件",
      conditionMustEvaluateToTrueForThisPolicyToExecuteIfUnsetThePolicyIsTheFallback:
        "条件必须评估为 true 才能执行此策略。如果未设置，则该策略是后备策略。",
      conditional: "条件式",
      conditionalEnablesConditionBasedSelectionOfTheTargetModelEachConditionIsEvaluate_12cw48o:
        "Conditional 支持基于条件选择目标模型。评估每个条件\n依次排列，直至找到最佳匹配。",
      conditionalPolicyEntriesAnEntryWithoutAConditionMustBeTheFinalFallback:
        "有条件的策略条目。没有条件的条目必须是最终的后备。",
      conditionalTargets: "有条件的目标",
      configDumpUnavailable: "配置转储不可用",
      configuration: "配置",
      configurationApiUnavailable: "配置 API 不可用",
      configurationForAwsBedrockGuardrailsIntegration:
        "AWS Bedrock Guardrails 集成的配置。",
      configurationForAzureContentSafetyIntegrationUsesTheAzureAiContentSafetyApisToDe_13mxkr6:
        "Azure 内容安全集成的配置。\n\n使用 Azure AI 内容安全 API 检测有害内容\n和越狱尝试。端点和身份验证是共享的\n跨所有启用的功能。",
      configurationForDynamicTracingPolicy: "动态跟踪策略配置",
      configurationForGoogleCloudModelArmorIntegration:
        "Google Cloud Model Armor 集成的配置。",
      configurationForStatefulSessionManagement: "有状态会话管理的配置",
      configurationForTheAnalyzeTextApi: "分析文本 API 的配置。",
      configurationForTheDetectJailbreakApi: "检测越狱 API 的配置。",
      configurationIsManagedByXdsThisViewReflectsTheActiveRuntimeDumpEditingIsDisabled:
        "配置由 XDS 管理。此视图反映当前运行时转储，无法编辑。",
      configurationMustBeAYamlObject: "配置必须是 YAML 对象。",
      configurationSaved: "配置已保存",
      configurationValidationFailedValue: "配置验证失败：{{value}}",
      configureAModelFirst: "请先配置模型",
      configureBindPortsAndListenersForGenericHttpAndTcpTraffic:
        "为通用 HTTP 和 TCP 流量配置绑定端口和监听器。",
      configureMcpTargetsServedByTheGateway: "配置网关服务的 MCP 目标。",
      configureNamedGatewayListenersThatLlmMcpUiAndRoutesCanAttachTo:
        "配置 LLM、MCP、UI 和路由可以附加到的命名网关监听器。",
      configureOpenCodeWithAnOpenAiCompatibleGatewayProvider:
        "使用兼容 OpenAI 的网关提供商配置 OpenCode。",
      configureTheAuthorizationRequestAndResponseMetadataExtraction:
        "配置授权请求和响应元数据提取。",
      configureTheJwksSourceUsedToVerifyTokenSignatures:
        "配置用于验证令牌签名的 JWKS 源。",
      configureThirdPartyInference: "配置第三方推理",
      configureTopLevelBehaviorForMcpGatewayTraffic:
        "配置 MCP 网关流量的顶级行为。",
      configureTopLevelBehaviorThatAppliesBeforeModelSpecificRouting:
        "配置在特定于模型的路由之前应用的顶级行为。",
      configureUiPolicies: "配置 UI 策略",
      configureVsCodeCopilotBusinessOrEnterpriseToUseTheGatewayProxy:
        "配置 VS Code Copilot Business 或 Enterprise 以使用网关代理。",
      configureWhereBrowserLoginStartsAndHowReturnedIdTokensAreValidated:
        "配置浏览器登录的起始位置以及如何验证返回的 ID 令牌。",
      configured: "配置好的",
      connection: "连接方式",
      consecutiveFailures: "连续失败",
      consecutiveUnhealthyResponsesRequiredBeforeEviction:
        "驱逐前需要连续做出不健康的反应。",
      context: "上下文",
      contextExtensionsAreStaticValuesMetadataValuesAreCelExpressions:
        "上下文扩展是静态值；元数据值是 CEL 表达式。",
      continue: "继续",
      continueTheRequestWhenTheExternalProcessingServiceFails:
        "当外部处理服务失败时继续请求。",
      continueWhenTheWebhookIsUnavailableOrErrors:
        "当 Webhook 不可用或出现错误时继续。",
      controlWhetherMcpRequestsMustPresentAValidJwt:
        "控制 MCP 请求是否必须提供有效的 JWT。",
      controlsHowAnEndpointPickerSelectedDestinationIsUsed:
        "控制如何使用端点选择器选择的目标。",
      controlsWhetherMcpRequestsMustIncludeAValidJwt:
        "控制 MCP 请求是否必须包含有效的 JWT。",
      controlsWhetherRequestsMustIncludeAJwtAndHowValidationFailuresAreHandled:
        "控制请求是否必须包含 JWT 以及如何处理验证失败。",
      controlsWhetherRequestsMustIncludeAValidApiKey:
        "控制请求是否必须包含有效的 API 密钥。",
      controlsWhetherRequestsMustIncludeValidBasicAuthCredentials:
        "控制请求是否必须包含有效的基本身份验证凭据。",
      controlsWhichIpAddressFamiliesTheDnsResolverWillQueryForUpstreamBackendConnectio_1w5pwyi:
        "控制 DNS 解析器将查询哪些 IP 地址系列\n上游（后端）连接。\n\n 在引擎盖下映射到 hickory_resolver 的 `LookupIpStrategy`。\n\n可以通过 `DNS_LOOKUP_FAMILY` 环境变量或\n配置文件中的 `dns.lookupFamily` 字段。\n\n请参阅：<https://www.envoyproxy.io/docs/envoy/latest/api-v3/config/cluster/v3/cluster.proto#enum-config-cluster-v3-cluster-dnslookupfamily>",
      controlsWhichIpAddressFamiliesTheDnsResolverWillQueryForUpstreamConnectionsAccep_h7l2v:
        "控制 DNS 解析器将查询哪些 IP 地址系列\n上游连接。\n接受的值：全部、自动、V4Preferred、V4Only、V6Only。\n默认为自动（enableIpv6 为 false 时仅支持 IPv4，均为 true 时）。",
      controlsWhichRequestAndResponsePartsAreSentToTheExternalProcessingService:
        "控制将哪些请求和响应部分发送到外部处理服务。",
      conversation: "对话",
      cookieNameContainingTheCredential: "包含凭证的 Cookie 名称。",
      copy: "复制",
      copyKey: "复制键",
      copyToClipboard: "复制到剪贴板",
      cost: "成本",
      costCatalogRefreshFailed: "成本目录刷新失败",
      costDeterminesTheOptionalExpressionToDetermineTheCostOfTheRequestIfUnsetTypeRequ_z12ji8:
        "cost 确定用于确定请求成本的可选表达式。\n如果未设置，则键入 `requests` 默认为 `1`，键入 `tokens` 默认为 `llm.totalTokens`。\n如果表达式无法计算，则跳过描述符。\n`requests` 类型的成本在请求处理期间进行评估。 `tokens` 类型的成本\n在请求完成后进行评估。",
      costExpression: "成本表达",
      costRefreshFailed: "成本刷新失败",
      costs: "成本",
      countEachRequestAsOneUnit: "将每个请求视为一个单元。",
      countLlmTokenUsage: "计算 LLM 令牌的使用情况。",
      createAKeySoCallersCanAuthenticateWithoutExposingProviderCredentials:
        "创建密钥，以便调用者可以在不暴露提供商凭据的情况下进行身份验证。",
      createAModelBeforeTestingChatTraffic: "在测试聊天流量之前创建模型。",
      createAnLlmModelBeforeWiringClientsToTheGateway:
        "在将客户端连接到网关之前创建 LLM 模型。",
      createAnMcpServerBeforeTestingMcpTraffic:
        "在测试 MCP 流量之前创建 MCP 服务器。",
      createTheFirstModelToMakeLlmTrafficAvailableThroughTheGateway:
        "创建第一个模型以使 LLM 流量可通过网关使用。",
      createTheLlmConfigurationSectionSoModelsProvidersKeysGuardrailsLogsAndPlayground_197f4qj:
        "创建 LLM 配置，以便管理模型、提供商、密钥、防护规则、日志和演练场工具。",
      createTheMcpConfigurationSectionSoServersAndMcpPlaygroundToolsCanBeConfigured:
        "创建 MCP 配置部分，以便可以配置服务器和 MCP Playground 工具。",
      createTheTrafficConfigurationSectionSoHttpGatewaysRoutesBackendsAndPoliciesCanBeConfigured:
        "创建流量配置部分，以便可以配置 HTTP 网关、路由、后端和策略。",
      createThis: "创建这个",
      credentialLocation: "凭据位置",
      credentials: "凭据",
      creditCard: "信用卡",
      creditCardNumberPattern: "信用卡号码模式。",
      csv: "CSV",
      currentPolicyYaml: "当前策略 YAML",
      currentTopLevelPolicyYaml: "当前顶级策略 YAML",
      cursorSettings: "光标设置",
      custom: "自定义",
      customAuthDetected: "检测到自定义身份验证",
      customCelFunctionsAvailableToAllCelExpressionsTheseCanDefineReUsableSnippetsThat_1mw84ev:
        '自定义 CEL 函数可用于所有 CEL 表达式。这些可以定义可重复使用的片段\n可以用在任何表达式中。\n配置为包含一个或多个定义的块字符串，例如：\n`customFunctions: |`\n`  isInternal() { request.headers["x-env"] == "internal" }`\n`  this.joined(prefix, parts...) { prefix + this + parts.join("") }`',
      customCosts: "定制费用",
      customHeaderLocation: "自定义请求头位置",
      customProvider: "定制提供商",
      customRegex: "自定义正则表达式",
      customRegexPatterns: "自定义正则表达式模式",
      customSessionNameRoleSessionNameForCloudTrailAndCostUsageReportAttributionMax64C_1kyvvc8:
        "CloudTrail 和成本和使用情况报告的自定义会话名称 (RoleSessionName)\n归因。最多 64 个字符，匹配 `[\\w+=,.@-]`。如果未设置，AWS 开发工具包\n生成一个随机会话名称。",
      customize: "自定义",
      databaseOnlyFieldsToAddComputedFromCelExpressions:
        "要添加的仅数据库字段，根据 CEL 表达式计算。",
      databaseSpecificAccessLogSettings: "特定于数据库的访问日志设置。",
      decodeValidApiKeysForLaterPolicyUseWarningThisAllowsRequestsWithMissingOrInvalidApiKeys:
        "解码有效的 API 密钥以供以后策略使用。\n警告：这允许请求缺少或无效的 API 密钥。",
      decodeValidJwtsForLaterPolicyUseWarningThisAllowsRequestsWithMissingOrInvalidJwts:
        "解码有效的 JWT 以供以后策略使用。\n警告：这允许请求缺少或无效的 JWT。",
      dedicatedPort: "专用端口",
      default: "默认",
      defaultRequestBodyValuesAddedOnlyWhenTheClientDidNotProvideThem:
        "仅当客户端未提供默认请求正文值时才添加它们。",
      defaultRequestValues: "默认请求值",
      defaultAuthorizationBearerToken: "默认：授权：不记名令牌",
      defaultsAllowsSettingDefaultValuesForTheRequestIfTheseAreNotPresentInTheRequestB_1hv3k3o:
        "defaults 允许为请求设置默认值。如果这些不存在于请求正文中，则会设置它们。\n即使设置后仍要覆盖，请使用 `overrides`。",
      defaultsDefinesProviderLevelPolicyDefaultsModelLevelPolicyFieldsOverrideThese:
        "defaults 定义提供商级别的策略默认值。模型级策略字段会覆盖这些字段。",
      defaultsYaml: "默认 YAML",
      defineReusableProviderCredentialsAndConnectionSettingsForModels:
        "为模型定义可重用的提供商凭据和连接设置。",
      definesHowTheProxyBehavesWhenAWebhookGuardrailIsUnreachableOrReturnsAnErrorDefau_1h8u7be:
        "定义当 webhook 防护规则无法访问或\n返回错误。\n\n默认为 `failClosed`。当关闭失败时，错误将被传播并\nLLM请求被拒绝。打开失败时，允许请求\n尽管 webhook 失败，但仍然通过。",
      definesHowTheProxyBehavesWhenTheRemoteRateLimitServiceIsUnavailableOrReturnsAnEr_15rgoat:
        "定义远程速率限制服务启动时代理的行为方式\n不可用或返回错误。\n\n默认为 `FailClosed`。关闭失败时，出现 500 Internal Server Error\n当服务不可用时返回。当打开失败时，请求是\n尽管服务失败仍允许通过。\n\n# 配置\n\n驼峰命名法（`failOpen`、`failClosed`）和帕斯卡命名法（`FailOpen`、\n`FailClosed`) 在配置文件中被接受",
      delayBetweenRetryAttempts: "重试尝试之间的延迟。",
      delete: "删除",
      deleteValue: "删除 {{value}}",
      deleteValue_pkbukw: "删除 {{value}}？",
      deleteBind: "删除绑定",
      deleteGateway: "删除网关",
      deleteGuardrail: "删除防护规则？",
      deleteKey: "删除密钥",
      deleteListener: "删除监听器",
      deleteMcpServer: "删除 MCP 服务器？",
      deleteModel: "删除模型",
      deletePolicy: "删除策略",
      deleteProvider: "删除提供商",
      deleteProvider_1j44lo: "删除提供商？",
      deleteRoute: "删除路由",
      deleteRoute_akv0fs: "删除路由？",
      deleteServer: "删除服务器",
      deleteVirtualApiKey: "删除虚拟 API 密钥？",
      deny: "拒绝",
      denyRequestsWhenTheRateLimitServiceIsUnavailable:
        "当速率限制服务不可用时拒绝请求。",
      denyStatus: "拒绝状态",
      denyTheRequestWhenTheAuthorizationServiceCannotMakeADecision:
        "当授权服务无法做出决定时拒绝请求。",
      denyTheRequestWhenThisCelExpressionIsTrue:
        "当此 CEL 表达式为 true 时拒绝请求。",
      denyTheRequestWithA500StatusWhenTheRateLimitServiceIsUnavailableDefault:
        "当速率限制服务不可用时（默认），拒绝状态为 500 的请求。",
      denyTheRequestWithTheConfiguredHttpStatusCode:
        "使用配置的 HTTP 状态代码拒绝请求。",
      denyWithStatus: "拒绝状态",
      descriptor: "描述符",
      descriptorEntriesSentToTheRemoteServiceValuesAreCelExpressionsEvaluatedFromTheRequest:
        "发送到远程服务的描述符条目。值是根据请求求值的 CEL 表达式。",
      descriptorEntryKeySentToTheRemoteRateLimitService:
        "发送到远程速率限制服务的描述符输入键。",
      descriptorKeyValueEntriesValuesAreCelExpressionsEvaluatedFromTheRequest:
        "描述符键/值条目。值是根据请求求值的 CEL 表达式。",
      descriptors: "描述符",
      descriptorsSentToTheRemoteRateLimitService:
        "发送到远程速率限制服务的描述符。",
      detectCommonSensitiveDataTypesWithBuiltInRegexRules:
        "使用内置正则表达式规则检测常见敏感数据类型。",
      detectJailbreakAttempts: "检测越狱尝试",
      detectTextJailbreakConfigurationForDetectingJailbreakAttemptsOnlyApplicableToRequestGuards:
        "检测文本越狱配置用于检测越狱尝试。\n仅适用于请求警卫。",
      detectUnhealthyBackendResponsesAndTemporarilyRemoveUnhealthyEndpoints:
        "检测不健康的后端响应并暂时删除不健康的端点。",
      detectedLegacyBindsConfig: "检测到旧版绑定配置",
      detectingConfigurationMode: "检测配置模式",
      detectingTrafficConfigurationMode: "正在检测流量配置模式",
      developer: "开发商",
      disable: "禁用",
      disableApiKeyPolicy: "禁用 API 密钥策略",
      disableApiKeyPolicy_ckgvai: "禁用 API 密钥策略",
      disableApiKeyPolicy_9229n3: "禁用 API 密钥策略？",
      disableVirtualApiKeyValidationRequestsWillNoLongerBeValidatedAgainstVirtualApiKeys:
        "禁用虚拟 API 密钥验证？将不再根据虚拟 API 密钥验证请求。",
      disabled: "已禁用",
      discardChanges: "放弃更改",
      discardUnsavedChanges: "放弃未保存的更改？",
      discovery: "发现",
      discoveryOverride: "发现覆盖",
      dnsResolverSettings: "DNS 解析器设置。",
      doNotApplyPromptGuardsToStreamingResponsesOrRealtimeWebsocketMessages:
        "不要将提示防护应用于流式响应或实时 Websocket 消息。",
      doNotExposeTheUiOnATrafficGateway: "不要在流量网关上公开 UI。",
      doNotOpenASocketTheBindIsRegisteredForRoutingOnlyAndIsReachableViaInProcessReEnt_9sz4lu:
        "不要打开套接字。绑定仅注册用于路由并且可访问\n通过进程内重新进入（例如另一个监听器将 CONNECT 流量重定向到它）。",
      doNotPreserveMcpSessionStateBetweenRequests:
        "不要在请求之间保留 MCP 会话状态。",
      doNotRunThisProcessorForMatchingMethods: "不要运行此处理器来匹配方法。",
      doNotSendHeadersToTheExternalProcessingService:
        "不要将请求头发送到外部处理服务。",
      doNotSendTheBodyToTheExternalProcessingService:
        "请勿将正文发送至外部处理服务。",
      doNotSendTheBodyToTheProcessor: "请勿将正文发送至处理器。",
      doNotSendThisPhaseToTheExternalProcessor:
        "不要将此阶段发送到外部处理器。",
      doNotSendTrailersToTheExternalProcessingService:
        "请勿将拖车发送至外部处理服务。",
      documentation: "文档",
      domain: "域名",
      download: "下载",
      duration: "持续时间",
      dynamic: "动态",
      dynamicBackendSelectionIsEnabledForThisBackend:
        "为此后端启用动态后端选择。",
      eachCelExpressionIsSavedUnderAllowDenyOrRequire:
        "每个 CEL 表达式都保存在允许、拒绝或要求下。",
      edit: "编辑",
      editValueGuard: "编辑{{value}}守卫",
      editBind: "编辑绑定",
      editGateway: "编辑网关",
      editKey: "编辑键",
      editListener: "编辑监听器",
      editModel: "编辑模型",
      editProvider: "编辑提供商",
      editRoute: "编辑路由",
      editServer: "编辑服务器",
      editTheFullGatewayYaml: "编辑完整的网关 YAML。",
      editThoseListenersThroughRawYamlOrSplitTheRoutesAcrossSeparateListeners:
        "通过原始 YAML 编辑这些监听器或将路由拆分到不同的监听器。",
      email: "电子邮件",
      emailAddressPattern: "电子邮件地址模式。",
      enable: "启用",
      enableIncludePromptsAndCompletionsInLogsIn:
        "启用“在日志中包含提示和完成”",
      enableValue: "启用 {{value}}",
      enableDeveloperMode: "启用开发者模式",
      enableDownstreamProxyProtocolHandlingOnThisGatewayOrPortIncludingVersionMatching_9ksq9m:
        "在此网关或端口上启用下游代理协议处理，包括\n版本匹配以及 PROXY 请求头是必需的还是可选的。",
      enableLlm: "启用 LLM",
      enableMcp: "启用MCP",
      enableOrDisableDownstreamHttpConnectHandling:
        "启用或禁用下游 HTTP CONNECT 处理。",
      enableTheCapabilitiesYouWantToOperateFromTheSetupPath:
        "从设置路径启用您想要操作的功能。",
      enableTraffic: "启用流量",
      enabled: "已启用",
      enabled_17fi4vy: "已启用",
      encodeHttp1HeaderNamesInLowercase: "将 HTTP/1 请求头名称编码为小写。",
      endpoint: "端点",
      endpointPickerBackendThatSelectsTheDestinationEndpoint:
        "选择目标端点的端点选择器后端。",
      enforceThatTheSubjectSMayActClaimAuthorizesTheActorBeforeExchanging:
        "在交换之前强制主体的 `may_act` 声明对参与者进行授权。",
      enforcement: "强制执行",
      enterTheGatewayUrlAndVirtualApiKeySaveThenRestartClaudeDesktop:
        "输入网关 URL 和虚拟 API 密钥，保存，然后重新启动 Claude Desktop。",
      entries: "条目",
      envVar: "环境变量",
      environmentMustBeAYamlMapping: "环境必须是 YAML 映射。",
      environmentYaml: "环境 YAML",
      error: "错误",
      evaluate: "评价",
      evaluatePolicyExpressionsAgainstSampleOrCustomRequestContextUsingTheGatewayCelEndpoint:
        "使用网关 CEL 端点根据示例或自定义请求上下文评估策略表达式。",
      evaluateRequestCountDescriptorsWhileProcessingTheRequest:
        "在处理请求时评估请求计数描述符。",
      evaluateTokenDescriptorsAfterTheLlmResponseCompletes:
        "LLM 响应完成后评估令牌描述符。",
      everyListedHeaderConditionMustMatch: "每个列出的请求头条件都必须匹配。",
      everyListedQueryConditionMustMatch: "每个列出的查询条件必须匹配。",
      evictionDuration: "驱逐持续时间",
      exampleComExampleCom: "example.com、*.example.com",
      expectedAYamlMapping: "需要 YAML 映射。",
      expectedTokenIssuerMatchedAgainstTheJwtIssClaim:
        "预期的令牌签发者，与 JWT `iss` 声明相匹配。",
      explicit: "显式的",
      explicitBackendReferenceBackendMustBeDefinedInTheTopLevelBackendsList:
        "显式后端引用。后端必须在顶级后端列表中定义",
      explicitEndpoints: "显式端点",
      explicitOutgoingModel: "显式传出模型",
      export: "导出",
      exposeHeaders: "公开请求头",
      exposeTheUiOnATrafficGatewayAndConfigurePoliciesThatProtectTheUi:
        "在流量网关上公开 UI 并配置保护 UI 的策略。",
      exposeToolNamesWithoutAddingTheTargetName:
        "公开工具名称而不添加目标名称。",
      expression: "表达式",
      expressionToDetermineTheAmountOfClientSamplingClientSamplingDeterminesWhetherToI_12geacf:
        "用于确定*客户端采样*数量的表达式。\n如果传入请求已有跟踪，则客户端采样确定是否启动新的跟踪范围。\n该值应计算为 0.0-1.0 (0-100%) 之间的浮点数或 true/false。\n这默认为“true”。",
      expressionToDetermineTheAmountOfRandomSamplingRandomSamplingWillInitiateANewTrac_1d5h2qd:
        "确定*随机采样*数量的表达式。\n如果传入请求尚无跟踪，则随机采样将启动新的跟踪范围。\n该值应计算为 0.0-1.0 (0-100%) 之间的浮点数或 true/false。\n默认为“假”。",
      externalAuthz: "外部授权",
      externalMcpPolicyProcessors: "外部 MCP 策略处理器。",
      externalProcessor: "外部处理器",
      externalServiceTheGatewayCallsForThisPolicy: "网关调用此策略的外部服务。",
      extraFormParametersAppendedToTheTokenRequestValuesAreCelExpressionsEvaluatedAgai_11xnn0t:
        "附加到令牌请求的额外表单参数。\n值是根据传入请求计算的 CEL 表达式。",
      failClosed: "失败时拒绝",
      failOpen: "失败时放行",
      failTheEntireSessionIfAnyTargetFailsToInitializeOrAnyUpstreamFailsDuringAFanoutT_f2p346:
        "如果任何目标无法初始化或任何目标失败，则整个会话失败\n上游在扇出期间失败。这是默认值并且匹配\n当前的行为。",
      failover: "故障转移",
      failoverEnablesPriorityBasedSelectionOfTheTargetModelWithinAPriorityLevelTheBest_1lo0fhc:
        "故障转移支持基于优先级的目标模型选择。\n在优先级内，通过考虑健康因素的综合评分选择最佳提供商\n和延迟。\n如果优先级内的所有模型都降级，请求将移至下一个优先级组。",
      failoverTargets: "故障转移目标",
      failureMode: "失败模式",
      featuresAndRoutesReferenceThisGatewayByName:
        "各项功能和路由通过名称引用此网关。",
      feedback: "反馈",
      fetchAnAccessToken: "获取访问令牌",
      fetchAnIdToken: "获取 id 令牌",
      fetchSigningKeysFromTheIssuerJwksEndpoint:
        "从签发者 JWKS 端点获取签名密钥。",
      fetchingRecentLlmCalls: "正在获取最近的 LLM 调用。",
      file: "文件",
      fillInterval: "填充间隔",
      fixTheHighlightedFieldsBeforeSaving: "请在保存前修正高亮字段。",
      fixTheHighlightedProcessorsBeforeSaving: "保存前修复突出显示的处理器。",
      forAzureTheApiVersionToUse: "对于 Azure：要使用的 API 版本",
      forAzureTheFoundryProjectNameRequiredForFoundryResourceType:
        "对于 Azure：Foundry 项目名称（Foundry 资源类型必需）",
      forAzureTheResourceNameOfTheDeployment: "对于 Azure：部署的资源名称",
      forAzureTheTypeOfAzureEndpointOpenAiOrFoundry:
        "对于 Azure：Azure 端点的类型（openAI 或 foundry）",
      forwardTheRequestToTheAdminApiUsingTheRequestSCurrentPathAndQuery:
        "使用请求的当前路径和查询将请求转发到管理 API。",
      forwardTheValidatedIncomingJwtToTheBackend:
        "将经过验证的传入 JWT 转发到后端。",
      foundry: "Foundry",
      fractionOfMatchingRequestsToMirrorFrom00To10:
        "镜像匹配请求的分数，从 0.0 到 1.0。",
      from: "来自",
      fromTheSameDirectory: "来自同一目录。",
      frontendPoliciesDefinesTopLevelPoliciesApplyingToAllTraffic:
        "frontendPolicies 定义适用于所有流量的顶级策略。",
      full: "满",
      fullDuplexStreamed: "全双工流式传输",
      fullyQuitAndRelaunchClaudeDesktopANew:
        "完全退出并重新启动 Claude Desktop。一个新的",
      gateway: "网关",
      gatewayValue: "网关 {{value}}",
      gatewayBaseUrl: "网关基础 URL",
      gatewayBinding: "网关绑定",
      gatewayError: "网关错误",
      gatewayOrGatewayListenerThatOwnsThisRoute:
        "拥有此路由的网关或网关监听器。",
      gatewayOverview: "网关概览",
      gatewayPolicies: "网关策略",
      gatewaySaved: "网关已保存",
      gatewaySent: "网关已发送",
      gatewaySurfaces: "网关功能入口",
      gatewayListenerRouteOrBackendThatThisPolicyAttachesTo:
        "此策略附加到的网关、监听器、路由或后端。",
      gateways: "网关",
      gatewaysAttachesTheLlmRoutesToNamedGatewaysThisCanTakeTheFormOfGatewayNameOrGate_n9bphz:
        "gateways 将 LLM 路由附加到命名网关。这可以采用 `<gateway-name>` 或 `<gateway-name>/<listener-name>` 的形式来附加到网关内的特定监听器。\n当省略且存在名为 `default` 的网关时，LLM API 路由将附加到该网关，除非设置了 `port`。",
      gatewaysAttachesTheMcpRoutesToNamedGatewaysThisCanTakeTheFormOfGatewayNameOrGate_19pj37b:
        "gateways 将 MCP 路由附加到命名网关。这可以采用 `<gateway-name>` 或 `<gateway-name>/<listener-name>` 的形式来附加到网关内的特定监听器。\n当省略且存在名为 `default` 的网关时，MCP 路由将附加到该网关，除非设置了端口。",
      gatewaysAttachesTheUiAndUiBackendRoutesToNamedGatewaysThisCanTakeTheFormOfGatewa_1hlnrin:
        "gateways 将 UI 和 UI 后端路由附加到命名网关。这可以采用 `<gateway-name>` 或 `<gateway-name>/<listener-name>` 的形式来附加到网关内的特定监听器。\n当省略且存在名为 `default` 的网关时，UI 路由将附加到它。",
      gatewaysAttachesThisRouteToNamedGatewaysOrGatewayListenersThisCanTakeTheFormOfGa_j7n552:
        "gateways 将此路由附加到命名网关或网关监听器。\n这可以采用 `<gateway-name>` 或 `<gateway-name>/<listener-name>` 的形式来附加到网关内的特定监听器。\n如果未设置，将使用“默认”网关。",
      gatewaysAttachesThisRouteToNamedTcpTlsGatewaysOrGatewayListenersThisCanTakeTheFo_6uai65:
        "gateways 将此路由附加到指定的 TCP/TLS 网关或网关监听器。\n这可以采用 `<gateway-name>` 或 `<gateway-name>/<listener-name>` 的形式来附加到网关内的特定监听器。\n如果未设置，将使用“默认”网关。",
      gatewaysDefinesTheEntrypointToTheProxySettingUpPortsAndListenersThatFeaturesLlmM_18ageg5:
        "gateways 定义代理的入口点，设置功能（LLM、MCP 和 UI）和路由可以附加的端口和监听器。\n每个网关定义一个代理将侦听的端口，以及该端口的可选 TLS 设置。",
      generateConnectionSettingsAndSnippetsForOpenAiCompatibleLlmClients:
        "为 OpenAI 兼容的 LLM 客户端生成连接设置和片段。",
      generatedModelConfig: "生成的模型配置",
      generatedProviderConfig: "生成的提供商配置",
      generatedVirtualModelConfig: "生成的虚拟模型配置",
      getStarted: "开始使用",
      gitHubCopilot: "GitHub Copilot",
      googleCredentials: "Google 凭据",
      googleModelArmor: "Google Model Armor",
      group: "组",
      groupAttribute: "群组属性",
      groupBy: "分组方式",
      group_sf1daa: "组：",
      groups: "用户组",
      gRpcDetails: "gRPC 详细信息",
      guardType: "防护类型",
      guardThisTakesEffectImmediately: "防护规则？此更改会立即生效。",
      guardrailIdentifier: "防护规则标识符",
      guardrailVersion: "防护规则版本",
      guardrails: "防护规则",
      guardrailsToApplyToEveryConfiguredModel: "适用于每个配置模型的防护规则。",
      guardrailsToApplyToTheRequestOrResponse: "应用于请求或响应的防护规则",
      guardsAppliedToClientRequestsBeforeTheyReachTheLlm:
        "在客户请求到达 LLM 之前，对其应用防护措施。",
      guardsAppliedToLlmResponsesBeforeTheyReachTheClient:
        "在 LLM 响应到达客户端之前，警卫对其进行应用。",
      haltOnBlocklistHit: "遇到阻止列表时停止",
      handleCorsPreflightRequestsAndAppendConfiguredCorsHeadersToApplicableRequests:
        "处理 CORS 预检请求并将配置的 CORS 请求头附加到适用的请求。",
      handleCsrfProtectionByValidatingRequestOriginsAgainstConfiguredAllowedOrigins:
        "通过根据配置的允许来源验证请求来源来处理 CSRF 保护。",
      headerAllowlist: "请求头白名单",
      headerCasingBehaviorForHttp1Responses: "HTTP/1 响应的请求头大小写行为。",
      headerLocation: "请求头位置",
      headerName: "请求头名称",
      headerName_8vzq77: "请求头名称",
      headerNameContainingTheCredential: "包含凭证的请求头名称。",
      headerNamesToRemove: "要删除的请求头名称。",
      headerPrefix: "请求头前缀",
      headerValue: "请求头值",
      headers: "请求头",
      headersToAddToTheAuthorizationRequestUsingCelExpressionsEmptyMeansAllHeaders:
        "使用 CEL 表达式添加到授权请求的请求头。空意味着所有标题。",
      headersToAddSetOrRemoveFromTheRejectionResponse:
        "要从拒绝响应中添加、设置或删除的请求头。",
      headersToAppendUsingCelExpressionsForValues:
        "使用 CEL 表达式附加值的请求头。",
      headersToAppendWithoutReplacingExistingValues:
        "要附加的请求头而不替换现有值。",
      headersToSetUsingCelExpressionsForValues:
        "使用 CEL 表达式设置值的请求头。",
      headersToSetReplacingAnyExistingValues:
        "要设置的请求头，替换任何现有值。",
      health: "健康",
      healthConfiguresOutlierDetectionForThisModelBackend:
        "health 为此模型后端配置异常值检测。",
      healthScoreThresholdBelowWhichAnUnhealthyResponseCanEvictTheBackend:
        "健康分数阈值，低于该阈值，不健康的响应可能会驱逐后端。",
      healthScoreToRestoreWhenTheBackendReturnsFromEviction:
        "当后端从驱逐中返回时恢复健康分数。",
      healthThreshold: "健康阈值",
      help: "帮助",
      hide: "隐藏",
      hideFullKey: "隐藏完整密钥",
      home: "首页",
      host: "主机",
      hostOrPortRewriteToApplyBeforeForwardingTheRequest:
        "主机或端口重写以在转发请求之前应用。",
      hostOrPortRewriteToApplyToTheRedirectUrl:
        "主机或端口重写以应用于重定向 URL。",
      hostname: "主机名",
      hostnameDefinesWhatHostnamesAreServedUnderThisListenerCanBeAWildcardThisAllowsSe_w5k5cr:
        "主机名定义在此监听器下提供服务的主机名。可以是通配符。\n这允许使用不同的 TLS 配置为多个域提供服务。\n如果未设置，则将提供所有域（隐式通配符）。",
      hostnameOrIpAddress: "主机名或 IP 地址",
      hostnames: "主机名",
      howDownstreamHttpConnectRequestsAreHandled:
        "如何处理下游 HTTP CONNECT 请求。",
      howLongAnIdleHttp1ConnectionMayStayOpen:
        "空闲 HTTP/1 连接可以保持打开状态多长时间。",
      howLongToEvictAnUnhealthyBackend: "驱逐不健康的后端需要多长时间。",
      howOftenTheLocalBucketIsRefilled: "本地存储桶重新装满的频率。",
      howRequestBodiesAreSentToTheExternalProcessingService:
        "如何将请求正文发送到外部处理服务。",
      howResponseBodiesAreSentToTheExternalProcessingService:
        "如何将响应正文发送到外部处理服务。",
      howTheGatewayConnectsToThisMcpTarget: "网关如何连接到此 MCP 目标。",
      howToUseTheDestinationReturnedByTheEndpointPicker:
        "如何使用端点选择器返回的目的地。",
      httpAndTcpListenersRoutesAndPolicyControls:
        "HTTP 和 TCP 监听器、路由和策略控制。",
      httpDetails: "HTTP 详细信息",
      httpProtocolSettingsForThisBackend: "该后端的 HTTP 协议设置。",
      httpResponseStatusCodesThatShouldBeRetried:
        "应重试的 HTTP 响应状态代码。",
      httpStatus: "HTTP状态",
      httpStatusCodeReturnedWhenContentIsRejected:
        "内容被拒绝时返回的 HTTP 状态代码。",
      httpStatusCodeToReturnForTheRedirect: "为重定向返回的 HTTP 状态代码。",
      httpStatusCodeToReturn: "要返回的 HTTP 状态代码。",
      httpVersionToUseWhenConnectingToTheBackend:
        "连接到后端时使用的 HTTP 版本。",
      httpProxy: "HTTP：代理",
      http2ConnectionFlowControlWindowSize: "HTTP/2 连接流量控制窗口大小。",
      http2StreamFlowControlWindowSize: "HTTP/2 流流量控制窗口大小。",
      identifierOfTheResourceAuthorizationServerTheIssuedIdJagIsBoundToThisAudience:
        "资源授权服务器的标识符。已发行的 ID-JAG 对该受众具有约束力。",
      identifyTheOauth2ClientUsedByTheGatewayDuringTheAuthorizationCodeFlow:
        "识别网关在授权代码流期间使用的 OAuth2 客户端。",
      identityProviderTypeUsedToDeriveMcpAuthorizationMetadataAndDefaultJwksUrls:
        "用于派生 MCP 授权元数据和默认 JWKS URL 的身份提供商类型。",
      ifATokenExistsValidateItWarningThisAllowsRequestsWithoutAJwtTokenAdditionally401_dgw23w:
        "如果令牌存在，请验证它。\n警告：这允许没有 JWT 令牌的请求！另外，401错误不会被返回，\n这不会触发客户端启动 oauth 流程。",
      inYourProjectRoot: "在你的项目根目录中。",
      includeMcpTools: "包括 MCP 工具（",
      includeMcpToolsValueServers_one: "包括 MCP 工具（{{count}} 个服务器）",
      includeMcpToolsValueServers_other: "包括 MCP 工具（{{count}} 个服务器）",
      includePromptsAndCompletionsInLogs: "在日志中包含提示和完成情况",
      includeRequestBody: "包含请求正文",
      includeRequestHeaders: "包含请求头",
      includeResponseHeaders: "包含响应请求头",
      incomingModel: "入站模型",
      incomingModelMatch: "传入模型匹配",
      incomingRequestHeadersToForwardToTheWebhook:
        "要转发到 Webhook 的传入请求头。",
      inheritance: "继承",
      initialize: "初始化",
      initializeAGatewayMcpSessionListToolsAndCallAToolThroughTheMcpListener:
        "初始化网关 MCP 会话、列出工具并通过 MCP 监听器调用工具。",
      initializeFirst: "请先初始化",
      initializeOrSendAToolRequestToInspectMcpBehavior:
        "初始化或发送工具请求以检查 MCP 行为。",
      initializeTheSessionAndSelectAToolToConfigureArguments:
        "初始化会话并选择一个工具来配置参数。",
      initialized: "已初始化",
      initializingMcpTools: "正在初始化 MCP 工具",
      inlineJson: "内嵌 JSON",
      inlineJwks: "内嵌 JWKS",
      inlineOverridesStoredInThisGatewayConfigurationValuesAreUsdPer1MTokens:
        "内联覆盖存储在此网关配置中。价值为每 100 万个令牌美元。",
      input: "输入",
      inputSchema: "输入模式",
      inspect: "检查",
      inspectModelOutputBeforeItIsReturnedToTheCaller:
        "在模型输出返回给调用方之前进行检查。",
      inspectPromptsBeforeTheyReachTheUpstreamModel:
        "在提示词到达上游模型之前进行检查。",
      inspectRecentLlmCallsAndRequestResponsePayloads:
        "检查最近的 LLM 调用和请求/响应负载。",
      integration: "整合",
      internalModelsCanBeTargetedByVirtualModelsButCannotBeRequestedDirectly:
        "虚拟模型可以定位内部模型，但不能直接请求。",
      interval: "间隔",
      intervalBetweenHttp2KeepalivePings:
        "HTTP/2 keepalive ping 之间的时间间隔。",
      invalidAuthorizationPolicy: "授权策略无效",
      invalidCustomCosts: "无效的定制成本",
      invalidGuardrails: "无效的防护规则",
      invalidJson: "无效的 JSON",
      invalidJwtPolicy: "无效的 JWT 策略",
      invalidMcpAuthenticationPolicy: "无效的 MCP 身份验证策略",
      invalidMcpGuardrailsPolicy: "无效的 MCP 防护规则策略",
      invalidModelPolicies: "无效的模型策略",
      invalidOidcPolicy: "无效的 OIDC 策略",
      invalidServer: "服务器无效",
      invalidYaml: "无效的 YAML",
      issuer: "签发者",
      issuerUsedForDiscoveryAndIdTokenValidation:
        "用于发现和 ID 令牌验证的签发者。",
      jailbreakApiVersion: "越狱API版本",
      jsonWebKeySetUsedToVerifyTokenSignaturesCanBeInlineFromAFileOrFetchedRemotely:
        "JSON Web 密钥集用于验证令牌签名。可以内联、从文件或远程获取。",
      jwksFile: "JWKS文件",
      jwksSource: "JWKS 来源",
      jwksSourceUsedToValidateReturnedIdTokens:
        "JWKS 源用于验证返回的 ID 令牌。",
      jwksUrl: "JWKS URL",
      jwtAuth: "JWT 身份验证",
      jwtValidationOptionsControllingWhichClaimsMustBePresentInATokenTheRequiredClaims_12osoae:
        'JWT 验证选项控制令牌中必须存在哪些声明。\n\n`required_claims` 集指定哪些 RFC 7519 注册声明必须\n在验证继续之前存在于令牌有效负载中。仅以下内容\n识别值：`exp`、`nbf`、`aud`、`iss`、`sub`。其他注册\n诸如 `iat` 和 `jti` 之类的声明**不**由底层强制执行\n`jsonwebtoken` 库将被默默忽略。\n\n这仅强制**存在**。标准声明，例如 `exp` 和 `nbf`\n独立验证它们的值（例如，始终检查过期时间\n当 `exp` 声明存在时，无论此设置如何）。\n\n默认为 `["exp"]`。',
      keepServingTrafficWhileSurfacingJwtDataWhenPossible:
        "在可用时提取 JWT 数据，同时继续转发流量。",
      key: "密钥",
      keyExchangeGroupsAllowedForNegotiatingTls: "允许协商 TLS 的密钥交换组。",
      keyValue: "密钥值",
      kind: "种类",
      last1Hour: "最后 1 小时",
      last12Hours: "过去 12 小时",
      last14Days: "过去 14 天",
      last24Hours: "过去 24 小时",
      last30Days: "过去 30 天",
      last7Days: "过去 7 天",
      leaveEmptyToUseDefault5xxAndConnectionFailureHandling:
        "留空以使用默认 5xx 和连接失败处理。",
      leaveTheAuthorityUnchanged: "保持权限不变。",
      letTheModelCallToolsExposedByTheMcpGateway:
        "让模型调用MCP网关公开的工具。",
      limitByRequestCount: "按请求数量限制。",
      limitByTokenCount: "按令牌数量限制。",
      limitOverride: "限制超越",
      limitType: "限位类型",
      limitOverrideDeterminesTheOptionalExpressionToDetermineTheLimitOfTheRequestThisT_6mrd6s:
        'limitOverride 确定可选表达式来确定请求的限制。\n这告诉远程服务器对请求应用什么限制。\n注意：这并不指定请求的“成本”，这是由 `cost` 字段完成的。\n该表达式必须计算为具有 `unit` 和 `requestsPerUnit` 键的映射。例如：\n`{"unit":"second","requestsPerUnit":100}`。\n有效单位：秒、分、小时、日、月、年\n如果表达式无法计算，则跳过描述符。',
      listener: "聆听者",
      listenerPolicies: "监听器策略",
      listenerThatOwnsThisRoute: "拥有该路由的监听器。",
      listenerYaml: "监听器 YAML",
      listeners: "监听器",
      listeners_1fzojr3: "监听器·",
      listenersDefinesMultipleNamedListenersUnderThisGatewayWhenSetOnlyPortMayBeConfig_e7d148:
        "Listeners 在此网关下定义了多个命名监听器。设置后，只能在顶级网关上配置 `port`。",
      llmCosts: "LLM 成本",
      llmDefinesASetOfLlmModelsToBeExposedByTheProxyWhenConfiguredLlmModelsWillBeServe_beutm3:
        "llm 定义了一组由代理公开的 LLM 模型。配置后，LLM 模型将是\n使用标准服务路径（`/v1/models`、`/v1/chat/completions` 等）在附加的 `gateways` 下提供服务。",
      llmGuardrails: "LLM 防护规则",
      llmModels: "LLM 模型",
      llmPlayground: "LLM 演练场",
      llmPolicies: "LLM 策略",
      llmProviders: "LLM 提供商",
      llmRequestFields: "LLM 请求字段",
      llmRequestModelStripPrefixAnthropic:
        'llmRequest.model.stripPrefix("anthropic/")',
      loadingAnalytics: "Loading analytics...",
      loadingEditor: "正在加载编辑器...",
      loadingGatewayConfiguration: "正在加载网关配置",
      loadingGateways: "加载网关",
      loadingGuardrails: "正在加载防护规则",
      loadingKeys: "加载密钥",
      loadingLogPayload: "加载日志负载",
      loadingMcpServers: "加载 MCP 服务器",
      loadingModelCatalog: "正在加载模型目录…",
      loadingModels: "加载模型",
      loadingProviders: "正在加载提供商",
      loadingRawConfiguration: "正在加载原始配置…",
      loadingRuntimePolicies: "加载运行时策略",
      loadingRuntimeTrafficConfiguration: "正在加载运行时流量配置",
      loadingTrafficListeners: "正在加载流量监听器",
      loadingTrafficRoutes: "正在加载流量路由",
      localFile: "本地文件",
      localRateLimit: "本地速率限制",
      localRateLimitsForIncomingRequests: "传入请求的本地速率限制。",
      localXdsPathIfNotSpecifiedTheCurrentConfigurationFileWillBeUsed:
        "本地 XDS 路径。如果未指定，将使用当前配置文件。",
      localConfigEvictionSubPolicyWithDurationAsStringMirrorsEviction:
        "本地/配置驱逐子策略，持续时间为字符串；镜像 `Eviction`。",
      localConfigHealthPolicyWithCelAsStringConvertedToPolicyByCompilingTheExpressionM_lbnrib:
        "以 CEL 作为字符串的本地/配置健康策略；通过编译表达式转换为策略。\n镜像原始 `Health` 消息结构。",
      location: "地点",
      logSettings: "日志设置",
      logSettings_12oqjpq: "日志设置",
      logs: "日志",
      logsApiError: "日志 API 错误",
      manageGateways: "管理网关",
      manageModelCostCatalogsUsedForAnalyticsAndRequestCostAttribution:
        "管理用于分析和请求成本归因的模型成本目录。",
      managed: "托管",
      managedOnGuardrails: "请在防护规则页面管理",
      managedOnVirtualApiKeys: "请在虚拟 API 密钥页面管理",
      manuallyProvideAuthorizationTokenAndSigningKeyMetadata:
        "手动提供授权、令牌和签名密钥元数据。",
      mapsToTheRequestAttributesFieldInProcessingRequestAndAllowsDynamicCelExpressions:
        "映射到ProcessingRequest中的请求`attributes`字段，并允许动态CEL表达式。",
      mapsToTheResponseAttributesFieldInProcessingRequestAndAllowsDynamicCelExpressions:
        "映射到ProcessingRequest中的响应`attributes`字段，并允许动态CEL表达式。",
      markThisAsLlmTrafficToEnableLlmProcessing:
        "将此标记为 LLM 流量以启用 LLM 处理。",
      markThisTrafficAsA2AToEnableA2AProcessingAndTelemetry:
        "将此流量标记为 A2A 以启用 A2A 处理和遥测。",
      maskMatchedText: "遮盖匹配文本",
      match: "匹配",
      matchAndOptionallyMaskCustomRegularExpressions:
        "匹配并可选地屏蔽自定义正则表达式。",
      matchConditionsAndModelSpecificPolicies: "匹配条件和特定于模型的策略",
      matchIncomingHttpAndTcpTrafficAndAttachInlineBackends:
        "匹配传入的 HTTP 和 TCP 流量并附加内联后端。",
      matches: "匹配条件",
      matchesSpecifiesTheConditionsUnderWhichThisModelShouldBeUsedInAdditionToMatchingTheModelName:
        "matches 指定除了匹配模型名称之外，还应使用该模型的条件。",
      maxAge: "最大年龄",
      maxRequestBytes: "最大请求字节数",
      maxTokens: "最大令牌数",
      maximumBodySizeToBufferInBytes: "缓冲区的最大主体大小（以字节为单位）。",
      maximumHttp2FrameSize: "最大 HTTP/2 帧大小。",
      maximumNumberOfAuthorizationResultsToKeepInTheCache:
        "缓存中保留的授权结果的最大数量。",
      maximumNumberOfHeadersAllowedInAnHttp1RequestChangingThisValueCausesAPerformance_j9b30b:
        "HTTP/1 请求中允许的最大请求头数。更改此值会导致\n即使设置低于默认值 100，性能也会下降。",
      maximumNumberOfTokenExchangeResponsesToKeepInTheCacheSetTo0ToDisable:
        "缓存中保留的令牌交换响应的最大数量。设置为 0 以禁用。",
      maximumNumberOfTokensThatCanAccumulateInTheLocalBucket:
        "本地桶中可以累积的最大令牌数。",
      maximumRequestBodySizeToSendToTheAuthorizationServiceDefaultsTo8192Bytes:
        "发送到授权服务的最大请求正文大小。默认为 8192 字节。",
      maximumRequestOrResponseBodySizeBufferedByTheFrontend:
        "前端缓冲的最大请求或响应正文大小。",
      maximumSizeOfHttp2RequestHeaders: "HTTP/2 请求头的最大大小。",
      maximumSupportedTlsVersionOnlyTls12And13AreSupported:
        "支持的最高 TLS 版本（仅支持 TLS 1.2 和 1.3）。",
      maximumTimeAConnectionMayStayOpenAfterThisDurationTheConnectionIsGracefullyClose_t76f83:
        "连接保持打开状态的最长时间。在此持续时间之后，连接会正常进行\n当前正在进行的请求完成后关闭。对于均匀流量分配很有用\n在扩展事件期间位于负载均衡器后面。",
      maximumTimeAllowedForABackendHttpRequest:
        "后端 HTTP 请求允许的最长时间。",
      maximumTimeAllowedForTheFullDownstreamRequestAndResponse:
        "完整下游请求和响应所允许的最长时间。",
      maximumTimeAllowedForTheUpstreamBackendRequest:
        "上游后端请求允许的最长时间。",
      maximumTimeAllowedToCompleteTheDownstreamTlsHandshake:
        "允许完成下游 TLS 握手的最长时间。",
      maximumTimeAllowedToEstablishABackendTcpConnection:
        "允许建立后端 TCP 连接的最长时间。",
      maximumTlsVersionAcceptedFromDownstreamClients:
        "从下游客户端接受的最大 TLS 版本。",
      mcpAuthentication: "MCP认证",
      mcpAuthorization: "MCP授权",
      mcpBehavior: "MCP 行为",
      mcpBrowserAccessIsNotAllowed: "不允许浏览器访问 MCP",
      mcpDefinesASetOfMcpServersExposedByTheProxyWhenConfiguredTheMcpServersWillBeServ_15ox9e0:
        "mcp 定义了一组由代理公开的 MCP 服务器。配置后，MCP 服务器将\n在 /mcp 和 /sse 处的附加 `gateways` 下提供服务。\n列出的所有 MCP 服务器将用作单个虚拟 MCP 服务器。",
      mcpGatewaySettings: "MCP 网关设置。",
      mcpGuardrails: "MCP 防护规则",
      mcpPlayground: "MCP 演练场",
      mcpPolicies: "MCP 策略",
      mcpRequestFailed: "MCP 请求失败",
      mcpServers: "MCP 服务器",
      mcpToolOutput: "MCP 工具输出",
      mcpGateway: "mcp://gateway",
      measure: "测量",
      menuAppearsInTheMenuBar: "菜单出现在菜单栏中。",
      messageOffset: "消息偏移量",
      messageOffsetUsedWhenChoosingWhereToPlaceCacheMarkers:
        "选择放置缓存标记的位置时使用的消息偏移量。",
      messages: "消息",
      messagesAppendedToTheEndOfEachChatRequest:
        "消息附加到每个聊天请求的末尾。",
      messagesPrependedToTheBeginningOfEachChatRequest:
        "消息添加到每个聊天请求的开头。",
      messagesToAddBeforeOrAfterTheClientPrompt:
        "在客户端提示之前或之后添加的消息。",
      metadata: "元数据",
      metadataAdvertisedToMcpClientsForOauthProtectedResources:
        "向 MCP 客户端通告 OAuth 受保护资源的元数据。",
      metadataContextYaml: "元数据上下文 YAML",
      metadataValuesToAddUsingCelExpressions: "使用 CEL 表达式添加的元数据值。",
      metadataValuesToExposeUnderTheExtauthzVariableAfterAuthorization:
        "授权后在 `extauthz` 变量下公开的元数据值。",
      metadataValuesToSendToTheAuthorizationServiceComputedFromCelExpressionsMapsToThe_1ed0p5i:
        "要发送到授权服务的元数据值，根据 CEL 表达式计算得出。\n映射到请求中的 `metadata_context.filter_metadata` 字段。\n如果未设置，则在还使用 JWT 身份验证时设置 `envoy.filters.http.jwt_authn`，以实现兼容性。",
      method: "方法",
      methodPhases: "方法阶段",
      microsoftEntra: "Microsoft Entra",
      migrateBindsToGateways: "迁移绑定到网关",
      minimalRawHttpRequestForDebuggingClientConnectivity:
        "用于调试客户端连接的最少原始 HTTP 请求。",
      minimumPromptSizeRequiredBeforeCacheMarkersAreAdded:
        "添加缓存标记之前所需的最小提示大小。",
      minimumSupportedTlsVersionOnlyTls12And13AreSupported:
        "支持的最低 TLS 版本（仅支持 TLS 1.2 和 1.3）。",
      minimumTlsVersionAcceptedFromDownstreamClients:
        "从下游客户端接受的最低 TLS 版本。",
      minimumTokens: "最低令牌数",
      mode: "模式。",
      model: "模型",
      modelCelExpression: "模型 CEL 表达式",
      modelCostCatalogSourcesEntriesAreMergedInOrderWithLaterEntriesTakingPrecedence:
        "模型成本目录来源；条目按顺序合并，后面的条目优先。",
      modelIsResolvedAgainstLlmModelsUsingTheSameWildcardMatchingAsClientRequests:
        "model 使用与客户端请求相同的通配符匹配来针对 llm.models 进行解析。",
      modelNameAliasesThatRewriteRequestedModelNames:
        "重写请求的模型名称的模型名称别名。",
      modelPolicies: "模型策略",
      modelUsesAWildcardSpecifyTheSpecificModel:
        "模型使用通配符；请指定具体模型。",
      modelWarnings: "模型警告",
      models: "模型",
      modelsDefinesTheSetOfModelsThatCanBeServedByThisGatewayTheModelNameRefersToTheMo_1qlvcg6:
        "models 定义了该网关可以提供服务的模型集合。模型名称指的是\n用户请求中匹配的名称；发送到实际 LLM 的模型可以按模型单独覆盖。",
      modelsKeysPoliciesAndChatTesting: "模型、密钥、策略和聊天测试。",
      moderationModel: "审核模型",
      moderationModelToUseDefaultsToOmniModerationLatest:
        "要使用的审核模型。默认为 `omni-moderation-latest`。",
      modifyRequestAndResponseDataForThisBackend:
        "修改此后端的请求和响应数据。",
      modifyRequestAndResponseHeadersBodiesOrMetadata:
        "修改请求和响应请求头、正文或元数据。",
      modifyRequestHeadersBeforeForwardingToThisBackend:
        "在转发到此后端之前修改请求头。",
      modifyRequestHeadersBeforeForwarding: "转发前修改请求头。",
      modifyResponseHeadersBeforeReturningToTheClient:
        "返回客户端之前修改响应请求头。",
      modifyResponseHeadersReturnedFromThisBackend:
        "修改从此后端返回的响应请求头。",
      ms: "毫秒",
      multipleListeners: "多个监听器",
      mutation: "变更",
      name: "名称",
      nameAlreadyExists: "名称已存在",
      nameIdentifiesThisListenerForGatewayReferencesLikeGatewaysGatewayNameListenerName:
        "name 标识此监听器以获取网关引用，例如 `gateways: gateway-name/listener-name`。",
      nameIsReferencedFromLlmModelsProviderReference:
        "名称是从 llm.models[].provider.reference 引用的。",
      nameIsRequired: "名称为必填项",
      nameIsTheNameOfTheModelWeAreMatchingFromAUsersRequestIfParamsModelIsSetThatWillB_1ti2su5:
        "name 是我们根据用户请求匹配的模型的名称。如果设置了 params.model，则\n将在向 LLM 提供商提出请求时使用。如果没有，则使用传入模型。",
      nameIsThePublicModelNameClientsRequest: "name 是客户请求的公共模型名称。",
      namespace: "命名空间",
      namespaceKeyCelExpression: "命名空间：键：CEL 表达式",
      never: "从不",
      neverPrefixCallsAreRoutedByToolNameWhichMustBeUniqueAcrossTargets:
        "从不添加前缀；调用按工具名称路由，因此工具名称在所有目标中必须唯一。",
      newKey: "新建密钥",
      noValueTransformationsConfigured: "尚未配置{{value}}转换。",
      noAdditionalMatchConditions: "没有额外的匹配条件。",
      noAnalyticsInTheSelectedWindow: "所选时间范围内没有分析数据。",
      noArgs: "无参数",
      noAuthorizationRules: "没有授权规则",
      noBackendsConfigured: "没有配置后端。",
      noCatalogMatchesCustomModelNamesAreAllowed:
        "目录中没有匹配项，可使用自定义模型名称。",
      noConfiguredModels: "没有已配置的模型",
      noCostCatalogsConfigured: "未配置成本目录",
      noCustomCosts: "没有自定义成本。",
      noGatewaySurfacesEnabledYet: "尚未启用网关功能入口",
      noGatewaysConfigured: "尚未配置网关",
      noGuardsConfigured: "尚未配置防护规则。",
      noHeaderConditions: "没有请求头条件。",
      noLegacyBindsConfigured: "未配置旧绑定",
      noListenersAreAttachedToThisBindInTheRuntimeDump:
        "在运行时转储中没有监听器附加到此绑定。",
      noListenersArePresentInTheActiveGatewayDump:
        "活动网关转储中不存在监听器。",
      noListenersOnThisBind: "此绑定上没有监听器",
      noLlmCallsMatchTheCurrentFilters: "没有符合当前筛选条件的 LLM 调用。",
      noMatches: "没有匹配项",
      noMcpGuardrailProcessors: "没有 MCP 防护处理器",
      noMcpMethodsConfigured: "尚未配置 MCP 方法。",
      noMcpServers: "没有 MCP 服务器",
      noMcpServersConfigured: "未配置 MCP 服务器",
      noMcpToolsAreAvailableFromTheMcpGateway: "MCP 网关未提供可用工具。",
      noMessagesYet: "还没有消息。",
      noModels: "没有模型",
      noModelsConfigured: "尚未配置模型",
      noPolicyFields: "没有策略字段",
      noProviderCredentialConfigured: "未配置提供商凭据。",
      noQueryConditions: "没有查询条件。",
      noResponseYet: "尚无响应",
      noRoutesArePresentInTheActiveGatewayDump: "活动网关转储中不存在路由。",
      noRuntimeListeners: "没有运行时监听器",
      noRuntimeRoutes: "没有运行时路由",
      noRuntimeTrafficConfiguration: "没有运行时流量配置",
      noSchemaPropertiesAreAvailableForThisPolicyObject:
        "此策略对象没有可用的架构属性。",
      noSharedProvidersConfigured: "尚未配置共享提供商",
      noToolsReturned: "未返回工具",
      noTopLevelPolicies: "没有顶层策略",
      noTopLevelPoliciesArePresentInTheActiveGatewayDump:
        "活动网关转储中不存在顶级策略。",
      noTrafficGatewaysConfigured: "尚未配置流量网关",
      noTrafficRoutesConfigured: "尚未配置流量路由",
      noValuesConfigured: "未配置任何值。",
      noValuesFound: "未找到值。",
      noVirtualApiKeys: "没有虚拟 API 密钥",
      none: "无",
      none_deku7v: "无",
      noneAdminInterfaceOnly: "无（仅限管理界面）",
      notEnabled: "未启用",
      notInitialized: "未初始化",
      numberOfTokensAddedToTheLocalBucketEachFillInterval:
        "每个填充间隔添加到本地存储桶的令牌数量。",
      oauthClientIdAdvertisedToMcpClientsWhenNeeded:
        "需要时向 MCP 客户端通告的 OAuth 客户端 ID。",
      oauth2ClientIdentifierUsedForAuthorizationAndTokenExchange:
        "用于授权和令牌交换的 OAuth2 客户端标识符。",
      oauth2ClientSecret: "OAuth2 客户端密钥",
      oauth2ClientSecretUsedForTokenExchange:
        "用于令牌交换的 OAuth2 客户端密钥。",
      of3Enabled: "已启用 3 个",
      off: "关闭",
      onboardProviderBackedModelsAndConfigureModelSpecificBehavior:
        "载入提供商支持的模型并配置特定于模型的行为。",
      onlyQueryForAIpv4Records: "仅查询 A (IPv4) 记录。",
      onlyQueryForAaaaIpv6Records: "仅查询 AAAA (IPv6) 记录。",
      onlyTheFinalConditionalTargetCanOmitACondition:
        "只有最终的条件目标可以省略条件。",
      open: "打开",
      openAListenerSocketOnTheBindSAddressTheNormalBehavior:
        "在绑定地址上打开监听器套接字（正常行为）。",
      openClaudeDesktopAndEnableDeveloperMode:
        "打开 Claude Desktop 并启用开发者模式：",
      openInPlayground: "在演练场开放",
      openAiEmbeddings: "OpenAI /嵌入",
      openAiRealtimeWebsockets: "OpenAI /实时（网络套接字）",
      openAiResponses: "OpenAI /回应",
      openAiV1ChatCompletions: "OpenAI /v1/chat/completions",
      openAiV1Models: "OpenAI /v1/模型",
      openAiJavaScriptSdk: "OpenAI JavaScript SDK",
      openAiModeration: "OpenAI 内容审核",
      openAiPythonSdk: "OpenAI Python SDK",
      openingValue: "开盘 {{value}}",
      operation: "操作",
      operations: "操作",
      optional: "可选",
      optional_1yfbac9: "可选",
      optionalAwsStsRoleToAssumeBeforeSigningRequests:
        "在签署请求之前承担的可选 AWS STS 角色。",
      optionalBearerToken: "可选的不记名令牌",
      optionalCelExpressionsForPopulatingUserAndGroupAttributesInDatabaseLogsIfNotSetA_1qxb9rt:
        "用于填充数据库日志中的用户和组属性的可选 CEL 表达式。如果未设置，将使用默认值。",
      optionalCelFilterWithKeepSemanticsWhenSetOnlyRequestsForWhichTheExpressionEvalua_1o212j0:
        "具有 KEEP 语义的可选 CEL 过滤器。设置后，仅请求表达式\n计算结果为 `true` 并导出其跟踪跨度；所有其他跨度都被丢弃。当\n未设置，不应用过滤（导出所有采样范围）。采样后合成\n（仅评估采样跨度）。这匹配 `accessLog.filter` （保持语义）：\n`true` 保留。缺失/错误字段的计算结果为 `false`，因此在计算错误时，跨度为\n下降（失败关闭）。",
      optionalCipherSuiteAllowlistOrderIsPreserved:
        "可选的密码套件白名单（保留顺序）。",
      optionalDiscoveryDocumentOverrideIfOmittedDiscoveryUsesIssuerWellKnownOpenidConfiguration:
        "可选的发现文档覆盖。如果省略，发现将使用\n`${issuer}/.well-known/openid-configuration`。",
      optionalMetadataAttachedToRequestsAuthenticatedWithThisKey:
        "附加到使用此密钥进行身份验证的请求的可选元数据。",
      optionalOauthClientId: "可选的 OAuth 客户端 ID",
      optionalPathOverrideForThisSpecificUpstreamFormat:
        "此特定上游格式的可选路径覆盖。",
      optionalPerPolicyOverrideForClientSamplingIfSetOverridesGlobalConfigForRequestsT_9my5ce:
        "用于客户端采样的可选每策略覆盖。如果设置，则覆盖全局配置\n使用此前端策略的请求。",
      optionalPerPolicyOverrideForRandomSamplingIfSetOverridesGlobalConfigForRequestsT_121cxle:
        "用于随机采样的可选每策略覆盖。如果设置，则覆盖全局配置\n使用此前端策略的请求。",
      optionalToken: "可选令牌",
      optional06DefaultIs2: "可选。 0-6；默认值为 2。",
      optionalDefaultsToHttpLocalhost11434V1:
        "可选。默认为 http://localhost:11434/v1。",
      optionalDefaultsToOmniModerationLatest:
        "可选。默认为omni-moderation-latest。",
      optionalDefaultsToUsCentral1: "可选。默认为 us-central1。",
      optionalIfUnsetVertexUsesGlobal: "可选。如果未设置，Vertex 将使用全局。",
      optionalLeaveUnsetToUseTheGatewayDefault:
        "可选。保留未设置以使用默认网关。",
      optionsForSendingTheRequestBodyToTheAuthorizationService:
        "用于将请求正文发送到授权服务的选项。",
      or: "或",
      orderedListOfPolicyProcessorsAppliedToMatchedMethodsTheFirstToRejectARequestShor_wabfd4:
        "应用于匹配方法的策略处理器的有序列表；第一个\n拒绝请求会使链短路。处理器可以运行在\n请求方或响应方，或两者；请参阅 `Processor.methods`。",
      other: "其他",
      otlpHttpPathUsedToExportLogs: "用于导出日志的 OTLP HTTP 路径。",
      otlpHttpPathUsedToExportTraces: "用于导出跟踪的 OTLP HTTP 路径。",
      otlpLogExportSettings: "OTLP 日志导出设置。",
      otlpPathDefaultIsV1Traces: "OTLP 路径。默认为 /v1/traces",
      otlpProtocolUsedToExportLogs: "用于导出日志的OTLP协议。",
      otlpProtocolUsedToExportTracesDefaultsToHttp:
        "用于导出跟踪的 OTLP 协议。默认为 HTTP。",
      otlpSpecificAccessLogFieldsIfUnsetTheParentAccessLogFieldsAreUsed:
        "OTLP 特定的访问日志字段。如果未设置，则使用父访问日志字段。",
      outgoingModel: "出站模型",
      output: "输出",
      overrideOpenAiBaseUrl: "覆盖 OpenAI 基本 URL",
      overrideRequestValues: "覆盖请求值",
      overrideTheDefaultBasePathPrefixForThisProvider:
        "覆盖此提供商的默认基本路径前缀。",
      overrideTheUpstreamHostForThisProvider: "覆盖此提供商的上游主机。",
      overrideTheUpstreamPathForThisProvider: "覆盖此提供商的上游路径。",
      overrideWhereThisPolicyReadsTheJwtFrom: "覆盖此策略读取 JWT 的位置。",
      overridesAllowsSettingValuesForTheRequestOverridingAnyExistingValues:
        "overrides 允许为请求设置值，覆盖任何现有值",
      overridesYaml: "覆盖 YAML",
      packAsBytes: "打包为字节",
      paramsCustomizesParametersForOutgoingRequestsThatUseThisProvider:
        "params 为使用此提供商的传出请求自定义参数。",
      paramsCustomizesParametersForTheOutgoingRequest:
        "params 自定义传出请求的参数",
      passThroughTheRequestWhileExtractingLlmTelemetryAndRateLimitInputsWhenPossible:
        "传递请求，同时在可能的情况下提取 LLM 遥测和速率限制输入。",
      passThroughTheRequestWithoutInterpretingItAsLlmTraffic:
        "传递请求而不将其解释为 LLM 流量。",
      passthroughControlsHowRequestsAreHandledByDefaultRequestsWillBeParsedAndTranslat_1kocxkq:
        "直通控制请求的处理方式。\n默认情况下，请求将根据需要进行解析和翻译。\n通过直通，它们将不被修改并可选择进行检查（使用 `detect`）。\n在此模式下，请求必须以提供商的本机格式发送。",
      pasteAJwksDocumentDirectlyIntoThePolicy: "将 JWKS 文档直接粘贴到策略中。",
      path: "路径",
      pathExpression: "路径表达式",
      pathMatch: "路径匹配",
      pathRewriteToApplyBeforeForwardingTheRequest:
        "在转发请求之前应用路径重写。",
      pathRewriteToApplyToTheRedirectUrl: "路径重写以应用于重定向 URL。",
      pemEncodedPrivateSigningKeyRsaOrEcMatchingAlg:
        "PEM 编码的私有签名密钥（RSA 或 EC，匹配 `alg`）。",
      permissive: "宽松",
      permitBrowserCredentialsOnCorsRequests: "允许 CORS 请求上的浏览器凭据",
      permitMatchingRequests: "允许匹配请求。",
      phone: "电话",
      phoneNumberPattern: "电话号码模式。",
      plan: "计划",
      platformTeam: "平台团队",
      playground: "演练场",
      playgroundRequestFailed: "演练场请求失败",
      pointThePythonSdkAtTheGatewayListener: "将 Python SDK 指向网关监听器。",
      policies: "策略",
      policies_raqot3: "策略",
      policiesDefinesAdditionalPoliciesThatCanBeAttachedToVariousOtherConfigurationsTh_1vsrjcq:
        "策略定义了可以附加到各种其他配置的附加策略。\n这是一项高级功能；用户通常应使用路由/网关下的内联 `policies` 字段。",
      policiesDefinesPoliciesForHandlingIncomingRequestsBeforeAModelIsSelected:
        "策略定义在选择模型之前处理传入请求的策略",
      policiesDefinesRouteLevelPoliciesForTheUiAndRequiredUiApiRoutes:
        "策略定义 UI 的路由级策略和所需的 UI API 路由。",
      policyModeIsValue: "策略模式为 {{value}}",
      priority_one: "{{count}} 个优先级",
      priority_other: "{{count}} 个优先级",
      priorityTargetSeparator: "{{value}}，{{value}}",
      policyNameUsedWhenAttachingThisPolicyToATarget:
        "将此策略附加到目标时使用的策略名称。",
      policySettingsToApplyToTheSelectedTarget: "应用到选定目标的策略设置。",
      policyState: "策略状态",
      policyYaml: "策略 YAML",
      port: "端口",
      portValue: "端口 {{value}}",
      portDefinesThePortToServeTheLlmRoutesUnderDeprecatedUseGatewaysInstead:
        "port 定义为 LLM 路由提供服务的端口。已弃用；请改用 `gateways`。",
      portIsThePortToListenOnForThisGateway: "port 是该网关侦听的端口。",
      portMustBeBetween1And65535: "端口必须介于 1 和 65535 之间。",
      portToBindOnOmitItForAnInternalWildcardBindWhichServesAnyDestinationPortViaInPro_1nj7ohf:
        "要绑定的端口。对于内部通配符绑定省略它（它服务于任何目标端口\n通过进程内路由）。除非 `mode` 是 `internal`，否则需要数字端口。",
      prefixMode: "前缀模式",
      prefixOnlyWhenNeededToAvoidToolNameConflicts:
        "仅在需要时使用前缀以避免工具名称冲突。",
      prefixToRemoveFromTheHeaderValueBeforeValidationSuchAsBearerOrBasic:
        "在验证之前从请求头值中删除的前缀，例如 `Bearer ` 或 `Basic `。",
      prefixForwardingTheRemainingModelAsIs: "前缀，按原样转发剩余模型。",
      preparingRequest: "正在准备请求",
      preserveMcpSessionsSoTargetsCanKeepPerSessionContext:
        "保留 MCP 会话，以便目标可以保留每个会话的上下文。",
      preserveOriginalHttp1RequestHeaderCasingWhenEncodingResponsesOnTheSameConnection:
        "在同一连接上对响应进行编码时，保留原始 HTTP/1 请求头大小写。",
      primary: "主要",
      primaryDatabaseUsedByLocalRuntimeFeatures:
        "本地运行时功能使用的主数据库。",
      priorityGroupsTargetsForFailoverLowerValuesArePreferred:
        "故障转移的优先组目标。优选较低的值。",
      privateKeyFileForTheClientCertificate: "客户端证书的私钥文件。",
      processingBehavior: "加工行为",
      processor: "处理器",
      processorsRunInOrderTheFirstRejectionStopsTheRequest:
        "处理器按顺序运行；第一次拒绝会停止请求。",
      projectId: "项目编号",
      projectLinks: "项目链接",
      promptAndResponseGuardrailsToApplyToLlmTraffic:
        "适用于 LLM 流量的提示和响应防护规则。",
      promptCaching: "提示词缓存",
      promptCachingSettingsForProvidersThatSupportCacheMarkers:
        "提示支持缓存标记的提供商的缓存设置。",
      promptLoggingIsOff: "提示记录已关闭",
      promptCachingConfiguresCachePointInsertionForSupportedLlmProviders:
        "PromptCaching 为支持的 LLM 提供商配置缓存点插入。",
      protectedResourceMetadata: "受保护的资源元数据",
      protectedResourceMetadataReturnedToMcpClients:
        "返回到 MCP 客户端的受保护资源元数据。",
      protocol: "协议",
      protocolControlsWhetherThisGatewayAcceptsHttpHttpsRoutesOrTcpTlsRoutesWhenOmitte_122yt2l:
        "协议控制此网关是否接受 HTTP/HTTPS 路由或 TCP/TLS 路由。当省略时，网关\n设置 tls 时默认为 HTTP 或 HTTPS。",
      protocolControlsWhetherThisListenerAcceptsHttpHttpsRoutesOrTcpTlsRoutesWhenOmitt_198kbon:
        "协议控制此监听器是否接受 HTTP/HTTPS 路由或 TCP/TLS 路由。当省略时，监听器\n设置 tls 时默认为 HTTP 或 HTTPS。",
      protocolUsedToCallTheAuthorizationServiceUseGRpcUnlessTheServiceOnlySupportsHttp:
        "用于调用授权服务的协议。除非服务仅支持 HTTP，否则请使用 gRPC。",
      provider: "提供商",
      providerApiKey: "提供商 API 密钥",
      providerIdentityForCostCatalogLookupAndTelemetryBuiltInNamedProvidersCohereMistr_1c2sljq:
        "用于成本目录查找和遥测的提供商身份。内置命名提供商\n（cohere，mistral，...）设置此项，以便它们的成本在正确的目录键下解决；\n裸露的自定义提供商可以将其设置为匹配目录条目。又回到了“习惯”。",
      providerMetadata: "提供商元数据",
      providerName: "提供商名称",
      providerOfTheLlmWeAreConnectingTo: "我们连接到的 LLM 提供商。",
      providerOfTheLlmWeAreConnectingToo: "我们也连接到的 LLM 提供商",
      providerReturned: "提供商返回",
      provider_1k5qy2a: "提供商：",
      providers: "提供商",
      providersDefinesReusableLlmProviderDefaultsThatModelsMayReference:
        "提供商定义了模型可以引用的可重用的LLM提供商默认值。",
      provisionIncomingCredentialsAndMetadataForCallers:
        "为呼叫者提供传入凭据和元数据。",
      proxyBackendUsedToTunnelTheConnection: "代理后端用于隧道连接。",
      proxyProtocolVersionsAcceptedFromDownstreamClients:
        "从下游客户端接受的 PROXY 协议版本。",
      publicModelsCanBeRequestedDirectlyByClientsAndAreIncludedInTheModelList:
        "公共模型可以由客户直接请求并包含在模型列表中。",
      publicUiGateway: "公共 UI 网关",
      query: "查询",
      queryForBothAAndAaaaRecordsInParallelAndUseAllResults:
        "并行查询 A 和 AAAA 记录并使用所有结果。",
      queryForBothAAndAaaaButPreferIpv4AddressesWhenBothAreAvailable:
        "查询 A 和 AAAA，但当两者都是时更喜欢 IPv4 地址\n可用。",
      queryName: "查询参数名称",
      queryParameterNameContainingTheCredential: "包含凭证的查询参数名称。",
      queryValue: "查询参数值",
      quickRanges: "快速范围",
      rateLimitDomainSentToTheRemoteRateLimitService:
        "发送到远程速率限制服务的速率限制域。",
      rawApiKey: "原始 API 密钥",
      rawConfiguration: "原始配置",
      rawConfigurationDiff: "原始配置差异",
      rawGuardYaml: "原始防护规则 YAML",
      rawJson: "原始 JSON",
      rawLogJson: "原始日志 JSON",
      rawValue: "原始值",
      readSigningKeysFromAFileOnTheGatewayHost:
        "从网关主机上的文件读取签名密钥。",
      readTheCredentialFromACelExpressionEvaluatedAgainstTheIncomingRequestCelExpressi_nxzl9m:
        "从针对传入请求评估的 CEL 表达式中读取凭据。\n返回凭据字符串的 CEL 表达式。此位置可以提取凭据，但无法插入它们。",
      readTheCredentialFromARequestCookie: "从请求 cookie 中读取凭据。",
      readTheCredentialFromAUrlQueryParameter: "从 URL 查询参数读取凭据。",
      readTheCredentialFromAnHttpHeader: "从 HTTP 请求头读取凭据。",
      readOnlyListenerInventoryFromTheActiveGatewayDump:
        "来自活动网关转储的只读监听器清单。",
      readOnlyRouteInventoryFromTheActiveGatewayDump:
        "来自活动网关转储的只读路由清单。",
      readOnlyTopLevelPoliciesFromTheActiveGatewayDump:
        "来自活动网关转储的只读顶级策略。",
      readinessProbeServerAddressInTheFormatIpPortLocalhostPortUnixPathToSocketOrOff:
        "就绪探针服务器地址，格式为“ip:port”、“localhost:port”、“unix:/path/to/socket”或“off”",
      readonlyMode: "只读模式",
      readonlyPoliciesUnavailable: "只读策略不可用",
      ready: "准备好了",
      realmShownInTheWwwAuthenticateResponseHeaderWhenCredentialsAreMissingOrInvalid:
        "当凭据丢失或无效时，`WWW-Authenticate` 响应请求头中显示的领域。",
      reasoning: "推理",
      recentCalls: "最近调用",
      redirectExpression: "重定向表达式",
      redirectUri: "重定向 URI",
      reference: "参考",
      refresh: "刷新",
      refreshBaseCosts: "刷新基础成本",
      refreshTheBaseCatalogToAddPricingDataFromModelsDev:
        "刷新基本目录以添加来自 models.dev 的定价数据。",
      refreshing: "Refreshing...",
      regex: "正则表达式",
      regexOrBuiltInPatternsToEvaluate: "要评估的正则表达式或内置模式。",
      regularExpressionPatternToEvaluate: "要评估的正则表达式模式。",
      rejectHttpConnectRequests: "拒绝 HTTP CONNECT 请求。",
      rejectMatchingRequests: "拒绝匹配的请求。",
      rejectRequest: "拒绝请求",
      rejectRequestsThatDoNotCarryAValidToken: "拒绝不携带有效令牌的请求。",
      rejectTheRequestOrResponseWhenContentMatches:
        "当内容匹配时拒绝请求或响应。",
      rejectTheRequestWhenADetectorMatches: "当检测器匹配时拒绝请求。",
      rejectTheRequestWhenARegexMatches: "当正则表达式匹配时拒绝请求。",
      rejectTheRequestWhenTheExternalProcessingServiceFails:
        "当外部处理服务失败时拒绝请求。",
      rejectTheRequestWhenTheWebhookGuardrailIsUnavailableDefault:
        "当 webhook 防护规则不可用时拒绝请求（默认）。",
      rejectWhenTheProcessorIsUnavailable: "当处理器不可用时拒绝。",
      rejectWhenTheWebhookIsUnavailableOrErrors:
        "当 Webhook 不可用或出现错误时拒绝。",
      rejectionBody: "拒绝体",
      rejectionStatus: "拒绝状态",
      reloadVsCodeAndTestCopilotSuggestionsOrChat:
        "重新加载 VS Code 并测试 Copilot 建议或聊天。",
      remoteRateLimit: "远程速率限制",
      remoteRateLimitChecksForIncomingRequests:
        "对传入请求的远程速率限制检查。",
      remoteRateLimitServiceAndDomainUsedWhenBuildingDescriptorChecks:
        "构建描述符检查时使用的远程速率限制服务和域。",
      remoteUrl: "远程 URL",
      remove: "移除",
      removeValue: "删除 {{value}}",
      removeAllLlmGuardrails: "移除所有 LLM 防护规则？",
      removeAllRequestAndResponseGuardrailsLlmTrafficWillNoLongerBeCheckedByTheseRules:
        "删除所有请求和响应防护规则？ LLM 流量将不再受这些规则检查。",
      removeBackend: "移除后端",
      removeConditionalTarget: "删除条件目标",
      removeCustomCost: "删除定制成本",
      removeDescriptor: "移除描述符",
      removeDescriptorEntry: "移除描述符条目",
      removeFailoverGroupValue: "删除故障转移组 {{value}}",
      removeGuardrail: "移除防护规则",
      removeGuardrail_1r9af69: "移除防护规则？",
      removeGuardrails: "移除防护规则",
      removeHeaderCondition: "移除请求头条件",
      removeHeaders: "删除标题",
      removeMatchValue: "移除匹配条件 {{value}}",
      removePattern: "移除模式",
      removeQueryCondition: "移除查询条件",
      removeTarget: "移除目标",
      removeThe: "删除",
      removeTheApiKeyPolicyEntirelyRequestsWillNotBeValidatedAgainstVirtualApiKeys:
        "完全删除 API 密钥策略。不会根据虚拟 API 密钥验证请求。",
      replaceMatchedContentAndContinue: "替换匹配的内容并继续。",
      replaceMatchingContentWithMaskedText: "用屏蔽文本替换匹配的内容。",
      replaceOnlyTheHostAndPreserveTheEffectivePort:
        "仅更换主机，保留有效端口。",
      replaceOnlyTheMatchedPathPrefix: "仅替换匹配的路径前缀。",
      replaceOnlyThePort: "仅更换端口。",
      replaceTheFullAuthorityIncludingHostAndOptionalPort:
        "替换完整权限，包括主机和可选端口。",
      replaceTheFullRequestPath: "替换完整的请求路径。",
      request: "请求",
      request_1058hua: "请求",
      requestAttributes: "请求属性",
      requestBody: "请求正文",
      requestBodyValuesComputedFromCelExpressions:
        "根据 CEL 表达式计算的请求正文值。",
      requestBodyValuesThatReplaceClientProvidedValues:
        "请求正文值替换客户端提供的值。",
      requestContextYaml: "请求上下文 YAML",
      requestDetail: "请求详情",
      requestExtraOauth2ScopesTheGatewayAlwaysIncludesOpenid:
        "请求额外的 OAuth2 范围。网关始终包含 openid。",
      requestGuards: "请求防护规则",
      requestHeaders: "请求头",
      requestHeadersToSendToTheAuthorizationServiceIfUnsetGRpcSendsAllRequestHeadersAn_136gzan:
        "发送到授权服务的请求头。\n如果未设置，gRPC 会发送所有请求头，而 HTTP 仅发送 `Authorization`。",
      requestInProgress: "请求进行中",
      requestLogIdentity: "请求日志身份",
      requestOriginsThatReceiveCorsResponseHeadersUseToMatchAnyOrigin:
        "接收 CORS 响应请求头的请求源。使用 `*` 匹配任何来源。",
      requestProgress: "请求进度",
      requestTrailers: "请求尾部",
      requestTransformations: "请求转换",
      requestHeadersModifiesHeadersInRequestsToTheLlmProvider:
        "requestHeaders 修改向 LLM 提供商发出的请求中的请求头。",
      requests: "请求数",
      requestsAreNeverRejectedThisIsUsefulForUsageOfClaimsInLaterStepsAuthorizationLog_etyjeb:
        "请求永远不会被拒绝。这对于在后续步骤（授权、日志记录等）中使用声明非常有用。\n警告：这允许没有 JWT 令牌的请求！另外，401错误不会被返回，\n这不会触发客户端启动 oauth 流程。",
      require: "要求",
      requireAProxyProtocolHeaderOnEachConnection:
        "每个连接上都需要 PROXY 协议请求头。",
      requireAValidApiKey: "需要有效的 API 密钥。",
      requireAValidJwtFromAConfiguredIssuer: "需要来自配置的签发者的有效 JWT。",
      requireAValidUsernameAndPassword: "需要有效的用户名和密码。",
      requireTheSelectedDestinationToMatchAgentgatewaySLocalServiceEndpoints:
        "要求所选目标与代理网关的本地服务端点相匹配。",
      requireThisCelExpressionToBeTrue: "要求此 CEL 表达式为真。",
      requireThisExpressionToBeTrue: "要求这个表达式为真。",
      requiredClaims: "所需声明",
      reset: "重置",
      resource: "资源",
      resourceAttributesToAddToTheTracerProviderOtelResourceThisCanBeUsedToSetThingsLi_k3nt2h:
        "要添加到跟踪器提供商 (OTel `Resource`) 的资源属性。\n这可用于动态设置 `service.name` 等内容。",
      resourceMetadataYaml: "资源元数据 YAML",
      response: "反应",
      response_nrnldq: "响应",
      responseAttributes: "响应属性",
      responseBody: "响应正文",
      responseBodyReturnedWhenContentIsRejected: "内容被拒绝时返回的响应正文。",
      responseCacheConfigurationDefaultsToAnInMemoryCacheWith8192EntriesAndA300sTtlWhe_12crnmm:
        "响应缓存配置。默认为内存缓存，包含 8192 个条目和 300 秒\n令牌端点省略 `expires_in` 时的 TTL。将 `maxEntries` 设置为 0 以禁用。",
      responseGuards: "响应防护规则",
      responseHeaders: "响应头",
      responseHeadersComputedFromCelExpressions:
        "根据 CEL 表达式计算的响应请求头。",
      responseReturnedWhenTheLlmResponseIsRejected:
        "LLM 响应被拒绝时返回的响应。",
      responseReturnedWhenTheRequestIsRejected: "请求被拒绝时返回的响应。",
      responseTrailers: "响应尾部",
      responseTransformations: "响应变换",
      responseHeadersModifiesHeadersInResponsesFromTheLlmProvider:
        "responseHeaders 修改来自 LLM 提供商的响应中的请求头。",
      restoreHealth: "恢复健康",
      restrictAcceptedMcpTokensByIssuerAndAudience:
        "限制签发者和受众接受的 MCP 令牌。",
      restrictAcceptedTokensByIssuerAudienceAndRequiredClaims:
        "限制签发者、受众和所需声明接受的令牌。",
      result: "结果",
      resultingYaml: "生成的 YAML",
      retryMatchingFailedUpstreamRequests: "重试匹配失败的上游请求。",
      returnAConfiguredResponseInsteadOfForwardingTheRequest:
        "返回配置的响应而不是转发请求。",
      returnARedirectResponseInsteadOfForwardingTheRequest:
        "返回重定向响应而不是转发请求。",
      returnARedirectResponseInsteadOfForwardingToThisBackend:
        "返回重定向响应而不是转发到此后端。",
      reviewMigration: "检查迁移",
      rewriteAllRequestsToThisAdminApiPathPreservingTheOriginalQueryString:
        "重写对此管理 API 路径的所有请求，保留原始查询字符串。",
      rewriteTheRequestPathOrAuthorityBeforeForwarding:
        "转发前重写请求路径或权限。",
      rfc7523TheSubjectTokenIsSentAsTheAssertion:
        "RFC 7523；主题令牌作为 `assertion` 发送。",
      rfc8693ActorTokenTypeUrnWhenOmittedDefaultsToAccessTokenAndIsStillSent:
        "RFC 8693 参与者令牌类型 URN；省略时默认为 access_token 并且仍然发送",
      rfc8693DelegationActorTokenTokenExchangeGrantOnly:
        "RFC 8693 委托参与者令牌。仅授予令牌交换。",
      rfc8693TokenExchangeTheSubjectTokenIsSentAsSubjectToken:
        "RFC 8693 令牌交换；主题令牌以 `subject_token` 形式发送。",
      rfc8693TokenTypeUrnWhenOmittedDefaultsToAccessToken:
        "RFC 8693 令牌类型 URN；省略时默认为 access_token",
      rootCertificateBundleUsedToVerifyTheBackendCertificate:
        "根证书捆绑用于验证后端证书。",
      routeClaudeDesktopThirdPartyInferenceThroughTheGateway:
        "通过网关路由 Claude Desktop 第三方推理。",
      routeFormats: "路由格式",
      routeGroup: "航线组",
      routeHttpConnectRequestsThroughNormalRouteMatching:
        "通过正常的路由匹配来路由 HTTP CONNECT 请求。",
      routePolicies: "路由策略",
      routeProtocolFamily: "路由协议族。",
      routeRequestsThroughAnEndpointPickerBeforeForwardingToThisBackend:
        "在转发到此后端之前通过端点选择器路由请求。",
      routeToTheInProcessAdminServiceInsteadOfANetworkUpstream:
        "路由到进程内管理服务而不是网络上游。",
      routeTypeOverridesSelectedByRequestPathSuffix:
        "路由类型覆盖由请求路径后缀选择的路由类型。",
      routeWindsurfTrafficThroughTheGatewayHttpProxySetting:
        "通过网关 HTTP 代理设置路由 Windsurf 流量。",
      routeYaml: "路由 YAML",
      routeGroupsProvidesASetOfRouteGroupsUsedForRouteDelegationThisIsAnAdvancedFeatur_12ntlx8:
        "RouteGroups 提供了一组用于路由委托的路由组。这是一项高级功能\n主要用于测试。",
      routes: "路由",
      routes_14u6307: "路由",
      routes_4p3286: "路由·",
      routesDefinesHttpRoutesAttachedToOneOrMoreNamedGateways:
        "路由定义附加到一个或多个命名网关的 HTTP 路由。",
      routing: "路由",
      routingSelectsAnExistingLlmModelBackendForEachRequest:
        "路由为每个请求选择一个现有的 LLM 模型后端。",
      routingStrategy: "路由策略",
      rule: "规则",
      run: "运行",
      runAfterTheMcpResponseIsAvailable: "MCP 响应可用后运行。",
      runBeforeForwardingTheMcpRequest: "在转发 MCP 请求之前运行。",
      runWithRequestAndResponseContext: "使用请求和响应上下文运行。",
      runtimeTraffic: "运行时流量",
      safety: "安全",
      save: "保存",
      saveFailed: "保存失败",
      savePolicy: "保存策略",
      saveUiGateway: "保存界面网关",
      schemeToUseInTheRedirectUrlSuchAsHttpOrHttps:
        "在重定向 URL 中使用的方案，例如 `http` 或 `https`。",
      scopes: "作用域",
      sdkSnippetsUseThisUrlWithV1Appended: "SDK 片段使用此 URL 并附加 /v1。",
      searchValue: "搜索 {{value}}",
      searchFor: "搜索",
      secretValueToSendToTheBackend: "发送到后端的秘密值。",
      security: "安全性",
      selectAConcreteModel: "请选择具体模型",
      selectAGateway: "选择网关。",
      selectAListener: "选择监听器。",
      selectGuardType: "选择防护规则类型",
      selectOrTypeAModel: "选择或输入模型",
      selectProvider: "选择提供商",
      selectsHowAnInternalBackendMapsProxyRequestsToTheAdminApi:
        "选择内部后端如何将代理请求映射到管理 API。",
      selectsWhichRfcTheRequestFollowsDefaultsToTokenExchangeRfc8693:
        "选择请求遵循哪个 RFC；默认为令牌交换（RFC 8693）。",
      send: "发送",
      sendABoundedBodyBufferAndAllowTruncation:
        "发送有界主体缓冲区并允许截断。",
      sendAConfiguredSecretValueToTheBackend: "将配置的秘密值发送到后端。",
      sendACopyOfMatchingRequestsToAnotherBackend:
        "将匹配请求的副本发送到另一个后端。",
      sendARealChatCompletionRequestThroughTheConfiguredGatewayForSetupDebugging:
        "通过已配置的网关发送真实的聊天补全请求，用于调试设置。",
      sendContentToAnExternalGuardrailService: "将内容发送到外部防护规则服务。",
      sendHeadersToTheExternalProcessingService: "将请求头发送到外部处理服务。",
      sendRequestAndResponseDataToAnExternalProcessingService:
        "将请求和响应数据发送到外部处理服务。",
      sendTheRequestToTheUpstreamLlmProviderAsIs:
        "将请求原样发送到上游 LLM 提供商",
      sendTheRequestToTheUpstreamLlmProviderAsIsButAttemptToExtractInformationFromItAn_a091kz:
        "按原样将请求发送到上游 LLM 提供商，但尝试从中提取信息\n并应用一部分策略（速率限制和遥测；无防护规则）。",
      sendThisPhaseToTheExternalProcessor: "将此阶段发送到外部处理器。",
      sendTrailersToTheExternalProcessingService: "将拖车发送至外部处理服务。",
      sending: "发送",
      sendingChatCompletion: "正在发送聊天补全请求",
      sendingToolResults: "正在发送工具结果",
      serverName: "服务器名称",
      serverNameToUseForTlsVerificationAndSni:
        "用于 TLS 验证和 SNI 的服务器名称。",
      servers: "服务器",
      serversToolsAndMcpPlaygroundFlows: "服务器、工具和 MCP 演练场流程。",
      service: "服务",
      serviceReferenceServiceMustBeDefinedInTheTopLevelServicesList:
        "服务参考。服务必须在顶级服务列表中定义。",
      servicesDefinesTheSetOfServicesThatTheProxyCanRouteToTheseConsistOfWorkloadsThis_9pwt7w:
        "services 定义代理可以路由到的服务集。这些由 `workloads` 组成。\n这是一项高级功能，主要用于测试；在路由上使用内联 `backends` 和\n策略通常是首选。",
      session: "会议",
      sessionTagsPassedToStsAssumeRoleForCostAttributionOnceActivatedAsCostAllocationT_1ce6dym:
        "会话标签传递给 STS AssumeRole 以进行成本归因。一旦激活为\n成本分配标签，每个标签都显示在 AWS 成本和使用情况报告中，位于\n`resourceTags/user:TagKey`。标签值可以是静态 (`value`) 或 CEL\n针对每个请求评估的表达式 (`expression`)。",
      sessionTokenOptional: "会话令牌（可选）",
      setHeaders: "设置标题",
      setRequestTimeoutLimits: "设置请求超时限制。",
      setTheProxyUrlTo: "将代理 URL 设置为",
      setUpGateways: "设置网关",
      setUpListeners: "设置监听器",
      setUpModels: "设置模型",
      setUpServers: "设置服务器",
      settings: "设置",
      settingsForExportingRequestTraces: "用于导出请求跟踪的设置。",
      settingsForHandlingIncomingHttpRequests: "用于处理传入 HTTP 请求的设置。",
      settingsForHandlingIncomingTcpConnections:
        "用于处理传入 TCP 连接的设置。",
      settingsForHandlingIncomingTlsConnections:
        "用于处理传入 TLS 连接的设置。",
      settingsForRequestAccessLogs: "请求访问日志的设置。",
      settingsForTemporarilyRemovingUnhealthyBackends:
        "用于暂时删除不健康后端的设置。",
      severityThreshold: "严重性阈值",
      severityThreshold06ForFourSeverityLevelsContentAtOrAboveThisLevelIsBlockedDefault2:
        "严重性阈值（FourSeverityLevels 为 0-6）。达到或高于此级别的内容将被阻止。默认值：2。",
      sha256HashOfAnApiKeyValueToAcceptInSha256HexFormat:
        "要接受的 API 密钥值的 SHA-256 哈希值，采用 `sha256:<hex>` 格式。",
      shaping: "塑形",
      show: "显示",
      showValueOptions: "显示 {{value}} 选项",
      showFullKey: "显示完整密钥",
      signBackendRequestsWithAwsCredentials: "使用 AWS 凭证签署后端请求。",
      signingKeys: "签名密钥",
      simpleChatCompletionMessageIsASimplifiedChatMessage:
        "SimpleChatCompletionMessage 是一个简化的聊天消息",
      skip: "跳过",
      skipCertificateTrustVerificationForTheBackendConnection:
        "跳过后端连接的证书信任验证。",
      skipFailedTargetsUpstreamsAndContinueServingFromHealthyOnesIfAllTargetsFailStillReturnAnError:
        "跳过失败的目标/上游并继续从健康的目标/上游提供服务。\n如果所有目标都失败，仍返回错误。",
      skipHostnameVerificationForTheBackendCertificate:
        "跳过后端证书的主机名验证。",
      skipSetup: "跳过设置",
      someExamples: "一些例子：",
      someListenersMixHttpAndTcpRoutes: "一些监听器混合 HTTP 和 TCP 路由",
      source: "来源",
      sourcesAreMergedInOrderLaterSourcesOverrideEarlierEntries:
        "来源按顺序合并。后来的来源覆盖了早期的条目。",
      spanAttributesToAddKeyedByAttributeName:
        "要添加的跨度属性，按属性名称键入。",
      specificModel: "特定模型",
      splitMixedListenersBeforeUsingTheRouteForm:
        "在使用路由表单之前拆分混合监听器。",
      ssn: "社会保障号码",
      standardRequestLogAttributesPopulatedForDatabaseBackedLocalRuntimeFeatures:
        "为数据库支持的本地运行时功能填充标准请求日志属性。",
      state: "状态",
      stateMode: "状态模式",
      stateful: "有状态的",
      stateless: "无国籍",
      static: "静态",
      staticContextValuesToSendToTheAuthorizationServiceMapsToTheContextExtensionsFieldInTheRequest:
        "要发送到授权服务的静态上下文值。\n映射到请求中的 `context_extensions` 字段。",
      staticResponseBodyEncodedAsBytes: "静态响应主体，编码为字节。",
      staticTagValue: "静态标记值。",
      statsMetricsServerAddressInTheFormatIpPortLocalhostPortUnixPathToSocketOrOff:
        "统计/指标服务器地址，格式为“ip:port”、“localhost:port”、“unix:/path/to/socket”或“off”",
      stream: "串流",
      streamTheBodyBidirectionallyWithTheExternalProcessingService:
        "通过外部处理服务双向传输正文。",
      streamTheFullBodyThroughTheExternalProcessor:
        "通过外部处理器传输全身数据。",
      streamFalse: "流：假",
      streaming: "流媒体",
      strict: "严格",
      structuredContent: "结构化内容",
      systemPrompt: "系统提示词",
      tagKey: "标签键。",
      target: "目标",
      target_one: "{{count}} 个目标",
      target_other: "{{count}} 个目标",
      targetModel: "目标型号",
      targetType: "目标类型",
      targetTheVisualEditorCurrentlySupportsHostTargetsOnly:
        "目标。可视化编辑器当前仅支持主机目标。",
      targets: "目标",
      targetsAreEvaluatedInOrderTheFirstMatchingConditionSelectsTheModel:
        "目标按顺序进行评估。第一个匹配条件选择模型。",
      targetsAreExistingModelNamesOrNamesMatchedByWildcardModelEntries:
        "目标是现有模型名称或与通配符模型条目匹配的名称。",
      targetsAreGroupedByPriorityLowerPriorityValuesAreTriedFirst:
        "目标按优先级分组。首先尝试较低优先级值。",
      tcpKeepaliveSettingsForBackendConnections:
        "后端连接的 TCP keepalive 设置。",
      tcpKeepaliveSettingsForDownstreamConnections:
        "下游连接的 TCP keepalive 设置。",
      tcpProtocolSettingsForThisBackend: "该后端的 TCP 协议设置。",
      tcpRoutesDefinesTcpRoutesAttachedToOneOrMoreNamedTcpTlsGateways:
        "tcpRoutes 定义附加到一个或多个指定 TCP/TLS 网关的 TCP 路由。",
      temperature02: "温度：0.2",
      templateId: "模板ID",
      theAes256GcmSessionProtectionKeyToBeUsedForSessionTokensIfNotSetSessionsWillNotB_kosx3y:
        "用于会话令牌的 AES-256-GCM 会话保护密钥。\n如果未设置，会话将不会被加密。\n例如，通过 `openssl rand -hex 32` 生成。",
      theAzureContentSafetyEndpointHostnameEGResourceNameCognitiveservicesAzureCom:
        "Azure 内容安全终结点主机名（例如“<资源名称>.cognitiveservices.azure.com”）",
      theAzureResourceNameUsedToConstructTheEndpointHost:
        "用于构造终结点主机的 Azure 资源名称。",
      theFoundryProjectNameRequiredWhenResourceTypeIsFoundryUsedToConstructPathsApiPro_acq7x8:
        "Foundry项目名称，当`resourceType`为`foundry`时必填。\n用于构造路径：`/api/projects/{projectName}/openai/v1/...`。\n这与用于主机的 `resourceName` 不同。",
      theGcpProjectId: "GCP 项目 ID",
      theGcpRegionDefaultUsCentral1: "GCP 区域（默认：us-central1）",
      theHttpEndpointClassSuchAsV1ChatCompletionsOrV1MessagesThisIsUsedBothForTheClien_pbt4i9:
        "HTTP 端点类别，例如 `/v1/chat/completions` 或 `/v1/messages`。\n\n它同时用于匹配到的客户端路由和最终发送到的上游路由。对于聊天请求，两者可能不同：客户端发起的 Anthropic `/v1/messages` 请求对应 `RouteType::Messages` 和 `InputFormat::Messages`，但转换后可能以 `RouteType::Completions` 发送到上游。\n\n`RouteType` 描述 HTTP 端点，`InputFormat` 描述解析后的客户端负载及返回给客户端的响应形状。该类型还包括 Detect 和 Passthrough 等模式。",
      theMaximumDurationToKeepAnIdleConnectionAlive:
        "保持空闲连接活动的最大持续时间。",
      theMaximumNumberOfConnectionsAllowedInThePoolPerHostnameIfSetThisWillLimitTheTot_2rbbla:
        "每个主机名池中允许的最大连接数。如果设置，这将限制\n与任何给定主机保持活动的连接总数。\n注意：仍然会创建多余的连接，只是它们不会保持空闲状态。\n如果未设置则没有限制",
      theModelToSendToTheProviderIfUnsetTheSameModelWillBeUsedFromTheRequest:
        "要发送给提供商的模型。\n如果未设置，则将使用请求中的相同模型。",
      theResourceAuthorizationServerWhichExchangesTheIdJagForAnAccessToken:
        "资源授权服务器，用 ID-JAG 交换访问令牌。",
      theTemplateIdForTheModelArmorConfiguration: "模型装甲配置的模板 ID",
      theTypeOfAzureEndpointToConnectTo: "要连接的 Azure 终结点的类型。",
      theTypeOfAzureEndpointDeterminesTheHostSuffix:
        "Azure 终结点的类型。确定主机后缀。",
      theUniqueIdentifierOfTheGuardrail: "防护规则唯一标识",
      theUserSIdPAuthorizationServerUsedForTheRfc8693TokenExchange:
        "用户的 IdP 授权服务器，用于 RFC 8693 令牌交换。",
      theVersionOfTheGuardrail: "防护规则的版本",
      thisCannotBeUndone_1x7m3fy: "此操作无法撤销。",
      thisConfigurationUsesLegacy: "此配置使用旧版",
      thisGuardUsesAShapeTheVisualEditorDoesNotSupportYetItWillBePreservedAsRawYaml:
        "该防护使用可视化编辑器尚不支持的形状。它将保留为原始 YAML。",
      thisPolicyUsesA: "该策略使用",
      thisPolicyUsesConditionalRateLimitEntriesTheVisualEditorCurrentlySupportsSimpleRateLimitsOnly:
        "此策略使用有条件的速率限制条目。可视化编辑器当前仅支持简单的速率限制。",
      thisToolDoesNotDeclareArguments: "该工具不声明参数。",
      timeToWaitForAnHttp2KeepalivePingResponse:
        "等待 HTTP/2 keepalive ping 响应的时间。",
      timingAndUsage: "时间和使用",
      tlsConfiguresTlsWhenConnectingToTheLlmProvider:
        "tls 在连接到 LLM 提供商时配置 TLS。",
      tlsDefinesTheTlsSettingsToServeTheLlmRoutesUnderWhenUsingPortDeprecatedUseGatewaysInstead:
        "tls 定义了使用 `port` 时为 LLM 路由提供服务的 TLS 设置。已弃用；请改用 `gateways`。",
      tlsEnablesHttpsForThisGatewayMaybeNotBeSetWithListeners:
        "tls 为此网关启用 HTTPS。可能没有用 `listeners` 设置",
      tlsEnablesHttpsForThisListener: "tls 为此监听器启用 HTTPS。",
      tlsSettingsUsedWhenConnectingToTheBackend:
        "连接到后端时使用的 TLS 设置。",
      tlsSettingsUsedWhenConnectingToThisBackend:
        "连接到此后端时使用的 TLS 设置。",
      to: "至",
      toCaptureRequestAndResponsePayloads: "捕获请求和响应负载。",
      toTheLlmCorsPolicySoThisPlaygroundCanCallTheGatewayFromTheBrowser:
        "到 LLM CORS 策略，以便这个演练场可以从浏览器调用网关。",
      toTheMcpCorsPolicyAndExposeMcpSessionIdSoThisPlaygroundCanKeepABrowserSession:
        "MCP CORS 策略并公开 Mcp-Session-Id，以便该演练场可以保持浏览器会话。",
      toTheMcpCorsPolicySoThePlaygroundCanListAndCallMcpToolsFromTheBrowser:
        "MCP CORS 策略，以便 Playground 可以从浏览器列出并调用 MCP 工具。",
      toggleTheme: "切换主题",
      tokenEndpoint: "令牌端点",
      tokenEndpointAuth: "令牌端点身份验证",
      tokenEndpointClientAuthenticationMethodForExplicitProviderConfigurationDiscovery_s7q91h:
        "用于显式提供商配置的令牌端点客户端身份验证方法。\n\n发现模式从提供商元数据中得出这一点。显式模式默认为\n省略时为 `clientSecretBasic`。",
      tokenEndpointPathOnTheBackendDefaultsTo:
        "后端的令牌端点路径；默认为“/”。",
      tokenEndpointUsedToExchangeTheAuthorizationCode:
        "用于交换授权代码的令牌端点。",
      tokenValidation: "令牌验证",
      tokens: "令牌",
      tokensPerFill: "每次填充的令牌数",
      tool: "工具",
      toolCall: "工具调用",
      toolOutput: "工具输出",
      toolPlayground: "工具演练场",
      toolResult: "工具结果",
      tools: "工具",
      toolsDiscovered: "发现的工具",
      toolsCallPromptsOr: "tools/call、prompts/* 或 *",
      topLevelRuntimePoliciesAreOnlyAvailableWhenTheGatewayIsRunningFromXdsConfig:
        "仅当网关从 XDS 配置运行时，顶级运行时策略才可用。",
      total: "总计",
      totalNumberOfAttemptsIncludingTheOriginalRequest:
        "尝试总数，包括原始请求。",
      traffic: "流量",
      trafficGateways: "流量网关",
      trafficListeners: "流量监听器",
      trafficOverTime: "流量趋势",
      trafficRoutes: "流量路由",
      trafficShaping: "流量整形",
      trafficThatMatchesThisRouteIsForwardedToTheseTargets:
        "与此路由匹配的流量将转发到这些目标。",
      transformTheRequestBeforeItIsForwarded: "在转发请求之前对其进行转换。",
      transformTheResponseBeforeItIsReturned: "在返回响应之前对其进行转换。",
      transformation: "转型",
      transformationAllowsSettingValuesFromCelExpressionsForTheRequestOverridingAnyExistingValues:
        "转换允许从 CEL 表达式为请求设置值，覆盖任何现有值。",
      transformations: "转换",
      transport: "传输",
      treatHttpConnectRequestsAsTunnels: "将 HTTP CONNECT 请求视为隧道。",
      troubleshooting: "故障排除",
      trustTheSelectedDestinationDirectlyWithoutLocalEndpointValidation:
        "直接信任选定的目标，无需本地端点验证。",
      trustedIssuersAndTheirSigningKeys: "受信任的签发者及其签名密钥。",
      ttlUsedWhenTheTokenEndpointOmitsExpiresInDefaultsTo300s:
        "当令牌端点省略 `expires_in` 时使用的 TTL。默认为 300 秒。",
      tunnelSettingsUsedWhenConnectingToTheBackend:
        "连接到后端时使用的隧道设置。",
      tunnelSettingsUsedWhenConnectingToThisBackend:
        "连接到此后端时使用的隧道设置。",
      type: "类型",
      uSSocialSecurityNumberPattern: "美国社会安全号码模式。",
      uiAccessPolicies: "UI 访问策略",
      uiDefinesSettingsForHowTheUiAndUiBackendIsExposedByDefaultTheUiIsExposedOnlyOnTh_ajchhz:
        "ui 定义 UI 和 UI 后端如何公开的设置。默认情况下，仅暴露 UI\n在管理界面上（通常是 localhost:15000）。此设置允许附加到 `gateways`\n对外提供服务，以及将策略附加到 UI 流量。\n强烈建议在外部公开 UI 时使用身份验证（通常是 OIDC）。",
      uiIsExposedWithoutAuthentication: "UI未经身份验证就暴露",
      uiSettings: "界面设置",
      unauthenticatedUsersCanAccessTheUiConsiderAddingAuthenticationOrAuthorizationPol_qnhsta:
        "未经身份验证的用户可以访问 UI；考虑添加身份验证或授权策略以保护 UI。",
      unhealthyExpression: "不健康表达式",
      unset: "未设置",
      unsupportedBackendShapeInThisForm: "此表单不支持该后端结构",
      unsupportedGuardShape: "不支持的防护规则结构",
      unsupportedRateLimitShape: "不支持的限流结构",
      unsupportedRemoteRateLimitShape: "不支持的远程限流结构",
      unsupportedTargetType: "不支持的目标类型",
      unused: "未使用",
      upstreamApiShapeThisCustomProviderSaysItAccepts:
        "该自定义提供商表示接受上游 API 形状。",
      upstreamModel: "上游型号",
      url: "URL",
      useABuiltInSensitiveDataPattern: "使用内置的敏感数据模式。",
      useACustomRegularExpression: "使用自定义正则表达式。",
      useAmbientAwsCredentialsOrStaticAccessKeysForBedrockSigning:
        "使用环境 AWS 凭证或静态访问密钥进行 Bedrock 签名。",
      useApplicationDefaultCredentialsOrAServiceAccountJsonFileForVertex:
        "使用 Vertex 的应用程序默认凭据或服务帐户 JSON 文件。",
      useAwsBedrockGuardrailsToEvaluateThePrompt:
        "使用 AWS Bedrock Guardrails 评估提示。",
      useAwsBedrockGuardrailsToEvaluateTheResponse:
        "使用 AWS Bedrock Guardrails 评估响应。",
      useAwsBedrockGuardrails: "使用 AWS Bedrock Guardrails。",
      useAzureAiContentSafety: "使用 Azure AI 内容安全。",
      useAzureContentSafetyToEvaluateThePrompt:
        "使用 Azure 内容安全来评估提示。",
      useAzureContentSafetyToEvaluateTheResponse:
        "使用 Azure 内容安全来评估响应。",
      useAzureDefaultCredentialsManagedIdentityOrAnAzureApiKey:
        "使用 Azure 默认凭据、托管标识或 Azure API 密钥。",
      useCrossAppAccessIdentityAssertionIdJagToObtainABackendAccessToken:
        "使用跨应用程序访问（身份断言/ID-JAG）来获取后端访问令牌。",
      useCursorSOpenAiBaseUrlOverrideWithAGatewayModel:
        "将 Cursor 的 OpenAI 基本 URL 覆盖与网关模型结合使用。",
      useCustomKey: "使用自定义密钥",
      useDefault: "使用默认值",
      useDefaultLocation: "使用默认位置",
      useEnvoyExternalAuthorizationOverGRpc: "通过 gRPC 使用 Envoy 外部授权。",
      useExplicitAwsCredentials: "使用显式 AWS 凭证",
      useExplicitAzureCredentials: "使用显式 Azure 凭据",
      useGoogleModelArmorForSafetyChecks:
        "使用 Google Model Armor 进行安全检查。",
      useGoogleModelArmorToEvaluateThePrompt:
        "使用 Google Model Armor 来评估提示。",
      useGoogleModelArmorToEvaluateTheResponse:
        "使用 Google Model Armor 来评估响应。",
      useImplicitAwsAuthenticationEnvironmentVariablesIamRolesEtc:
        "使用隐式 AWS 身份验证（环境变量、IAM 角色等）",
      useImplicitAzureAuthNoteThatThisIsForDeveloperUseCasesOnly:
        "使用隐式 Azure 身份验证。请注意，这仅适用于开发人员用例！",
      useOauthTokenExchangeFlowsToObtainABackendAccessToken:
        "使用 OAuth 令牌交换流程获取后端访问令牌。",
      useOpenAiModerationChecksForIncomingPrompts:
        "使用 OpenAI 审核检查传入的提示。",
      useOpenAiModerationToEvaluateThePrompt: "使用 OpenAI 审核来评估提示。",
      useOpenAiCompatibleEnvironmentVariablesWhenRunningCodexAgainstTheGateway:
        "针对网关运行 Codex 时，使用 OpenAI 兼容的环境变量。",
      useStrictModeWhenKeysShouldBeMandatory:
        "当密钥应该是强制性的时，请使用严格模式。",
      useTheGatewayAsAnOpenAiCompatibleChatCompletionsEndpoint:
        "使用网关作为与 OpenAI 兼容的聊天完成端点。",
      useTheGatewayUrlAndKeyWithClaudeCompatibleModelRoutesWhenConfigured:
        "配置时，将网关 URL 和密钥与 Claude 兼容模型路由一起使用。",
      useTheIssuerMetadataEndpointUnlessAnOverrideIsProvided:
        "使用签发者元数据端点，除非提供覆盖。",
      useTheSelectedBackendHostWhenPossible: "尽可能使用选定的后端主机。",
      useThisWhenTheUpstreamExposesOneOrMoreLlmCompatibleHttpApisAtYourOwnEndpoint:
        "当上游在您自己的端点公开一个或多个与 LLM 兼容的 HTTP API 时，请使用此选项。",
      useTrafficGatewaysForNewHttpRoutingConfiguration:
        "使用流量网关进行新的 HTTP 路由配置。",
      usedBy: "使用者",
      user: "用户",
      userAgent: "用户代理",
      userAgents: "用户代理",
      userAttribute: "用户属性",
      userDatabaseInHtpasswdFormatCanBeInlineOrLoadedFromAFile:
        "htpasswd 格式的用户数据库。可以内联或从文件加载。",
      userMessage: "用户消息",
      user_19x0vko: "用户：",
      users: "用户",
      validateATokenWhenOneIsPresent: "当存在令牌时验证令牌。",
      validateCredentialsWhenPresentThisIsTheDefaultOptionWarningThisAllowsRequestsWit_kr9lgb:
        "验证凭据（如果存在）。\n这是默认选项。\n警告：这允许没有基本身份验证凭据的请求。",
      validateJwtsAgainstASingleTrustedTokenIssuer:
        "针对单个可信令牌签发者验证 JWT。",
      validateJwtsAgainstOneOrMoreTrustedTokenIssuers:
        "针对一个或多个可信令牌签发者验证 JWT。",
      validateTheApiKeyWhenPresentThisIsTheDefaultOptionWarningThisAllowsRequestsWithoutAnApiKey:
        "验证 API 密钥（如果存在）。\n这是默认选项。\n警告：这允许没有 API 密钥的请求。",
      validateTheJwtWhenPresentThisIsTheDefaultOptionWarningThisAllowsRequestsWithoutAJwt:
        "验证 JWT（如果存在）。\n这是默认选项。\n警告：这允许没有 JWT 的请求。",
      validationMode: "验证模式",
      validationModeForApiKeyAuthentication: "API密钥认证的验证模式。",
      validationModeForBasicAuth: "基本身份验证的验证模式。",
      valueToReturnInAccessControlMaxAgeForAllowedPreflightRequests:
        "对于允许的预检请求，在 `Access-Control-Max-Age` 中返回的值。",
      valuesToReturnInAccessControlAllowHeadersForAllowedPreflightRequests:
        "对于允许的预检请求，在 `Access-Control-Allow-Headers` 中返回的值。",
      valuesToReturnInAccessControlAllowMethodsForAllowedPreflightRequests:
        "对于允许的预检请求，在 `Access-Control-Allow-Methods` 中返回的值。",
      valuesToReturnInAccessControlExposeHeadersForAllowedCorsResponses:
        "对于允许的 CORS 响应，在 `Access-Control-Expose-Headers` 中返回的值。",
      vertexAiRegionSpecialValuesGlobalUsesTheGlobalEndpointWhileUsAndEuUseRestrictedM_xwa0mk:
        "顶点 AI 区域。特殊值：`global` 使用全局端点，而 `us` 和 `eu`\n使用受限的多区域端点。其他值被视为区域位置。",
      vertexProject: "Vertex 项目",
      vertexRegion: "Vertex 区域",
      viewValue: "查看 {{value}}",
      viewValueDetails: "查看 {{value}} 详情",
      viewDiff: "查看差异",
      virtual: "虚拟",
      virtualApiKey: "虚拟 API 密钥",
      virtualApiKeyModeIsValueUnauthenticatedRequestsMayBeAccepted:
        "虚拟 API 密钥模式为{{value}}；可能会接受未经身份验证的请求。",
      virtualApiKeys: "虚拟 API 密钥",
      virtualModel: "虚拟模型",
      virtualModelName: "虚拟模型名称",
      virtualModelsDefinesASetOfModelsThatCanBeServedFromTheGatewayTheModelNameRefersT_17dk90d:
        "virtualModels 定义了一组可以从网关提供服务的模型。模型名称指的是\n用户请求中匹配的名称。与 `models` 字段不同，虚拟模型会根据配置的逻辑动态路由到特定模型（在 `models` 中配置）。",
      visibilityControlsWhetherClientsCanRequestThisModelDirectlyRatherThanOnlyViaAVirtualModel:
        "可见性控制客户端是否可以直接请求此模型（而不是仅通过 `virtualModel`）。",
      vsCodeSettings: "VS 代码设置",
      waitingForFinalResponse: "正在等待最终响应",
      waitingForModelResponse: "正在等待模型响应",
      warnings: "警告",
      warnings_1j8s2pg: "警告",
      webhook: "Webhook",
      webhookTarget: "Webhook 目标",
      weight: "权重",
      weighted: "加权",
      weightedEnablesWeightBasedSelectionOfTheTargetModel:
        "加权可以基于权重选择目标模型。",
      weightedTargets: "加权目标",
      welcomeToAgentgateway: "欢迎来到代理网关",
      whenMustEvaluateToTrueForThisTargetToBeSelectedOmitOnlyOnTheFinalFallbackTarget:
        "`when` 必须计算为 true 才会选择此目标；仅在最终回退目标上省略。",
      whenThePolicyRunsGatewayPoliciesRunBeforeRouteSelectionWhileRoutePoliciesRunAfte_1ihyj7g:
        "当策略执行时。网关策略在选路之前运行，而路由策略在选路之后运行。\n除非策略需要影响路由选择，否则默认使用路由策略。",
      whenTrueFurtherAnalysisStopsIfABlocklistIsHit:
        "如果为 true，则如果命中阻止列表，则进一步分析将停止",
      whenTrueSkipSpiffeTrustDomainVerificationOnInboundHboneConnections:
        "如果为 true，则跳过入站 HBONE 连接上的 SPIFFE 信任域验证。",
      whereTheActorTokenIsReadFromInTheIncomingRequestTheCelExpressionSourceIsPermitte_1ufgpgq:
        "从传入请求中读取参与者令牌的位置。允许使用 CEL `expression` 源（仅提取）。与主题令牌不同，参与者令牌没有默认来源。",
      whereTheSubjectTokenIsReadFromAndItsTokenTypeDefaultsToTheAuthorizationBearerHea_18ffgbu:
        "从何处读取主题令牌及其令牌类型。默认为\n令牌类型为 access_token 的授权承载请求头。",
      whereTheTokenIsReadFromInTheIncomingRequestTheCelExpressionSourceIsPermittedExtractionOnly:
        "从传入请求中读取令牌的位置。 CEL `expression`\n允许来源（仅限提取）。",
      whereToPlaceTheExchangedTokenInTheBackendRequestDefaultsToTheAuthorizationHeader_1az5m3h:
        "在后端请求中将交换的令牌放置在何处。默认为\n带有“Bearer”前缀的授权请求头。 CEL `expression` 源是\n此处无效（无法插入）。",
      whereToPlaceTheForwardedCredentialInTheBackendRequest:
        "将转发的凭据放置在后端请求中的位置。",
      whereToPlaceTheSecretInTheBackendRequest: "在后端请求中将秘密放在哪里。",
      whereToReadTheApiKeyFromInIncomingRequests:
        "从传入请求中读取 API 密钥的位置。",
      whereToReadTheBasicAuthCredentialsFromInIncomingRequests:
        "从传入请求中读取基本身份验证凭据的位置。",
      whereToReadTheJwtFromInIncomingMcpRequests:
        "从传入的 MCP 请求中读取 JWT 的位置。",
      whereToReadTheJwtFromInIncomingRequests: "从传入请求中读取 JWT 的位置。",
      whetherDownstreamConnectionsMustIncludeAProxyProtocolHeader:
        "下游连接是否必须包含 PROXY 协议请求头。",
      whetherRequestHeadersAreSentToTheExternalProcessingService:
        "请求头是否发送到外部处理服务。",
      whetherRequestTrailersAreSentToTheExternalProcessingService:
        "请求预告片是否发送到外部处理服务。",
      whetherResponseHeadersAreSentToTheExternalProcessingService:
        "响应请求头是否发送到外部处理服务。",
      whetherResponseTrailersAreSentToTheExternalProcessingService:
        "响应尾部是否发送到外部处理服务。",
      whetherTheBindOpensAnOsListenerSocketDefaultsToStandardBindsThePortSetToInternal_jnh5tq:
        "绑定是否打开操作系统监听器套接字。默认为 `standard`（绑定端口）。\n设置为 `internal` 以创建不绑定套接字的仅路由绑定。",
      whetherTheExternalProcessingServiceCanChangeProcessingModesDuringARequest:
        "外部处理服务是否可以在请求期间更改处理模式。",
      whetherThisDescriptorLimitsRequestsOrLlmTokens:
        "此描述符是否限制请求或 LLM 令牌。",
      whetherThisLimitCountsRequestsOrLlmTokens:
        "此限制是否计算请求或 LLM 令牌。",
      whetherToEnableEdns0ExtensionMechanismsForDnsInTheResolverWhenNoneTheSystemProvi_1wj6cfa:
        "是否在解析器中启用EDNS0（DNS扩展机制）。\n当 `None` 时，保留系统提供的解析器设置。\n也可以通过 `DNS_EDNS0` 环境变量进行设置。",
      whetherToSendAPartialBodyWhenTheRequestExceedsMaxRequestBytes:
        "当请求超过`maxRequestBytes`时是否发送部分主体。",
      whetherToSendTheBodyAsRawBytesForGRpcAuthorizationChecks:
        "是否将正文作为原始字节发送以进行 gRPC 授权检查。",
      whetherToTokenizeOnTheRequestFlowThisEnablesUsToDoMoreAccurateRateLimitsSinceWeK_dor0ya:
        "是否对请求流进行标记。这使我们能够进行更准确的速率限制，\n因为我们预先知道请求的（部分）成本。\n这伴随着昂贵的操作成本。",
      whetherToTokenizeTheRequestBeforeForwardingItUpstream:
        "是否在将请求转发到上游之前对请求进行标记。",
      whichIncomingRequestHeadersAreForwardedToThePolicyServer:
        "哪些传入请求头被转发到策略服务器。",
      whichTrafficGatewayExposesTheUi: "哪个流量网关公开 UI。",
      windsurfSettings: "风帆冲浪设置",
      workloadsDefinesTheSetOfWorkloadsThatTheProxyCanServeTheseAreSelectedByServicesT_su2rlz:
        "工作负载定义代理可以服务的工作负载集。这些由 `services` 选择。\n这是一项高级功能，主要用于测试；在路由上使用内联 `backends` 和\n策略通常是首选。",
      x: "x",
      yamlValueReturnedByCelEvaluation: "CEL 评估返回的 YAML 值。",
      addressOfTheCertificateAuthorityUsedToIssueSpiffeCertificates:
        "用于签发 SPIFFE 证书的证书颁发机构地址。",
      addressOfTheXDsControlPlaneUsedForDynamicConfiguration:
        "用于动态配置的 xDS 控制平面地址。",
      alwaysPrefixNamesEvenWithASingleTarget:
        "始终为名称添加前缀，即使只有一个目标。",
      arnOfTheBedrockAgentCoreRuntimeArnAwsBedrockAgentcoreRegionAccountRuntimeId:
        "Bedrock AgentCore 运行时的 ARN（arn:aws:bedrock-agentcore:REGION:ACCOUNT:runtime/ID）。",
      authenticationConfigurationForConnectingToTheLlmProvider:
        "连接 LLM 提供商时使用的身份验证配置。",
      authenticationTokenForCommunicatingWithTheCertificateAuthority:
        "与证书颁发机构通信时使用的身份验证令牌。",
      authenticationTokenForCommunicatingWithTheXDsControlPlane:
        "与 xDS 控制平面通信时使用的身份验证令牌。",
      awsRegionForTheBedrockEndpoint: "Bedrock 端点所在的 AWS 区域。",
      awsRegionToUseForTheBedrockProvider: "Bedrock 提供商使用的 AWS 区域。",
      azureApiVersionQueryParameterForTheEndpoint:
        "端点使用的 Azure API 版本查询参数。",
      backendLevelPoliciesForTcpBackendsSuchAsTlsAuthenticationAndTunneling:
        "TCP 后端的后端级策略，例如 TLS、身份验证和隧道。",
      backendLevelPoliciesSuchAsTlsAuthenticationAndTransformations:
        "后端级策略，例如 TLS、身份验证和转换。",
      backendLevelPoliciesSuchAsTlsAuthenticationTransformationsAndHealthChecks:
        "后端级策略，例如 TLS、身份验证、转换和健康检查。",
      backendPoliciesAppliedToTrafficToThisProvider:
        "应用于流向此提供商流量的后端策略。",
      basePricingRatesForThisModel: "此模型的基础定价费率。",
      behaviorWhenTheBodyExceedsMaxBytesFailClosedRejectOrFailOpenContinue:
        "请求正文超过 maxBytes 时的行为：failClosed（拒绝）或 failOpen（继续）。",
      cachePointInsertionForLlmProvidersThatSupportPromptCaching:
        "针对支持提示词缓存的 LLM 提供商插入缓存点的配置。",
      celExpressionEvaluatedAgainstEachRequestToProduceTheSessionNameForExampleJwtSubO_68dvwh:
        '针对每个请求求值以生成会话名称的 CEL 表达式，例如 `jwt.sub` 或 `request.headers["x-team"]`。如果表达式在请求处理时无法生成有效的会话名称，请求将被拒绝。',
      celExpressionsThatComputeRequestPayloadFieldsOverridingExistingValues:
        "用于计算请求负载字段并覆盖现有值的 CEL 表达式。",
      celExpressionThatSelectsWhichRequestsAreLogged:
        "用于选择要记录哪些请求的 CEL 表达式。",
      conditionsPathMethodHeadersQueryThatSelectThisRoute:
        "用于选择此路由的条件（路径、方法、请求头和查询参数）。",
      configDefinesTopLevelSettingsForDnsAdminNetworkingObservabilityAndSessionManagem_yywaxh:
        "config 定义 DNS、管理、网络、可观测性和会话管理的顶层设置。与其他部分不同，这些设置仅在启动时应用，不会动态重新加载。",
      configurationForUpstreamConnectionsIncludingKeepalivesTimeoutsAndPooling:
        "上游连接配置，包括保活、超时和连接池。",
      connectionUrlForTheRequestLogDatabaseAPostgresOrPostgresqlUrlUsesPostgresAnyOthe_14gqjn4:
        "请求日志数据库的连接 URL。以 postgres:// 或 postgresql:// 开头的 URL 使用 Postgres；其他值均视为 SQLite 数据库。",
      connectToARemoteMcpServerOverHttpWithServerSentEventsSseStreaming:
        "通过 HTTP 连接远程 MCP 服务器，并使用服务器发送事件（SSE）进行流式传输。",
      contextLengthPricingTiersThatOverrideTheBaseRates:
        "覆盖基础费率的上下文长度定价层级。",
      contextTokenThresholdAboveWhichThisTierSRatesApply:
        "超过此上下文令牌阈值后应用本层级的费率。",
      controlsHowUpstreamToolPromptNamesAreExposedToClients:
        "控制如何向客户端公开上游工具和提示词名称。",
      costPer1MInputAudioTokensFallsBackToTheInputRateIfUnset:
        "每 100 万个输入音频令牌的费用。未设置时使用输入费率。",
      costPer1MInputPromptTokens: "每 100 万个输入（提示词）令牌的费用。",
      costPer1MOutputAudioTokensFallsBackToTheOutputRateIfUnset:
        "每 100 万个输出音频令牌的费用。未设置时使用输出费率。",
      costPer1MOutputCompletionTokens: "每 100 万个输出（补全）令牌的费用。",
      costPer1MReasoningTokensFallsBackToTheOutputRateIfUnset:
        "每 100 万个推理令牌的费用。未设置时使用输出费率。",
      costPer1MTokensReadFromCache: "每 100 万个从缓存读取的令牌的费用。",
      costPer1MTokensWrittenToCache: "每 100 万个写入缓存的令牌的费用。",
      customFieldsToAddToAllMetrics: "添加到所有指标的自定义字段。",
      customFieldsToAddToOrRemoveFromLogEntries:
        "要在日志条目中添加或移除的自定义字段。",
      customFieldsToAddToOrRemoveFromTraceSpans:
        "要在追踪跨度中添加或移除的自定义字段。",
      customSessionNameRoleSessionNameForCloudTrailAndCostUsageReportAttributionEither_88b0jv:
        "用于 CloudTrail 和成本与使用情况报告归因的自定义会话名称（RoleSessionName）。可以是静态字符串，也可以是包含针对每个请求求值的 CEL 表达式的 `{expression: ...}`。最长 64 个字符，需匹配 `[\\w+=,.@-]`。未设置时，AWS SDK 会生成随机会话名称。",
      distributedTracingConfiguration: "分布式追踪配置。",
      durationAfterWhichUnusedPooledConnectionsAreReleased:
        "释放连接池中未使用连接前的等待时长。",
      enableIpv6AddressResolutionAndBindingDefaultsToTrue:
        "启用 IPv6 地址解析和绑定。默认为 true。",
      enableTcpKeepaliveProbesOnBackendConnectionsDefaultsToTrue:
        "在后端连接上启用 TCP 保活探测。默认为 true。",
      endpointQualifierVersionOrAliasForTheAgentCoreRuntimeInvocation:
        "调用 AgentCore 运行时时使用的端点限定符（版本或别名）。",
      exactOrRegexPatternTheHeaderValueMustMatch:
        "请求头值必须匹配的精确值或正则表达式。",
      exactOrRegexPatternTheQueryParameterValueMustMatch:
        "查询参数值必须匹配的精确值或正则表达式。",
      fieldNamesToRemoveFromLogEntries: "要从日志条目中移除的字段名称。",
      gatewayLevelPoliciesAppliedToAllTrafficOnThisListener:
        "应用于此监听器全部流量的网关级策略。",
      googleCloudProjectIdForVertexAi:
        "Vertex AI 使用的 Google Cloud 项目 ID。",
      googleCloudProjectIdToUseForTheVertexAiProvider:
        "Vertex AI 提供商使用的 Google Cloud 项目 ID。",
      googleCloudRegionToUseForTheVertexAiProvider:
        "Vertex AI 提供商使用的 Google Cloud 区域。",
      hboneHttp2ConnectTunnelProtocolConfiguration:
        "HBONE（HTTP/2 CONNECT 隧道）协议配置。",
      headersToAddSetOrRemoveOnRequestsToTheLlmProvider:
        "向 LLM 提供商发送请求时要添加、设置或移除的请求头。",
      headersToAddSetOrRemoveOnResponsesFromTheLlmProvider:
        "从 LLM 提供商返回响应时要添加、设置或移除的响应头。",
      headersToDropTakesPrecedenceOverTheAllowList:
        "要丢弃的请求头；其优先级高于允许列表。",
      headersToForwardAnEmptyListForwardsAllHeaders:
        "要转发的请求头；空列表表示转发所有请求头。",
      hostnameOrIpAddressOfTheMcpServer: "MCP 服务器的主机名或 IP 地址。",
      hostnameOrIpAddressOfTheUpstreamToRouteTo:
        "要路由到的上游主机名或 IP 地址。",
      hostnameOrUriOfTheMcpServerForExampleHttpsExampleComOrExampleCom443:
        "MCP 服务器的主机名或 URI，例如 `https://example.com` 或 `example.com:443`。",
      howToNamespaceToolNamesWhenMultiplexingAlwaysPrefixWithTheTargetNameOrOnlyPrefix_198h208:
        "多路复用时工具名称的命名空间方式：`always` 表示始终添加目标名称前缀，`conditional` 表示仅在需要时添加。",
      http2ConnectionLevelFlowControlWindowSizeInBytesDefaultsTo16MiB:
        "HTTP/2 连接级流量控制窗口大小（字节）。默认为 16 MiB。",
      http2MaximumFrameSizeInBytesDefaultsTo1MiB:
        "HTTP/2 最大帧大小（字节）。默认为 1 MiB。",
      http2PerStreamFlowControlWindowSizeInBytesDefaultsTo4MiB:
        "HTTP/2 单流流量控制窗口大小（字节）。默认为 4 MiB。",
      httpHeaderOrPseudoHeaderNameSuchAsMethodToMatch:
        "要匹配的 HTTP 请求头或伪请求头名称（例如 `:method`）。",
      httpHeadersThatMustMatchForThisRouteToApply:
        "应用此路由时必须匹配的 HTTP 请求头。",
      httpHeadersToIncludeOnOtlpTraceExportsSuchAsAuthenticationHeaders:
        "导出 OTLP 追踪时包含的 HTTP 请求头，例如身份验证请求头。",
      httpMethodThatMustMatchForThisRouteToApply:
        "应用此路由时必须匹配的 HTTP 方法。",
      httpRoutesAttachedDirectlyToThisListener:
        "直接附加到此监听器的 HTTP 路由。",
      httpRoutesGroupedTogetherForDelegationAndReuse:
        "为委派和复用而组合在一起的 HTTP 路由。",
      identifierForTheClusterThisGatewayRunsInDefaultsToKubernetes:
        '此网关所在集群的标识符。默认为 "Kubernetes"。',
      identifierForThisBackendReferencedByRoutes:
        "此后端的标识符，供路由引用。",
      identifierForThisRouteGroupReferencedByDelegatingRoutes:
        "此路由组的标识符，供委派路由引用。",
      identifierOfTheBedrockGuardrailToApply: "要应用的 Bedrock 防护栏标识符。",
      idleTimeBeforeTheFirstKeepaliveProbeIsSent:
        "发送第一次保活探测前的空闲时长。",
      kubernetesNamespaceForThisGatewayInstance:
        "此网关实例所在的 Kubernetes 命名空间。",
      kubernetesServiceAccountForThisGatewayUsedInItsSpiffeIdentity:
        "此网关使用的 Kubernetes 服务账号，用于其 SPIFFE 身份。",
      llmProvidersInThisGroupLoadBalancedTogether:
        "此组中共同参与负载均衡的 LLM 提供商。",
      loggingConfigurationIncludingFilterLevelFormatAndCustomFields:
        "日志配置，包括过滤器、级别、格式和自定义字段。",
      logLevelASingleLevelEGInfoACommaSeparatedStringOfPerModuleLevelsEGInfoAgentCoreT_1appp3y:
        "日志级别：可以是单个级别（如 `info`）、以逗号分隔的各模块级别字符串（如 `info,agent_core=trace`），或各模块级别列表（如 `[info, agent_core=trace]`）。",
      logOutputFormatTextOrJson: "日志输出格式：`text` 或 `json`。",
      logStoreDatabaseConfigurationEnablesRequestLoggingToADatabaseBackend:
        "日志存储数据库配置；用于启用将请求日志记录到数据库后端。",
      mapOfFieldNameToACelExpressionThatComputesTheValueToAddToLogs:
        "字段名称到 CEL 表达式的映射，表达式用于计算要添加到日志中的值。",
      mapOfFieldNameToACelExpressionThatComputesTheValueToAddToMetrics:
        "字段名称到 CEL 表达式的映射，表达式用于计算要添加到指标中的值。",
      mapOfModelIdToItsPricingRatesAndTiers:
        "模型 ID 到其定价费率和层级的映射。",
      mapOfProviderNameToItsSupportedModelsAndPricing:
        "提供商名称到其支持模型和定价的映射。",
      maximumConcurrentStreamsPerPooledConnectionDefaultsTo100:
        "每个池化连接允许的最大并发流数。默认为 100。",
      maximumTimeToWaitForConnectionsToCloseGracefullyDuringShutdown:
        "关闭期间等待连接正常关闭的最长时间。",
      maximumTimeToWaitWhenEstablishingAConnectionToAnUpstreamDefaultsTo10Seconds:
        "与上游建立连接时的最长等待时间。默认为 10 秒。",
      mcpServerTargetsToMultiplexTogether: "要进行多路复用的 MCP 服务器目标。",
      messageRoleSuchAsSystemUserOrAssistant:
        '消息角色，例如 "system"、"user" 或 "assistant"。',
      messageTextContent: "消息的文本内容。",
      metricNamesToExcludeFromCollection: "不采集的指标名称。",
      metricsConfigurationIncludingMetricRemovalAndCustomFields:
        "指标配置，包括移除指标和自定义字段。",
      minimumTimeToAllowForGracefulConnectionTerminationDefaultsToZero:
        "允许连接正常终止的最短时间。默认为零。",
      modelCostCatalogProvidedInlineAsAString:
        "以字符串形式内联提供的模型成本目录。",
      modelCostCatalogProvidedInlineAsStructuredData:
        "以结构化数据形式内联提供的模型成本目录。",
      modelIdToSendToAnthropicOverridingTheModelInTheClientRequest:
        "发送给 Anthropic 的模型 ID，将覆盖客户端请求中的模型。",
      modelIdToSendToAzureOverridingTheModelInTheClientRequest:
        "发送给 Azure 的模型 ID，将覆盖客户端请求中的模型。",
      modelIdToSendToBedrockOverridingTheModelInTheClientRequest:
        "发送给 Bedrock 的模型 ID，将覆盖客户端请求中的模型。",
      modelIdToSendToGeminiOverridingTheModelInTheClientRequest:
        "发送给 Gemini 的模型 ID，将覆盖客户端请求中的模型。",
      modelIdToSendToGitHubCopilotOverridingTheModelInTheClientRequest:
        "发送给 GitHub Copilot 的模型 ID，将覆盖客户端请求中的模型。",
      modelIdToSendToOpenAiOverridingTheModelInTheClientRequest:
        "发送给 OpenAI 的模型 ID，将覆盖客户端请求中的模型。",
      modelIdToSendToTheProviderOverridingTheModelInTheClientRequest:
        "发送给提供商的模型 ID，将覆盖客户端请求中的模型。",
      modelIdToSendToVertexAiOverridingTheModelInTheClientRequest:
        "发送给 Vertex AI 的模型 ID，将覆盖客户端请求中的模型。",
      namedListenersBoundOnThisPortWhichMayUseDifferentProtocolsAndTls:
        "绑定到此端口的具名监听器，可使用不同的协议和 TLS 配置。",
      nameIdentifyingThisListenerReferencedByGatewaysGatewayNameListenerName:
        "此监听器的名称，通过 `gateways: gateway-name/listener-name` 引用。",
      nameIdentifyingThisMcpTargetUsedToPrefixToolAndResourceNamesWhenMultiplexing:
        "此 MCP 目标的名称，多路复用时用于为工具和资源名称添加前缀。",
      nameIdentifyingThisProviderReferencedByLlmModelsProvider:
        "此提供商的名称，通过 `llm.models[].provider` 引用。",
      nameIdentifyingThisResource: "此资源的名称。",
      nameIdentifyingThisRoute: "此路由的名称。",
      nameOfTheGatewayThisTargetReferences: "此目标引用的网关名称。",
      nameOfTheListenerSetResource: "监听器集资源的名称。",
      nameOfTheTargetServiceAsDefinedInTheTopLevelServicesList:
        "目标服务的名称，该服务定义在顶层 `services` 列表中。",
      nameOfThisGatewayRequiredWhenXDsIsConfigured:
        "此网关的名称。配置 xDS 时为必填项。",
      namespaceOfTheGatewayThisTargetReferences: "此目标引用的网关命名空间。",
      namespaceOfTheListenerSetResource: "监听器集资源的命名空间。",
      namespaceScopingThisListener: "限定此监听器作用域的命名空间。",
      namespaceScopingThisResourceUsedInFullyQualifiedNamespaceNameReferences:
        "限定此资源作用域的命名空间，用于完全限定的 `namespace/name` 引用。",
      namespaceScopingThisRoute: "限定此路由作用域的命名空间。",
      namespaceScopingThisRouteUsedInFullyQualifiedNamespaceNameReferences:
        "限定此路由作用域的命名空间，用于完全限定的 `namespace/name` 引用。",
      networkNameForThisGatewayUsedForLocalityAwareRouting:
        "此网关的网络名称，用于位置感知路由。",
      neverPrefixNamesWithMultipleTargetsCallsAreRoutedByLookingUpWhichTargetServesThe_1js7ysf:
        "从不为名称添加前缀；存在多个目标时，通过查找提供该名称的目标来路由调用。要求名称在所有目标中唯一。",
      numberOfUnacknowledgedProbesBeforeTheConnectionIsConsideredDead:
        "连接被视为已断开前允许的未确认探测次数。",
      numberOfWorkerThreadsForTheAsyncRuntimeAcceptsANumberOrAStringSuchAsAuto:
        '异步运行时的工作线程数。可以是数字，也可以是 "auto" 等字符串。',
      oauth20ClientSecretSentViaHttpBasicAuthToTheAuthorizationServer:
        "通过 HTTP Basic 身份验证发送给授权服务器的 OAuth 2.0 客户端密钥。",
      otlpCollectorEndpointUrlForExportingTraces:
        "用于导出追踪数据的 OTLP 收集器端点 URL。",
      otlpTransportProtocolGrpcOrHttp: "OTLP 传输协议：`grpc` 或 `http`。",
      outlierDetectionAndHealthCheckingForThisProviderBackend:
        "对此提供商后端执行异常检测和健康检查。",
      pathMatchRuleExactPrefixOrRegexDefaultsToAPrefixMatch:
        '路径匹配规则（精确、前缀或正则表达式）。默认为 "/" 前缀匹配。',
      pathToAFileOnDiskContainingTheModelCostCatalog:
        "磁盘上包含模型成本目录的文件路径。",
      pathToAFileOnDiskToLoadTheValueFrom: "用于加载值的磁盘文件路径。",
      pathToARootCaCertificateFileUsedToValidateClientCertificates:
        "用于验证客户端证书的根 CA 证书文件路径。",
      pathToTheTlsCertificateFileLeafCertificateOrCaCertificateInDynamicCaMode:
        "TLS 证书文件路径（叶证书；在动态 CA 模式下则为 CA 证书）。",
      pathToTheTlsPrivateKeyFile: "TLS 私钥文件路径。",
      policiesAppliedToMcpRequests: "应用于 MCP 请求的策略。",
      policiesAppliedToThisMcpTarget: "应用于此 MCP 目标的策略。",
      portOnTheMcpServerToConnectTo: "要连接的 MCP 服务器端口。",
      portOnTheTargetServiceToRouteTo: "要路由到的目标服务端口。",
      portToTargetAsAnAlternativeToListenerName:
        "要作为目标的端口，可替代 listener_name。",
      prefixNamesWithTheTargetNameOnlyWhenThereAreMultipleTargets:
        "仅在存在多个目标时，使用目标名称作为名称前缀。",
      pricingRatesForThisTierOverlaidOnTheBaseModelRates:
        "此层级的定价费率，将覆盖到模型基础费率之上。",
      protocolThisListenerAcceptsHttpHttpsTcpTlsOrHbone:
        "此监听器接受的协议：HTTP、HTTPS、TCP、TLS 或 HBONE。",
      protocolUsedToTunnelBackendConnectionsSuchAsDirectOrHbone:
        "用于建立后端连接隧道的协议，例如 Direct 或 HBONE。",
      queryParameterNameToMatch: "要匹配的查询参数名称。",
      queryParametersThatMustMatchForThisRouteToApply:
        "应用此路由时必须匹配的查询参数。",
      relativeProportionOfTrafficSentToThisTargetModelDefaultsTo1:
        "发送到此目标模型的相对流量比例。默认为 1。",
      relativeWeightForLoadBalancingAcrossBackendsDefaultsTo1:
        "在各后端之间进行负载均衡的相对权重。默认为 1。",
      relativeWeightForLoadBalancingAcrossTcpBackendsDefaultsTo1:
        "在各 TCP 后端之间进行负载均衡的相对权重。默认为 1。",
      requestHeadersToMatchForConditionalModelRouting:
        "条件模型路由需要匹配的请求头。",
      requestPathOnTheMcpServer: "MCP 服务器上的请求路径。",
      requestPayloadFieldsToSetOverridingAnyExistingValuesInTheRequest:
        "要设置的请求负载字段，将覆盖请求中的所有现有值。",
      requestPayloadFieldsToSetWhenNotAlreadyPresentInTheRequest:
        "仅当请求中尚不存在时才设置的请求负载字段。",
      resourceKindUsedInPolicyTargetReferences:
        "策略目标引用中使用的资源种类。",
      routeLevelPoliciesAppliedBeforeBackendSelection:
        "选择后端之前应用的路由级策略。",
      routeToAServiceDefinedInTheTopLevelServicesList:
        "路由到顶层 `services` 列表中定义的服务。",
      sessionNameRoleSessionNameInConfigurationFormAStaticStringOrACelExpressionEvalua_zywvwc:
        "配置形式的会话名称（RoleSessionName）：可以是静态字符串，也可以是针对每个请求求值的 CEL 表达式。此字段无标签，因此普通字符串仍保持原有含义。",
      specificListenerWithinTheGatewayIfUnsetTargetsTheGatewayItself:
        "网关内的特定监听器；未设置时以网关本身为目标。",
      specificListenerWithinTheListenerSetToTarget:
        "监听器集中要作为目标的特定监听器。",
      specificRuleWithinTheRouteForTargetedPolicyReferences:
        "路由内的特定规则，用于定向策略引用。",
      specificRuleWithinThisRoute: "此路由内的特定规则。",
      spiffeTrustDomainForThisGateway: "此网关的 SPIFFE 信任域。",
      staticSessionName: "静态会话名称。",
      supportedApiPayloadFormatsAndOptionalPathOverridesForThisProvider:
        "此提供商支持的 API 负载格式，以及可选的路径覆盖。",
      tcpKeepaliveConfigurationForUpstreamConnections:
        "上游连接的 TCP 保活配置。",
      tcpLevelPoliciesAppliedToTrafficOnThisRoute:
        "应用于此路由流量的 TCP 级策略。",
      tcpRoutesAttachedDirectlyToThisListener:
        "直接附加到此监听器的 TCP 路由。",
      theUpstreamLlmProviderTypeAndItsConfiguration:
        "上游 LLM 提供商类型及其配置。",
      timeBetweenSuccessiveKeepaliveProbes: "连续两次保活探测之间的时间。",
      timeToLiveForMcpSessionsBeforeTheyAreClosedAutomaticallyDefaultsTo30Minutes:
        "MCP 会话自动关闭前的生存时间。默认为 30 分钟。",
      tlsConfigurationForConnectingToTheLlmProvider:
        "连接 LLM 提供商时使用的 TLS 配置。",
      tlsConfigurationForConnectionsToTheTcpRouteSBackend:
        "连接 TCP 路由后端时使用的 TLS 配置。",
      tlsConfigurationUsedWithTheHttpsAndTlsProtocols:
        "与 HTTPS 和 TLS 协议配合使用的 TLS 配置。",
      tunnelingConfigurationForConnectingToTheLlmProvider:
        "连接 LLM 提供商时使用的隧道配置。",
      versionOfTheBedrockGuardrailToApply: "要应用的 Bedrock 防护栏版本。",
      weightedBackendsThisRouteForwardsTrafficTo:
        "此路由将流量转发到的加权后端。",
      weightedBackendsThisTcpRouteForwardsTrafficTo:
        "此 TCP 路由将流量转发到的加权后端。",
      whetherToKeepAPersistentSessionAcrossRequestsStatefulOrCreateOnePerRequestStateless:
        "是在多个请求之间保留持久会话（Stateful），还是为每个请求创建独立会话（Stateless）。",
      yourChangesHaveNotBeenSavedAndWillBeLost:
        "你的更改尚未保存，关闭后将丢失。",
    },
  },
} as const satisfies LocaleShape<typeof en>;

export default zhCN;
