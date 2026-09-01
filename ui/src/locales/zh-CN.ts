import type en from '@/locales/en';

type LocaleShape<T> = {
	readonly [Key in keyof T]: T[Key] extends string ? string : LocaleShape<T[Key]>;
};

const zhCN = {
	translation: {
		common: {
			apply: '应用',
			auto: '自动',
			cancel: '取消',
			close: '关闭',
			confirm: '确认',
			copied: '已复制',
			discardChanges: '放弃更改',
			noMatches: '没有匹配项',
			noOptions: '没有可选项',
			noMatchesCustomValues: '没有匹配项，但可以使用自定义值。',
			noValuesFound: '未找到值。',
			notAvailable: '不适用',
			save: '保存',
			search: '搜索 {{label}}',
			searchPlaceholder: '搜索 {{label}}…',
			select: '选择',
			showOptions: '显示 {{label}} 的选项',
			viewDiff: '查看差异'
		},
		dateRange: {
			apply: '应用',
			cancel: '取消',
			from: '开始时间',
			interval: '间隔',
			last1Hour: '最近 1 小时',
			last12Hours: '最近 12 小时',
			last24Hours: '最近 24 小时',
			last7Days: '最近 7 天',
			last14Days: '最近 14 天',
			last30Days: '最近 30 天',
			quickRanges: '快捷时间范围',
			to: '结束时间',
			second_one: '{{count}} 秒',
			second_other: '{{count}} 秒',
			minute_one: '{{count}} 分钟',
			minute_other: '{{count}} 分钟',
			hour_one: '{{count}} 小时',
			hour_other: '{{count}} 小时',
			day_one: '{{count}} 天',
			day_other: '{{count}} 天',
			invalidDate: '无效日期'
		},
		drawer: {
			discardUnsavedChanges: '放弃未保存的更改？',
			unsavedChangesMessage: '你的更改尚未保存，关闭后将丢失。'
		},
		schema: {
			array: '数组',
			help: '帮助',
			object: '对象',
			oneOf: '以下选项之一：',
			required: '必填',
			value: '值'
		},
		shell: {
			clientSetup: '客户端设置',
			documentation: '文档',
			feedback: '反馈',
			gatewayOverview: '网关概览',
			llmConfiguration: 'LLM 配置',
			mcpConfiguration: 'MCP 配置',
			policyTools: '策略工具',
			primaryNavigation: '主导航',
			projectLinks: '项目链接',
			toggleTheme: '切换主题',
			trafficConfiguration: '流量配置'
		},
		language: {
			english: 'English',
			select: '选择语言',
			simplifiedChinese: '简体中文'
		},
		nav: {
			analytics: '分析',
			celPlayground: 'CEL 演练场',
			chatPlayground: '聊天演练场',
			clientSetup: '客户端设置',
			costs: '成本',
			gateway: '网关',
			gateways: '网关',
			getStarted: '开始使用',
			guardrails: '防护规则',
			home: '首页',
			keys: '虚拟 API 密钥',
			listeners: '监听器',
			llm: 'LLM',
			logs: '日志',
			mcp: 'MCP',
			models: '模型',
			policies: '策略',
			providers: '提供商',
			rawConfiguration: '原始配置',
			routes: '路由',
			servers: '服务器',
			settings: '设置',
			toolPlayground: '工具演练场',
			tools: '工具',
			traffic: '流量'
		},
		copy: {
			assistant: '助手',
			audio: '音频',
			authMethod: '身份验证方式',
			backendAuth: '后端身份验证',
			backendAuthCannotBeEmpty: '后端身份验证配置不能为空。',
			backendAuthYaml: '后端身份验证 YAML',
			celCredentialExtractionOnly:
				'CEL 表达式可以提取凭据，但不能写入凭据。后端身份验证请改用请求头、查询参数或 Cookie。',
			cache: '缓存',
			cacheHit: '缓存命中',
			cacheValue: '缓存：{{value}}',
			chooseWhereTheCredentialIsReadFromOrWrittenTo: '选择读取或写入凭据的位置。',
			cookie: 'Cookie',
			cookieName: 'Cookie 名称',
			collapse: '收起',
			completed: '完成时间',
			database: '数据库',
			addMcpServer: '添加 MCP 服务器',
			anotherGatewayIsAlreadyTheDefaultGateway: '已有其他网关被设为默认网关。',
			apiKeyAuthenticationIsDisabled: 'API 密钥身份验证已禁用',
			apiKeyPolicyConfigDiff: 'API 密钥策略配置差异',
			baseCatalog: '基础目录',
			baseCostsAreStoredInValueFileWritesAreDisabledInHybridMode:
				'基础成本存储在 {{value}} 中。混合模式下无法写入文件。',
			blankFinalConditionMeansFallback: '最后一个条件留空时将作为回退项',
			byDefaultCallersSendAuthorizationBearerKey:
				'默认情况下，调用方通过 Authorization: Bearer key 发送密钥。',
			cannotEnterCustomModelNamesSet: '无法输入自定义模型名称；请设置',
			chooseAUniqueGatewayName: '请选择唯一的网关名称。',
			configurationDatabaseUnavailable: '配置数据库不可用',
			configureProviders: '配置提供商',
			corsUpdateFailed: 'CORS 更新失败',
			createVirtualKey: '创建虚拟密钥',
			customInlineOverlay: '自定义内联覆盖',
			customOverrides: '自定义覆盖',
			customExpression: '自定义表达式',
			defaultUnhealthyDetectionConfigured: '已配置默认异常检测。',
			customizeWhereVirtualApiKeysAreReadFromTheRequest:
				'自定义从请求中的哪个位置读取虚拟 API 密钥。',
			defaultGateway: '默认网关',
			defaultNameIsReserved: 'default 名称为保留名称',
			defaultsConfigured: '已配置默认值',
			deleteDatabaseResource: '删除数据库资源',
			deleteDatabaseResourceQuestion: '删除数据库资源？',
			deleteDatabaseResourceValue: '要删除数据库资源 {{value}}/{{value}} 吗？此操作无法撤销。',
			deleteFailed: '删除失败',
			deleteResource: '删除资源',
			disabledFieldsAreManagedByTheFileConfigurationOtherMcpSettingsCanStillBeSavedToTheDatabase:
				'已禁用的字段由文件配置管理。其他 MCP 设置仍可保存到数据库。',
			editMcpServer: '编辑 MCP 服务器',
			editVirtualKey: '编辑虚拟密钥',
			editVirtualModel: '编辑虚拟模型',
			enableApiKeyAuth: '启用 API 密钥身份验证',
			enableApiKeyAuthentication: '启用 API 密钥身份验证',
			enableApiKeyAuthenticationBeforeProvisioningVirtualKeys:
				'请先启用 API 密钥身份验证，再创建虚拟密钥。',
			failedToLoadAnalytics: '加载分析数据失败',
			failedToLoadLogDetail: '加载日志详情失败',
			failedToLoadLogs: '加载日志失败',
			failedToRefreshBaseCostCatalog: '刷新基础成本目录失败',
			failedToSaveCustomCosts: '保存自定义成本失败',
			fallback: '回退',
			fallbackGroupValue: '回退组 {{value}}',
			fileConfigurationIsReadOnlyInHybridModeCopyThisDiffAndUpdateTheConfigurationFileDirectly:
				'文件配置在混合模式下为只读。请复制此差异，直接更新配置文件。',
			fileOwnedGatewaysCannotBeDeletedHere: '无法在此处删除文件所属的网关',
			fileOwnedGatewaysMustBeEditedInRawConfiguration: '文件所属的网关必须在原始配置中编辑',
			fileOwnedKeysCannotBeDeletedHere: '无法在此处删除文件所属的密钥',
			fileOwnedListenersCannotBeDeletedHere: '无法在此处删除文件所属的监听器',
			fileOwnedListenersMustBeEditedInRawConfiguration: '文件所属的监听器必须在原始配置中编辑',
			fileOwnedModelsCannotBeDeletedHere: '无法在此处删除文件所属的模型',
			fileOwnedServersCannotBeDeletedHere: '无法在此处删除文件所属的服务器',
			finalTransformation: '最终转换',
			firstAttempt: '首次尝试',
			forModelsMissingFromTheProviderList: '，用于提供商列表中缺少的模型。',
			gatewayConfigDiff: '网关配置差异',
			gatewayListenerConfigDiff: '网关监听器配置差异',
			gatewayNameAlreadyExists: '网关名称已存在',
			hideJson: '隐藏 JSON',
			inline: '内联',
			inspectConfigurationResourcesStoredInTheDatabase: '查看存储在数据库中的配置资源。',
			invalidConfigurationYaml: '配置 YAML 无效',
			invalidPolicyConfiguration: '策略配置无效',
			invalidRateValueUseANonNegativeDecimalWithUpTo6DecimalPlaces:
				'费率“{{value}}”无效。请输入非负数，小数位数不超过 6 位。',
			invalidServerConfiguration: '服务器配置无效',
			keepExisting: '保留现有密钥',
			loadingDatabaseResources: '正在加载数据库资源',
			logSettingsConfigDiff: '日志设置配置差异',
			logStreamFailed: '日志流加载失败',
			mcpServerConfigDiff: 'MCP 服务器配置差异',
			mcpSettingsConfigDiff: 'MCP 设置配置差异',
			modelConfigDiff: '模型配置差异',
			modelIsRequiredForEveryCustomCostRow: '每个自定义成本行都必须填写模型。',
			modelResourceDiff: '模型资源差异',
			noAuthorizationRulesConfigured: '尚未配置授权规则',
			noDatabaseResources: '暂无数据库资源',
			noDefaultsConfigured: '尚未配置默认值',
			noDiff: '没有差异',
			noEnabledTraffic: '没有已启用的流量',
			noFieldsConfigured: '尚未配置字段',
			noModelPoliciesConfigured: '尚未配置模型策略',
			noModelSelected: '未选择模型',
			noOverridesConfigured: '尚未配置覆盖值',
			orExportTheVariablesBelowBeforeStartingASession: '，或在启动会话前导出下列变量。',
			overrideActiveClickToWriteThisChangeToTheConfigurationFile:
				'覆盖已生效。点击可将此更改写入配置文件。',
			overridesConfigured: '已配置覆盖值',
			pointGooseSOpenAiProviderAtTheGatewayHostAndChatCompletionsPath:
				'将 Goose 的 OpenAI 提供商指向网关主机和聊天补全路径。',
			providerIsRequiredForEveryCustomCostRow: '每个自定义成本行都必须填写提供商。',
			providerRequestFields: '提供商请求字段',
			removeGroup: '移除组',
			removeRule: '移除规则',
			replaceKey: '替换密钥',
			revision: '修订号',
			ruleValue: '规则 {{value}}',
			saveGateway: '保存网关',
			saveKey: '保存密钥',
			saveListener: '保存监听器',
			saveModel: '保存模型',
			saveServer: '保存服务器',
			saveSettings: '保存设置',
			saveVirtualModel: '保存虚拟模型',
			selectDefaultGatewayToUseTheNameValue: '请选择“默认网关”以使用名称 {{value}}。',
			someSettingsAreFileOwned: '部分设置由文件配置管理',
			sourcesAreMergedInOrderDatabaseSourcesLoadFirstAndLaterFileSourcesOverrideThem:
				'配置源按顺序合并：先加载数据库源，后加载的文件源会覆盖前者。',
			storesPromptAndCompletionContentInTheDatabasePayload:
				'将提示词和补全内容存储在数据库负载中。',
			stripPrefix: '移除前缀',
			stripValue: '移除 {{value}}',
			thisApiKeyPolicyIsFileOwnedAndCannotBeModifiedInHybridMode:
				'此 API 密钥策略归文件配置所有，无法在混合模式下修改。',
			thisWillMakeTheGatewayTheDefaultGatewayAndImpactValueTraffic:
				'这会将该网关设为默认网关。受影响的流量：{{value}}。',
			toPersistTheSettingsAddThemTo: '如需持久保存这些设置，请将其添加到',
			unavailableWhileGatewayTlsOrPoliciesAreConfigured: '网关已配置 TLS 或策略时无法使用此选项。',
			unknown: '未知',
			unknownSource: '未知来源',
			unnamedKey: '未命名密钥',
			updated: '更新时间',
			useNamedListenersForPerHostnameTlsAndPolicies:
				'使用具名监听器，为每个主机名单独配置 TLS 和策略。',
			useThisGatewayForEnabledTrafficWithoutAnExplicitGatewaySelection:
				'让未明确选择网关的已启用流量使用此网关。',
			valueFieldsConfigured: '已配置 {{value}} 个字段',
			valueMessages_one: '{{count}} 条消息',
			valueMessages_other: '{{count}} 条消息',
			valueNeedsAtLeastOneRate: '{{value}}/{{value}} 至少需要一项费率。',
			valueRecentCalls: '最近 {{value}} 次调用',
			valueRulesConfigured_one: '已配置 {{count}} 条规则',
			valueRulesConfigured_other: '已配置 {{count}} 条规则',
			viewJson: '查看 JSON',
			virtualApiKeyConfigDiff: '虚拟 API 密钥配置差异',
			virtualApiKeyResourceDiff: '虚拟 API 密钥资源差异',
			virtualModelConfigDiff: '虚拟模型配置差异',
			virtualModelResourceDiff: '虚拟模型资源差异',
			deletePolicyFromThisConfiguration: '要从此配置中删除 {{value}} 策略吗？',
			deletePolicyQuestion: '删除策略？',
			editBackendAuthPolicyYamlDirectly:
				'直接编辑策略 YAML，并使用 Schema 自动补全。适用于尚无结构化编辑器的方式：key、AWS、GCP、Azure、Copilot、OAuth 和跨应用访问。',
			emptyMessage: '空消息',
			emptyResult: '空结果',
			firstToken: '首个令牌',
			generated: '已生成',
			header: '请求头',
			in: '输入',
			loadingCostConfiguration: '正在加载成本配置',
			loadingLlmConfiguration: '正在加载 LLM 配置',
			loadingPolicies: '正在加载策略',
			locationType: '位置类型',
			overrideWhereThisPolicyReadsTheCredential: '覆盖此策略读取凭据的位置。',
			overrideWhereTheValidatedCredentialIsSent: '覆盖已验证凭据的发送位置。',
			out: '输出',
			passthrough: '透传',
			policyIsNotEnabled: '策略未启用',
			prompt: '提示词',
			promptCachingConfigured: '已配置提示词缓存',
			queryParameter: '查询参数',
			queryParameterName: '查询参数名称',
			rawYaml: '原始 YAML',
			reportedTotal: '上报总计',
			selectHowTheGatewayAuthenticatesToTheBackend: '选择网关向后端进行身份验证的方式。',
			showAllValueCharacters: '显示全部（{{value}} 个字符）',
			speed: '速度',
			status: '状态',
			storage: '存储',
			system: '系统',
			time: '时间',
			uncached: '未缓存',
			usage: '用量',
			valueCharacters: '{{value}} 个字符',
			loadingLogs: '正在加载日志',
			noLogEntries: '没有日志记录',
			yModelcontextprotocolServerFilesystemTmp: '-y @modelcontextprotocol/server-filesystem /tmp',
			valueValueValue: ':{{value}} · {{value}} · {{value}}',
			thisCannotBeUndone: '？此操作无法撤销。',
			trafficCanNoLongerBeSentToThisTarget: '？无法再将流量发送到该目标。',
			trafficMatchingThisRouteWillNoLongerReachItsBackends: '？匹配该路由的流量将不再到达其后端。',
			considerMovingListenerOwnershipTo: '。考虑将监听器所有权移至',
			oauth2Auth: '"/oauth2/auth"',
			value: '{{value}}',
			valueAllValueListeners: '{{value}}（所有 {{value}} 监听器）',
			valueAllListeners: '{{value}}（所有监听器）',
			valueValueConfigured: '已配置 {{value}} 个{{value}}。',
			noRequestHeaderChangesConfigured: '未配置请求头变更',
			noResponseHeaderChangesConfigured: '未配置响应头变更',
			requestHeaderChangesConfigured_one: '已配置 {{count}} 项请求头变更',
			requestHeaderChangesConfigured_other: '已配置 {{count}} 项请求头变更',
			responseHeaderChangesConfigured_one: '已配置 {{count}} 项响应头变更',
			responseHeaderChangesConfigured_other: '已配置 {{count}} 项响应头变更',
			valueBinds_one: '{{count}} 个绑定',
			valueBinds_other: '{{count}} 个绑定',
			valueByValue: '{{value}}（按{{value}}）',
			valueConfiguredServers_one: '{{count}} 个已配置服务器',
			valueConfiguredServers_other: '{{count}} 个已配置服务器',
			valueEnabled: '{{value}} 已启用',
			valueGateways_one: '{{count}} 个网关',
			valueGateways_other: '{{count}} 个网关',
			valueListenerValueMixHttpAndTcpRoutes: '部分监听器同时包含 HTTP 和 TCP 路由',
			valueListeners_one: '{{count}} 个监听器',
			valueListeners_other: '{{count}} 个监听器',
			valueModels_one: '{{count}} 个模型',
			valueModels_other: '{{count}} 个模型',
			valueOf3Enabled: '已启用 {{value}} / 3 项',
			valuePathOverride: '{{value}} 路径覆盖',
			valuePolicies_one: '{{count}} 项策略',
			valuePolicies_other: '{{count}} 项策略',
			valuePrioritiesValueTargets: '{{value}} 个优先级，{{value}} 个目标',
			valueQuickKeys: '{{value}} 快捷键',
			valueRoutes_one: '{{count}} 条路由',
			valueRoutes_other: '{{count}} 条路由',
			valueRows_one: '{{count}} 行',
			valueRows_other: '{{count}} 行',
			valueRules_one: '{{count}} 条规则',
			valueRules_other: '{{count}} 条规则',
			valueRulesWithFallback_one: '{{count}} 条规则，含回退',
			valueRulesWithFallback_other: '{{count}} 条规则，含回退',
			valueSharedProviders_one: '{{count}} 个共享提供商',
			valueSharedProviders_other: '{{count}} 个共享提供商',
			valueTokensValueCalls: '{{value}} 个令牌 / {{value}} 次调用',
			valueTotal: '{{value}}总计',
			valueVirtualModels_one: '{{count}} 个虚拟模型',
			valueVirtualModels_other: '{{count}} 个虚拟模型',
			valueWarningValue_one: '{{count}} 条警告',
			valueWarningValue_other: '{{count}} 条警告',
			valueWeightedTargets_one: '{{count}} 个加权目标',
			valueWeightedTargets_other: '{{count}} 个加权目标',
			valueValue: '{{value}}/{{value}}',
			audienceParametersNamingTheTargetServicesAtTheAuthorizationServer:
				'`audience` 参数在授权服务器上命名目标服务。',
			clientIdParameterIdentifyingTheGatewayAtTheAuthorizationServer:
				'`client_id` 参数标识授权服务器上的网关。',
			clientIdClientSecretSentInTheHttpBasicAuthorizationHeaderRfc6749231:
				'`client_id`/`client_secret` 在 HTTP 基本授权请求头中发送（RFC 6749 §2.3.1）。',
			clientIdClientSecretSentInTheRequestFormBody:
				'`client_id`/`client_secret` 在请求表单正文中发送。',
			privateKeyJwtClientAssertionRfc7523: '`privateKeyJwt` 客户端断言（RFC 7523）。',
			requestedTokenTypeParameterWhenUnsetTheFormFieldIsOmittedAndADeclaredResponseTyp_46odee:
				'`requested_token_type` 参数。未设置时省略此表单字段，并预期声明的响应类型为 `access_token`。',
			resourceParametersNamingTheProtectedResourceApis: '`resource` 命名受保护资源 API 的参数。',
			resourceParametersWithTheTargetServiceUris: '带有目标服务 URI 的 `resource` 参数。',
			scopeValuesForTheRequestedTokenSentSpaceDelimited:
				'所请求令牌的 `scope` 值，以空格分隔发送。',
			text200Ok: '200 OK',
			text400BadRequest: '400 错误请求',
			text401Unauthorized: '401 未经授权',
			text403Forbidden: '403 禁止访问',
			text404NotFound: '404 未找到',
			text429RateLimited: '429 请求受限',
			text500ServerError: '500 服务器错误',
			aCustomProviderSAdvertisedUpstreamWireFormatUnlikeInputFormatThisDescribesWhatTh_fgckra:
				'自定义提供商声明的上游传输格式。\n\n与 `InputFormat` 不同，此处描述后端接受的格式，而不是客户端发送的格式。与 `RouteType` 不同，它只适用于可转换或透传的 LLM 负载端点；`models`、`passthrough`、`detect` 等通用路由没有 `ProviderFormat`。',
			aSourceOfModelCostCatalogData: '模型成本目录数据的来源。',
			aValidTokenIssuedByAConfiguredIssuerMustBePresentThisIsTheDefaultOption:
				'必须存在由配置的签发者颁发的有效令牌。\n这是默认选项。',
			absoluteCallbackUriHandledByTheGatewayThisPolicyAlwaysRedirectsUnauthenticatedNo_1lp94cf:
				'由网关处理的绝对回调 URI。\n此策略会将所有未经身份验证且并非回调的请求重定向到登录流程。',
			acceptAnyRequestHeaderInBrowserPreflightChecks: '接受浏览器预检检查中的任何请求头',
			acceptConnectionsWithOrWithoutAProxyProtocolHeader:
				'接受带有或不带有 PROXY 协议请求头的连接。',
			acceptProxyProtocolV1OrV2: '接受 PROXY 协议 v1 或 v2。',
			acceptProxyProtocolV1: '接受 PROXY 协议 v1。',
			acceptProxyProtocolV2: '接受 PROXY 协议 v2。',
			acceptedTokenAudiencesMatchedAgainstTheJwtAudClaimWhenSet:
				'接受的令牌受众，在设置时与 JWT `aud` 声明进行匹配。',
			acceptedTokenAudiencesMatchedAgainstTheJwtAudClaim:
				'接受的令牌受众，与 JWT `aud` 声明相匹配。',
			access: '访问',
			accessLogFieldNamesToRemove: '要删除的访问日志字段名称。',
			accessLogFieldsToAddComputedFromCelExpressions:
				'要添加的访问日志字段（根据 CEL 表达式计算）。',
			action: '操作',
			actionToTakeWhenARegexRuleMatches: '正则表达式规则匹配时要采取的操作。',
			actions: '操作',
			activeRuntimeResourcesFromTheGatewayDump: '来自网关转储的活动运行时资源。',
			adc: 'ADC',
			adcCompatibleGoogleCredentialJsonIfNotSetAmbientCredentialsAreUsed:
				'与 ADC 兼容的 Google 凭证 JSON。如果未设置，则使用环境凭据。',
			add: '添加',
			addValueGuard: '添加 {{value}} 防护',
			addAccessControlAllowCredentialsTrueOnAllowedCorsResponses:
				'在允许的 CORS 响应上添加 `Access-Control-Allow-Credentials: true`。',
			addACelExpressionToStartAuthorizingRequests: '添加 CEL 表达式以开始授权请求。',
			addAGatewayBeforeAttachingRoutes: '在附加路由之前添加网关。',
			addAGatewayBeforeExposingTheUi: '在公开 UI 之前添加网关。',
			addAGatewayBeforeHttpTrafficCanBeServed: '请先添加网关，再提供 HTTP 流量服务。',
			addAListenerBeforeHttpOrTcpTrafficCanBeServed:
				'请先添加监听器，再提供 HTTP 或 TCP 流量服务。',
			addAListenerToStartMatchingTrafficOnThisPort: '添加监听器以开始匹配此端口上的流量。',
			addAModelBeforeLlmTrafficCanBeServed: '请先添加模型，再提供 LLM 流量服务。',
			addANameBeforeCreatingThisVirtualApiKey: '在创建此虚拟 API 密钥之前添加名称。',
			addANamedGatewayBeforeAttachingLlmMcpUiOrRoutes:
				'在附加 LLM、MCP、UI 或路由之前添加命名网关。',
			addAProviderWhenMultipleModelsShouldShareTheSameCredentialsOrUpstreamConnectionSettings:
				'当多个模型应共享相同的凭据或上游连接设置时添加提供商。',
			addARemotePolicyProcessorToInspectMcpRequestsAndResponses:
				'添加远程策略处理器来检查 MCP 请求和响应。',
			addATargetSoTheGatewayCanExposeMcpTraffic: '添加目标，以便网关可以公开 MCP 流量。',
			addAnMcpTargetBeforeToolsAreAvailable: '请先添加 MCP 目标，再使用工具。',
			addBackend: '添加后端',
			addBind: '添加绑定',
			addCacheMarkersToChatMessagesWhenSupportedByTheProvider:
				'当提供商支持时，将缓存标记添加到聊天消息中。',
			addCacheMarkersToSystemPromptsWhenSupportedByTheProvider:
				'当提供商支持时，将缓存标记添加到系统提示中。',
			addCacheMarkersToToolDefinitionsWhenSupportedByTheProvider:
				'当提供商支持时，将缓存标记添加到工具定义中。',
			addCurrentOrigin: '添加当前源',
			addDescriptor: '添加描述符',
			addEntry: '添加条目',
			addFallback: '添加后备',
			addFallbackGroup: '添加后备组',
			addGateway: '添加网关',
			addGuard: '添加防护规则',
			addHeader: '添加请求头',
			addHeaders: '添加请求头',
			addListener: '添加监听器',
			addMatch: '添加匹配条件',
			addModel: '添加模型',
			addModelCost: '添加模型成本',
			addModelUsingProvider: '使用提供商添加模型',
			addPattern: '添加模式',
			addProcessor: '添加处理器',
			addProvider: '添加提供商',
			addQuery: '添加查询条件',
			addRequestHeaders: '添加请求头',
			addResponseHeaders: '添加响应头',
			addRoute: '添加路由',
			addRule: '添加规则',
			addServer: '添加服务器',
			addTarget: '添加目标',
			addVirtualModel: '添加虚拟模型',
			additionalMetadataToSendToTheExternalProcessingServiceMapsToTheMetadataContextFi_d3ztkj:
				'要发送到外部处理服务的附加元数据。\n此配置映射到 `ProcessingRequest` 的 `metadata_context.filter_metadata` 字段，并支持动态 CEL 表达式。',
			additionalOauth2ScopesToRequestOpenidIsAlwaysIncluded:
				'要请求的其他 OAuth2 作用域。始终包含 `openid`。',
			additionalScopes: '附加作用域',
			additionalSubjectAlternativeNamesAcceptedForTheBackendCertificate:
				'后端证书接受的其他主题备用名称。',
			additionalTrustedOriginsAllowedToSendStateChangingRequests:
				'允许发送状态更改请求的其他受信任源。',
			addsGenAiPromptAndGenAiCompletionAttributesToAccessLogs:
				'添加 `gen_ai.prompt` 和 `gen_ai.completion` 属性以访问日志。',
			adminUiAddressInTheFormatIpPortLocalhostPortUnixPathToSocketOrOff:
				'管理 UI 地址，格式为 "ip:port"、"localhost:port"、"unix:/path/to/socket" 或 "off"',
			advanced: '高级',
			agentgateway: 'agentgateway',
			agentgatewayHome: 'agentgateway 首页',
			agentgatewayIsAGatewayThatCanRouteSecureAndObserveLlmMcpAndTraditionalApiTraffic_sbsjep:
				'agentgateway 是一种可路由、保护和观测 LLM、MCP 及传统 API 流量的网关。请选择要启用的一项或多项功能，然后继续。',
			agentgatewayRoutesRequestsByMatchingAnIncomingModelNameAndThenSendingItToTheConf_w5k7w1:
				'Agentgateway 通过匹配传入模型名称来路由请求，然后将其发送到配置的模型。传出模型可以从传入模型传递、进行转换或者是静态模型。',
			agwSkAutoGenerate: 'agw_sk_*****（自动生成）',
			ai: '人工智能',
			all: '全部',
			allValue: '全部{{value}}',
			allow: '允许',
			allowAllRequestHeaders: '允许所有请求头',
			allowCredentials: '允许凭据',
			allowModeOverride: '允许模式覆盖',
			allowPartialMessage: '允许部分消息',
			allowRequestsWhenTheRateLimitServiceIsUnavailable: '当速率限制服务不可用时允许请求。',
			allowTheRequestThroughWhenTheRateLimitServiceIsUnavailable:
				'当速率限制服务不可用时允许请求通过。',
			allowTheRequestThroughWhenTheWebhookGuardrailIsUnavailable:
				'当 webhook 防护规则不可用时允许请求通过。',
			allowTheRequestWhenTheAuthorizationServiceCannotMakeADecision:
				'当授权服务无法做出决定时允许请求。',
			allowTheRequestWhenThisCelExpressionIsTrue: '当此 CEL 表达式的计算结果为 `true` 时允许请求。',
			allowTrafficWhenTheProcessorIsUnavailable: '当处理器不可用时允许流量。',
			allowDenyFilterOverRequestHeadersMirroringExtAuthzEmptyAllowedForwardsEveryHeade_17m99zk:
				'用于筛选允许或禁止转发的请求头，与 `ext_authz` 的行为一致：`allowed` 为空时转发所有请求头和伪请求头（如 `:authority`、`:method`）；非空时仅转发列出的名称。`disallowed` 始终优先。普通请求头名称匹配不区分大小写，伪请求头则精确匹配。',
			allowedHeaders: '允许的请求头',
			allowedMethods: '允许的方法',
			allowedOrigins: '允许的来源',
			allowlistOnlyMethodsListedHereRunThroughThisProcessorAtTheConfiguredPhaseKeysMay_1ppmyo1:
				'允许列表：只有此处列出的方法会在指定阶段通过此处理器。键可以是精确值（`tools/call`）、前缀通配符（`tools/*`）、后缀通配符（`*/list`），也可以用 `*` 匹配所有方法。未匹配任何键的方法会绕过此处理器；匹配优先级请参阅 [`phase::resolve`]。',
			alpnProtocolsAdvertisedToDownstreamClients: '向下游客户端通告的 ALPN 协议。',
			alpnProtocolsToOfferToTheBackend: '提供给后端的 ALPN 协议。',
			always: '总是',
			alwaysPrefixExposedToolNamesWithTheTargetName: '始终使用目标名称作为公开工具名称的前缀。',
			ambient: '环境',
			anApiKeyToAttachToTheRequestIfUnsetThisWillBeAutomaticallyDetectedFromTheEnvironment:
				'附加到请求的 API 密钥。\n如果未设置，则会自动从环境中检测到。',
			anAwsStsSessionTagPassedToAssumeRoleForCostAttributionExactlyOneOfValueAndExpressionMustBeSet:
				'传递给 AWS STS `AssumeRole`、用于成本归因的会话标签。\n`value` 和 `expression` 必须且只能设置其中一项。',
			analytics: '分析',
			analyticsApiError: '分析 API 错误',
			analyzeApiVersion: '分析 API 版本',
			analyzeLlmTrafficByModelUserAndProvider: '按模型、用户和提供商分析 LLM 流量。',
			analyzeTextConfigurationForDetectingHarmfulContentCategoriesHateSelfHarmSexualVi_zwlwnr:
				'分析文本配置以检测有害内容类别\n（仇恨、自残、性、暴力）和黑名单匹配。',
			andForwardTheModelAsIs: '并按原样转发模型。',
			andHasNo: '并且没有',
			andSave: '并保存。',
			andSearchFor: '并搜索',
			andSendTo: '并发送至',
			andSetItTo: '并将其设置为',
			andSetTheAdvancedProxyUrl: '并设置高级代理 URL。',
			andStripThe: '并剥离',
			anotherVirtualKeyAlreadyUsesThisNameTheKeyWillStillBeCreatedWithAUniqueMetadataId:
				'另一个虚拟键已使用该名称。密钥仍将使用唯一的元数据 ID 创建。',
			anthropicV1Messages: 'Anthropic /v1/messages',
			anthropicV1MessagesCountTokens: 'Anthropic /v1/messages/count_tokens',
			anyStatus: '任何状态',
			apiKey: 'API 密钥',
			apiKeyValueToAccept: '要接受的 API 密钥值。',
			apiKeys: 'API 密钥',
			apiKeysThatAreAcceptedByThisPolicy: '本策略接受的 API 密钥。',
			apiVersionToUseDefault20240215Preview: '要使用的 API 版本（默认值：“2024-02-15-preview”）',
			apiVersionToUseDefault20240901: '要使用的 API 版本（默认值：“2024-09-01”）',
			apis: 'API',
			apply: '应用',
			applyAuthorization: '应用授权',
			applyChanges: '应用更改',
			applyCors: '应用 CORS',
			applyMcpCors: '应用 MCP CORS',
			applyPromptAndResponseGuardrailsToAllLlmModels: '将提示和响应防护规则应用于所有 LLM 模型。',
			applyPromptGuardsToStreamingResponsesAndRealtimeWebsocketMessages:
				'将提示防护应用于流式响应和实时 WebSocket 消息。',
			applyRegexBasedMaskingOrRejectionRules: '应用基于正则表达式的屏蔽或拒绝规则。',
			arguments: '参数',
			argumentsJson: '参数 JSON',
			argumentsMustBeAJsonObject: '参数必须是 JSON 对象。',
			asACustomModelThenTestFrom: '作为自定义模型，然后进行测试',
			ask: '询问',
			askATestQuestion: '输入一个测试问题…',
			atLeastOneMatchGroupMustMatchWithinAGroupEveryHeaderConditionMustMatch:
				'至少有一个匹配组必须匹配。在组内，每个请求头条件都必须匹配。',
			attachARouteToAGateway: '将路由附加到网关。',
			attachHttpAndTcpRoutesToTrafficGateways: '将 HTTP 和 TCP 路由附加到流量网关。',
			attributeKeysToRemoveFromTheEmittedSpanAttributesThisIsAppliedBeforeAttributesAr_1mndxj6:
				'要从导出的跨度属性中删除的属性键。\n\n此操作在计算或添加 `attributes` 之前执行，可用于移除默认属性或避免重复。',
			attributes: '属性',
			audienceForTheTokenIfNotSetTheDestinationHostWillBeUsed:
				'令牌的受众。如果未设置，将使用目标主机。',
			audiences: '受众',
			audioIn: '音频输入',
			audioOut: '音频输出',
			auth: '身份验证',
			authConfiguresAuthenticationWhenConnectingToTheLlmProvider:
				'`auth` 用于配置连接 LLM 提供商时的身份验证。',
			authenticateBrowserRequestsWithOidcAuthorizationCodeFlow:
				'使用 OIDC 授权码流程验证浏览器请求。',
			authenticateIncomingRequestsWithApiKeys: '使用 API 密钥验证传入请求。',
			authenticateIncomingRequestsWithBasicAuthCredentialsFromAnHtpasswdUserDatabase:
				'使用 htpasswd 用户数据库中的基本身份验证凭据对传入请求进行身份验证。',
			authenticateIncomingRequestsWithJwtBearerTokens: '使用 JWT Bearer 令牌验证入站请求。',
			authenticateMcpClients: '验证 MCP 客户端。',
			authenticateToAzureServices: '对 Azure 服务进行身份验证。',
			authenticateToGitHubCopilot: '向 GitHub Copilot 进行身份验证。',
			authenticateToGoogleCloudServices: '向 Google Cloud 服务进行身份验证。',
			authentication: '身份验证',
			authenticationCredentialsSentToTheBackend: '身份验证凭据发送到后端。',
			authenticationCredentialsSentToThisBackend: '发送到此后端的身份验证凭据。',
			authorization: '授权',
			authorizationBehavior: '授权行为',
			authorizationConfiguresHttpAuthorizationRulesForRequestsToThisModel:
				'`authorization` 用于配置此模型请求的 HTTP 授权规则。',
			authorizationEndpoint: '授权端点',
			authorizationEndpointUsedToStartTheBrowserLoginFlow: '用于启动浏览器登录流程的授权端点。',
			authorizationHeader: 'Authorization 请求头',
			authorizationResponseHeadersToCopyIntoTheBackendRequest: '授权响应请求头复制到后端请求中。',
			authorizationRulesForIncomingHttpRequests: '传入 HTTP 请求的授权规则。',
			authorizationRulesForMcpRequests: 'MCP 请求的授权规则。',
			authorizeIncomingRequestsAfterThisBackendIsSelected: '选择此后端后授权传入请求。',
			authorizeIncomingRequestsByCallingAnExternalAuthorizationServiceAfterThisBackendIsSelected:
				'选择此后端后，通过调用外部授权服务来授权传入请求。',
			authorizeIncomingRequestsByCallingAnExternalAuthorizationService:
				'通过调用外部授权服务来授权传入请求。',
			automaticallyChooseBasedOnTheEnableIpv6SettingWhenIpv6IsEnabledThisBehavesLikeV4_nsr4ii:
				'根据 `enable_ipv6` 设置自动选择。启用 IPv6 时，其行为类似于 `V4Preferred`；否则使用 `V4Only`。',
			automaticallyDetectAuthenticationMethodBasedOnEnvironmentUsesWorkloadIdentityOnK_y198si:
				'根据环境自动检测认证方法。\n在 K8s 上使用工作负载身份、Azure VM 上的托管身份或本地开发人员工具。',
			awsAccessKeyId: 'AWS 访问密钥 ID',
			awsCredentials: 'AWS 凭证',
			awsIamRoleArnToAssume: '要代入的 AWS IAM 角色 ARN。',
			awsRegion: 'AWS 区域',
			awsRegionWhereTheGuardrailIsDeployed: '部署防护规则的 AWS 区域',
			awsSecretAccessKey: 'AWS 秘密访问密钥',
			awsSigV4SigningServiceNameForExampleBedrockBedrockAgentcoreOrExecuteApi:
				'AWS SigV4 签名服务名称（例如“bedrock”、“bedrock-agentcore”或“execute-api”）。',
			azureAiFoundryProjectEndpointResourceNameServicesAiAzureComRequiresProjectNameTo_bpdjpb:
				'Azure AI Foundry 项目端点：`{resourceName}.services.ai.azure.com`。\n需要通过 `project_name` 构建 `/api/projects/{project}/openai/v1/...` 形式的路径。',
			azureApiVersion: 'Azure API 版本',
			azureContentSafety: 'Azure 内容安全',
			azureCredentials: 'Azure 凭据',
			azureOpenAiServiceEndpointResourceNameOpenaiAzureCom:
				'Azure OpenAI 服务端点：`{resourceName}.openai.azure.com`',
			azureProjectName: 'Azure 项目名称',
			azureResourceName: 'Azure 资源名称',
			azureResourceType: 'Azure 资源类型',
			backToHome: '返回首页',
			backend: '后端',
			backendHostUrlForGuardrailChecks: '用于防护规则检查的后端主机 URL。',
			backendPolicies: '后端策略',
			backendPoliciesForAwsAuthenticationOptionalDefaultsToImplicitAwsAuth:
				'AWS 身份验证的后端策略（可选，默认为隐式 AWS 身份验证）',
			backendPoliciesForAzureAuthenticationOptionalDefaultsToImplicitAzureAuth:
				'Azure 身份验证的后端策略（可选，默认为隐式 Azure 身份验证）',
			backendPoliciesForGcpAuthenticationOptionalDefaultsToImplicitGcpAuth:
				'GCP 身份验证的后端策略（可选，默认为隐式 GCP 身份验证）',
			backendPoliciesPreserved: '保留后端策略',
			backendPoliciesUsedWhenCallingTheModerationProvider: '调用审核提供商时使用的后端策略。',
			backendPoliciesUsedWhenConnectingToTheService: '连接到服务时使用的后端策略。',
			backendReference: '后端引用',
			backendThatReceivesGuardrailWebhookRequests: '接收防护规则 Webhook 请求的后端。',
			backendThatReceivesMirroredRequestCopies: '接收镜像请求副本的后端。',
			backendYaml: '后端 YAML',
			backends: '后端',
			backends_i9thuc: '后端',
			backendsDefinesExplicitBackendsThatCanBeReferencedByRoutesAndPoliciesTypicallyIn_1a5i8ts:
				'`backends` 定义可由路由和策略引用的显式后端。\n路由和策略通常使用内联后端；此配置可让多个配置复用同一后端。',
			backendTunnelConfiguresTunnelingWhenConnectingToTheLlmProvider:
				'`backendTunnel` 用于配置连接 LLM 提供商时使用的隧道。',
			baseCostCatalogRefreshedValueModelsFromValueProviders:
				'基础成本目录已刷新：{{value}} 个模型，涉及 {{value}} 个提供商。',
			baseUrl: '基础 URL',
			baseUrlForTheUpstreamProviderExpandsToHostOverridePathPrefixAndTlsForHttpsUrls:
				'上游提供商的基础 URL。HTTPS URL 会展开为 `hostOverride`、`pathPrefix` 和 `tls`。',
			basicAuth: '基本身份验证',
			bearerToken: 'Bearer 令牌',
			bedrockGuardrails: 'Bedrock 防护规则',
			behaviorWhenOneOrMoreMcpTargetsFailToInitializeOrFailDuringFanoutDefaultsToFailClosed:
				'当一个或多个 MCP 目标无法初始化或在扇出期间失败时的行为。\n默认为 `failClosed`。',
			behaviorWhenTheAuthorizationServiceIsUnavailableOrReturnsAnError:
				'授权服务不可用或返回错误时的行为。',
			behaviorWhenTheExternalProcessingServiceIsUnavailableOrReturnsAnError:
				'外部处理服务不可用或返回错误时的行为。',
			behaviorWhenTheProcessorIsUnavailableOrReturnsAnError: '处理器不可用或返回错误时的行为。',
			behaviorWhenTheRemoteRateLimitServiceIsUnavailableOrReturnsAnErrorDefaultsToFail_1bpcema:
				'远程速率限制服务不可用或返回错误时的处理方式。\n默认为 `failClosed`；服务失败时拒绝请求并返回 500 状态码。',
			behaviorWhenTheWebhookIsUnreachableOrReturnsAnErrorDefaultsToFailClosed:
				'Webhook 无法访问或返回错误时的行为。\n默认为 `failClosed`。',
			bind: '绑定',
			bindPort: '绑定端口',
			bindPortThisListenerIsAttachedTo: '绑定此监听器所附加的端口。',
			bindThisSurfaceOnItsOwnListenerPort: '将此功能入口绑定到独立的监听端口。',
			bindsDefinesTheLowLevelApiForConfiguringTheProxyEachBindRepresentsASinglePortThe_96e01v:
				'`binds` 定义用于配置代理的底层 API。\n每个绑定代表代理监听的一个端口，以及该端口对应的完整配置（监听器、路由和后端）。\n此字段已弃用，建议改用 `gateways` 和 `routes`。',
			blocklistNamesToCheckAgainst: '要检查的阻止列表名称',
			blocklists: '阻止列表',
			bodyExpression: '正文表达式',
			bodyOptions: '正文选项',
			breakdown: '明细',
			browserAccessIsNotAllowed: '不允许浏览器访问',
			browserBasedOidcAuthenticationPolicyExplicitModeIsStillOidcItSuppliesProviderMet_1en29xp:
				'基于浏览器的 OIDC 身份验证策略。\n\n显式模式仍使用 OIDC，但会手动提供身份提供商元数据，而不使用发现机制。\n未经身份验证的非回调请求始终会重定向到身份提供商的登录流程。需要在身份验证失败时直接返回错误的路由，应改用其他身份验证策略。',
			bufferAndSendTheBodyUpToTheConfiguredLimit: '缓冲并发送正文，直至达到配置的限制。',
			bufferAndSendTheFullBodyToTheExternalProcessingService:
				'缓冲并将完整正文发送到外部处理服务。',
			bufferIncomingRequestBodiesBeforeForwarding: '在转发之前缓冲传入的请求正文。',
			bufferRequestAndResponseBodies: '缓冲请求和响应正文。',
			bufferTheFullBodyBeforeSendingItToTheProcessor: '在将整个正文发送到处理器之前对其进行缓冲。',
			bufferUpstreamResponseBodiesBeforeSendingThemToTheClient:
				'在将上游响应正文发送到客户端之前对其进行缓冲。',
			buffered: '已缓冲',
			bufferedPartial: '部分缓冲',
			builtInDetectors: '内置检测器',
			builtInPatternName: '内置模式名称。',
			caSin: 'CA SIN',
			cacheAuthorizationResultsUsingCelExpressionsAsTheCacheKeyWarningTheSafetyOfThisF_1hqhg1t:
				'使用 CEL 表达式作为缓存键来缓存授权结果。\n警告：此功能是否安全，取决于缓存键能否准确包含服务器决策所依赖的字段。例如，如果服务器根据请求头 A 返回不同结果，但缓存键只包含请求头 B，用户可能会错误命中缓存。',
			cacheRead: '缓存读取',
			cacheWrite: '缓存写入',
			callAWebhookToEvaluateThePrompt: '调用 Webhook 来评估提示。',
			callAWebhookToEvaluateTheResponse: '调用 Webhook 来评估响应。',
			callAnHttpAuthorizationService: '调用 HTTP 授权服务。',
			callTheAuthorizationServiceUsingHttp: '使用 HTTP 调用授权服务。',
			callTheAuthorizationServiceUsingTheGRpcAuthorizationProtocol:
				'使用 gRPC 授权协议调用授权服务。',
			callTool: '调用工具',
			callingValueMcpValue: '正在调用 {{value}} 个 MCP 工具',
			calls: '调用',
			canBeAWildcard: '可以是通配符',
			canadianSocialInsuranceNumberPattern: '加拿大社会保险号码模式。',
			cancel: '取消',
			catalogSources: '目录来源',
			celAuthorizationForDownstreamNetworkConnections: '用于下游网络连接的 CEL 授权。',
			celAuthorizationRulesForDownstreamNetworkConnections: '用于下游网络连接的 CEL 授权规则。',
			celAuthorizationRulesForMcpToolsPromptsAndResources: 'MCP 工具、提示和资源的 CEL 授权规则。',
			celAuthorizationRulesToEvaluateForARequest: '用于评估请求的 CEL 授权规则。',
			celError: 'CEL 错误',
			celExpression: 'CEL 表达式',
			celExpressionEvaluatedAgainstEachRequestToProduceTheTagValueForExampleJwtSubOrRe_1jdxqht:
				'针对每个请求计算 CEL 表达式以生成标签值，例如 `jwt.sub` 或\n`request.headers["x-app"]`。如果表达式在请求时未生成有效的标签值，则请求会被拒绝。',
			celExpressionEvaluatedAgainstEachResponseToDecideWhetherToRetryAResponseIsRetrie_qrheq5:
				'针对每个响应计算此 CEL 表达式，以决定是否重试。当响应状态码在 `codes` 中，或表达式结果为 `true` 时触发重试。',
			celExpressionEvaluatedAgainstTheRequestBeforeAnyAttemptWhenFalseRetriesAreDisabl_sapdox:
				'在发起任何请求之前，根据当前请求计算此 CEL 表达式。结果为 `false` 时禁用重试，只执行首次请求。例如：`request.method == "GET"`。\n重试需要在内存中缓冲请求正文以便重新发送。若已知请求无法重试（例如流式传输或 WebSocket），可通过此表达式避免相应开销。',
			celExpressionThatComputesARedirectUrlWhenAuthorizationFailsWhenTheAuthorizationS_vhwf5d:
				'授权失败时用于计算重定向 URL 的 CEL 表达式。\n授权服务返回未授权结果时，网关会重定向到该 URL，而不是直接返回错误。',
			celExpressionThatComputesAReplacementBody: '计算替换体的 CEL 表达式。',
			celExpressionThatComputesTheAuthorizationRequestBodyStringsAndBytesAreUsedDirect_1etgvrf:
				'用于计算授权请求正文的 CEL 表达式。\n字符串和字节会直接使用，其他类型的值会编码为 JSON。\n设置后，网关会使用表达式结果，而不再转发传入的请求正文。',
			celExpressionThatComputesTheAuthorizationRequestPath: '计算授权请求路径的 CEL 表达式。',
			celExpressionThatComputesTheResponseBody: '计算响应正文的 CEL 表达式。',
			celExpressionThatDecidesWhetherARequestIsExportedOverOtlp:
				'决定是否通过 OTLP 导出请求的 CEL 表达式。',
			celExpressionThatDecidesWhetherARequestIsLogged: '决定是否记录请求的 CEL 表达式。',
			celExpressionThatReturnsHowLongCachedAuthorizationResultsAreReusedTheExpressionI_kb9kvi:
				'返回授权结果缓存复用期限的 CEL 表达式。\n将授权响应应用到请求后再计算此表达式；结果必须是时长或时间戳。',
			celExpressionUsedToComputeTheDescriptorEntryValue: 'CEL 表达式用于计算描述符条目值。',
			celExpressionUsedToPopulateTheAgentgatewayGroupRequestLogAttribute:
				'用于填充 `agentgateway.group` 请求日志属性的 CEL 表达式。',
			celExpressionUsedToPopulateTheAgentgatewayUserRequestLogAttribute:
				'用于填充 `agentgateway.user` 请求日志属性的 CEL 表达式。',
			celExpressionUsedToPopulateTheAgentgatewayGroupRequestLogAttribute_n5btzx:
				'用于填充 agentgateway.group 请求日志属性的 CEL 表达式。',
			celExpressionUsedToPopulateTheAgentgatewayUserRequestLogAttribute_r3ojz7:
				'用于填充 agentgateway.user 请求日志属性的 CEL 表达式。',
			celExpressionWhereTrueMarksTheBackendResponseAsUnhealthyWhenUnsetAny5xxResponseO_19ajrab:
				'用于判断后端响应是否不健康的 CEL 表达式；结果为 `true` 时标记为不健康。\n未设置时，任何 5xx 响应或连接失败都视为不健康。',
			celExpressionsEvaluatedPerRequestAndSentToTheProcessorAsMetadata:
				'CEL 表达式根据请求进行评估并作为元数据发送到处理器。',
			celExpressionsSentAsAttributesToTheProcessor: 'CEL 表达式作为属性发送到处理器。',
			celExpressionsThatMakeUpTheCacheKeyEmptyKeysAreAcceptedButDoNotProduceCacheHits:
				'组成缓存键的 CEL 表达式。接受空键，但不会产生缓存命中。',
			celPlayground: 'CEL 演练场',
			celReference: 'CEL 参考',
			certificate: '证书',
			certificateSourceModeStaticModeUsesCertKeyAsTheLeafCertificateDynamicCaModeUsesC_1dwhpmp:
				'证书来源模式。静态模式将 `cert`/`key` 用作叶证书；动态 CA 模式则将其用作证书颁发机构，按需签发 SNI 叶证书。',
			chatPlayground: '聊天演练场',
			chooseFailureBehaviorAndWhichRequestResponsePhasesAreSent:
				'选择失败行为以及发送哪些请求/响应阶段。',
			chooseHowLlmTrafficIsExposed: '选择如何公开 LLM 流量。',
			chooseHowMcpIsExposed: '选择 MCP 的暴露方式。',
			chooseHowTheGatewayBehavesWhenARequestHasNoTokenOrATokenCannotBeVerified:
				'选择当请求没有令牌或令牌无法验证时网关的行为方式。',
			chooseProtocolAndFailOpenFailClosedBehavior: '选择协议和故障开放/故障关闭行为。',
			chooseSessionToolPrefixAndFailureBehavior: '选择会话、工具前缀和失败行为。',
			cipherSuitesAllowedForDownstreamTls: '密码套件允许下游 TLS。',
			claimRequirementsToEnforceAfterTheTokenSignatureIsVerified:
				'验证令牌签名后强制执行的声明要求。',
			claimsThatMustBePresentInTheTokenBeforeValidationOnlyExpNbfAudIssSubAreEnforcedO_ux04jc:
				'验证前令牌中必须存在的声明。\n仅强制检查 `exp`、`nbf`、`aud`、`iss` 和 `sub`；其他声明（包括 `iat` 和 `jti`）会被忽略。\n默认为 `exp`。使用空列表表示不要求任何声明。',
			claudeCode: 'Claude Code',
			claudeDesktop: 'Claude Desktop',
			claudeSubscriptionKeyDetected: '检测到 Claude 订阅密钥',
			clear: '清除',
			clearAuthorization: '清除授权',
			clearEnvironment: '清除环境',
			clearFilters: '清除筛选条件',
			client: '客户端',
			clientAuthenticationUsedWhenCallingTheTokenEndpoint: '调用令牌端点时使用的客户端身份验证。',
			clientAuthenticationUsedWhenCallingTheTokenEndpointWhenUnsetNoClientAuthenticationFieldsAreSent:
				'调用令牌端点时使用的客户端身份验证。\n未设置时，不会发送任何客户端身份验证字段。',
			clientCertificateFileToPresentToTheBackend: '要呈现给后端的客户端证书文件。',
			clientId: '客户端 ID',
			clientIdOptional: '客户端 ID（可选）',
			clientRequested: '客户端请求',
			clientSecret: '客户端密钥',
			clientSecretBasic: '客户端密钥 Basic',
			clientSecretPost: '客户端密钥 POST',
			clientSetup: '客户端设置',
			close: '关闭',
			codexCli: 'Codex CLI',
			cohereV2RerankDocumentReranking: 'Cohere /v2/rerank（文档重新排名）',
			commaSeparatedListOfAdditionalSpiffeTrustDomainsAcceptedOnInboundHboneConnection_ib2a3q:
				'入站 HBONE 连接接受的其他 SPIFFE 信任域（逗号分隔列表）。本地 `trust_domain` 始终隐式包含。',
			commaSeparatedNames: '逗号分隔的名称。',
			command: '命令',
			condition: '条件',
			conditionMustEvaluateToTrueForThisPolicyToExecuteIfUnsetThePolicyIsTheFallback:
				'仅当条件的计算结果为 `true` 时才执行此策略。未设置条件时，该策略作为回退策略。',
			conditional: '条件式',
			conditionalEnablesConditionBasedSelectionOfTheTargetModelEachConditionIsEvaluate_12cw48o:
				'条件选择支持基于条件选择目标模型。每个条件按顺序评估，直到找到最佳匹配。',
			conditionalPolicyEntriesAnEntryWithoutAConditionMustBeTheFinalFallback:
				'有条件的策略条目。没有条件的条目必须是最终的后备。',
			conditionalTargets: '有条件的目标',
			configDumpUnavailable: '配置转储不可用',
			configuration: '配置',
			configurationApiUnavailable: '配置 API 不可用',
			configurationForAwsBedrockGuardrailsIntegration: 'AWS Bedrock Guardrails 集成的配置。',
			configurationForAzureContentSafetyIntegrationUsesTheAzureAiContentSafetyApisToDe_13mxkr6:
				'Azure 内容安全集成配置。\n\n使用 Azure AI 内容安全 API 检测有害内容和越狱尝试。所有已启用的功能共用同一端点和身份验证配置。',
			configurationForDynamicTracingPolicy: '动态跟踪策略配置',
			configurationForGoogleCloudModelArmorIntegration: 'Google Cloud Model Armor 集成的配置。',
			configurationForStatefulSessionManagement: '有状态会话管理的配置',
			configurationForTheAnalyzeTextApi: '分析文本 API 的配置。',
			configurationForTheDetectJailbreakApi: '检测越狱 API 的配置。',
			configurationIsManagedByXdsThisViewReflectsTheActiveRuntimeDumpEditingIsDisabled:
				'配置由 XDS 管理。此视图反映当前运行时转储，无法编辑。',
			configurationMustBeAYamlObject: '配置必须是 YAML 对象。',
			configurationSaved: '配置已保存',
			configurationValidationFailedValue: '配置验证失败：{{value}}',
			configureAModelFirst: '请先配置模型',
			configureBindPortsAndListenersForGenericHttpAndTcpTraffic:
				'为通用 HTTP 和 TCP 流量配置绑定端口和监听器。',
			configureMcpTargetsServedByTheGateway: '配置网关服务的 MCP 目标。',
			configureNamedGatewayListenersThatLlmMcpUiAndRoutesCanAttachTo:
				'配置 LLM、MCP、UI 和路由可以附加到的命名网关监听器。',
			configureOpenCodeWithAnOpenAiCompatibleGatewayProvider:
				'为 OpenCode 配置兼容 OpenAI 的网关提供商。',
			configureTheAuthorizationRequestAndResponseMetadataExtraction:
				'配置授权请求和响应元数据提取。',
			configureTheJwksSourceUsedToVerifyTokenSignatures: '配置用于验证令牌签名的 JWKS 源。',
			configureThirdPartyInference: '配置第三方推理',
			configureTopLevelBehaviorForMcpGatewayTraffic: '配置 MCP 网关流量的顶级行为。',
			configureTopLevelBehaviorThatAppliesBeforeModelSpecificRouting:
				'配置在特定于模型的路由之前应用的顶级行为。',
			configureUiPolicies: '配置 UI 策略',
			configureVsCodeCopilotBusinessOrEnterpriseToUseTheGatewayProxy:
				'配置 VS Code Copilot Business 或 Enterprise 以使用网关代理。',
			configureWhereBrowserLoginStartsAndHowReturnedIdTokensAreValidated:
				'配置浏览器登录的起始位置以及如何验证返回的 ID 令牌。',
			configured: '已配置',
			connection: '连接设置',
			consecutiveFailures: '连续失败',
			consecutiveUnhealthyResponsesRequiredBeforeEviction: '驱逐前所需的连续不健康响应次数。',
			context: '上下文',
			contextExtensionsAreStaticValuesMetadataValuesAreCelExpressions:
				'上下文扩展是静态值；元数据值是 CEL 表达式。',
			continue: '继续',
			continueToValue: '继续前往 {{value}}',
			continueTheRequestWhenTheExternalProcessingServiceFails: '当外部处理服务失败时继续请求。',
			continueWhenTheWebhookIsUnavailableOrErrors: '当 Webhook 不可用或出现错误时继续。',
			controlWhetherMcpRequestsMustPresentAValidJwt: '控制 MCP 请求是否必须提供有效的 JWT。',
			controlsHowAnEndpointPickerSelectedDestinationIsUsed: '控制如何使用端点选择器选择的目标。',
			controlsWhetherMcpRequestsMustIncludeAValidJwt: '控制 MCP 请求是否必须包含有效的 JWT。',
			controlsWhetherRequestsMustIncludeAJwtAndHowValidationFailuresAreHandled:
				'控制请求是否必须包含 JWT 以及如何处理验证失败。',
			controlsWhetherRequestsMustIncludeAValidApiKey: '控制请求是否必须包含有效的 API 密钥。',
			controlsWhetherRequestsMustIncludeValidBasicAuthCredentials:
				'控制请求是否必须包含有效的基本身份验证凭据。',
			controlsWhichIpAddressFamiliesTheDnsResolverWillQueryForUpstreamBackendConnectio_1w5pwyi:
				'控制 DNS 解析器在建立上游（后端）连接时查询哪些 IP 地址族。\n\n底层映射到 `hickory_resolver` 的 `LookupIpStrategy`。\n\n可通过 `DNS_LOOKUP_FAMILY` 环境变量或配置文件中的 `dns.lookupFamily` 字段设置。\n\n参见：<https://www.envoyproxy.io/docs/envoy/latest/api-v3/config/cluster/v3/cluster.proto#enum-config-cluster-v3-cluster-dnslookupfamily>',
			controlsWhichIpAddressFamiliesTheDnsResolverWillQueryForUpstreamConnectionsAccep_h7l2v:
				'控制 DNS 解析器为上游连接查询哪些 IP 地址族。\n可选值：`All`、`Auto`、`V4Preferred`、`V4Only`、`V6Only`。\n默认为 `Auto`（`enableIpv6` 为 `false` 时仅查询 IPv4，为 `true` 时同时查询 IPv4 和 IPv6）。',
			controlsWhichRequestAndResponsePartsAreSentToTheExternalProcessingService:
				'控制将哪些请求和响应部分发送到外部处理服务。',
			conversation: '对话',
			cookieNameContainingTheCredential: '包含凭证的 Cookie 名称。',
			copy: '复制',
			copyKey: '复制键',
			copyToClipboard: '复制到剪贴板',
			cost: '成本',
			costCatalogRefreshFailed: '成本目录刷新失败',
			costDeterminesTheOptionalExpressionToDetermineTheCostOfTheRequestIfUnsetTypeRequ_z12ji8:
				'`cost` 指定用于计算请求成本的可选表达式。\n未设置时，`requests` 类型默认为 `1`，`tokens` 类型默认为 `llm.totalTokens`。\n表达式计算失败时跳过该描述符。`requests` 类型的成本在处理请求时计算，`tokens` 类型则在请求完成后计算。',
			costExpression: '成本表达',
			costRefreshFailed: '成本刷新失败',
			costs: '成本',
			countEachRequestAsOneUnit: '将每个请求视为一个单元。',
			countLlmTokenUsage: '计算 LLM 令牌的使用情况。',
			createAKeySoCallersCanAuthenticateWithoutExposingProviderCredentials:
				'创建密钥，以便调用者可以在不暴露提供商凭据的情况下进行身份验证。',
			createAModelBeforeTestingChatTraffic: '在测试聊天流量之前创建模型。',
			createAnLlmModelBeforeWiringClientsToTheGateway: '在将客户端连接到网关之前创建 LLM 模型。',
			createAnMcpServerBeforeTestingMcpTraffic: '在测试 MCP 流量之前创建 MCP 服务器。',
			createTheFirstModelToMakeLlmTrafficAvailableThroughTheGateway:
				'创建第一个模型以使 LLM 流量可通过网关使用。',
			createTheLlmConfigurationSectionSoModelsProvidersKeysGuardrailsLogsAndPlayground_197f4qj:
				'创建 LLM 配置，以便管理模型、提供商、密钥、防护规则、日志和演练场工具。',
			createTheMcpConfigurationSectionSoServersAndMcpPlaygroundToolsCanBeConfigured:
				'创建 MCP 配置部分，以便可以配置服务器和 MCP Playground 工具。',
			createTheTrafficConfigurationSectionSoHttpGatewaysRoutesBackendsAndPoliciesCanBeConfigured:
				'创建流量配置部分，以便可以配置 HTTP 网关、路由、后端和策略。',
			createThis: '创建 ',
			credentialLocation: '凭据位置',
			credentials: '凭据',
			creditCard: '信用卡',
			creditCardNumberPattern: '信用卡号码模式。',
			csv: 'CSV',
			currentPolicyYaml: '当前策略 YAML',
			currentTopLevelPolicyYaml: '当前顶级策略 YAML',
			cursorSettings: 'Cursor 设置',
			custom: '自定义',
			customAuthDetected: '检测到自定义身份验证',
			customCelFunctionsAvailableToAllCelExpressionsTheseCanDefineReUsableSnippetsThat_1mw84ev:
				'自定义 CEL 函数可供所有 CEL 表达式调用，用于定义可在多个表达式中复用的逻辑片段。\n请将一个或多个函数定义配置为块字符串，例如：\n`customFunctions: |`\n`  isInternal() { request.headers["x-env"] == "internal" }`\n`  this.joined(prefix, parts...) { prefix + this + parts.join("") }`',
			customCosts: '自定义成本',
			customHeaderLocation: '自定义请求头位置',
			customProvider: '自定义提供商',
			customRegex: '自定义正则表达式',
			customRegexPatterns: '自定义正则表达式模式',
			customSessionNameRoleSessionNameForCloudTrailAndCostUsageReportAttributionMax64C_1kyvvc8:
				'用于 CloudTrail 和成本与使用情况报告归因的自定义会话名称（`RoleSessionName`）。\n最多 64 个字符，匹配 `[\\w+=,.@-]`。如果未设置，AWS SDK 会生成一个随机会话名称。',
			customize: '自定义',
			databaseOnlyFieldsToAddComputedFromCelExpressions:
				'要添加的仅数据库字段，根据 CEL 表达式计算。',
			databaseSpecificAccessLogSettings: '特定于数据库的访问日志设置。',
			decodeValidApiKeysForLaterPolicyUseWarningThisAllowsRequestsWithMissingOrInvalidApiKeys:
				'解码有效的 API 密钥，供后续策略使用。\n警告：缺少 API 密钥或密钥无效的请求仍可继续处理。',
			decodeValidJwtsForLaterPolicyUseWarningThisAllowsRequestsWithMissingOrInvalidJwts:
				'解码有效的 JWT，供后续策略使用。\n警告：缺少 JWT 或 JWT 无效的请求仍可继续处理。',
			dedicatedPort: '专用端口',
			default: '默认',
			defaultRequestBodyValuesAddedOnlyWhenTheClientDidNotProvideThem:
				'仅当客户端未提供默认请求正文值时才添加它们。',
			defaultRequestValues: '默认请求值',
			defaultAuthorizationBearerToken: '默认：Authorization: Bearer Token',
			defaultsAllowsSettingDefaultValuesForTheRequestIfTheseAreNotPresentInTheRequestB_1hv3k3o:
				'`defaults` 用于设置请求的默认值。仅当请求正文中不存在相应字段时，才会写入这些值。\n如需无条件覆盖已有字段，请使用 `overrides`。',
			defaultsDefinesProviderLevelPolicyDefaultsModelLevelPolicyFieldsOverrideThese:
				'`defaults` 定义提供商级策略的默认值，模型级策略字段会覆盖这些值。',
			defaultsYaml: '默认 YAML',
			defineReusableProviderCredentialsAndConnectionSettingsForModels:
				'为模型定义可重用的提供商凭据和连接设置。',
			definesHowTheProxyBehavesWhenAWebhookGuardrailIsUnreachableOrReturnsAnErrorDefau_1h8u7be:
				'定义 Webhook 防护规则无法访问或返回错误时代理的行为。\n\n默认为 `failClosed`。失败关闭时，错误会向上传播并拒绝 LLM 请求；失败放行时，即使 Webhook 失败也允许请求通过。',
			definesHowTheProxyBehavesWhenTheRemoteRateLimitServiceIsUnavailableOrReturnsAnEr_15rgoat:
				'定义远程速率限制服务不可用或返回错误时代理的行为。\n\n默认为 `failClosed`。失败关闭时，服务不可用会返回 500 内部服务器错误；失败放行时，即使服务失败也允许请求通过。\n\n配置文件同时接受驼峰命名（`failOpen`、`failClosed`）和帕斯卡命名（`FailOpen`、`FailClosed`）。',
			delayBetweenRetryAttempts: '重试尝试之间的延迟。',
			delete: '删除',
			deleteValue: '删除 {{value}}',
			deleteValue_pkbukw: '删除 {{value}}？',
			deleteBind: '删除绑定',
			deleteGateway: '删除网关',
			deleteGuardrail: '删除防护规则？',
			deleteKey: '删除密钥',
			deleteListener: '删除监听器',
			deleteMcpServer: '删除 MCP 服务器？',
			deleteModel: '删除模型',
			deletePolicy: '删除策略',
			deleteProvider: '删除提供商',
			deleteProvider_1j44lo: '删除提供商？',
			deleteRoute: '删除路由',
			deleteRoute_akv0fs: '删除路由？',
			deleteServer: '删除服务器',
			deleteVirtualApiKey: '删除虚拟 API 密钥？',
			deny: '拒绝',
			denyRequestsWhenTheRateLimitServiceIsUnavailable: '当速率限制服务不可用时拒绝请求。',
			denyStatus: '拒绝状态',
			denyTheRequestWhenTheAuthorizationServiceCannotMakeADecision:
				'当授权服务无法做出决定时拒绝请求。',
			denyTheRequestWhenThisCelExpressionIsTrue: '当此 CEL 表达式的计算结果为 `true` 时拒绝请求。',
			denyTheRequestWithA500StatusWhenTheRateLimitServiceIsUnavailableDefault:
				'当速率限制服务不可用时（默认），拒绝状态为 500 的请求。',
			denyTheRequestWithTheConfiguredHttpStatusCode: '使用配置的 HTTP 状态代码拒绝请求。',
			denyWithStatus: '拒绝状态',
			descriptor: '描述符',
			descriptorEntriesSentToTheRemoteServiceValuesAreCelExpressionsEvaluatedFromTheRequest:
				'发送到远程服务的描述符条目。值是根据请求求值的 CEL 表达式。',
			descriptorEntryKeySentToTheRemoteRateLimitService: '发送到远程速率限制服务的描述符输入键。',
			descriptorKeyValueEntriesValuesAreCelExpressionsEvaluatedFromTheRequest:
				'描述符键/值条目。值是根据请求求值的 CEL 表达式。',
			descriptors: '描述符',
			descriptorsSentToTheRemoteRateLimitService: '发送到远程速率限制服务的描述符。',
			detectCommonSensitiveDataTypesWithBuiltInRegexRules:
				'使用内置正则表达式规则检测常见敏感数据类型。',
			detectJailbreakAttempts: '检测越狱尝试',
			detectTextJailbreakConfigurationForDetectingJailbreakAttemptsOnlyApplicableToRequestGuards:
				'文本越狱检测配置。\n仅适用于请求防护规则。',
			detectUnhealthyBackendResponsesAndTemporarilyRemoveUnhealthyEndpoints:
				'检测不健康的后端响应并暂时删除不健康的端点。',
			detectedLegacyBindsConfig: '检测到旧版绑定配置',
			detectingConfigurationMode: '检测配置模式',
			detectingTrafficConfigurationMode: '正在检测流量配置模式',
			developer: '开发者',
			disable: '禁用',
			disableApiKeyPolicy: '禁用 API 密钥策略',
			disableApiKeyPolicy_ckgvai: '禁用 API 密钥策略',
			disableApiKeyPolicy_9229n3: '禁用 API 密钥策略？',
			disableVirtualApiKeyValidationRequestsWillNoLongerBeValidatedAgainstVirtualApiKeys:
				'禁用虚拟 API 密钥验证？将不再根据虚拟 API 密钥验证请求。',
			disabled: '已禁用',
			discardChanges: '放弃更改',
			discardUnsavedChanges: '放弃未保存的更改？',
			discovery: '发现',
			discoveryOverride: '发现覆盖',
			dnsResolverSettings: 'DNS 解析器设置。',
			doNotApplyPromptGuardsToStreamingResponsesOrRealtimeWebsocketMessages:
				'不要将提示防护应用于流式响应或实时 WebSocket 消息。',
			doNotExposeTheUiOnATrafficGateway: '不要在流量网关上公开 UI。',
			doNotOpenASocketTheBindIsRegisteredForRoutingOnlyAndIsReachableViaInProcessReEnt_9sz4lu:
				'不打开套接字。此绑定仅注册用于路由，可通过进程内重新进入访问（例如，另一个监听器将 CONNECT 流量重定向到此绑定）。',
			doNotPreserveMcpSessionStateBetweenRequests: '不要在请求之间保留 MCP 会话状态。',
			doNotRunThisProcessorForMatchingMethods: '不要运行此处理器来匹配方法。',
			doNotSendHeadersToTheExternalProcessingService: '不要将请求头发送到外部处理服务。',
			doNotSendTheBodyToTheExternalProcessingService: '请勿将正文发送至外部处理服务。',
			doNotSendTheBodyToTheProcessor: '请勿将正文发送至处理器。',
			doNotSendThisPhaseToTheExternalProcessor: '不要将此阶段发送到外部处理器。',
			doNotSendTrailersToTheExternalProcessingService:
				'请勿将尾部字段（trailers）发送到外部处理服务。',
			documentation: '文档',
			domain: '域名',
			download: '下载',
			duration: '持续时间',
			dynamic: '动态',
			dynamicBackendSelectionIsEnabledForThisBackend: '为此后端启用动态后端选择。',
			eachCelExpressionIsSavedUnderAllowDenyOrRequire:
				'每个 CEL 表达式都保存在 `allow`、`deny` 或 `require` 下。',
			edit: '编辑',
			editValueGuard: '编辑 {{value}} 防护',
			evictValue: '驱逐时长 {{value}}',
			editBind: '编辑绑定',
			editGateway: '编辑网关',
			editKey: '编辑键',
			editListener: '编辑监听器',
			editModel: '编辑模型',
			editProvider: '编辑提供商',
			editRoute: '编辑路由',
			editServer: '编辑服务器',
			editTheFullGatewayYaml: '编辑完整的网关 YAML。',
			editThoseListenersThroughRawYamlOrSplitTheRoutesAcrossSeparateListeners:
				'通过原始 YAML 编辑这些监听器或将路由拆分到不同的监听器。',
			email: '电子邮件',
			emailAddressPattern: '电子邮件地址模式。',
			enable: '启用',
			enableIncludePromptsAndCompletionsInLogsIn: '启用“在日志中包含提示词和补全内容”',
			enableValue: '启用 {{value}}',
			enableDeveloperMode: '启用开发者模式',
			enableDownstreamProxyProtocolHandlingOnThisGatewayOrPortIncludingVersionMatching_9ksq9m:
				'在此网关或端口上启用下游 PROXY 协议处理，包括版本匹配，以及请求头必需或可选的设置。',
			enableLlm: '启用 LLM',
			enableMcp: '启用 MCP',
			enableOrDisableDownstreamHttpConnectHandling: '启用或禁用下游 HTTP CONNECT 处理。',
			enableTheCapabilitiesYouWantToOperateFromTheSetupPath: '启用你希望在设置流程中使用的功能。',
			enableTraffic: '启用流量',
			enabled: '已启用',
			enabled_17fi4vy: '已启用',
			encodeHttp1HeaderNamesInLowercase: '将 HTTP/1 请求头名称编码为小写。',
			endpoint: '端点',
			endpointPickerBackendThatSelectsTheDestinationEndpoint: '选择目标端点的端点选择器后端。',
			enforceThatTheSubjectSMayActClaimAuthorizesTheActorBeforeExchanging:
				'在交换之前强制主体的 `may_act` 声明对参与者进行授权。',
			enforcement: '强制执行',
			enterTheGatewayUrlAndVirtualApiKeySaveThenRestartClaudeDesktop:
				'输入网关 URL 和虚拟 API 密钥并保存，然后重启 Claude Desktop。',
			entries: '条目',
			envVar: '环境变量',
			environmentMustBeAYamlMapping: '环境必须是 YAML 映射。',
			environmentYaml: '环境变量 YAML',
			error: '错误',
			evaluate: '评估',
			evaluatePolicyExpressionsAgainstSampleOrCustomRequestContextUsingTheGatewayCelEndpoint:
				'使用网关 CEL 端点根据示例或自定义请求上下文评估策略表达式。',
			evaluateRequestCountDescriptorsWhileProcessingTheRequest: '在处理请求时评估请求计数描述符。',
			evaluateTokenDescriptorsAfterTheLlmResponseCompletes: 'LLM 响应完成后评估令牌描述符。',
			everyListedHeaderConditionMustMatch: '每个列出的请求头条件都必须匹配。',
			everyListedQueryConditionMustMatch: '每个列出的查询条件必须匹配。',
			evictionDuration: '驱逐持续时间',
			exampleComExampleCom: 'example.com、*.example.com',
			expectedAYamlMapping: '需要 YAML 映射。',
			expectedTokenIssuerMatchedAgainstTheJwtIssClaim:
				'预期的令牌签发者，与 JWT `iss` 声明相匹配。',
			explicit: '显式的',
			explicitBackendReferenceBackendMustBeDefinedInTheTopLevelBackendsList:
				'显式后端引用。后端必须在顶级后端列表中定义',
			explicitEndpoints: '显式端点',
			explicitOutgoingModel: '显式传出模型',
			export: '导出',
			exposeHeaders: '公开响应头',
			exposeTheUiOnATrafficGatewayAndConfigurePoliciesThatProtectTheUi:
				'在流量网关上公开 UI 并配置保护 UI 的策略。',
			exposeToolNamesWithoutAddingTheTargetName: '公开工具名称而不添加目标名称。',
			expression: '表达式',
			expressionToDetermineTheAmountOfClientSamplingClientSamplingDeterminesWhetherToI_12geacf:
				'用于确定客户端采样率的表达式。\n如果传入请求已有跟踪，客户端采样会决定是否启动新的跟踪跨度。\n表达式结果应为 0.0 到 1.0（0% 到 100%）之间的浮点数，或布尔值 `true`/`false`。\n默认为 `true`。',
			expressionToDetermineTheAmountOfRandomSamplingRandomSamplingWillInitiateANewTrac_1d5h2qd:
				'用于确定随机采样率的表达式。\n如果传入请求尚无跟踪，随机采样会决定是否启动新的跟踪跨度。\n表达式结果应为 0.0 到 1.0（0% 到 100%）之间的浮点数，或布尔值 `true`/`false`。\n默认为 `false`。',
			externalAuthz: '外部授权',
			externalMcpPolicyProcessors: '外部 MCP 策略处理器。',
			externalProcessor: '外部处理器',
			externalServiceTheGatewayCallsForThisPolicy: '网关调用此策略的外部服务。',
			extraFormParametersAppendedToTheTokenRequestValuesAreCelExpressionsEvaluatedAgai_11xnn0t:
				'附加到令牌请求的额外表单参数。\n值是根据传入请求计算的 CEL 表达式。',
			failClosed: '失败时拒绝',
			failOpen: '失败时放行',
			failuresValue: '{{value}} 次失败',
			failTheEntireSessionIfAnyTargetFailsToInitializeOrAnyUpstreamFailsDuringAFanoutT_f2p346:
				'如果任何目标无法初始化，或扇出期间任何上游失败，则整个会话失败。\n这是默认行为，与当前行为一致。',
			failover: '故障转移',
			failoverEnablesPriorityBasedSelectionOfTheTargetModelWithinAPriorityLevelTheBest_1lo0fhc:
				'故障转移支持按优先级选择目标模型。\n在同一优先级内，根据健康状况和延迟的综合评分选择最佳提供商。\n如果当前优先级内的所有模型均已降级，请求会转到下一个优先级组。',
			failoverTargets: '故障转移目标',
			failureMode: '失败模式',
			featuresAndRoutesReferenceThisGatewayByName: '各项功能和路由通过名称引用此网关。',
			feedback: '反馈',
			fetchAnAccessToken: '获取访问令牌',
			fetchAnIdToken: '获取 ID 令牌',
			fetchSigningKeysFromTheIssuerJwksEndpoint: '从签发者 JWKS 端点获取签名密钥。',
			fetchingRecentLlmCalls: '正在获取最近的 LLM 调用。',
			file: '文件',
			fillInterval: '填充间隔',
			fixTheHighlightedFieldsBeforeSaving: '请在保存前修正高亮字段。',
			fixTheHighlightedProcessorsBeforeSaving: '保存前修复突出显示的处理器。',
			forAzureTheApiVersionToUse: '对于 Azure：要使用的 API 版本',
			forAzureTheFoundryProjectNameRequiredForFoundryResourceType:
				'对于 Azure：Foundry 项目名称（Foundry 资源类型必需）',
			forAzureTheResourceNameOfTheDeployment: '对于 Azure：部署的资源名称',
			forAzureTheTypeOfAzureEndpointOpenAiOrFoundry:
				'对于 Azure：Azure 端点的类型（openAI 或 foundry）',
			forwardTheRequestToTheAdminApiUsingTheRequestSCurrentPathAndQuery:
				'使用请求的当前路径和查询将请求转发到管理 API。',
			forwardTheValidatedIncomingJwtToTheBackend: '将经过验证的传入 JWT 转发到后端。',
			foundry: 'Foundry',
			fractionOfMatchingRequestsToMirrorFrom00To10: '镜像匹配请求的分数，从 0.0 到 1.0。',
			from: '来自',
			fromTheSameDirectory: '来自同一目录。',
			frontendPoliciesDefinesTopLevelPoliciesApplyingToAllTraffic:
				'`frontendPolicies` 定义适用于所有流量的顶级策略。',
			full: '完整',
			fullDuplexStreamed: '全双工流式传输',
			fullyQuitAndRelaunchClaudeDesktopANew: '完全退出并重新启动 Claude Desktop。重新启动后会显示',
			gateway: '网关',
			gatewayValue: '网关 {{value}}',
			gatewayBaseUrl: '网关基础 URL',
			gatewayBinding: '网关绑定',
			gatewayError: '网关错误',
			gatewayOrGatewayListenerThatOwnsThisRoute: '拥有此路由的网关或网关监听器。',
			gatewayOverview: '网关概览',
			gatewayPolicies: '网关策略',
			gatewaySaved: '网关已保存',
			gatewaySent: '网关已发送',
			gatewaySurfaces: '网关功能',
			gatewayListenerRouteOrBackendThatThisPolicyAttachesTo:
				'此策略附加到的网关、监听器、路由或后端。',
			gateways: '网关',
			gatewaysAttachesTheLlmRoutesToNamedGatewaysThisCanTakeTheFormOfGatewayNameOrGate_n9bphz:
				'`gateways` 将 LLM 路由挂载到具名网关。可使用 `<gateway-name>` 挂载到网关，或使用 `<gateway-name>/<listener-name>` 挂载到网关内的特定监听器。\n省略此字段且存在名为 `default` 的网关时，LLM API 路由会挂载到该网关；设置了 `port` 时除外。',
			gatewaysAttachesTheMcpRoutesToNamedGatewaysThisCanTakeTheFormOfGatewayNameOrGate_19pj37b:
				'`gateways` 将 MCP 路由挂载到具名网关。可使用 `<gateway-name>` 挂载到网关，或使用 `<gateway-name>/<listener-name>` 挂载到网关内的特定监听器。\n省略此字段且存在名为 `default` 的网关时，MCP 路由会挂载到该网关；设置了端口时除外。',
			gatewaysAttachesTheUiAndUiBackendRoutesToNamedGatewaysThisCanTakeTheFormOfGatewa_1hlnrin:
				'`gateways` 将 UI 及其后端路由挂载到具名网关。可使用 `<gateway-name>` 挂载到网关，或使用 `<gateway-name>/<listener-name>` 挂载到网关内的特定监听器。\n省略此字段且存在名为 `default` 的网关时，UI 路由会挂载到该网关。',
			gatewaysAttachesThisRouteToNamedGatewaysOrGatewayListenersThisCanTakeTheFormOfGa_j7n552:
				'`gateways` 将此路由挂载到具名网关或网关监听器。\n可使用 `<gateway-name>` 挂载到网关，或使用 `<gateway-name>/<listener-name>` 挂载到网关内的特定监听器。\n未设置时使用名为 `default` 的网关。',
			gatewaysAttachesThisRouteToNamedTcpTlsGatewaysOrGatewayListenersThisCanTakeTheFo_6uai65:
				'`gateways` 将此路由挂载到指定的 TCP/TLS 网关或网关监听器。\n可使用 `<gateway-name>` 挂载到网关，或使用 `<gateway-name>/<listener-name>` 挂载到网关内的特定监听器。\n未设置时使用名为 `default` 的网关。',
			gatewaysDefinesTheEntrypointToTheProxySettingUpPortsAndListenersThatFeaturesLlmM_18ageg5:
				'`gateways` 定义代理入口点，为 LLM、MCP、UI 和路由提供可挂载的端口与监听器。\n每个网关定义代理监听的端口，以及该端口可选的 TLS 设置。',
			generateConnectionSettingsAndSnippetsForOpenAiCompatibleLlmClients:
				'为兼容 OpenAI 的 LLM 客户端生成连接设置和代码片段。',
			generatedModelConfig: '生成的模型配置',
			generatedProviderConfig: '生成的提供商配置',
			generatedVirtualModelConfig: '生成的虚拟模型配置',
			getStarted: '开始使用',
			gitHubCopilot: 'GitHub Copilot',
			googleCredentials: 'Google 凭据',
			googleModelArmor: 'Google Model Armor',
			group: '组',
			groupAttribute: '群组属性',
			groupBy: '分组方式',
			group_sf1daa: '组：',
			groups: '用户组',
			gRpcDetails: 'gRPC 详细信息',
			guardType: '防护类型',
			guardThisTakesEffectImmediately: '防护规则？此更改会立即生效。',
			guardrailIdentifier: '防护规则标识符',
			guardrailVersion: '防护规则版本',
			guardrails: '防护规则',
			guardrailsToApplyToEveryConfiguredModel: '适用于每个配置模型的防护规则。',
			guardrailsToApplyToTheRequestOrResponse: '应用于请求或响应的防护规则',
			guardsAppliedToClientRequestsBeforeTheyReachTheLlm:
				'在客户请求到达 LLM 之前，对其应用防护措施。',
			guardsAppliedToLlmResponsesBeforeTheyReachTheClient:
				'在 LLM 响应到达客户端之前应用防护措施。',
			haltOnBlocklistHit: '遇到阻止列表时停止',
			handleCorsPreflightRequestsAndAppendConfiguredCorsHeadersToApplicableRequests:
				'处理 CORS 预检请求并将配置的 CORS 请求头附加到适用的请求。',
			handleCsrfProtectionByValidatingRequestOriginsAgainstConfiguredAllowedOrigins:
				'通过根据配置的允许来源验证请求来源来处理 CSRF 保护。',
			headerAllowlist: '请求头允许列表',
			headerCasingBehaviorForHttp1Responses: 'HTTP/1 响应头的大小写行为。',
			headerLocation: '请求头位置',
			headerName: '请求头名称',
			headerName_8vzq77: '请求头名称',
			headerNameContainingTheCredential: '包含凭证的请求头名称。',
			headerNamesToRemove: '要删除的请求或响应头名称。',
			headerPrefix: '请求头前缀',
			headerValue: '请求头值',
			headers: '请求头',
			headersToAddToTheAuthorizationRequestUsingCelExpressionsEmptyMeansAllHeaders:
				'使用 CEL 表达式添加到授权请求的请求头。留空表示所有请求头。',
			headersToAddSetOrRemoveFromTheRejectionResponse: '要在拒绝响应中添加、设置或删除的响应头。',
			headersToAppendUsingCelExpressionsForValues: '使用 CEL 表达式附加值的请求或响应头。',
			headersToAppendWithoutReplacingExistingValues: '要附加的请求或响应头，且不替换现有值。',
			headersToSetUsingCelExpressionsForValues: '使用 CEL 表达式设置值的请求或响应头。',
			headersToSetReplacingAnyExistingValues: '要设置的请求或响应头，替换任何现有值。',
			health: '健康',
			healthConfiguresOutlierDetectionForThisModelBackend:
				'`health` 用于为此模型后端配置异常值检测。',
			healthScoreThresholdBelowWhichAnUnhealthyResponseCanEvictTheBackend:
				'健康分数阈值，低于该阈值，不健康的响应可能会驱逐后端。',
			healthScoreToRestoreWhenTheBackendReturnsFromEviction: '当后端从驱逐中返回时恢复健康分数。',
			healthThreshold: '健康阈值',
			help: '帮助',
			hide: '隐藏',
			hideFullKey: '隐藏完整密钥',
			home: '首页',
			host: '主机',
			hostOrPortRewriteToApplyBeforeForwardingTheRequest: '主机或端口重写以在转发请求之前应用。',
			hostOrPortRewriteToApplyToTheRedirectUrl: '主机或端口重写以应用于重定向 URL。',
			hostname: '主机名',
			hostnameDefinesWhatHostnamesAreServedUnderThisListenerCanBeAWildcardThisAllowsSe_w5k5cr:
				'定义此监听器提供服务的主机名，可以使用通配符。\n借助此字段，可为多个域名使用不同的 TLS 配置。\n未设置时匹配所有域名，相当于隐式通配符。',
			hostnameOrIpAddress: '主机名或 IP 地址',
			hostnames: '主机名',
			howDownstreamHttpConnectRequestsAreHandled: '如何处理下游 HTTP CONNECT 请求。',
			howLongAnIdleHttp1ConnectionMayStayOpen: '空闲 HTTP/1 连接可以保持打开状态多长时间。',
			howLongToEvictAnUnhealthyBackend: '驱逐不健康的后端需要多长时间。',
			howOftenTheLocalBucketIsRefilled: '本地存储桶重新装满的频率。',
			howRequestBodiesAreSentToTheExternalProcessingService: '如何将请求正文发送到外部处理服务。',
			howResponseBodiesAreSentToTheExternalProcessingService: '如何将响应正文发送到外部处理服务。',
			howTheGatewayConnectsToThisMcpTarget: '网关如何连接到此 MCP 目标。',
			howToUseTheDestinationReturnedByTheEndpointPicker: '如何使用端点选择器返回的目的地。',
			httpAndTcpListenersRoutesAndPolicyControls: 'HTTP 和 TCP 监听器、路由和策略控制。',
			httpDetails: 'HTTP 详细信息',
			httpProtocolSettingsForThisBackend: '该后端的 HTTP 协议设置。',
			httpResponseStatusCodesThatShouldBeRetried: '应重试的 HTTP 响应状态代码。',
			httpStatus: 'HTTP 状态',
			httpStatusCodeReturnedWhenContentIsRejected: '内容被拒绝时返回的 HTTP 状态代码。',
			httpStatusCodeToReturnForTheRedirect: '为重定向返回的 HTTP 状态代码。',
			httpStatusCodeToReturn: '要返回的 HTTP 状态代码。',
			httpVersionToUseWhenConnectingToTheBackend: '连接到后端时使用的 HTTP 版本。',
			httpProxy: 'HTTP 代理',
			http2ConnectionFlowControlWindowSize: 'HTTP/2 连接流量控制窗口大小。',
			http2StreamFlowControlWindowSize: 'HTTP/2 流的流量控制窗口大小。',
			identifierOfTheResourceAuthorizationServerTheIssuedIdJagIsBoundToThisAudience:
				'资源授权服务器的标识符。已发行的 ID-JAG 对该受众具有约束力。',
			identifyTheOauth2ClientUsedByTheGatewayDuringTheAuthorizationCodeFlow:
				'识别网关在授权代码流期间使用的 OAuth2 客户端。',
			identityProviderTypeUsedToDeriveMcpAuthorizationMetadataAndDefaultJwksUrls:
				'用于派生 MCP 授权元数据和默认 JWKS URL 的身份提供商类型。',
			ifATokenExistsValidateItWarningThisAllowsRequestsWithoutAJwtTokenAdditionally401_dgw23w:
				'请求包含令牌时验证该令牌。\n警告：未携带 JWT 的请求仍会通过。此模式也不会返回 401，因此客户端不会启动 OAuth 流程。',
			inYourProjectRoot: '，并将其放在项目根目录中。',
			includeMcpTools: '包括 MCP 工具（',
			includeMcpToolsValueServers_one: '包括 MCP 工具（{{count}} 个服务器）',
			includeMcpToolsValueServers_other: '包括 MCP 工具（{{count}} 个服务器）',
			includePromptsAndCompletionsInLogs: '在日志中包含提示词和补全内容',
			includeRequestBody: '包含请求体',
			includeRequestHeaders: '包含请求头',
			includeResponseHeaders: '包含响应头',
			incomingModel: '传入模型',
			incomingModelMatch: '传入模型匹配模式',
			incomingRequestHeadersToForwardToTheWebhook: '要转发到 Webhook 的传入请求头。',
			inheritance: '继承',
			initialize: '初始化',
			initializeAGatewayMcpSessionListToolsAndCallAToolThroughTheMcpListener:
				'初始化网关 MCP 会话、列出工具并通过 MCP 监听器调用工具。',
			initializeFirst: '请先初始化',
			initializeOrSendAToolRequestToInspectMcpBehavior:
				'初始化会话或发送工具请求，以检查 MCP 行为。',
			initializeTheSessionAndSelectAToolToConfigureArguments: '初始化会话并选择工具以配置参数。',
			initialized: '已初始化',
			initializingMcpTools: '正在初始化 MCP 工具',
			inlineJson: '内联 JSON',
			inlineJwks: '内联 JWKS',
			inlineOverridesStoredInThisGatewayConfigurationValuesAreUsdPer1MTokens:
				'内联覆盖项存储在此网关配置中。数值单位为每 1M 个令牌的美元价格。',
			input: '输入',
			inputSchema: '输入模式',
			inspect: '检查',
			inspectModelOutputBeforeItIsReturnedToTheCaller: '在模型输出返回给调用方之前进行检查。',
			inspectPromptsBeforeTheyReachTheUpstreamModel: '在提示词到达上游模型之前进行检查。',
			inspectRecentLlmCallsAndRequestResponsePayloads: '检查最近的 LLM 调用和请求/响应负载。',
			integration: '集成',
			internalModelsCanBeTargetedByVirtualModelsButCannotBeRequestedDirectly:
				'虚拟模型可以将内部模型作为目标，但不能直接请求内部模型。',
			interval: '间隔',
			intervalBetweenHttp2KeepalivePings: 'HTTP/2 保活探测之间的时间间隔。',
			invalidAuthorizationPolicy: '授权策略无效',
			invalidCustomCosts: '自定义成本无效',
			invalidGuardrails: '无效的防护规则',
			invalidJson: '无效的 JSON',
			invalidJwtPolicy: '无效的 JWT 策略',
			invalidMcpAuthenticationPolicy: '无效的 MCP 身份验证策略',
			invalidMcpGuardrailsPolicy: '无效的 MCP 防护规则策略',
			invalidModelPolicies: '无效的模型策略',
			invalidOidcPolicy: '无效的 OIDC 策略',
			invalidServer: '服务器无效',
			invalidYaml: '无效的 YAML',
			issuer: '签发者',
			issuerUsedForDiscoveryAndIdTokenValidation: '用于发现和 ID 令牌验证的签发者。',
			jailbreakApiVersion: '越狱 API 版本',
			jsonWebKeySetUsedToVerifyTokenSignaturesCanBeInlineFromAFileOrFetchedRemotely:
				'JSON Web 密钥集用于验证令牌签名。可以内联、从文件或远程获取。',
			jwksFile: 'JWKS 文件',
			jwksSource: 'JWKS 来源',
			jwksSourceUsedToValidateReturnedIdTokens: 'JWKS 源用于验证返回的 ID 令牌。',
			jwksUrl: 'JWKS URL',
			jwtAuth: 'JWT 身份验证',
			jwtValidationOptionsControllingWhichClaimsMustBePresentInATokenTheRequiredClaims_12osoae:
				'JWT 验证选项，用于控制令牌中必须包含哪些声明。\n\n`required_claims` 集合指定继续验证前，令牌负载中必须存在的 RFC 7519 注册声明。仅支持 `exp`、`nbf`、`aud`、`iss` 和 `sub`。`iat`、`jti` 等其他注册声明不会由底层 `jsonwebtoken` 库强制检查，并会被忽略。\n\n此设置只检查声明是否存在；`exp`、`nbf` 等标准声明的值仍会单独验证。例如，只要令牌含有 `exp`，无论此设置如何，系统都会检查令牌是否过期。\n\n默认为 `["exp"]`。',
			keepServingTrafficWhileSurfacingJwtDataWhenPossible:
				'在可用时提取 JWT 数据，同时继续转发流量。',
			key: '密钥',
			keyExchangeGroupsAllowedForNegotiatingTls: '允许协商 TLS 的密钥交换组。',
			keyValue: '密钥值',
			kind: '类型',
			last1Hour: '最近 1 小时',
			last12Hours: '最近 12 小时',
			last14Days: '最近 14 天',
			last24Hours: '最近 24 小时',
			last30Days: '最近 30 天',
			last7Days: '最近 7 天',
			leaveEmptyToUseDefault5xxAndConnectionFailureHandling: '留空以使用默认 5xx 和连接失败处理。',
			leaveTheAuthorityUnchanged: '保持 `:authority` 不变。',
			letTheModelCallToolsExposedByTheMcpGateway: '允许模型调用 MCP 网关公开的工具。',
			limitByRequestCount: '按请求数量限制。',
			limitByTokenCount: '按令牌数量限制。',
			limitOverride: '限制覆盖',
			limitType: '限制类型',
			limitOverrideDeterminesTheOptionalExpressionToDetermineTheLimitOfTheRequestThisT_6mrd6s:
				'`limitOverride` 指定一个可选表达式，用于计算请求限制，并告知远程服务器应应用哪项限制。\n注意：此字段不指定请求“成本”；成本由 `cost` 字段指定。\n表达式结果必须是包含 `unit` 和 `requestsPerUnit` 键的映射，例如：\n`{"unit":"second","requestsPerUnit":100}`。\n有效单位为 `second`、`minute`、`hour`、`day`、`month`、`year`。\n表达式计算失败时跳过该描述符。',
			listener: '监听器',
			listenerPolicies: '监听器策略',
			listenerThatOwnsThisRoute: '拥有该路由的监听器。',
			listenerYaml: '监听器 YAML',
			listeners: '监听器',
			listeners_1fzojr3: '监听器 ·',
			listenersDefinesMultipleNamedListenersUnderThisGatewayWhenSetOnlyPortMayBeConfig_e7d148:
				'`listeners` 用于在此网关下定义多个具名监听器。设置后，顶层网关只能配置 `port`。',
			llmCosts: 'LLM 成本',
			llmDefinesASetOfLlmModelsToBeExposedByTheProxyWhenConfiguredLlmModelsWillBeServe_beutm3:
				'`llm` 定义代理公开的一组 LLM 模型。配置后，这些模型会在关联的 `gateways` 下通过标准服务路径提供服务（`/v1/models`、`/v1/chat/completions` 等）。',
			llmGuardrails: 'LLM 防护规则',
			llmModels: 'LLM 模型',
			llmPlayground: 'LLM 演练场',
			llmPolicies: 'LLM 策略',
			llmProviders: 'LLM 提供商',
			llmRequestFields: 'LLM 请求字段',
			llmRequestModelStripPrefixAnthropic: 'llmRequest.model.stripPrefix("anthropic/")',
			loadingAnalytics: '正在加载分析数据…',
			loadingEditor: '正在加载编辑器…',
			loadingGatewayConfiguration: '正在加载网关配置',
			loadingGateways: '正在加载网关',
			loadingGuardrails: '正在加载防护规则',
			loadingKeys: '正在加载密钥',
			loadingLogPayload: '正在加载日志负载',
			loadingMcpServers: '正在加载 MCP 服务器',
			loadingModelCatalog: '正在加载模型目录…',
			loadingModels: '正在加载模型',
			loadingProviders: '正在加载提供商',
			loadingRawConfiguration: '正在加载原始配置…',
			loadingRuntimePolicies: '正在加载运行时策略',
			loadingRuntimeTrafficConfiguration: '正在加载运行时流量配置',
			loadingTrafficListeners: '正在加载流量监听器',
			loadingTrafficRoutes: '正在加载流量路由',
			localFile: '本地文件',
			localRateLimit: '本地速率限制',
			localRateLimitsForIncomingRequests: '传入请求的本地速率限制。',
			localXdsPathIfNotSpecifiedTheCurrentConfigurationFileWillBeUsed:
				'本地 XDS 路径。如果未指定，将使用当前配置文件。',
			localConfigEvictionSubPolicyWithDurationAsStringMirrorsEviction:
				'本地/配置驱逐子策略，持续时间为字符串；镜像 `Eviction`。',
			localConfigHealthPolicyWithCelAsStringConvertedToPolicyByCompilingTheExpressionM_lbnrib:
				'本地配置中的健康策略以 CEL 字符串表示，编译表达式后转换为策略。\n其结构与原始 `Health` 消息一致。',
			location: '位置',
			logSettings: '日志设置',
			logSettings_12oqjpq: '日志设置',
			logs: '日志',
			logsApiError: '日志 API 错误',
			manageGateways: '管理网关',
			manageModelCostCatalogsUsedForAnalyticsAndRequestCostAttribution:
				'管理用于分析和请求成本归因的模型成本目录。',
			managed: '托管',
			managedOnGuardrails: '请在防护规则页面管理',
			managedOnVirtualApiKeys: '请在虚拟 API 密钥页面管理',
			manuallyProvideAuthorizationTokenAndSigningKeyMetadata:
				'手动提供授权、令牌和签名密钥元数据。',
			mapsToTheRequestAttributesFieldInProcessingRequestAndAllowsDynamicCelExpressions:
				'映射到 ProcessingRequest 中的请求 `attributes` 字段，并允许动态 CEL 表达式。',
			mapsToTheResponseAttributesFieldInProcessingRequestAndAllowsDynamicCelExpressions:
				'映射到 ProcessingRequest 中的响应 `attributes` 字段，并允许动态 CEL 表达式。',
			markThisAsLlmTrafficToEnableLlmProcessing: '将此标记为 LLM 流量以启用 LLM 处理。',
			markThisTrafficAsA2AToEnableA2AProcessingAndTelemetry:
				'将此流量标记为 A2A 以启用 A2A 处理和遥测。',
			maskMatchedText: '遮盖匹配文本',
			match: '匹配',
			matchAndOptionallyMaskCustomRegularExpressions: '匹配并可选地屏蔽自定义正则表达式。',
			matchConditionsAndModelSpecificPolicies: '匹配条件和模型级策略',
			matchIncomingHttpAndTcpTrafficAndAttachInlineBackends:
				'匹配传入的 HTTP 和 TCP 流量并附加内联后端。',
			matches: '匹配条件',
			matchesSpecifiesTheConditionsUnderWhichThisModelShouldBeUsedInAdditionToMatchingTheModelName:
				'`matches` 用于指定除模型名称外，使用该模型还需满足的条件。',
			maxAge: '最长有效期',
			maxRequestBytes: '最大请求字节数',
			maxTokens: '最大令牌数',
			maximumBodySizeToBufferInBytes: '可缓冲的最大正文大小（字节）。',
			maximumHttp2FrameSize: '最大 HTTP/2 帧大小。',
			maximumNumberOfAuthorizationResultsToKeepInTheCache: '缓存中保留的授权结果的最大数量。',
			maximumNumberOfHeadersAllowedInAnHttp1RequestChangingThisValueCausesAPerformance_j9b30b:
				'HTTP/1 请求允许包含的最大请求头数量。更改此值会降低性能，即使将其调低到默认值 100 以下也是如此。',
			maximumNumberOfTokenExchangeResponsesToKeepInTheCacheSetTo0ToDisable:
				'缓存中保留的令牌交换响应的最大数量。设置为 0 以禁用。',
			maximumNumberOfTokensThatCanAccumulateInTheLocalBucket: '本地令牌桶可累积的最大令牌数。',
			maximumRequestBodySizeToSendToTheAuthorizationServiceDefaultsTo8192Bytes:
				'发送到授权服务的请求体最大大小。默认为 8192 字节。',
			maximumRequestOrResponseBodySizeBufferedByTheFrontend: '前端可缓冲的请求体或响应体最大大小。',
			maximumSizeOfHttp2RequestHeaders: 'HTTP/2 请求头的最大大小。',
			maximumSupportedTlsVersionOnlyTls12And13AreSupported:
				'支持的最高 TLS 版本（仅支持 TLS 1.2 和 1.3）。',
			maximumTimeAConnectionMayStayOpenAfterThisDurationTheConnectionIsGracefullyClose_t76f83:
				'连接可以保持打开的最长时间。超过该时长后，将等待当前处理中的请求完成，再正常关闭连接。这有助于在扩缩容期间，让负载均衡器后的流量分布更加均匀。',
			maximumTimeAllowedForABackendHttpRequest: '后端 HTTP 请求允许的最长时间。',
			maximumTimeAllowedForTheFullDownstreamRequestAndResponse:
				'完整下游请求和响应所允许的最长时间。',
			maximumTimeAllowedForTheUpstreamBackendRequest: '上游后端请求允许的最长时间。',
			maximumTimeAllowedToCompleteTheDownstreamTlsHandshake: '允许完成下游 TLS 握手的最长时间。',
			maximumTimeAllowedToEstablishABackendTcpConnection: '允许建立后端 TCP 连接的最长时间。',
			maximumTlsVersionAcceptedFromDownstreamClients: '允许下游客户端使用的最高 TLS 版本。',
			mcpAuthentication: 'MCP 身份验证',
			mcpAuthorization: 'MCP 授权',
			mcpBehavior: 'MCP 行为',
			mcpBrowserAccessIsNotAllowed: '不允许通过浏览器访问 MCP',
			mcpDefinesASetOfMcpServersExposedByTheProxyWhenConfiguredTheMcpServersWillBeServ_15ox9e0:
				'`mcp` 定义代理公开的一组 MCP 服务器。配置后，这些服务器将在关联的 `gateways` 下通过 `/mcp` 和 `/sse` 提供服务。列表中的所有 MCP 服务器将作为一个虚拟 MCP 服务器提供服务。',
			mcpGatewaySettings: 'MCP 网关设置。',
			mcpGuardrails: 'MCP 防护规则',
			mcpPlayground: 'MCP 演练场',
			mcpPolicies: 'MCP 策略',
			mcpRequestFailed: 'MCP 请求失败',
			mcpServers: 'MCP 服务器',
			mcpToolOutput: 'MCP 工具输出',
			mcpGateway: 'mcp://gateway',
			measure: '测量',
			menuAppearsInTheMenuBar: '菜单出现在菜单栏中。',
			messageOffset: '消息偏移量',
			messageOffsetUsedWhenChoosingWhereToPlaceCacheMarkers:
				'选择放置缓存标记的位置时使用的消息偏移量。',
			messages: '消息',
			messagesAppendedToTheEndOfEachChatRequest: '消息附加到每个聊天请求的末尾。',
			messagesPrependedToTheBeginningOfEachChatRequest: '消息添加到每个聊天请求的开头。',
			messagesToAddBeforeOrAfterTheClientPrompt: '在客户端提示之前或之后添加的消息。',
			metadata: '元数据',
			metadataAdvertisedToMcpClientsForOauthProtectedResources:
				'向 MCP 客户端公布的 OAuth 受保护资源元数据。',
			metadataContextYaml: '元数据上下文 YAML',
			metadataValuesToAddUsingCelExpressions: '使用 CEL 表达式添加的元数据值。',
			metadataValuesToExposeUnderTheExtauthzVariableAfterAuthorization:
				'授权后在 `extauthz` 变量下公开的元数据值。',
			metadataValuesToSendToTheAuthorizationServiceComputedFromCelExpressionsMapsToThe_1ed0p5i:
				'要发送到授权服务的元数据值，根据 CEL 表达式计算得出。\n映射到请求中的 `metadata_context.filter_metadata` 字段。\n如果未设置，则在还使用 JWT 身份验证时设置 `envoy.filters.http.jwt_authn`，以实现兼容性。',
			method: '方法',
			methodPhases: '方法阶段',
			microsoftEntra: 'Microsoft Entra',
			migrateBindsToGateways: '将绑定迁移到网关',
			minimalRawHttpRequestForDebuggingClientConnectivity:
				'用于调试客户端连接的最简原始 HTTP 请求。',
			minimumPromptSizeRequiredBeforeCacheMarkersAreAdded: '添加缓存标记前所需的最小提示词长度。',
			minimumSupportedTlsVersionOnlyTls12And13AreSupported:
				'支持的最低 TLS 版本（仅支持 TLS 1.2 和 1.3）。',
			minimumTlsVersionAcceptedFromDownstreamClients: '允许下游客户端使用的最低 TLS 版本。',
			minimumTokens: '最小令牌数',
			mode: '模式',
			model: '模型',
			modelCelExpression: '模型 CEL 表达式',
			modelCostCatalogSourcesEntriesAreMergedInOrderWithLaterEntriesTakingPrecedence:
				'模型成本目录来源；条目按顺序合并，后面的条目优先。',
			modelIsResolvedAgainstLlmModelsUsingTheSameWildcardMatchingAsClientRequests:
				'`model` 会按照与客户端请求相同的通配符匹配规则，在 `llm.models` 中解析。',
			modelNameAliasesThatRewriteRequestedModelNames: '用于重写请求模型名称的别名。',
			modelPolicies: '模型策略',
			modelUsesAWildcardSpecifyTheSpecificModel: '模型使用通配符；请指定具体模型。',
			modelWarnings: '模型警告',
			models: '模型',
			modelsDefinesTheSetOfModelsThatCanBeServedByThisGatewayTheModelNameRefersToTheMo_1qlvcg6:
				'`models` 定义该网关可提供服务的模型集合。模型名称用于匹配用户请求中的模型；发送给实际 LLM 的模型可针对每个模型单独覆盖。',
			modelsKeysPoliciesAndChatTesting: '模型、密钥、策略和聊天测试。',
			moderationModel: '内容审核模型',
			moderationModelToUseDefaultsToOmniModerationLatest:
				'要使用的内容审核模型。默认为 `omni-moderation-latest`。',
			modifyRequestAndResponseDataForThisBackend: '修改此后端的请求和响应数据。',
			modifyRequestAndResponseHeadersBodiesOrMetadata:
				'修改请求头、响应头、请求体、响应体或元数据。',
			modifyRequestHeadersBeforeForwardingToThisBackend: '在转发到此后端之前修改请求头。',
			modifyRequestHeadersBeforeForwarding: '转发前修改请求头。',
			modifyResponseHeadersBeforeReturningToTheClient: '返回客户端之前修改响应头。',
			modifyResponseHeadersReturnedFromThisBackend: '修改此后端返回的响应头。',
			ms: '毫秒',
			multipleListeners: '多个监听器',
			mutation: '变更',
			name: '名称',
			nameAlreadyExists: '名称已存在',
			nameIdentifiesThisListenerForGatewayReferencesLikeGatewaysGatewayNameListenerName:
				'`name` 用于标识此监听器，以便通过 `gateways: gateway-name/listener-name` 等形式引用。',
			nameIsReferencedFromLlmModelsProviderReference:
				'该名称由 `llm.models[].provider.reference` 引用。',
			nameIsRequired: '名称为必填项',
			nameIsTheNameOfTheModelWeAreMatchingFromAUsersRequestIfParamsModelIsSetThatWillB_1ti2su5:
				'`name` 是用于匹配用户请求的模型名称。如果设置了 `params.model`，向 LLM 提供商发送请求时将使用该值；否则使用请求中传入的模型名称。',
			nameIsThePublicModelNameClientsRequest: '`name` 是客户端请求时使用的公开模型名称。',
			namespace: '命名空间',
			namespaceKeyCelExpression: '命名空间：键：CEL 表达式',
			never: '从不',
			neverPrefixCallsAreRoutedByToolNameWhichMustBeUniqueAcrossTargets:
				'从不添加前缀；调用按工具名称路由，因此工具名称在所有目标中必须唯一。',
			newKey: '新建密钥',
			noValueTransformationsConfigured: '尚未配置{{value}}转换。',
			noAdditionalMatchConditions: '没有额外的匹配条件。',
			noAnalyticsInTheSelectedWindow: '所选时间范围内没有分析数据。',
			noAdditionalScopesConfigured: '未配置附加作用域。',
			noArgs: '无参数',
			noAuthorizationRules: '没有授权规则',
			noAudienceRestrictionConfigured: '未配置受众限制。',
			noBackendsConfigured: '没有配置后端。',
			noCatalogMatchesCustomModelNamesAreAllowed: '目录中没有匹配项，可使用自定义模型名称。',
			noConfiguredModels: '没有已配置的模型',
			noCostCatalogsConfigured: '未配置成本目录',
			noCustomCosts: '没有自定义成本。',
			noGatewaySurfacesEnabledYet: '尚未启用网关功能入口',
			noGatewaysConfigured: '尚未配置网关',
			noGuardsConfigured: '尚未配置防护规则。',
			noHealthPolicyConfigured: '未配置健康策略',
			noHeaderConditions: '没有请求头条件。',
			noHostConfigured: '未配置主机',
			noLegacyBindsConfigured: '未配置旧绑定',
			noListenersAreAttachedToThisBindInTheRuntimeDump: '在运行时转储中没有监听器附加到此绑定。',
			noListenersArePresentInTheActiveGatewayDump: '活动网关转储中不存在监听器。',
			noListenersOnThisBind: '此绑定上没有监听器',
			noLlmCallsMatchTheCurrentFilters: '没有符合当前筛选条件的 LLM 调用。',
			noMatches: '没有匹配项',
			noMcpGuardrailProcessors: '没有 MCP 防护处理器',
			noMcpMethodsConfigured: '尚未配置 MCP 方法。',
			noMcpServers: '没有 MCP 服务器',
			noMcpServersConfigured: '未配置 MCP 服务器',
			noMcpToolsAreAvailableFromTheMcpGateway: 'MCP 网关没有可用工具。',
			noMessagesYet: '还没有消息。',
			noModels: '没有模型',
			noModelsConfigured: '尚未配置模型',
			noPolicyFields: '没有策略字段',
			noPoliciesConfigured: '尚未配置策略',
			noPromptCachingConfigured: '未配置提示词缓存',
			noProviderCredentialConfigured: '未配置提供商凭据。',
			noQueryConditions: '没有查询条件。',
			noResponseYet: '尚无响应',
			noSystemPrompt: '没有系统提示词',
			noRoutesArePresentInTheActiveGatewayDump: '活动网关转储中不存在路由。',
			noRuntimeListeners: '没有运行时监听器',
			noRuntimeRoutes: '没有运行时路由',
			noRuntimeTrafficConfiguration: '没有运行时流量配置',
			noSchemaPropertiesAreAvailableForThisPolicyObject: '此策略对象没有可用的架构属性。',
			noSourceConfigured: '未配置来源。',
			noSharedProvidersConfigured: '尚未配置共享提供商',
			noToolsReturned: '未返回任何工具',
			noTopLevelPolicies: '没有顶层策略',
			noTopLevelPoliciesArePresentInTheActiveGatewayDump: '活动网关转储中不存在顶级策略。',
			noTrafficGatewaysConfigured: '尚未配置流量网关',
			noTrafficRoutesConfigured: '尚未配置流量路由',
			noValuesConfigured: '未配置任何值。',
			noValuesFound: '未找到值。',
			noVirtualApiKeys: '没有虚拟 API 密钥',
			none: '无',
			none_deku7v: '无',
			noneAdminInterfaceOnly: '无（仅限管理界面）',
			notEnabled: '未启用',
			notInitialized: '未初始化',
			numberOfTokensAddedToTheLocalBucketEachFillInterval: '每个填充间隔向本地令牌桶补充的令牌数。',
			oauthClientIdAdvertisedToMcpClientsWhenNeeded: '需要时向 MCP 客户端公布的 OAuth 客户端 ID。',
			oauth2ClientIdentifierUsedForAuthorizationAndTokenExchange:
				'用于授权和令牌交换的 OAuth 2.0 客户端标识符。',
			oauth2ClientSecret: 'OAuth 2.0 客户端密钥',
			oauth2ClientSecretUsedForTokenExchange: '用于令牌交换的 OAuth 2.0 客户端密钥。',
			of3Enabled: '已启用 3 个',
			off: '关闭',
			onboardProviderBackedModelsAndConfigureModelSpecificBehavior:
				'接入提供商模型并配置模型级行为。',
			onlyQueryForAIpv4Records: '仅查询 A（IPv4）记录。',
			onlyQueryForAaaaIpv6Records: '仅查询 AAAA（IPv6）记录。',
			onlyTheFinalConditionalTargetCanOmitACondition: '只有最终的条件目标可以省略条件。',
			open: '打开',
			openAListenerSocketOnTheBindSAddressTheNormalBehavior:
				'在绑定地址上打开监听器套接字（正常行为）。',
			openClaudeDesktopAndEnableDeveloperMode: '打开 Claude Desktop 并启用开发者模式：',
			openInPlayground: '在演练场中打开',
			openAiEmbeddings: 'OpenAI /embeddings',
			openAiRealtimeWebsockets: 'OpenAI /realtime（WebSocket）',
			openAiResponses: 'OpenAI /responses',
			openAiV1ChatCompletions: 'OpenAI /v1/chat/completions',
			openAiV1Models: 'OpenAI /v1/models',
			openAiJavaScriptSdk: 'OpenAI JavaScript SDK',
			openAiModeration: 'OpenAI 内容审核',
			openAiPythonSdk: 'OpenAI Python SDK',
			openingValue: '正在打开 {{value}}',
			operation: '操作',
			operations: '操作',
			optional: '可选',
			optional_1yfbac9: '可选',
			optionalAwsStsRoleToAssumeBeforeSigningRequests: '签署请求前可以选择代入的 AWS STS 角色。',
			optionalBearerToken: '可选的 `Bearer` 令牌',
			optionalCelExpressionsForPopulatingUserAndGroupAttributesInDatabaseLogsIfNotSetA_1qxb9rt:
				'用于填充数据库日志中的用户和组属性的可选 CEL 表达式。如果未设置，将使用默认值。',
			optionalCelFilterWithKeepSemanticsWhenSetOnlyRequestsForWhichTheExpressionEvalua_1o212j0:
				'具有 KEEP 语义的可选 CEL 过滤器。设置后，仅导出表达式求值结果为 `true` 的请求追踪跨度，其余跨度会被丢弃。未设置时不进行过滤，即导出所有已采样跨度。过滤在采样后执行，只对已采样跨度求值。其行为与 `accessLog.filter` 的 KEEP 语义一致：`true` 表示保留。字段缺失或出错时，求值结果为 `false`，因此会丢弃该跨度（失败关闭）。',
			optionalCipherSuiteAllowlistOrderIsPreserved: '可选的密码套件白名单（保留顺序）。',
			optionalDiscoveryDocumentOverrideIfOmittedDiscoveryUsesIssuerWellKnownOpenidConfiguration:
				'可选的发现文档覆盖。如果省略，发现将使用\n`${issuer}/.well-known/openid-configuration`。',
			optionalMetadataAttachedToRequestsAuthenticatedWithThisKey:
				'附加到使用此密钥进行身份验证的请求的可选元数据。',
			optionalOauthClientId: '可选的 OAuth 客户端 ID',
			optionalPathOverrideForThisSpecificUpstreamFormat: '此特定上游格式的可选路径覆盖。',
			optionalPerPolicyOverrideForClientSamplingIfSetOverridesGlobalConfigForRequestsT_9my5ce:
				'客户端采样的可选按策略覆盖项。设置后，会覆盖使用此前端策略的请求的全局配置。',
			optionalPerPolicyOverrideForRandomSamplingIfSetOverridesGlobalConfigForRequestsT_121cxle:
				'随机采样的可选按策略覆盖项。设置后，会覆盖使用此前端策略的请求的全局配置。',
			optionalToken: '可选令牌',
			optional06DefaultIs2: '可选，取值范围为 0 到 6，默认为 2。',
			optionalDefaultsToHttpLocalhost11434V1: '可选，默认为 `http://localhost:11434/v1`。',
			optionalDefaultsToOmniModerationLatest: '可选，默认为 `omni-moderation-latest`。',
			optionalDefaultsToUsCentral1: '可选，默认为 `us-central1`。',
			optionalIfUnsetVertexUsesGlobal: '可选；未设置时，Vertex 将使用 `global` 端点。',
			optionalLeaveUnsetToUseTheGatewayDefault: '可选；不设置即可使用网关默认值。',
			optionsForSendingTheRequestBodyToTheAuthorizationService: '向授权服务发送请求体的选项。',
			or: '或',
			orderedListOfPolicyProcessorsAppliedToMatchedMethodsTheFirstToRejectARequestShor_wabfd4:
				'应用于匹配方法的策略处理器的有序列表；第一个\n拒绝请求会使链短路。处理器可以运行在\n请求方或响应方，或两者；请参阅 `Processor.methods`。',
			other: '其他',
			otlpHttpPathUsedToExportLogs: '用于导出日志的 OTLP HTTP 路径。',
			otlpHttpPathUsedToExportTraces: '用于导出跟踪的 OTLP HTTP 路径。',
			otlpLogExportSettings: 'OTLP 日志导出设置。',
			otlpPathDefaultIsV1Traces: 'OTLP 路径，默认为 `/v1/traces`。',
			otlpProtocolUsedToExportLogs: '用于导出日志的 OTLP 协议。',
			otlpProtocolUsedToExportTracesDefaultsToHttp: '用于导出跟踪的 OTLP 协议。默认为 HTTP。',
			otlpSpecificAccessLogFieldsIfUnsetTheParentAccessLogFieldsAreUsed:
				'OTLP 特定的访问日志字段。如果未设置，则使用父访问日志字段。',
			outgoingModel: '传出模型',
			output: '输出',
			overrideOpenAiBaseUrl: '覆盖 OpenAI 基础 URL',
			overrideRequestValues: '覆盖请求值',
			overrideTheDefaultBasePathPrefixForThisProvider: '覆盖此提供商的默认基本路径前缀。',
			overrideTheUpstreamHostForThisProvider: '覆盖此提供商的上游主机。',
			overrideTheUpstreamPathForThisProvider: '覆盖此提供商的上游路径。',
			overrideWhereThisPolicyReadsTheJwtFrom: '覆盖此策略读取 JWT 的位置。',
			overridesAllowsSettingValuesForTheRequestOverridingAnyExistingValues:
				'`overrides` 用于设置请求值，并覆盖已有值。',
			overridesYaml: '覆盖 YAML',
			packAsBytes: '打包为字节',
			paramsCustomizesParametersForOutgoingRequestsThatUseThisProvider:
				'`params` 用于自定义使用此提供商的传出请求参数。',
			paramsCustomizesParametersForTheOutgoingRequest: '`params` 用于自定义传出请求的参数。',
			passThroughTheRequestWhileExtractingLlmTelemetryAndRateLimitInputsWhenPossible:
				'尽可能提取 LLM 遥测和速率限制输入，同时透传请求。',
			passThroughTheRequestWithoutInterpretingItAsLlmTraffic: '透传请求，不将其解析为 LLM 流量。',
			passthroughControlsHowRequestsAreHandledByDefaultRequestsWillBeParsedAndTranslat_1kocxkq:
				'`passthrough` 控制请求的处理方式。默认情况下，系统会根据需要解析并转换请求；启用透传后，请求不会被修改，但可选择使用 `detect` 进行检查。在此模式下，请求必须采用提供商的原生格式。',
			pasteAJwksDocumentDirectlyIntoThePolicy: '将 JWKS 文档直接粘贴到策略中。',
			path: '路径',
			pathExpression: '路径表达式',
			pathMatch: '路径匹配',
			pathRewriteToApplyBeforeForwardingTheRequest: '在转发请求之前应用路径重写。',
			pathRewriteToApplyToTheRedirectUrl: '应用于重定向 URL 的路径重写。',
			pemEncodedPrivateSigningKeyRsaOrEcMatchingAlg:
				'PEM 编码的私有签名密钥（RSA 或 EC，匹配 `alg`）。',
			permissive: '宽松',
			permitBrowserCredentialsOnCorsRequests: '允许浏览器凭据用于 CORS 请求',
			permitMatchingRequests: '允许匹配请求。',
			phone: '电话',
			phoneNumberPattern: '电话号码模式。',
			plan: '计划',
			platformTeam: '平台团队',
			playground: '演练场',
			playgroundRequestFailed: '演练场请求失败',
			pointThePythonSdkAtTheGatewayListener: '将 Python SDK 指向网关监听器。',
			policies: '策略',
			policies_raqot3: '策略',
			policiesDefinesAdditionalPoliciesThatCanBeAttachedToVariousOtherConfigurationsTh_1vsrjcq:
				'`policies` 定义可附加到其他各类配置的额外策略。这是一项高级功能；通常应使用路由或网关下的内联 `policies` 字段。',
			policiesDefinesPoliciesForHandlingIncomingRequestsBeforeAModelIsSelected:
				'`policies` 定义在选择模型前处理传入请求的策略。',
			policiesDefinesRouteLevelPoliciesForTheUiAndRequiredUiApiRoutes:
				'`policies` 定义 UI 及其必要 API 路由使用的路由级策略。',
			policyModeIsValue: '策略模式为 {{value}}',
			priority_one: '{{count}} 个优先级',
			priority_other: '{{count}} 个优先级',
			priorityTargetSeparator: '{{value}}，{{value}}',
			policyNameUsedWhenAttachingThisPolicyToATarget: '将此策略附加到目标时使用的策略名称。',
			policySettingsToApplyToTheSelectedTarget: '应用到选定目标的策略设置。',
			policyState: '策略状态',
			policyYaml: '策略 YAML',
			port: '端口',
			portValue: '端口 {{value}}',
			portDefinesThePortToServeTheLlmRoutesUnderDeprecatedUseGatewaysInstead:
				'`port` 定义用于提供 LLM 路由服务的端口。该字段已弃用，请改用 `gateways`。',
			portIsThePortToListenOnForThisGateway: '`port` 是该网关监听的端口。',
			portMustBeBetween1And65535: '端口必须介于 1 和 65535 之间。',
			portToBindOnOmitItForAnInternalWildcardBindWhichServesAnyDestinationPortViaInPro_1nj7ohf:
				'要绑定的端口。内部通配符绑定通过进程内路由服务任意目标端口，因此可以省略此项。除非 `mode` 为 `internal`，否则必须指定数字端口。',
			prefixMode: '前缀模式',
			prefixOnlyWhenNeededToAvoidToolNameConflicts: '仅在需要时使用前缀以避免工具名称冲突。',
			prefixToRemoveFromTheHeaderValueBeforeValidationSuchAsBearerOrBasic:
				'验证前要从请求头值中移除的前缀，例如 `Bearer ` 或 `Basic `。',
			prefixForwardingTheRemainingModelAsIs: '作为前缀匹配，并按原样转发余下的模型名称。',
			preparingRequest: '正在准备请求',
			preserveMcpSessionsSoTargetsCanKeepPerSessionContext:
				'保留 MCP 会话，以便目标可以保留每个会话的上下文。',
			preserveOriginalHttp1RequestHeaderCasingWhenEncodingResponsesOnTheSameConnection:
				'在同一连接上对响应进行编码时，保留原始 HTTP/1 请求头大小写。',
			primary: '主要',
			primaryDatabaseUsedByLocalRuntimeFeatures: '本地运行时功能使用的主数据库。',
			priorityGroupsTargetsForFailoverLowerValuesArePreferred:
				'按优先级对故障转移目标分组；数值越小，优先级越高。',
			privateKeyFileForTheClientCertificate: '客户端证书的私钥文件。',
			processingBehavior: '处理行为',
			processor: '处理器',
			processorsRunInOrderTheFirstRejectionStopsTheRequest:
				'处理器按顺序运行；第一次拒绝会停止请求。',
			projectId: '项目 ID',
			projectLinks: '项目链接',
			promptAndResponseGuardrailsToApplyToLlmTraffic: '应用于 LLM 流量的提示词和响应防护规则。',
			promptCaching: '提示词缓存',
			promptCachingSettingsForProvidersThatSupportCacheMarkers:
				'支持缓存标记的提供商所使用的提示词缓存设置。',
			promptLoggingIsOff: '提示词日志记录已关闭',
			promptCachingConfiguresCachePointInsertionForSupportedLlmProviders:
				'`promptCaching` 用于为支持缓存标记的 LLM 提供商配置缓存点插入。',
			protectedResourceMetadata: '受保护的资源元数据',
			protectedResourceMetadataReturnedToMcpClients: '返回给 MCP 客户端的受保护资源元数据。',
			protocol: '协议',
			protocolControlsWhetherThisGatewayAcceptsHttpHttpsRoutesOrTcpTlsRoutesWhenOmitte_122yt2l:
				'`protocol` 控制此网关接受 HTTP/HTTPS 路由还是 TCP/TLS 路由。省略时默认为 HTTP；设置 `tls` 后默认为 HTTPS。',
			protocolControlsWhetherThisListenerAcceptsHttpHttpsRoutesOrTcpTlsRoutesWhenOmitt_198kbon:
				'`protocol` 控制此监听器接受 HTTP/HTTPS 路由还是 TCP/TLS 路由。省略时默认为 HTTP；设置 `tls` 后默认为 HTTPS。',
			protocolUsedToCallTheAuthorizationServiceUseGRpcUnlessTheServiceOnlySupportsHttp:
				'用于调用授权服务的协议。除非服务仅支持 HTTP，否则请使用 gRPC。',
			provider: '提供商',
			providerApiKey: '提供商 API 密钥',
			providerIdentityForCostCatalogLookupAndTelemetryBuiltInNamedProvidersCohereMistr_1c2sljq:
				'用于成本目录查找和遥测的提供商标识。内置命名提供商（如 `cohere`、`mistral`）会设置此项，使其成本按正确的目录键解析；自定义提供商也可将其设为匹配目录条目的值。未设置时回退为 `custom`。',
			providerMetadata: '提供商元数据',
			providerName: '提供商名称',
			providerOfTheLlmWeAreConnectingTo: '所连接的 LLM 提供商。',
			providerOfTheLlmWeAreConnectingToo: '所连接的 LLM 提供商',
			providerReturned: '提供商响应',
			provider_1k5qy2a: '提供商：',
			providers: '提供商',
			providersDefinesReusableLlmProviderDefaultsThatModelsMayReference:
				'`providers` 定义可由模型引用的可复用 LLM 提供商默认配置。',
			provisionIncomingCredentialsAndMetadataForCallers: '为调用方提供传入凭据和元数据。',
			proxyBackendUsedToTunnelTheConnection: '用于建立隧道连接的代理后端。',
			proxyProtocolVersionsAcceptedFromDownstreamClients: '下游客户端可使用的 PROXY 协议版本。',
			publicModelsCanBeRequestedDirectlyByClientsAndAreIncludedInTheModelList:
				'公开模型可由客户端直接请求，并会包含在模型列表中。',
			publicUiGateway: '公开 UI 网关',
			query: '查询',
			queryForBothAAndAaaaRecordsInParallelAndUseAllResults:
				'并行查询 A 和 AAAA 记录并使用所有结果。',
			queryForBothAAndAaaaButPreferIpv4AddressesWhenBothAreAvailable:
				'查询 A 和 AAAA；当两者均可用时优先使用 IPv4 地址。',
			queryName: '查询参数名称',
			queryParameterNameContainingTheCredential: '包含凭证的查询参数名称。',
			queryValue: '查询参数值',
			quickRanges: '快速范围',
			rateLimitDomainSentToTheRemoteRateLimitService: '发送到远程速率限制服务的速率限制域。',
			rawApiKey: '原始 API 密钥',
			rawConfiguration: '原始配置',
			rawConfigurationDiff: '原始配置差异',
			rawGuardYaml: '原始防护规则 YAML',
			rawJson: '原始 JSON',
			rawLogJson: '原始日志 JSON',
			rawValue: '原始值',
			readSigningKeysFromAFileOnTheGatewayHost: '从网关主机上的文件读取签名密钥。',
			readTheCredentialFromACelExpressionEvaluatedAgainstTheIncomingRequestCelExpressi_nxzl9m:
				'从针对传入请求求值的 CEL 表达式中读取凭据。该表达式必须返回凭据字符串。此位置只能提取凭据，不能插入凭据。',
			readTheCredentialFromARequestCookie: '从请求 Cookie 中读取凭据。',
			readTheCredentialFromAUrlQueryParameter: '从 URL 查询参数读取凭据。',
			readTheCredentialFromAnHttpHeader: '从 HTTP 请求头读取凭据。',
			readOnlyListenerInventoryFromTheActiveGatewayDump: '活动网关转储中的只读监听器清单。',
			readOnlyRouteInventoryFromTheActiveGatewayDump: '活动网关转储中的只读路由清单。',
			readOnlyTopLevelPoliciesFromTheActiveGatewayDump: '活动网关转储中的只读顶级策略。',
			readinessProbeServerAddressInTheFormatIpPortLocalhostPortUnixPathToSocketOrOff:
				'就绪探针服务器地址，格式为 `ip:port`、`localhost:port`、`unix:/path/to/socket` 或 `off`。',
			readonlyMode: '只读模式',
			readonlyPoliciesUnavailable: '只读策略不可用',
			ready: '就绪',
			realmShownInTheWwwAuthenticateResponseHeaderWhenCredentialsAreMissingOrInvalid:
				'凭据缺失或无效时，`WWW-Authenticate` 响应头中显示的 realm 值。',
			reasoning: '推理',
			recentCalls: '最近调用',
			redirectExpression: '重定向表达式',
			redirectUri: '重定向 URI',
			reference: '引用',
			refresh: '刷新',
			refreshBaseCosts: '刷新基础成本',
			refreshTheBaseCatalogToAddPricingDataFromModelsDev:
				'刷新基础目录，以添加来自 models.dev 的定价数据。',
			refreshing: '正在刷新…',
			regex: '正则表达式',
			regexOrBuiltInPatternsToEvaluate: '要评估的正则表达式或内置模式。',
			regularExpressionPatternToEvaluate: '要评估的正则表达式模式。',
			rejectHttpConnectRequests: '拒绝 HTTP CONNECT 请求。',
			rejectMatchingRequests: '拒绝匹配的请求。',
			rejectRequest: '拒绝请求',
			rejectRequestsThatDoNotCarryAValidToken: '拒绝不携带有效令牌的请求。',
			rejectTheRequestOrResponseWhenContentMatches: '当内容匹配时拒绝请求或响应。',
			rejectTheRequestWhenADetectorMatches: '当检测器匹配时拒绝请求。',
			rejectTheRequestWhenARegexMatches: '当正则表达式匹配时拒绝请求。',
			rejectTheRequestWhenTheExternalProcessingServiceFails: '当外部处理服务失败时拒绝请求。',
			rejectTheRequestWhenTheWebhookGuardrailIsUnavailableDefault:
				'当 Webhook 防护规则不可用时拒绝请求（默认）。',
			rejectWhenTheProcessorIsUnavailable: '当处理器不可用时拒绝。',
			rejectWhenTheWebhookIsUnavailableOrErrors: '当 Webhook 不可用或出现错误时拒绝。',
			rejectionBody: '拒绝响应正文',
			rejectionStatus: '拒绝状态',
			reloadVsCodeAndTestCopilotSuggestionsOrChat: '重新加载 VS Code 并测试 Copilot 建议或聊天。',
			remoteRateLimit: '远程速率限制',
			remoteRateLimitChecksForIncomingRequests: '对传入请求的远程速率限制检查。',
			remoteRateLimitServiceAndDomainUsedWhenBuildingDescriptorChecks:
				'构建描述符检查时使用的远程速率限制服务和域。',
			remoteUrl: '远程 URL',
			remove: '移除',
			removeValue: '移除 {{value}}',
			removeAllLlmGuardrails: '移除所有 LLM 防护规则？',
			removeAllRequestAndResponseGuardrailsLlmTrafficWillNoLongerBeCheckedByTheseRules:
				'移除所有请求和响应防护规则？LLM 流量将不再由这些规则检查。',
			removeBackend: '移除后端',
			removeConditionalTarget: '移除条件目标',
			removeCustomCost: '移除自定义成本',
			removeDescriptor: '移除描述符',
			removeDescriptorEntry: '移除描述符条目',
			removeFailoverGroupValue: '移除故障转移组 {{value}}',
			removeGuardrail: '移除防护规则',
			removeGuardrail_1r9af69: '移除防护规则？',
			removeGuardrails: '移除防护规则',
			removeHeaderCondition: '移除请求头条件',
			removeHeaders: '移除请求头',
			removeResponseHeaders: '移除响应头',
			removeMatchValue: '移除匹配条件 {{value}}',
			removePattern: '移除模式',
			removeQueryCondition: '移除查询条件',
			removeTarget: '移除目标',
			removeThe: '移除',
			removeTheApiKeyPolicyEntirelyRequestsWillNotBeValidatedAgainstVirtualApiKeys:
				'完全移除 API 密钥策略。不会根据虚拟 API 密钥验证请求。',
			replaceMatchedContentAndContinue: '替换匹配的内容并继续。',
			replaceMatchingContentWithMaskedText: '用屏蔽文本替换匹配的内容。',
			replaceOnlyTheHostAndPreserveTheEffectivePort: '仅替换主机并保留有效端口。',
			replaceOnlyTheMatchedPathPrefix: '仅替换匹配的路径前缀。',
			replaceOnlyThePort: '仅替换端口。',
			replaceTheFullAuthorityIncludingHostAndOptionalPort:
				'替换完整的 `:authority`，包括主机和可选端口。',
			replaceTheFullRequestPath: '替换完整的请求路径。',
			request: '请求',
			request_1058hua: '请求',
			requestAttributes: '请求属性',
			requestBody: '请求正文',
			requestBodyValuesComputedFromCelExpressions: '根据 CEL 表达式计算的请求正文值。',
			requestBodyValuesThatReplaceClientProvidedValues: '替换客户端提供值的请求正文值。',
			requestContextYaml: '请求上下文 YAML',
			requestDetail: '请求详情',
			requestExtraOauth2ScopesTheGatewayAlwaysIncludesOpenid:
				'请求额外的 OAuth 2.0 作用域。网关始终包含 `openid`。',
			requestGuards: '请求防护规则',
			requestHeaders: '请求头',
			requestHeadersToSendToTheAuthorizationServiceIfUnsetGRpcSendsAllRequestHeadersAn_136gzan:
				'发送到授权服务的请求头。\n如果未设置，gRPC 会发送所有请求头，而 HTTP 仅发送 `Authorization`。',
			requestInProgress: '请求进行中',
			requestLogIdentity: '请求日志身份',
			requestOriginsThatReceiveCorsResponseHeadersUseToMatchAnyOrigin:
				'接收 CORS 响应头的请求来源。使用 `*` 匹配任何来源。',
			requestProgress: '请求进度',
			requestTrailers: '请求尾部字段',
			requestTransformations: '请求转换',
			requestHeadersModifiesHeadersInRequestsToTheLlmProvider:
				'`requestHeaders` 用于修改发送给 LLM 提供商的请求头。',
			requests: '请求数',
			requestsAreNeverRejectedThisIsUsefulForUsageOfClaimsInLaterStepsAuthorizationLog_etyjeb:
				'请求永远不会被拒绝。这对于在后续步骤（授权、日志记录等）中使用声明非常有用。\n警告：这允许不带 JWT 令牌的请求！此外不会返回 401 错误，因此不会触发客户端启动 OAuth 流程。',
			require: '要求',
			requireAProxyProtocolHeaderOnEachConnection: '每个连接上都需要 PROXY 协议请求头。',
			requireAValidApiKey: '需要有效的 API 密钥。',
			requireAValidJwtFromAConfiguredIssuer: '需要来自配置的签发者的有效 JWT。',
			requireAValidUsernameAndPassword: '需要有效的用户名和密码。',
			requireTheSelectedDestinationToMatchAgentgatewaySLocalServiceEndpoints:
				'要求所选目标与 agentgateway 的本地服务端点匹配。',
			requireThisCelExpressionToBeTrue: '要求此 CEL 表达式的求值结果为 `true`。',
			requireThisExpressionToBeTrue: '要求此表达式的求值结果为 `true`。',
			requiredClaims: '所需声明',
			reset: '重置',
			resource: '资源',
			resourceAttributesToAddToTheTracerProviderOtelResourceThisCanBeUsedToSetThingsLi_k3nt2h:
				'要添加到追踪提供程序（OTel `Resource`）的资源属性，可用于动态设置 `service.name` 等值。',
			resourceMetadataYaml: '资源元数据 YAML',
			response: '响应',
			response_nrnldq: '响应',
			responseAttributes: '响应属性',
			responseBody: '响应正文',
			responseBodyReturnedWhenContentIsRejected: '内容被拒绝时返回的响应正文。',
			responseCacheConfigurationDefaultsToAnInMemoryCacheWith8192EntriesAndA300sTtlWhe_12crnmm:
				'响应缓存配置。默认使用最多包含 8192 个条目的内存缓存；令牌端点省略 `expires_in` 时，TTL 默认为 300 秒。将 `maxEntries` 设为 0 可禁用缓存。',
			responseGuards: '响应防护规则',
			responseHeaders: '响应头',
			responseHeadersComputedFromCelExpressions: '根据 CEL 表达式计算的响应头。',
			responseReturnedWhenTheLlmResponseIsRejected: 'LLM 响应被拒绝时返回的响应。',
			responseReturnedWhenTheRequestIsRejected: '请求被拒绝时返回的响应。',
			responseTrailers: '响应尾部字段',
			responseTransformations: '响应转换',
			responseHeadersModifiesHeadersInResponsesFromTheLlmProvider:
				'`responseHeaders` 用于修改 LLM 提供商返回的响应头。',
			restoreHealth: '恢复健康',
			restrictAcceptedMcpTokensByIssuerAndAudience: '按签发者和受众限制可接受的 MCP 令牌。',
			restrictAcceptedTokensByIssuerAudienceAndRequiredClaims:
				'按签发者、受众和必需声明限制可接受的令牌。',
			result: '结果',
			resultingYaml: '生成的 YAML',
			retryMatchingFailedUpstreamRequests: '重试匹配失败的上游请求。',
			returnAConfiguredResponseInsteadOfForwardingTheRequest: '返回配置的响应而不是转发请求。',
			returnARedirectResponseInsteadOfForwardingTheRequest: '返回重定向响应而不是转发请求。',
			returnARedirectResponseInsteadOfForwardingToThisBackend: '返回重定向响应而不是转发到此后端。',
			reviewMigration: '查看迁移',
			rewriteAllRequestsToThisAdminApiPathPreservingTheOriginalQueryString:
				'重写对此管理 API 路径的所有请求，保留原始查询字符串。',
			rewriteTheRequestPathOrAuthorityBeforeForwarding: '转发前重写请求路径或 `authority`。',
			rfc7523TheSubjectTokenIsSentAsTheAssertion: 'RFC 7523；主题令牌作为 `assertion` 发送。',
			rfc8693ActorTokenTypeUrnWhenOmittedDefaultsToAccessTokenAndIsStillSent:
				'RFC 8693 参与者令牌类型 URN；省略时默认为 `access_token`，但仍会发送。',
			rfc8693DelegationActorTokenTokenExchangeGrantOnly:
				'RFC 8693 委托参与者令牌。仅用于令牌交换授权。',
			rfc8693TokenExchangeTheSubjectTokenIsSentAsSubjectToken:
				'RFC 8693 令牌交换；主题令牌以 `subject_token` 形式发送。',
			rfc8693TokenTypeUrnWhenOmittedDefaultsToAccessToken:
				'RFC 8693 令牌类型 URN；省略时默认为 `access_token`。',
			rootCertificateBundleUsedToVerifyTheBackendCertificate: '用于验证后端证书的根证书包。',
			routeClaudeDesktopThirdPartyInferenceThroughTheGateway:
				'通过网关路由 Claude Desktop 第三方推理。',
			routeFormats: '路由格式',
			routeGroup: '路由组',
			routeHttpConnectRequestsThroughNormalRouteMatching:
				'通过正常的路由匹配来路由 HTTP CONNECT 请求。',
			routePolicies: '路由策略',
			routeProtocolFamily: '路由协议族。',
			routeRequestsThroughAnEndpointPickerBeforeForwardingToThisBackend:
				'在转发到此后端之前，通过端点选择器为请求选择端点。',
			routeToTheInProcessAdminServiceInsteadOfANetworkUpstream:
				'路由到进程内管理服务而不是网络上游。',
			routeTypeOverridesSelectedByRequestPathSuffix: '根据请求路径后缀选择的路由类型覆盖项。',
			routeWindsurfTrafficThroughTheGatewayHttpProxySetting:
				'通过网关 HTTP 代理设置路由 Windsurf 流量。',
			routeYaml: '路由 YAML',
			routeGroupsProvidesASetOfRouteGroupsUsedForRouteDelegationThisIsAnAdvancedFeatur_12ntlx8:
				'`routeGroups` 提供一组用于路由委派的路由组。这是一项高级功能，主要用于测试。',
			routes: '路由',
			routes_14u6307: '路由',
			routes_4p3286: '路由 ·',
			routesDefinesHttpRoutesAttachedToOneOrMoreNamedGateways:
				'`routes` 定义附加到一个或多个命名网关的 HTTP 路由。',
			routing: '路由',
			routingSelectsAnExistingLlmModelBackendForEachRequest:
				'`routing` 会为每个请求选择现有的 LLM 模型后端。',
			routingStrategy: '路由策略',
			rule: '规则',
			run: '运行',
			runAfterTheMcpResponseIsAvailable: 'MCP 响应可用后运行。',
			runBeforeForwardingTheMcpRequest: '在转发 MCP 请求之前运行。',
			runWithRequestAndResponseContext: '使用请求和响应上下文运行。',
			runtimeTraffic: '运行时流量',
			safety: '安全',
			save: '保存',
			saveFailed: '保存失败',
			savePolicy: '保存策略',
			saveUiGateway: '保存 UI 网关',
			schemeToUseInTheRedirectUrlSuchAsHttpOrHttps:
				'在重定向 URL 中使用的方案，例如 `http` 或 `https`。',
			scopes: '作用域',
			sdkSnippetsUseThisUrlWithV1Appended: 'SDK 代码片段使用此 URL，并在末尾附加 `/v1`。',
			searchValue: '搜索 {{value}}',
			searchFor: '搜索',
			secretValueToSendToTheBackend: '要发送到后端的机密值。',
			security: '安全性',
			selectAConcreteModel: '请选择具体模型',
			selectAGateway: '选择网关。',
			selectAListener: '选择监听器。',
			selectGuardType: '选择防护规则类型',
			selectOrTypeAModel: '选择或输入模型',
			selectProvider: '选择提供商',
			selectsHowAnInternalBackendMapsProxyRequestsToTheAdminApi:
				'选择内部后端如何将代理请求映射到管理 API。',
			selectsWhichRfcTheRequestFollowsDefaultsToTokenExchangeRfc8693:
				'选择请求遵循哪个 RFC；默认为令牌交换（RFC 8693）。',
			send: '发送',
			sendABoundedBodyBufferAndAllowTruncation: '发送有界正文缓冲区，并允许截断。',
			sendAConfiguredSecretValueToTheBackend: '将配置的机密值发送到后端。',
			sendACopyOfMatchingRequestsToAnotherBackend: '将匹配请求的副本发送到另一个后端。',
			sendARealChatCompletionRequestThroughTheConfiguredGatewayForSetupDebugging:
				'通过已配置的网关发送真实的聊天补全请求，用于调试设置。',
			sendContentToAnExternalGuardrailService: '将内容发送到外部防护规则服务。',
			sendHeadersToTheExternalProcessingService: '将请求头发送到外部处理服务。',
			sendRequestAndResponseDataToAnExternalProcessingService:
				'将请求和响应数据发送到外部处理服务。',
			sendTheRequestToTheUpstreamLlmProviderAsIs: '将请求原样发送到上游 LLM 提供商',
			sendTheRequestToTheUpstreamLlmProviderAsIsButAttemptToExtractInformationFromItAn_a091kz:
				'按原样将请求发送到上游 LLM 提供商，但尝试从中提取信息\n并应用一部分策略（速率限制和遥测；无防护规则）。',
			sendThisPhaseToTheExternalProcessor: '将此阶段发送到外部处理器。',
			sendTrailersToTheExternalProcessingService: '将尾部字段发送到外部处理服务。',
			sending: '正在发送',
			sendingChatCompletion: '正在发送聊天补全请求',
			sendingToolResults: '正在发送工具结果',
			serverName: '服务器名称',
			serverNameToUseForTlsVerificationAndSni: '用于 TLS 验证和 SNI 的服务器名称。',
			servers: '服务器',
			serversToolsAndMcpPlaygroundFlows: '服务器、工具和 MCP 演练场流程。',
			service: '服务',
			serviceReferenceServiceMustBeDefinedInTheTopLevelServicesList:
				'服务引用。服务必须在顶级服务列表中定义。',
			servicesDefinesTheSetOfServicesThatTheProxyCanRouteToTheseConsistOfWorkloadsThis_9pwt7w:
				'`services` 定义代理可以路由到的服务集合，这些服务由 `workloads` 组成。这是一项高级功能，主要用于测试；通常优先使用路由上的内联 `backends` 和策略。',
			session: '会话',
			sessionTagsPassedToStsAssumeRoleForCostAttributionOnceActivatedAsCostAllocationT_1ce6dym:
				'传递给 STS AssumeRole 的会话标签，用于成本归因。标签激活为成本分配标签后，会显示在 AWS 成本和使用情况报告的 `resourceTags/user:TagKey` 下。标签值可以是静态值（`value`），也可以是针对每个请求求值的 CEL 表达式（`expression`）。',
			sessionTokenOptional: '会话令牌（可选）',
			setHeaders: '设置请求头',
			setResponseHeaders: '设置响应头',
			setRequestTimeoutLimits: '设置请求超时限制。',
			setTheProxyUrlTo: '将代理 URL 设置为',
			setUpGateways: '设置网关',
			setUpListeners: '设置监听器',
			setUpModels: '设置模型',
			setUpServers: '设置服务器',
			settings: '设置',
			settingsForExportingRequestTraces: '用于导出请求追踪的设置。',
			settingsForHandlingIncomingHttpRequests: '用于处理传入 HTTP 请求的设置。',
			settingsForHandlingIncomingTcpConnections: '用于处理传入 TCP 连接的设置。',
			settingsForHandlingIncomingTlsConnections: '用于处理传入 TLS 连接的设置。',
			settingsForRequestAccessLogs: '请求访问日志的设置。',
			settingsForTemporarilyRemovingUnhealthyBackends: '用于临时移除不健康后端的设置。',
			severityThreshold: '严重性阈值',
			severityThreshold06ForFourSeverityLevelsContentAtOrAboveThisLevelIsBlockedDefault2:
				'严重性阈值。使用 `FourSeverityLevels` 时取值范围为 0 到 6；达到或超过该级别的内容会被阻止。默认为 2。',
			sha256HashOfAnApiKeyValueToAcceptInSha256HexFormat:
				'要接受的 API 密钥值的 SHA-256 哈希值，采用 `sha256:<hex>` 格式。',
			shaping: '流量整形',
			show: '显示',
			showValueOptions: '显示 {{value}} 选项',
			showFullKey: '显示完整密钥',
			signBackendRequestsWithAwsCredentials: '使用 AWS 凭证为后端请求签名。',
			signingKeys: '签名密钥',
			simpleChatCompletionMessageIsASimplifiedChatMessage:
				'`SimpleChatCompletionMessage` 表示简化的聊天消息。',
			skip: '跳过',
			skipCertificateTrustVerificationForTheBackendConnection: '跳过后端连接的证书信任验证。',
			skipFailedTargetsUpstreamsAndContinueServingFromHealthyOnesIfAllTargetsFailStillReturnAnError:
				'跳过失败的目标/上游并继续从健康的目标/上游提供服务。\n如果所有目标都失败，仍返回错误。',
			skipHostnameVerificationForTheBackendCertificate: '跳过后端证书的主机名验证。',
			skipSetup: '跳过设置',
			someExamples: '示例：',
			someListenersMixHttpAndTcpRoutes: '一些监听器混合 HTTP 和 TCP 路由',
			source: '来源',
			sourcesAreMergedInOrderLaterSourcesOverrideEarlierEntries:
				'各来源按顺序合并，后面的来源会覆盖前面来源中的条目。',
			spanAttributesToAddKeyedByAttributeName: '要添加的跨度属性，按属性名称索引。',
			specificModel: '特定模型',
			splitMixedListenersBeforeUsingTheRouteForm: '在使用路由表单之前拆分混合监听器。',
			ssn: '社会保障号码',
			standardRequestLogAttributesPopulatedForDatabaseBackedLocalRuntimeFeatures:
				'为数据库支持的本地运行时功能填充标准请求日志属性。',
			state: '状态',
			stateMode: '状态模式',
			stateful: '有状态',
			stateless: '无状态',
			static: '静态',
			staticContextValuesToSendToTheAuthorizationServiceMapsToTheContextExtensionsFieldInTheRequest:
				'要发送到授权服务的静态上下文值。\n映射到请求中的 `context_extensions` 字段。',
			staticResponseBodyEncodedAsBytes: '以字节编码的静态响应正文。',
			staticTagValue: '静态标记值。',
			statsMetricsServerAddressInTheFormatIpPortLocalhostPortUnixPathToSocketOrOff:
				'统计和指标服务器地址，格式为 `ip:port`、`localhost:port`、`unix:/path/to/socket` 或 `off`。',
			stream: '流式传输',
			streamTheBodyBidirectionallyWithTheExternalProcessingService:
				'通过外部处理服务双向传输正文。',
			streamTheFullBodyThroughTheExternalProcessor: '通过外部处理器传输完整正文。',
			streamFalse: '`stream: false`',
			streaming: '流式传输',
			strict: '严格',
			structuredContent: '结构化内容',
			systemPrompt: '系统提示词',
			tagKey: '标签键。',
			target: '目标',
			target_one: '{{count}} 个目标',
			target_other: '{{count}} 个目标',
			targetModel: '目标模型',
			targetType: '目标类型',
			targetTheVisualEditorCurrentlySupportsHostTargetsOnly:
				'目标。可视化编辑器目前仅支持主机目标。',
			targets: '目标',
			targetsAreEvaluatedInOrderTheFirstMatchingConditionSelectsTheModel:
				'目标会按顺序求值，第一个条件匹配的目标会被选中。',
			targetsAreExistingModelNamesOrNamesMatchedByWildcardModelEntries:
				'目标可以是现有模型名称，也可以是与通配符模型条目匹配的名称。',
			targetsAreGroupedByPriorityLowerPriorityValuesAreTriedFirst:
				'目标按优先级分组，并优先尝试数值较小的组。',
			tcpKeepaliveSettingsForBackendConnections: '后端连接的 TCP 保活设置。',
			tcpKeepaliveSettingsForDownstreamConnections: '下游连接的 TCP 保活设置。',
			tcpProtocolSettingsForThisBackend: '该后端的 TCP 协议设置。',
			tcpRoutesDefinesTcpRoutesAttachedToOneOrMoreNamedTcpTlsGateways:
				'`tcpRoutes` 定义附加到一个或多个具名 TCP/TLS 网关的 TCP 路由。',
			temperature02: '`temperature: 0.2`',
			templateId: '模板 ID',
			theAes256GcmSessionProtectionKeyToBeUsedForSessionTokensIfNotSetSessionsWillNotB_kosx3y:
				'用于会话令牌的 AES-256-GCM 会话保护密钥。\n如果未设置，会话将不会被加密。\n例如，通过 `openssl rand -hex 32` 生成。',
			theAzureContentSafetyEndpointHostnameEGResourceNameCognitiveservicesAzureCom:
				'Azure 内容安全端点的主机名，例如 `<resource-name>.cognitiveservices.azure.com`。',
			theAzureResourceNameUsedToConstructTheEndpointHost: '用于构造端点主机的 Azure 资源名称。',
			theFoundryProjectNameRequiredWhenResourceTypeIsFoundryUsedToConstructPathsApiPro_acq7x8:
				'Foundry 项目名称；当 `resourceType` 为 `foundry` 时必填。\n用于构造路径：`/api/projects/{projectName}/openai/v1/...`。\n这与用于主机的 `resourceName` 不同。',
			theGcpProjectId: 'GCP 项目 ID',
			theGcpRegionDefaultUsCentral1: 'GCP 区域（默认：`us-central1`）',
			theHttpEndpointClassSuchAsV1ChatCompletionsOrV1MessagesThisIsUsedBothForTheClien_pbt4i9:
				'HTTP 端点类型，例如 `/v1/chat/completions` 或 `/v1/messages`。\n\n它同时用于匹配的客户端路由和最终发送请求的上游路由。对于聊天请求，两者可能不同：客户端发起的 Anthropic `/v1/messages` 请求对应 `RouteType::Messages` 和 `InputFormat::Messages`，但转换后可能以 `RouteType::Completions` 发送到上游。\n\n`RouteType` 描述 HTTP 端点，`InputFormat` 描述解析后的客户端负载及返回给客户端的响应形状。该类型还包括 `Detect` 和 `Passthrough` 等模式。',
			theMaximumDurationToKeepAnIdleConnectionAlive: '保持空闲连接活动的最大持续时间。',
			theMaximumNumberOfConnectionsAllowedInThePoolPerHostnameIfSetThisWillLimitTheTot_2rbbla:
				'每个主机名的连接池所允许的最大连接数。设置后，会限制与任一主机保持活动的连接总数。注意：系统仍会创建超出限制的连接，但不会让这些连接保持空闲。未设置时不作限制。',
			theModelToSendToTheProviderIfUnsetTheSameModelWillBeUsedFromTheRequest:
				'要发送给提供商的模型。未设置时，使用请求中的模型。',
			theResourceAuthorizationServerWhichExchangesTheIdJagForAnAccessToken:
				'资源授权服务器，用 ID-JAG 交换访问令牌。',
			theTemplateIdForTheModelArmorConfiguration: 'Model Armor 配置的模板 ID',
			theTypeOfAzureEndpointToConnectTo: '要连接的 Azure 端点类型。',
			theTypeOfAzureEndpointDeterminesTheHostSuffix: 'Azure 端点类型，用于确定主机后缀。',
			theUniqueIdentifierOfTheGuardrail: '防护规则的唯一标识',
			theUserSIdPAuthorizationServerUsedForTheRfc8693TokenExchange:
				'用户的 IdP 授权服务器，用于 RFC 8693 令牌交换。',
			theVersionOfTheGuardrail: '防护规则的版本',
			thisCannotBeUndone_1x7m3fy: '此操作无法撤销。',
			thisConfigurationUsesLegacy: '此配置使用旧版',
			thisGuardUsesAShapeTheVisualEditorDoesNotSupportYetItWillBePreservedAsRawYaml:
				'该防护使用了可视化编辑器尚不支持的结构，将保留为原始 YAML。',
			thisPolicyUsesA: '该策略使用不受支持的目标类型：',
			thisPolicyUsesConditionalRateLimitEntriesTheVisualEditorCurrentlySupportsSimpleRateLimitsOnly:
				'此策略使用条件式速率限制条目。可视化编辑器目前仅支持简单速率限制。',
			thisToolDoesNotDeclareArguments: '该工具不声明参数。',
			timeToWaitForAnHttp2KeepalivePingResponse: '等待 HTTP/2 保活探测响应的时间。',
			timingAndUsage: '耗时与用量',
			tlsConfiguresTlsWhenConnectingToTheLlmProvider: '`tls` 用于配置连接 LLM 提供商时采用的 TLS。',
			tlsDefinesTheTlsSettingsToServeTheLlmRoutesUnderWhenUsingPortDeprecatedUseGatewaysInstead:
				'`tls` 定义使用 `port` 提供 LLM 路由服务时采用的 TLS 设置。该字段已弃用，请改用 `gateways`。',
			tlsEnablesHttpsForThisGatewayMaybeNotBeSetWithListeners:
				'`tls` 为此网关启用 HTTPS，不能与 `listeners` 同时设置。',
			tlsEnablesHttpsForThisListener: '`tls` 为此监听器启用 HTTPS。',
			tlsSettingsUsedWhenConnectingToTheBackend: '连接到后端时使用的 TLS 设置。',
			tlsSettingsUsedWhenConnectingToThisBackend: '连接到此后端时使用的 TLS 设置。',
			to: '至',
			toCaptureRequestAndResponsePayloads: '捕获请求和响应负载。',
			toTheLlmCorsPolicySoThisPlaygroundCanCallTheGatewayFromTheBrowser:
				'到 LLM CORS 策略，以便这个演练场可以从浏览器调用网关。',
			toTheMcpCorsPolicyAndExposeMcpSessionIdSoThisPlaygroundCanKeepABrowserSession:
				'MCP CORS 策略并公开 `Mcp-Session-Id`，以便该演练场保持浏览器会话。',
			toTheMcpCorsPolicySoThePlaygroundCanListAndCallMcpToolsFromTheBrowser:
				'MCP CORS 策略，以便演练场可以从浏览器列出并调用 MCP 工具。',
			toggleTheme: '切换主题',
			tokenEndpoint: '令牌端点',
			tokenEndpointAuth: '令牌端点身份验证',
			tokenEndpointClientAuthenticationMethodForExplicitProviderConfigurationDiscovery_s7q91h:
				'显式提供商配置所使用的令牌端点客户端身份验证方法。发现模式会从提供商元数据推导该值；显式模式省略此项时，默认为 `clientSecretBasic`。',
			tokenEndpointPathOnTheBackendDefaultsTo: '后端的令牌端点路径，默认为 `/`。',
			tokenEndpointUsedToExchangeTheAuthorizationCode: '用于交换授权代码的令牌端点。',
			tokenValidation: '令牌验证',
			tokens: '令牌',
			tokensPerFill: '每次填充的令牌数',
			tool: '工具',
			toolCall: '工具调用',
			toolOutput: '工具输出',
			toolPlayground: '工具演练场',
			toolResult: '工具结果',
			tools: '工具',
			toolsDiscovered: '发现的工具',
			toolsCallPromptsOr: '`tools/call`、`prompts/*` 或 `*`',
			topLevelRuntimePoliciesAreOnlyAvailableWhenTheGatewayIsRunningFromXdsConfig:
				'仅当网关从 XDS 配置运行时，顶级运行时策略才可用。',
			topLevelConfigurationSectionAlreadyExists: '顶层配置节已存在。',
			total: '总计',
			totalNumberOfAttemptsIncludingTheOriginalRequest: '尝试总数，包括原始请求。',
			traffic: '流量',
			trafficGateways: '流量网关',
			trafficListeners: '流量监听器',
			trafficOverTime: '流量趋势',
			trafficRoutes: '流量路由',
			trafficBindDeleteWithListeners_one: '这还会移除 {{count}} 个监听器及其路由。',
			trafficBindDeleteWithListeners_other: '这还会移除 {{count}} 个监听器及其路由。',
			trafficDeleteWarning: '使用 {{value}} 的流量将不再提供服务。',
			trafficShaping: '流量整形',
			trafficThatMatchesThisRouteIsForwardedToTheseTargets: '与此路由匹配的流量将转发到这些目标。',
			transformTheRequestBeforeItIsForwarded: '在转发请求之前对其进行转换。',
			transformTheResponseBeforeItIsReturned: '在返回响应之前对其进行转换。',
			transformation: '转换',
			transformationAllowsSettingValuesFromCelExpressionsForTheRequestOverridingAnyExistingValues:
				'转换允许使用 CEL 表达式为请求设置值，并覆盖任何现有值。',
			transformations: '转换',
			transport: '传输',
			treatHttpConnectRequestsAsTunnels: '将 HTTP CONNECT 请求视为隧道。',
			troubleshooting: '故障排除',
			trustTheSelectedDestinationDirectlyWithoutLocalEndpointValidation:
				'直接信任选定的目标，无需本地端点验证。',
			trustedIssuersAndTheirSigningKeys: '受信任的签发者及其签名密钥。',
			ttlUsedWhenTheTokenEndpointOmitsExpiresInDefaultsTo300s:
				'当令牌端点省略 `expires_in` 时使用的 TTL。默认为 300 秒。',
			tunnelSettingsUsedWhenConnectingToTheBackend: '连接到后端时使用的隧道设置。',
			tunnelSettingsUsedWhenConnectingToThisBackend: '连接到此后端时使用的隧道设置。',
			type: '类型',
			uSSocialSecurityNumberPattern: '美国社会安全号码模式。',
			uiAccessPolicies: 'UI 访问策略',
			uiDefinesSettingsForHowTheUiAndUiBackendIsExposedByDefaultTheUiIsExposedOnlyOnTh_ajchhz:
				'`ui` 定义 UI 及其后端的公开方式。默认情况下，UI 仅通过管理界面（通常为 `localhost:15000`）提供。此设置可将 UI 附加到 `gateways` 以对外提供服务，也可为 UI 流量附加策略。对外公开 UI 时，强烈建议启用身份验证（通常使用 OIDC）。',
			uiIsExposedWithoutAuthentication: 'UI 在未进行身份验证的情况下公开',
			uiSettings: 'UI 设置',
			unauthenticatedUsersCanAccessTheUiConsiderAddingAuthenticationOrAuthorizationPol_qnhsta:
				'未经身份验证的用户可以访问 UI；考虑添加身份验证或授权策略以保护 UI。',
			unhealthyExpression: '不健康表达式',
			unset: '未设置',
			unsupportedBackendShapeInThisForm: '此表单不支持该后端结构',
			unsupportedGuard: '不支持的防护规则',
			unsupportedGuardShape: '不支持的防护规则结构',
			unsupportedRateLimitShape: '不支持的速率限制结构',
			unsupportedRemoteRateLimitShape: '不支持的远程速率限制结构',
			unsupportedTargetType: '不支持的目标类型',
			unused: '未使用',
			upstreamApiShapeThisCustomProviderSaysItAccepts: '该自定义提供商表示接受的上游 API 格式。',
			upstreamModel: '上游模型',
			url: 'URL',
			useABuiltInSensitiveDataPattern: '使用内置的敏感数据模式。',
			useACustomRegularExpression: '使用自定义正则表达式。',
			useAmbientAwsCredentialsOrStaticAccessKeysForBedrockSigning:
				'使用环境 AWS 凭证或静态访问密钥进行 Bedrock 签名。',
			useApplicationDefaultCredentialsOrAServiceAccountJsonFileForVertex:
				'使用 Vertex 的应用程序默认凭据或服务账号 JSON 文件。',
			useAwsBedrockGuardrailsToEvaluateThePrompt: '使用 AWS Bedrock Guardrails 评估提示词。',
			useAwsBedrockGuardrailsToEvaluateTheResponse: '使用 AWS Bedrock Guardrails 评估响应。',
			useAwsBedrockGuardrails: '使用 AWS Bedrock Guardrails。',
			useAzureAiContentSafety: '使用 Azure AI 内容安全。',
			useAzureContentSafetyToEvaluateThePrompt: '使用 Azure 内容安全评估提示词。',
			useAzureContentSafetyToEvaluateTheResponse: '使用 Azure 内容安全来评估响应。',
			useAzureDefaultCredentialsManagedIdentityOrAnAzureApiKey:
				'使用 Azure 默认凭据、托管身份或 Azure API 密钥。',
			useCrossAppAccessIdentityAssertionIdJagToObtainABackendAccessToken:
				'使用跨应用程序访问（身份断言/ID-JAG）来获取后端访问令牌。',
			useCursorSOpenAiBaseUrlOverrideWithAGatewayModel:
				'将 Cursor 的 OpenAI 基本 URL 覆盖功能用于网关模型。',
			useCustomKey: '使用自定义密钥',
			useDefault: '使用默认值',
			useDefaultLocation: '使用默认位置',
			useEnvoyExternalAuthorizationOverGRpc: '通过 gRPC 使用 Envoy 外部授权。',
			useExplicitAwsCredentials: '使用显式 AWS 凭证',
			useExplicitAzureCredentials: '使用显式 Azure 凭据',
			useGoogleModelArmorForSafetyChecks: '使用 Google Model Armor 进行安全检查。',
			useGoogleModelArmorToEvaluateThePrompt: '使用 Google Model Armor 评估提示词。',
			useGoogleModelArmorToEvaluateTheResponse: '使用 Google Model Armor 来评估响应。',
			useImplicitAwsAuthenticationEnvironmentVariablesIamRolesEtc:
				'使用隐式 AWS 身份验证（环境变量、IAM 角色等）',
			useImplicitAzureAuthNoteThatThisIsForDeveloperUseCasesOnly:
				'使用隐式 Azure 身份验证。请注意，这仅适用于开发者场景！',
			useOauthTokenExchangeFlowsToObtainABackendAccessToken:
				'使用 OAuth 令牌交换流程获取后端访问令牌。',
			useOpenAiModerationChecksForIncomingPrompts: '使用 OpenAI 内容审核检查传入的提示词。',
			useOpenAiModerationToEvaluateThePrompt: '使用 OpenAI 内容审核评估提示词。',
			useOpenAiCompatibleEnvironmentVariablesWhenRunningCodexAgainstTheGateway:
				'针对网关运行 Codex 时，使用 OpenAI 兼容的环境变量。',
			useStrictModeWhenKeysShouldBeMandatory: '必须提供密钥时，请使用严格模式。',
			useTheGatewayAsAnOpenAiCompatibleChatCompletionsEndpoint:
				'将网关作为兼容 OpenAI API 的聊天补全端点。',
			useTheGatewayUrlAndKeyWithClaudeCompatibleModelRoutesWhenConfigured:
				'配置后，将网关 URL 和密钥用于 Claude 兼容的模型路由。',
			useTheIssuerMetadataEndpointUnlessAnOverrideIsProvided:
				'除非指定覆盖值，否则使用签发者元数据端点。',
			useTheSelectedBackendHostWhenPossible: '尽可能使用选定的后端主机。',
			useThisWhenTheUpstreamExposesOneOrMoreLlmCompatibleHttpApisAtYourOwnEndpoint:
				'当上游通过自有端点公开一个或多个兼容 LLM 的 HTTP API 时，请使用此选项。',
			useTrafficGatewaysForNewHttpRoutingConfiguration: '使用流量网关进行新的 HTTP 路由配置。',
			usedBy: '使用方',
			user: '用户',
			userAgent: '用户代理',
			userAgents: '用户代理',
			userAttribute: '用户属性',
			userDatabaseInHtpasswdFormatCanBeInlineOrLoadedFromAFile:
				'`htpasswd` 格式的用户数据库，可内联提供或从文件加载。',
			userMessage: '用户消息',
			user_19x0vko: '用户：',
			users: '用户',
			validateATokenWhenOneIsPresent: '当存在令牌时验证令牌。',
			validateCredentialsWhenPresentThisIsTheDefaultOptionWarningThisAllowsRequestsWit_kr9lgb:
				'存在凭据时进行验证。\n这是默认选项。\n警告：这会允许未携带 Basic Auth 凭据的请求。',
			validateJwtsAgainstASingleTrustedTokenIssuer: '针对单个可信令牌签发者验证 JWT。',
			validateJwtsAgainstOneOrMoreTrustedTokenIssuers: '针对一个或多个可信令牌签发者验证 JWT。',
			validateTheApiKeyWhenPresentThisIsTheDefaultOptionWarningThisAllowsRequestsWithoutAnApiKey:
				'验证 API 密钥（如果存在）。\n这是默认选项。\n警告：这允许没有 API 密钥的请求。',
			validateTheJwtWhenPresentThisIsTheDefaultOptionWarningThisAllowsRequestsWithoutAJwt:
				'验证 JWT（如果存在）。\n这是默认选项。\n警告：这允许没有 JWT 的请求。',
			validationMode: '验证模式',
			validationModeForApiKeyAuthentication: 'API 密钥身份验证的验证模式。',
			validationModeForBasicAuth: 'Basic Auth 的验证模式。',
			valueToReturnInAccessControlMaxAgeForAllowedPreflightRequests:
				'对于允许的预检请求，在 `Access-Control-Max-Age` 中返回的值。',
			valuesToReturnInAccessControlAllowHeadersForAllowedPreflightRequests:
				'对于允许的预检请求，在 `Access-Control-Allow-Headers` 中返回的值。',
			valuesToReturnInAccessControlAllowMethodsForAllowedPreflightRequests:
				'对于允许的预检请求，在 `Access-Control-Allow-Methods` 中返回的值。',
			valuesToReturnInAccessControlExposeHeadersForAllowedCorsResponses:
				'对于允许的 CORS 响应，在 `Access-Control-Expose-Headers` 中返回的值。',
			vertexAiRegionSpecialValuesGlobalUsesTheGlobalEndpointWhileUsAndEuUseRestrictedM_xwa0mk:
				'Vertex AI 区域。特殊值：`global` 使用全局端点，而 `us` 和 `eu`\n使用受限的多区域端点。其他值被视为区域位置。',
			vertexProject: 'Vertex 项目',
			vertexRegion: 'Vertex 区域',
			viewValue: '查看 {{value}}',
			viewValueDetails: '查看 {{value}} 详情',
			viewDiff: '查看差异',
			virtual: '虚拟',
			virtualApiKey: '虚拟 API 密钥',
			virtualApiKeyModeIsValueUnauthenticatedRequestsMayBeAccepted:
				'虚拟 API 密钥模式为 {{value}}；可能会接受未经身份验证的请求。',
			virtualApiKeys: '虚拟 API 密钥',
			virtualModel: '虚拟模型',
			virtualModelName: '虚拟模型名称',
			virtualModelsDefinesASetOfModelsThatCanBeServedFromTheGatewayTheModelNameRefersT_17dk90d:
				'`virtualModels` 定义可由网关提供服务的一组虚拟模型。模型名称指与用户请求匹配的模型名称。与 `models` 字段不同，虚拟模型会根据配置的逻辑动态路由到 `models` 中配置的具体模型。',
			visibilityControlsWhetherClientsCanRequestThisModelDirectlyRatherThanOnlyViaAVirtualModel:
				'`visibility` 控制客户端能否直接请求此模型，而非只能通过 `virtualModel` 使用。',
			vsCodeSettings: 'VS Code 设置',
			waitingForFinalResponse: '正在等待最终响应',
			waitingForModelResponse: '正在等待模型响应',
			warnings: '警告',
			warnings_1j8s2pg: '警告',
			webhook: 'Webhook',
			webhookTarget: 'Webhook 目标',
			weight: '权重',
			weighted: '加权',
			weightedEnablesWeightBasedSelectionOfTheTargetModel:
				'`weighted` 启用基于权重的目标模型选择。',
			weightedTargets: '加权目标',
			welcomeToAgentgateway: '欢迎使用 agentgateway',
			whenMustEvaluateToTrueForThisTargetToBeSelectedOmitOnlyOnTheFinalFallbackTarget:
				'只有当条件表达式的计算结果为真时，才会选择此目标；最后一个回退目标应省略该条件。',
			whenThePolicyRunsGatewayPoliciesRunBeforeRouteSelectionWhileRoutePoliciesRunAfte_1ihyj7g:
				'网关策略在选择路由前运行，路由策略在选择路由后运行。除非策略需要影响路由选择，否则默认使用路由策略。',
			whenTrueFurtherAnalysisStopsIfABlocklistIsHit: '启用后，命中阻止列表即停止后续分析。',
			whenTrueSkipSpiffeTrustDomainVerificationOnInboundHboneConnections:
				'启用后，跳过入站 HBONE 连接的 SPIFFE 信任域验证。',
			whereTheActorTokenIsReadFromInTheIncomingRequestTheCelExpressionSourceIsPermitte_1ufgpgq:
				'从传入请求中读取参与者令牌的位置。允许使用 CEL `expression` 源（仅提取）。与主题令牌不同，参与者令牌没有默认来源。',
			whereTheSubjectTokenIsReadFromAndItsTokenTypeDefaultsToTheAuthorizationBearerHea_18ffgbu:
				'主题令牌的读取位置及其令牌类型。默认从 `Authorization: Bearer` 请求头读取，令牌类型为 `access_token`。',
			whereTheTokenIsReadFromInTheIncomingRequestTheCelExpressionSourceIsPermittedExtractionOnly:
				'从传入请求中读取令牌的位置。允许使用 CEL `expression` 源，但只能提取令牌。',
			whereToPlaceTheExchangedTokenInTheBackendRequestDefaultsToTheAuthorizationHeader_1az5m3h:
				'交换所得令牌在后端请求中的放置位置。默认放入 `Authorization` 请求头，并添加 `Bearer ` 前缀。此处不能使用 CEL `expression` 源，因为它无法插入令牌。',
			whereToPlaceTheForwardedCredentialInTheBackendRequest: '将转发的凭据放置在后端请求中的位置。',
			whereToPlaceTheSecretInTheBackendRequest: '机密值在后端请求中的放置位置。',
			whereToReadTheApiKeyFromInIncomingRequests: '从传入请求中读取 API 密钥的位置。',
			whereToReadTheBasicAuthCredentialsFromInIncomingRequests:
				'从传入请求中读取 Basic Auth 凭据的位置。',
			whereToReadTheJwtFromInIncomingMcpRequests: '从传入的 MCP 请求中读取 JWT 的位置。',
			whereToReadTheJwtFromInIncomingRequests: '从传入请求中读取 JWT 的位置。',
			whetherDownstreamConnectionsMustIncludeAProxyProtocolHeader:
				'下游连接是否必须包含 PROXY 协议请求头。',
			whetherRequestHeadersAreSentToTheExternalProcessingService: '请求头是否发送到外部处理服务。',
			whetherRequestTrailersAreSentToTheExternalProcessingService:
				'是否将请求尾部字段发送到外部处理服务。',
			whetherResponseHeadersAreSentToTheExternalProcessingService:
				'是否将响应头发送到外部处理服务。',
			whetherResponseTrailersAreSentToTheExternalProcessingService:
				'响应尾部是否发送到外部处理服务。',
			whetherTheBindOpensAnOsListenerSocketDefaultsToStandardBindsThePortSetToInternal_jnh5tq:
				'绑定是否打开操作系统监听器套接字。默认为 `standard`（绑定端口）。\n设置为 `internal` 以创建不绑定套接字的仅路由绑定。',
			whetherTheExternalProcessingServiceCanChangeProcessingModesDuringARequest:
				'外部处理服务是否可以在请求期间更改处理模式。',
			whetherThisDescriptorLimitsRequestsOrLlmTokens: '此描述符是否限制请求或 LLM 令牌。',
			whetherThisLimitCountsRequestsOrLlmTokens: '此限制是否计算请求或 LLM 令牌。',
			whetherToEnableEdns0ExtensionMechanismsForDnsInTheResolverWhenNoneTheSystemProvi_1wj6cfa:
				'是否在解析器中启用 EDNS0（DNS 扩展机制）。\n当为 `None` 时，保留系统提供的解析器设置。\n也可以通过 `DNS_EDNS0` 环境变量进行设置。',
			whetherToSendAPartialBodyWhenTheRequestExceedsMaxRequestBytes:
				'请求正文超过 `maxRequestBytes` 时，是否发送部分正文。',
			whetherToSendTheBodyAsRawBytesForGRpcAuthorizationChecks:
				'是否将正文作为原始字节发送以进行 gRPC 授权检查。',
			whetherToTokenizeOnTheRequestFlowThisEnablesUsToDoMoreAccurateRateLimitsSinceWeK_dor0ya:
				'是否在请求流程中进行分词。这样可以预先获知请求的部分成本，从而提高速率限制的准确性，但也会增加计算开销。',
			whetherToTokenizeTheRequestBeforeForwardingItUpstream: '是否在将请求转发到上游前进行分词。',
			whichIncomingRequestHeadersAreForwardedToThePolicyServer:
				'哪些传入请求头被转发到策略服务器。',
			whichTrafficGatewayExposesTheUi: '哪个流量网关公开 UI。',
			windsurfSettings: 'Windsurf 设置',
			workloadsDefinesTheSetOfWorkloadsThatTheProxyCanServeTheseAreSelectedByServicesT_su2rlz:
				'`workloads` 定义代理可以提供服务的工作负载集合，并由 `services` 选择。这是一项高级功能，主要用于测试；通常优先使用路由上的内联 `backends` 和策略。',
			x: 'x',
			yamlValueReturnedByCelEvaluation: 'CEL 求值返回的 YAML 值。',
			addressOfTheCertificateAuthorityUsedToIssueSpiffeCertificates:
				'用于签发 SPIFFE 证书的证书颁发机构地址。',
			addressOfTheXDsControlPlaneUsedForDynamicConfiguration: '用于动态配置的 xDS 控制平面地址。',
			alwaysPrefixNamesEvenWithASingleTarget: '始终为名称添加前缀，即使只有一个目标。',
			arnOfTheBedrockAgentCoreRuntimeArnAwsBedrockAgentcoreRegionAccountRuntimeId:
				'Bedrock AgentCore 运行时的 ARN（`arn:aws:bedrock-agentcore:REGION:ACCOUNT:runtime/ID`）。',
			authenticationConfigurationForConnectingToTheLlmProvider:
				'连接 LLM 提供商时使用的身份验证配置。',
			authenticationTokenForCommunicatingWithTheCertificateAuthority:
				'与证书颁发机构通信时使用的身份验证令牌。',
			authenticationTokenForCommunicatingWithTheXDsControlPlane:
				'与 xDS 控制平面通信时使用的身份验证令牌。',
			awsRegionForTheBedrockEndpoint: 'Bedrock 端点所在的 AWS 区域。',
			awsRegionToUseForTheBedrockProvider: 'Bedrock 提供商使用的 AWS 区域。',
			azureApiVersionQueryParameterForTheEndpoint: '端点使用的 Azure API 版本查询参数。',
			backendLevelPoliciesForTcpBackendsSuchAsTlsAuthenticationAndTunneling:
				'TCP 后端的后端级策略，例如 TLS、身份验证和隧道。',
			backendLevelPoliciesSuchAsTlsAuthenticationAndTransformations:
				'后端级策略，例如 TLS、身份验证和转换。',
			backendLevelPoliciesSuchAsTlsAuthenticationTransformationsAndHealthChecks:
				'后端级策略，例如 TLS、身份验证、转换和健康检查。',
			backendPoliciesAppliedToTrafficToThisProvider: '用于处理发往此提供商流量的后端策略。',
			basePricingRatesForThisModel: '此模型的基础定价费率。',
			behaviorWhenTheBodyExceedsMaxBytesFailClosedRejectOrFailOpenContinue:
				'请求正文超过 `maxBytes` 时的处理方式：`failClosed`（拒绝）或 `failOpen`（继续）。',
			cachePointInsertionForLlmProvidersThatSupportPromptCaching:
				'针对支持提示词缓存的 LLM 提供商插入缓存点的配置。',
			celExpressionEvaluatedAgainstEachRequestToProduceTheSessionNameForExampleJwtSubO_68dvwh:
				'针对每个请求求值以生成会话名称的 CEL 表达式，例如 `jwt.sub` 或 `request.headers["x-team"]`。如果表达式在请求处理时无法生成有效的会话名称，请求将被拒绝。',
			celExpressionsThatComputeRequestPayloadFieldsOverridingExistingValues:
				'用于计算请求负载字段并覆盖现有值的 CEL 表达式。',
			celExpressionThatSelectsWhichRequestsAreLogged: '用于选择要记录哪些请求的 CEL 表达式。',
			conditionsPathMethodHeadersQueryThatSelectThisRoute:
				'用于选择此路由的条件（路径、方法、请求头和查询参数）。',
			configDefinesTopLevelSettingsForDnsAdminNetworkingObservabilityAndSessionManagem_yywaxh:
				'`config` 定义 DNS、管理、网络、可观测性和会话管理的顶层设置。与其他部分不同，这些设置仅在启动时应用，不会动态重新加载。',
			configurationForUpstreamConnectionsIncludingKeepalivesTimeoutsAndPooling:
				'上游连接配置，包括保活、超时和连接池。',
			connectionUrlForTheRequestLogDatabaseAPostgresOrPostgresqlUrlUsesPostgresAnyOthe_14gqjn4:
				'请求日志数据库的连接 URL。以 `postgres://` 或 `postgresql://` 开头时使用 Postgres，其他值均视为 SQLite 数据库。',
			connectToARemoteMcpServerOverHttpWithServerSentEventsSseStreaming:
				'通过 HTTP 连接远程 MCP 服务器，并使用服务器发送事件（SSE）进行流式传输。',
			contextLengthPricingTiersThatOverrideTheBaseRates: '覆盖基础费率的上下文长度定价层级。',
			contextTokenThresholdAboveWhichThisTierSRatesApply:
				'超过此上下文令牌阈值后应用本层级的费率。',
			controlsHowUpstreamToolPromptNamesAreExposedToClients:
				'控制如何向客户端公开上游工具和提示词名称。',
			costPer1MInputAudioTokensFallsBackToTheInputRateIfUnset:
				'每 100 万个输入音频令牌的费用。未设置时使用输入费率。',
			costPer1MInputPromptTokens: '每 100 万个输入令牌（提示词）的费用。',
			costPer1MOutputAudioTokensFallsBackToTheOutputRateIfUnset:
				'每 100 万个输出音频令牌的费用。未设置时使用输出费率。',
			costPer1MOutputCompletionTokens: '每 100 万个输出令牌（补全）的费用。',
			costPer1MReasoningTokensFallsBackToTheOutputRateIfUnset:
				'每 100 万个推理令牌的费用。未设置时使用输出费率。',
			costPer1MTokensReadFromCache: '每从缓存读取 100 万个令牌的费用。',
			costPer1MTokensWrittenToCache: '每向缓存写入 100 万个令牌的费用。',
			customFieldsToAddToAllMetrics: '添加到所有指标的自定义字段。',
			customFieldsToAddToOrRemoveFromLogEntries: '要在日志条目中添加或移除的自定义字段。',
			customFieldsToAddToOrRemoveFromTraceSpans: '要在追踪跨度中添加或移除的自定义字段。',
			customSessionNameRoleSessionNameForCloudTrailAndCostUsageReportAttributionEither_88b0jv:
				'用于 CloudTrail 和成本与使用情况报告归因的自定义会话名称（`RoleSessionName`）。可以是静态字符串，也可以是包含针对每个请求求值的 CEL 表达式的 `{expression: ...}`。最长 64 个字符，需匹配 `[\\w+=,.@-]`。未设置时，AWS SDK 会生成随机会话名称。',
			distributedTracingConfiguration: '分布式追踪配置。',
			durationAfterWhichUnusedPooledConnectionsAreReleased: '释放连接池中未使用连接前的等待时长。',
			enableIpv6AddressResolutionAndBindingDefaultsToTrue:
				'启用 IPv6 地址解析和绑定。默认为 `true`。',
			enableTcpKeepaliveProbesOnBackendConnectionsDefaultsToTrue:
				'在后端连接上启用 TCP 保活探测。默认为 `true`。',
			endpointQualifierVersionOrAliasForTheAgentCoreRuntimeInvocation:
				'调用 AgentCore 运行时时使用的端点限定符（版本或别名）。',
			exactOrRegexPatternTheHeaderValueMustMatch: '请求头值必须匹配的精确值或正则表达式。',
			exactOrRegexPatternTheQueryParameterValueMustMatch:
				'查询参数值必须匹配的精确值或正则表达式。',
			fieldNamesToRemoveFromLogEntries: '要从日志条目中移除的字段名称。',
			gatewayLevelPoliciesAppliedToAllTrafficOnThisListener: '应用于此监听器全部流量的网关级策略。',
			googleCloudProjectIdForVertexAi: 'Vertex AI 使用的 Google Cloud 项目 ID。',
			googleCloudProjectIdToUseForTheVertexAiProvider:
				'Vertex AI 提供商使用的 Google Cloud 项目 ID。',
			googleCloudRegionToUseForTheVertexAiProvider: 'Vertex AI 提供商使用的 Google Cloud 区域。',
			hboneHttp2ConnectTunnelProtocolConfiguration: 'HBONE（HTTP/2 CONNECT 隧道）协议配置。',
			headersToAddSetOrRemoveOnRequestsToTheLlmProvider:
				'向 LLM 提供商发送请求时要添加、设置或移除的请求头。',
			headersToAddSetOrRemoveOnResponsesFromTheLlmProvider:
				'从 LLM 提供商返回响应时要添加、设置或移除的响应头。',
			headersToDropTakesPrecedenceOverTheAllowList: '要丢弃的请求头；其优先级高于允许列表。',
			headersToForwardAnEmptyListForwardsAllHeaders: '要转发的请求头；空列表表示转发所有请求头。',
			hostnameOrIpAddressOfTheMcpServer: 'MCP 服务器的主机名或 IP 地址。',
			hostnameOrIpAddressOfTheUpstreamToRouteTo: '要路由到的上游主机名或 IP 地址。',
			hostnameOrUriOfTheMcpServerForExampleHttpsExampleComOrExampleCom443:
				'MCP 服务器的主机名或 URI，例如 `https://example.com` 或 `example.com:443`。',
			howToNamespaceToolNamesWhenMultiplexingAlwaysPrefixWithTheTargetNameOrOnlyPrefix_198h208:
				'多路复用时工具名称的命名空间方式：`always` 表示始终添加目标名称前缀，`conditional` 表示仅在需要时添加。',
			http2ConnectionLevelFlowControlWindowSizeInBytesDefaultsTo16MiB:
				'HTTP/2 连接级流量控制窗口大小（字节）。默认为 16 MiB。',
			http2MaximumFrameSizeInBytesDefaultsTo1MiB: 'HTTP/2 最大帧大小（字节）。默认为 1 MiB。',
			http2PerStreamFlowControlWindowSizeInBytesDefaultsTo4MiB:
				'HTTP/2 单流流量控制窗口大小（字节）。默认为 4 MiB。',
			httpHeaderOrPseudoHeaderNameSuchAsMethodToMatch:
				'要匹配的 HTTP 请求头或伪请求头名称（例如 `:method`）。',
			httpHeadersThatMustMatchForThisRouteToApply: '应用此路由时必须匹配的 HTTP 请求头。',
			httpHeadersToIncludeOnOtlpTraceExportsSuchAsAuthenticationHeaders:
				'导出 OTLP 追踪时包含的 HTTP 请求头，例如身份验证请求头。',
			httpMethodThatMustMatchForThisRouteToApply: '应用此路由时必须匹配的 HTTP 方法。',
			httpRoutesAttachedDirectlyToThisListener: '直接附加到此监听器的 HTTP 路由。',
			httpRoutesGroupedTogetherForDelegationAndReuse: '为委派和复用而组合在一起的 HTTP 路由。',
			identifierForTheClusterThisGatewayRunsInDefaultsToKubernetes:
				'此网关所在集群的标识符。默认为 `Kubernetes`。',
			identifierForThisBackendReferencedByRoutes: '此后端的标识符，供路由引用。',
			identifierForThisRouteGroupReferencedByDelegatingRoutes: '此路由组的标识符，供委派路由引用。',
			identifierOfTheBedrockGuardrailToApply: '要应用的 Bedrock 防护规则标识符。',
			idleTimeBeforeTheFirstKeepaliveProbeIsSent: '发送第一次保活探测前的空闲时长。',
			kubernetesNamespaceForThisGatewayInstance: '此网关实例所在的 Kubernetes 命名空间。',
			kubernetesServiceAccountForThisGatewayUsedInItsSpiffeIdentity:
				'此网关使用的 Kubernetes 服务账号，用于其 SPIFFE 身份。',
			llmProvidersInThisGroupLoadBalancedTogether: '此组中共同参与负载均衡的 LLM 提供商。',
			loggingConfigurationIncludingFilterLevelFormatAndCustomFields:
				'日志配置，包括过滤器、级别、格式和自定义字段。',
			logLevelASingleLevelEGInfoACommaSeparatedStringOfPerModuleLevelsEGInfoAgentCoreT_1appp3y:
				'日志级别：可以是单个级别（如 `info`）、以逗号分隔的各模块级别字符串（如 `info,agent_core=trace`），或各模块级别列表（如 `[info, agent_core=trace]`）。',
			logOutputFormatTextOrJson: '日志输出格式：`text` 或 `json`。',
			logStoreDatabaseConfigurationEnablesRequestLoggingToADatabaseBackend:
				'日志存储数据库配置；用于启用将请求日志记录到数据库后端。',
			mapOfFieldNameToACelExpressionThatComputesTheValueToAddToLogs:
				'字段名称到 CEL 表达式的映射，表达式用于计算要添加到日志中的值。',
			mapOfFieldNameToACelExpressionThatComputesTheValueToAddToMetrics:
				'字段名称到 CEL 表达式的映射，表达式用于计算要添加到指标中的值。',
			mapOfModelIdToItsPricingRatesAndTiers: '模型 ID 到其定价费率和层级的映射。',
			mapOfProviderNameToItsSupportedModelsAndPricing: '提供商名称到其支持模型和定价的映射。',
			maximumConcurrentStreamsPerPooledConnectionDefaultsTo100:
				'每个池化连接允许的最大并发流数。默认为 100。',
			maximumTimeToWaitForConnectionsToCloseGracefullyDuringShutdown:
				'关闭期间等待连接正常关闭的最长时间。',
			maximumTimeToWaitWhenEstablishingAConnectionToAnUpstreamDefaultsTo10Seconds:
				'与上游建立连接时的最长等待时间。默认为 10 秒。',
			mcpServerTargetsToMultiplexTogether: '要进行多路复用的 MCP 服务器目标。',
			messageRoleSuchAsSystemUserOrAssistant: '消息角色，例如 `system`、`user` 或 `assistant`。',
			messageTextContent: '消息的文本内容。',
			metricNamesToExcludeFromCollection: '不采集的指标名称。',
			metricsConfigurationIncludingMetricRemovalAndCustomFields:
				'指标配置，包括移除指标和自定义字段。',
			minimumTimeToAllowForGracefulConnectionTerminationDefaultsToZero:
				'允许连接正常终止的最短时间。默认为零。',
			modelCostCatalogProvidedInlineAsAString: '以字符串形式内联提供的模型成本目录。',
			modelCostCatalogProvidedInlineAsStructuredData: '以结构化数据形式内联提供的模型成本目录。',
			modelIdToSendToAnthropicOverridingTheModelInTheClientRequest:
				'发送给 Anthropic 的模型 ID，将覆盖客户端请求中的模型。',
			modelIdToSendToAzureOverridingTheModelInTheClientRequest:
				'发送给 Azure 的模型 ID，将覆盖客户端请求中的模型。',
			modelIdToSendToBedrockOverridingTheModelInTheClientRequest:
				'发送给 Bedrock 的模型 ID，将覆盖客户端请求中的模型。',
			modelIdToSendToGeminiOverridingTheModelInTheClientRequest:
				'发送给 Gemini 的模型 ID，将覆盖客户端请求中的模型。',
			modelIdToSendToGitHubCopilotOverridingTheModelInTheClientRequest:
				'发送给 GitHub Copilot 的模型 ID，将覆盖客户端请求中的模型。',
			modelIdToSendToOpenAiOverridingTheModelInTheClientRequest:
				'发送给 OpenAI 的模型 ID，将覆盖客户端请求中的模型。',
			modelIdToSendToTheProviderOverridingTheModelInTheClientRequest:
				'发送给提供商的模型 ID，将覆盖客户端请求中的模型。',
			modelIdToSendToVertexAiOverridingTheModelInTheClientRequest:
				'发送给 Vertex AI 的模型 ID，将覆盖客户端请求中的模型。',
			namedListenersBoundOnThisPortWhichMayUseDifferentProtocolsAndTls:
				'绑定到此端口的具名监听器，可使用不同的协议和 TLS 配置。',
			nameIdentifyingThisListenerReferencedByGatewaysGatewayNameListenerName:
				'此监听器的名称，通过 `gateways: gateway-name/listener-name` 引用。',
			nameIdentifyingThisMcpTargetUsedToPrefixToolAndResourceNamesWhenMultiplexing:
				'此 MCP 目标的名称，多路复用时用于为工具和资源名称添加前缀。',
			nameIdentifyingThisProviderReferencedByLlmModelsProvider:
				'此提供商的名称，通过 `llm.models[].provider` 引用。',
			nameIdentifyingThisResource: '此资源的名称。',
			nameIdentifyingThisRoute: '此路由的名称。',
			nameOfTheGatewayThisTargetReferences: '此目标引用的网关名称。',
			nameOfTheListenerSetResource: '监听器集资源的名称。',
			nameOfTheTargetServiceAsDefinedInTheTopLevelServicesList:
				'目标服务的名称，该服务定义在顶层 `services` 列表中。',
			nameOfThisGatewayRequiredWhenXDsIsConfigured: '此网关的名称。配置 xDS 时为必填项。',
			namespaceOfTheGatewayThisTargetReferences: '此目标引用的网关命名空间。',
			namespaceOfTheListenerSetResource: '监听器集资源的命名空间。',
			namespaceScopingThisListener: '限定此监听器作用域的命名空间。',
			namespaceScopingThisResourceUsedInFullyQualifiedNamespaceNameReferences:
				'限定此资源作用域的命名空间，用于完全限定的 `namespace/name` 引用。',
			namespaceScopingThisRoute: '限定此路由作用域的命名空间。',
			namespaceScopingThisRouteUsedInFullyQualifiedNamespaceNameReferences:
				'限定此路由作用域的命名空间，用于完全限定的 `namespace/name` 引用。',
			networkNameForThisGatewayUsedForLocalityAwareRouting: '此网关的网络名称，用于地域感知路由。',
			neverPrefixNamesWithMultipleTargetsCallsAreRoutedByLookingUpWhichTargetServesThe_1js7ysf:
				'从不为名称添加前缀；存在多个目标时，通过查找提供该名称的目标来路由调用。要求名称在所有目标中唯一。',
			numberOfUnacknowledgedProbesBeforeTheConnectionIsConsideredDead:
				'连接被视为已断开前允许的未确认探测次数。',
			numberOfWorkerThreadsForTheAsyncRuntimeAcceptsANumberOrAStringSuchAsAuto:
				'异步运行时的工作线程数。可以是数字，也可以是 `auto` 等字符串。',
			oauth20ClientSecretSentViaHttpBasicAuthToTheAuthorizationServer:
				'通过 HTTP Basic Auth 发送给授权服务器的 OAuth 2.0 客户端密钥。',
			otlpCollectorEndpointUrlForExportingTraces: '用于导出追踪数据的 OTLP 收集器端点 URL。',
			otlpTransportProtocolGrpcOrHttp: 'OTLP 传输协议：`grpc` 或 `http`。',
			outlierDetectionAndHealthCheckingForThisProviderBackend:
				'对此提供商后端执行异常检测和健康检查。',
			pathMatchRuleExactPrefixOrRegexDefaultsToAPrefixMatch:
				'路径匹配规则（精确、前缀或正则表达式）。默认为 `/` 前缀匹配。',
			pathToAFileOnDiskContainingTheModelCostCatalog: '磁盘上包含模型成本目录的文件路径。',
			pathToAFileOnDiskToLoadTheValueFrom: '用于加载值的磁盘文件路径。',
			pathToARootCaCertificateFileUsedToValidateClientCertificates:
				'用于验证客户端证书的根 CA 证书文件路径。',
			pathToTheTlsCertificateFileLeafCertificateOrCaCertificateInDynamicCaMode:
				'TLS 证书文件路径（叶证书；在动态 CA 模式下则为 CA 证书）。',
			pathToTheTlsPrivateKeyFile: 'TLS 私钥文件路径。',
			policiesAppliedToMcpRequests: '应用于 MCP 请求的策略。',
			policiesAppliedToThisMcpTarget: '应用于此 MCP 目标的策略。',
			portOnTheMcpServerToConnectTo: '要连接的 MCP 服务器端口。',
			portOnTheTargetServiceToRouteTo: '要路由到的目标服务端口。',
			portToTargetAsAnAlternativeToListenerName: '作为目标的端口，可代替 `listener_name`。',
			prefixNamesWithTheTargetNameOnlyWhenThereAreMultipleTargets:
				'仅在存在多个目标时，使用目标名称作为名称前缀。',
			pricingRatesForThisTierOverlaidOnTheBaseModelRates:
				'此层级的定价费率，会叠加在模型基础费率之上。',
			protocolThisListenerAcceptsHttpHttpsTcpTlsOrHbone:
				'此监听器接受的协议：HTTP、HTTPS、TCP、TLS 或 HBONE。',
			protocolUsedToTunnelBackendConnectionsSuchAsDirectOrHbone:
				'用于建立后端连接隧道的协议，例如 `Direct` 或 `HBONE`。',
			queryParameterNameToMatch: '要匹配的查询参数名称。',
			queryParametersThatMustMatchForThisRouteToApply: '应用此路由时必须匹配的查询参数。',
			relativeProportionOfTrafficSentToThisTargetModelDefaultsTo1:
				'发送到此目标模型的相对流量比例。默认为 1。',
			relativeWeightForLoadBalancingAcrossBackendsDefaultsTo1:
				'在各后端之间进行负载均衡的相对权重。默认为 1。',
			relativeWeightForLoadBalancingAcrossTcpBackendsDefaultsTo1:
				'在各 TCP 后端之间进行负载均衡的相对权重。默认为 1。',
			requestHeadersToMatchForConditionalModelRouting: '条件模型路由需要匹配的请求头。',
			requestPathOnTheMcpServer: 'MCP 服务器上的请求路径。',
			requestPayloadFieldsToSetOverridingAnyExistingValuesInTheRequest:
				'要设置的请求负载字段，将覆盖请求中的所有现有值。',
			requestPayloadFieldsToSetWhenNotAlreadyPresentInTheRequest:
				'仅当请求中尚不存在时才设置的请求负载字段。',
			resourceKindUsedInPolicyTargetReferences: '策略目标引用中使用的资源类型。',
			routeLevelPoliciesAppliedBeforeBackendSelection: '选择后端之前应用的路由级策略。',
			routeToAServiceDefinedInTheTopLevelServicesList: '路由到顶层 `services` 列表中定义的服务。',
			sessionNameRoleSessionNameInConfigurationFormAStaticStringOrACelExpressionEvalua_zywvwc:
				'配置中的会话名称（`RoleSessionName`）：可以是静态字符串，也可以是针对每个请求求值的 CEL 表达式。该字段未使用带标签格式，因此普通字符串仍保持原有含义。',
			specificListenerWithinTheGatewayIfUnsetTargetsTheGatewayItself:
				'网关内的特定监听器；未设置时以网关本身为目标。',
			specificListenerWithinTheListenerSetToTarget: '监听器集中要作为目标的特定监听器。',
			specificRuleWithinTheRouteForTargetedPolicyReferences: '路由内的特定规则，用于定向策略引用。',
			specificRuleWithinThisRoute: '此路由内的特定规则。',
			spiffeTrustDomainForThisGateway: '此网关的 SPIFFE 信任域。',
			staticSessionName: '静态会话名称。',
			supportedApiPayloadFormatsAndOptionalPathOverridesForThisProvider:
				'此提供商支持的 API 负载格式，以及可选的路径覆盖。',
			tcpKeepaliveConfigurationForUpstreamConnections: '上游连接的 TCP 保活配置。',
			tcpLevelPoliciesAppliedToTrafficOnThisRoute: '应用于此路由流量的 TCP 级策略。',
			tcpRoutesAttachedDirectlyToThisListener: '直接附加到此监听器的 TCP 路由。',
			theUpstreamLlmProviderTypeAndItsConfiguration: '上游 LLM 提供商类型及其配置。',
			timeBetweenSuccessiveKeepaliveProbes: '连续两次保活探测之间的时间间隔。',
			timeToLiveForMcpSessionsBeforeTheyAreClosedAutomaticallyDefaultsTo30Minutes:
				'MCP 会话自动关闭前的生存时间。默认为 30 分钟。',
			tlsConfigurationForConnectingToTheLlmProvider: '连接 LLM 提供商时使用的 TLS 配置。',
			tlsConfigurationForConnectionsToTheTcpRouteSBackend: '连接 TCP 路由后端时使用的 TLS 配置。',
			tlsConfigurationUsedWithTheHttpsAndTlsProtocols: '与 HTTPS 和 TLS 协议配合使用的 TLS 配置。',
			tunnelingConfigurationForConnectingToTheLlmProvider: '连接 LLM 提供商时使用的隧道配置。',
			versionOfTheBedrockGuardrailToApply: '要应用的 Bedrock 防护规则版本。',
			weightedBackendsThisRouteForwardsTrafficTo: '此路由将流量转发到的加权后端。',
			weightedBackendsThisTcpRouteForwardsTrafficTo: '此 TCP 路由将流量转发到的加权后端。',
			whetherToKeepAPersistentSessionAcrossRequestsStatefulOrCreateOnePerRequestStateless:
				'是在多个请求之间保留持久会话（`Stateful`），还是为每个请求创建独立会话（`Stateless`）。',
			yourChangesHaveNotBeenSavedAndWillBeLost: '你的更改尚未保存，关闭后将丢失。',
			configDefinesTopLevelSettingsForDnsAdminNetworkingObservabilityAndSessionManagem_2uetmx:
				'`config` 定义 DNS、管理、网络、可观测性和会话管理的顶层设置。与其他部分不同，这些设置仅在启动时应用；只有 `modelCatalog` 会动态重新加载。',
			controlsWhetherUiManagedConfigurationIsWrittenToTheConfigFileOrADbOverlay:
				'控制 UI 管理的配置写入配置文件还是数据库覆盖层。',
			maximumNumberOfConnectionsToOpenInThisDatabaseSConnectionPoolDefaultsTo5WhenTheR_y8kw5t:
				'此数据库连接池最多打开的连接数，默认为 5。当请求日志存储和配置存储使用相同的数据库设置时，它们会共享一个连接池，并受此上限限制。',
			storeAllUiManagedConfigurationInTheLocalConfigFile:
				'将所有 UI 管理的配置存储在本地配置文件中。',
			readAFileBaselineAndStoreUiManagedOverlayResourcesInTheConfiguredDatabase:
				'读取文件基线，并将 UI 管理的覆盖资源存储到配置的数据库中。',
			injectArtificialLatencyBeforeForwardingRequests: '在转发请求前注入人为延迟。',
			denyTheRequestWhenThisCelExpressionIsTrueThisModeIsNotRecommendedBecauseExpressi_8r8xmb:
				'当此 CEL 表达式的计算结果为 `true` 时拒绝请求。不建议使用此模式，因为表达式求值失败时不会拒绝请求；优先使用 `Allow` 或 `Require`。如果必须使用，请针对求值错误谨慎设计表达式。',
			celExpressionThatComputesTheFullSetOfHeadersReplacingAllExistingHeadersTheExpres_k52u6e:
				'用于计算完整请求头集合的 CEL 表达式，会替换所有现有请求头。表达式必须求值为请求头名称到值的映射（值可以是字符串；重复请求头可以使用字符串数组）。伪请求头（`:method`、`:path` 等）会被忽略；请使用 `set`/`add` 显式设置。`replace` 会在 `add`/`set`/`remove` 之前应用，因此后续操作仍基于替换后的请求头执行。',
			signAShortLivedJwtWithAPrivateKeyOnEachRequest: '使用私钥为每个请求签发短期 JWT。',
			awsSigV4SigningRegionForExampleUsEast1IfUnsetTypedAwsBackendsMayProvideThisAutom_19lcckx:
				'AWS SigV4 签名区域（例如 `us-east-1`）。未设置时，类型化 AWS 后端可能会自动提供该区域；否则使用环境中的 AWS 区域。',
			signsAShortLivedJwtWithAPrivateKeyOnEachRequestAndSendsItToTheBackendForUpstream_1x1fp0x:
				'为每个请求使用私钥签发短期 JWT，并将其发送到后端。适用于要求每个请求使用密钥对 JWT（例如 Snowflake SQL API）的上游，而不是静态凭据。',
			jwsSigningAlgorithmDefaultsToRs256: 'JWS 签名算法。默认为 `RS256`。',
			optionalJwsKeyIdHeader: '可选的 JWS 密钥 ID 请求头。',
			staticClaimsAddedToEveryTokenEGIssSubAudValuesMayBeAnyJsonValueEGAStringNumberBo_tgmtv7:
				'添加到每个令牌的静态声明（例如 `iss`、`sub`、`aud`）。值可以是任意 JSON 值（例如字符串、数字、布尔值或数组）。`iat`、`exp` 和 `nbf` 由签名器保留，不能在此配置。',
			tokenLifetimeUsedForExpDefaultsTo300s: '用于 `exp` 的令牌有效期。默认为 300 秒。',
			whereTheSignedTokenIsWrittenDefaultsToTheAuthorizationHeaderWithABearerPrefix:
				'签名令牌的写入位置。默认为带 `Bearer ` 前缀的 `Authorization` 请求头。',
			requestedTokenTypeParameterWhenUnsetItIsOmittedFromTheRequestRfc8693MakesItOptio_25iu5i:
				'`requested_token_type` 参数。未设置时会从请求中省略（RFC 8693 将其定义为可选）。某些提供商（例如 Auth0 自定义令牌交换）会拒绝显式的 `access_token` 值与自定义 `subject_token_type` 搭配。',
			pemEncodedX509CertificateChainLeafFirstTheLeafPublicKeyMustCorrespondToSigningKe_9b0e33:
				'PEM 编码的 X.509 证书链，叶证书在前。叶证书公钥必须与 `signing_key` 对应，令牌端点才能验证断言。如果不匹配或比较失败，会记录日志，但不会阻止加载。',
			jwsCertificateHeaderEmittedFromCertificateRequiredWhenCertificateIsSet:
				'从 `certificate` 生成的 JWS 证书请求头。设置 `certificate` 时必填。',
			sendTheX509CertificateChainInX5c: '在 `x5c` 中发送 X.509 证书链。',
			sendTheLeafCertificateSSha256ThumbprintInX5tS256:
				'在 `x5t#S256` 中发送叶证书的 SHA-256 指纹。',
			subjectTokenSentToTheIdentityProviderDefaultsToAnOpenIdConnectIdTokenReadFromThe_uefi1w:
				'发送给身份提供商的主题令牌。默认为从 Authorization Bearer 请求头读取的 OpenID Connect ID 令牌。',
			whereToReadTheSubjectTokenDefaultsToTheAuthorizationBearerHeader:
				'主题令牌的读取位置。默认为 Authorization Bearer 请求头。',
			rfc8693SubjectTokenTypeUriDefaultsToAnOpenIdConnectIdToken:
				'RFC 8693 主题令牌类型 URI。默认为 OpenID Connect ID 令牌。',
			anAdditionalCredentialToInjectOnTheBackendRequest: '要注入后端请求的附加凭据。',
			whereTheCredentialIsInsertedOnTheBackendRequest: '凭据在后端请求中的插入位置。',
			credentialValue: '凭据值。',
			policiesToConnectToTheProxyBackend: '连接代理后端所需的策略。',
			jsonWebKeySetUsedToVerifyTokenSignaturesCanBeInlineFromAFileOrFetchedRemotelyIfO_n5iwa6:
				'用于验证令牌签名的 JSON Web 密钥集（JWKS）。可内联、从文件读取或远程获取。省略时，会根据签发者和提供商派生 JWKS URL。',
			oauthClientSecretInjectedIntoProxiedTokenRequestsForConfidentialClientsCurrently_1390oc4:
				'为机密客户端的代理令牌请求注入 OAuth 客户端密钥。目前由 `entra` 提供商使用；其 Web 平台应用注册要求在令牌端点提供客户端密钥。',
			requestBodyValuesComputedFromCelExpressionsTheseAreAppliedAfterConversionToThePr_bijwyz:
				'根据 CEL 表达式计算的请求正文值。这些值会在转换为提供商的请求格式后应用。',
			headersToSetOnTheWebhookRequestComputedFromCelExpressionsKeysMayBeHeaderNamesOrT_1d832f8:
				'根据 CEL 表达式计算并设置的 Webhook 请求头。键可以是请求头名称或 `:path`、`:method`、`:authority` 伪请求头；设置 `:path` 会替换默认的 `/request`/`/response` 路径。表达式针对原始传入请求求值（与 `transformation` 策略相同），因此 `request.*` 和 `jwt.*` 指向客户端请求。',
			artificialLatencyInjectedBeforeTheRequestIsForwardedToTheBackendEitherADurationS_1jzhdfx:
				'在请求转发到后端前注入人为延迟。可以是 `2s` 等时长字符串，也可以是针对请求求值并返回时长的 CEL 表达式（例如 `duration("500ms")`），或解释为毫秒数的数字（例如用于概率延迟的 `random() < 0.1 ? 500 : 0`，或用于抖动的 `int(random() * 500)`）。非正结果不会注入延迟。',
			celExpressionEvaluatedAgainstTheRequestToComputeTheDialTargetEGExtprocWorkerPodI_gl1myq:
				'针对请求求值、用于计算拨号目标的 CEL 表达式（例如 `extproc.workerPodIp + ":" + string(extproc.workerPodPort)`，用于读取 extProc 策略已设置的动态元数据）。必须求值为 `host:port` 字符串。表达式及提供其动态元数据的策略均被信任，可用于选择拨号目标。未设置时，从请求自身的 `:authority`/URI 读取目标。',
			transportPoliciesForConnectingToThisTargetSBackendNotSupportedOnStdioTargetsMcpP_141bjhs:
				'用于连接此目标后端的传输策略。stdio 目标不支持这些策略。MCP 策略（`mcpAuthorization`、`mcpGuardrails`）应用于完整目标集合，应配置在路由或 `mcp.policies` 上。',
			configurationForRunningOpenAiInlineModerationOnRequestInputAndGeneratedOutput:
				'对请求输入和生成输出运行 OpenAI 内联审核的配置。',
			theModerationModelToUseDefaultsToOmniModerationLatest:
				'使用的审核模型。默认为 `omni-moderation-latest`。',
			policiesToApplyToRequestInputAndGeneratedOutput: '应用于请求输入和生成输出的策略。',
			policyForRequestInputModeration: '请求输入审核策略。',
			policyForGeneratedOutputModeration: '生成输出审核策略。',
			applyBestEffortSessionAffinityUsingARequestValueSelectedByACelExpressionRequests_1wx29fs:
				'使用 CEL 表达式选择的请求值，尽力实现会话亲和性。具有相同值的请求会一致地负载均衡到同一健康服务端点或 AI 提供商，但可用后端发生变化时可能重新映射。',
			configuresBestEffortSessionAffinityUsingAnExistingRequestAttributeTheSourceCelEx_1udd2uh:
				'使用现有请求属性配置尽力而为的会话亲和性。来源 CEL 表达式选择亲和值。具有相同值的请求会一致地负载均衡到同一健康服务端点或 AI 提供商。与会话持久化不同，此策略不会识别或跟踪之前选择的后端，因此可用后端发生变化时可能重新映射值。',
			celExpressionEvaluatedAgainstRequestStateItMustReturnAStringOrBytesValueExamples_64u9wd:
				'针对请求状态求值的 CEL 表达式。必须返回字符串或字节值。示例：`request.headers["x-session-id"]` 或 `string(source.address)`。',
			llmDetailStoredInTheDatabaseMetadataStoresRequestMetadataUsageTimingAndCostWitho_69c2bv:
				'存储在数据库中的 LLM 详情。`metadata` 会在专用负载表中存储请求元数据、用量、时间和成本，但不存储提示词或补全内容。`full` 还会捕获并存储这些内容。省略时保留旧行为：通过 CEL 表达式捕获的内容也会存储在负载中。',
			storeLlmMetadataWithoutPromptOrCompletionContent:
				'存储 LLM 元数据，但不存储提示词或补全内容。',
			storeLlmMetadataAndPromptCompletionContent: '存储 LLM 元数据以及提示词和补全内容。',
			aNamedCustomProviderConfigurationMaintainedByAgentgatewayThesePresetsDeliberatel_rc86d8:
				'由 agentgateway 维护的命名自定义提供商配置。这些预设与 `Provider` 并列：独立配置和 xDS 配置都会在此展开，从而保持端点和格式行为一致。',
			idIsAStableIdentityForThisModelConfigEntryTheNameFieldRemainsTheModelMatchPattern:
				'`id` 是此模型配置条目的稳定标识；`name` 字段仍表示模型匹配模式。',
			finalTransformationAllowsSettingValuesFromCelExpressionsForTheRequestOverridingA_5b0fab:
				'`final_transformation` 允许使用 CEL 表达式为请求设置值，并覆盖现有值。它在请求转换为提供商格式后执行，因此可以进行提供商特定的转换。',
			browserOriginsThatMayCallThisListenerUseExactOriginsSuchAsHttpLocalhost19000:
				'可调用此监听器的浏览器来源。请使用 `http://localhost:19000` 等精确来源。',
			requestHeadersAllowedByBrowserPreflightChecksUseWhileDebuggingThenNarrowItForProduction:
				'浏览器预检请求允许的请求头。调试时可使用 `*`，生产环境请缩小范围。',
			httpMethodsAllowedByBrowserPreflightChecksPlaygroundsTypicallyNeedGetAndPost:
				'浏览器预检请求允许的 HTTP 方法。演练场通常需要 GET 和 POST。',
			responseHeadersBrowserJavaScriptCanReadMcpPlaygroundsNeedMcpSessionId:
				'浏览器 JavaScript 可以读取的响应头。MCP 演练场需要 `Mcp-Session-Id`。',
			strictRequiresAValidJwtOptionalValidatesOnlyWhenPresentAndPermissiveNeverRejectsRequests:
				'`strict` 要求有效 JWT，`optional` 仅在 JWT 存在时验证，`permissive` 从不拒绝请求。',
			expectedIssuerClaimForAcceptedJwts: '已接受 JWT 应具备的预期签发者声明。',
			acceptedAudienceClaimsLeaveEmptyOnlyWhenTheGatewayShouldNotEnforceAudienceMatching:
				'已接受的受众声明。仅当网关不应强制匹配受众时留空。',
			jwksUsedToValidateJwtSignaturesThisMayBeInlineJsonAFileReferenceOrARemoteUrlObject:
				'用于验证 JWT 签名的 JWKS。可以是内联 JSON、文件引用或远程 URL 对象。',
			whetherThisLimitCountsRequestsImmediatelyOrTokensAfterAnLlmResponseCompletes:
				'此限制是立即按请求计数，还是在 LLM 响应完成后按令牌计数。',
			howOftenTokensAreReplenishedSuchAs1s60sOr1m: '令牌补充的频率，例如 `1s`、`60s` 或 `1m`。',
			maximumBurstSizeForThisLocalRateLimitBucket: '此本地速率限制桶允许的最大突发量。',
			numberOfTokensAddedBackToTheBucketEveryFillInterval: '每个填充间隔向桶中补充的令牌数。',
			selectTheGuardrailIntegrationOrRuleTypeToApply: '选择要应用的防护规则集成或规则类型。',
			configuredFromSchema: '已根据架构配置。',
			playgroundLlmCorsInstruction:
				'将 {{value}} 添加到 LLM CORS 策略，以便此演练场可以从浏览器调用网关。',
			playgroundMcpCorsInstruction:
				'将 {{value}} 添加到 MCP CORS 策略，以便此演练场可以从浏览器列出并调用 MCP 工具。',
			playgroundMcpSessionCorsInstruction:
				'将 {{value}} 添加到 MCP CORS 策略并公开 `Mcp-Session-Id`，以便此演练场可以保持浏览器会话。',
			gooseModelNamesInstruction:
				'无法输入自定义模型名称；对于提供商列表中缺失的模型，请在 {{value}} 中设置 {{value}}。',
			unsupportedTargetDescription:
				'该策略使用不受支持的 {{value}} 目标。可视化编辑器目前仅支持主机目标。',
			unsupportedRemoteRateLimitTarget:
				'该策略使用 {{value}} 目标。可视化编辑器目前仅支持主机目标。',
			kubernetesService: 'Kubernetes 服务',
			configureCors: '配置 CORS',
			viewPolicy: '查看策略',
			viewRoute: '查看路由',
			viewListener: '查看监听器',
			modelMatchSendTo: '匹配 {{value}}，并将其发送到 {{value}}。',
			modelMatchForwardAsIs: '匹配 {{value}}，并按原样转发模型。',
			modelMatchStripPrefix: '匹配 {{value}}，去除 {{value}} 前缀，并按原样转发剩余模型。',
			claudeDesktopRestartInstruction:
				'完全退出并重新启动 Claude Desktop。重新启动后，打开菜单栏中的“开发者”菜单。',
			claudeDesktopOpenDeveloperMenu: '打开“开发者”>“配置第三方推理”>“网关”。',
			openCodeCreateConfigInstruction: '在项目根目录中创建 {{value}}。',
			openCodeRunInstruction: '在同一目录中运行 {{value}}。',
			cursorOpenModelsInstruction: '打开 Cursor 设置 > 模型。',
			cursorOverrideBaseUrlInstruction: '启用“覆盖 OpenAI 基本 URL”，并将其设置为 {{value}}。',
			cursorAddModelInstruction: '添加 {{value}} 作为自定义模型，然后在“询问”或“计划”模式中测试。',
			copilotOpenSettingsInstruction: '打开 VS Code 设置并搜索 {{value}}。',
			copilotEditSettingsInstruction: '编辑 {{value}} 并设置高级代理 URL。',
			windsurfOpenSettingsInstruction: '打开 Windsurf 设置。',
			windsurfSetProxyInstruction: '将 HTTP 代理 URL 设置为 {{value}} 并保存。',
			legacyBindsWarning:
				'此配置使用 {{value}}，但没有 {{value}}。请考虑将监听器所有权移至 {{value}}。',
			ruleNumber: '规则 {{value}}',
			processorNumber: '处理器 {{value}}',
			descriptorNumber: '描述符 {{value}}',
			deleteNamedResourceQuestion: '要删除“{{value}}”吗？此操作无法撤销。',
			deleteTargetQuestion: '要删除“{{value}}”吗？删除后，流量将无法再发送到此目标。',
			deleteRouteQuestion: '要删除“{{value}}”吗？匹配此路由的流量将不再到达其后端。',
			invalid: '无效',
			unrestricted: '不限制',
			unrestrictedLowercase: '不限制',
			denyAll: '全部拒绝',
			denyAllLowercase: '全部拒绝',
			allModels: '所有模型',
			budgets: '预算',
			modelAccess: '模型访问',
			selectedModels: '已选模型',
			accessMode: '访问模式',
			allowedModelPatterns: '允许的模型匹配模式',
			modelPatternPlaceholder: 'gpt-5.5 或 openai/*',
			noModelPatternsConfigured: '未配置模型模式。',
			invalidBudgets: '预算配置无效',
			invalidModelAccess: '模型访问配置无效',
			addAtLeastOneModelPatternOrSelectDenyAll: '至少添加一个模型匹配模式，或选择“全部拒绝”。',
			wildcardCannotBeCombinedWithOtherModelPatterns: '“*”不能与其他模型匹配模式同时使用。',
			wildcardsOnlySupportedAtPatternBeginningOrEnd: '通配符仅支持出现在模式开头或结尾。',
			modelPatternCanContainAtMostOneWildcard: '每个模型匹配模式最多包含一个通配符。',
			thisKeyCanRequestAnyModel: '此密钥可请求任意模型。',
			requestsMayOnlyUseModelsMatchingPatternsBelow: '请求只能使用与以下匹配模式相符的模型。',
			thisKeyCannotRequestAnyModel: '此密钥不能请求任何模型。',
			limitWhichRequestedModelNamesThisKeyCanUse: '限制此密钥可请求的模型名称。',
			attachCustomMetadataToRequestsAuthenticatedWithThisKey:
				'为使用此密钥进行身份验证的请求附加自定义元数据。',
			capHowMuchThisKeyCanSpendOrConsumeDuringEachRollingWindow:
				'限制此密钥在每个滚动时间窗口内可支出的金额或可消耗的令牌数。',
			budgetNamesMustBePresentAndUniqueRollingWindowsAreRequiredAndAmountsMustBeNonNegative:
				'预算名称必须填写且唯一，必须设置滚动时间窗口，金额必须为非负值；令牌限额必须为整数。',
			noBudgetsConfiguredUsageIsUnlimited: '未配置预算，用量不受限制。',
			untitledBudget: '未命名预算',
			exceeded: '已超限',
			done: '完成',
			removeBudget: '移除预算 {{value}}',
			stableIdentifierUsedForAccounting: '用于核算的稳定标识符。',
			monthlySpend: 'monthly-spend',
			rollingWindow: '滚动窗口',
			rollingWindowExamples: '示例：24h、7d 或 30d。',
			limitAmount: '限额',
			limitUnit: '限额单位',
			whenLimitIsReached: '达到限额时',
			budgetAmount: '预算 {{value}} 的金额',
			budgetUnit: '预算 {{value}} 的单位',
			budgetEnforcement: '预算 {{value}} 的执行方式',
			blockRequests: '阻止请求',
			return429: '返回 429',
			auditOnly: '仅审计',
			continueServing: '继续提供服务',
			addBudget: '添加预算',
			loadingUsage: '正在加载用量…',
			liveUsageUnavailable: '实时用量不可用。',
			budgetUsedOfLimit: '已使用 {{value}} / {{value}}',
			budgetUsageLive: '{{value}}% · 将在 {{value}} 重置',
			budgetUsageNotRecorded: '尚未记录用量 · {{value}} 滚动窗口',
			usdAmount: '${{value}}',
			tokenAmount: '{{value}} 个令牌',
			budgetSummaryTooltip: '{{value}} / {{value}}，每 {{value}}',
			budget_one: '{{count}} 个预算',
			budget_other: '{{count}} 个预算',
			pattern_one: '{{count}} 个匹配模式',
			pattern_other: '{{count}} 个匹配模式',
			patterns: '{{count}} 个匹配模式',
			entry_one: '{{count}} 项',
			entry_other: '{{count}} 项',
			addAMetadataNameBeforeSavingThisVirtualApiKey: '保存此虚拟 API key 前，请先添加元数据名称。',
			cannotBeCombinedWithOtherModelPatterns: '`*` 不能与其他模型匹配模式组合。',
			wildcardsAreOnlySupportedAtTheBeginningOrEndOfAPattern:
				'通配符只能位于匹配模式的开头或结尾。',
			aModelPatternCanContainAtMostOneWildcard: '一个模型匹配模式最多只能包含一个通配符。',
			valueBudgets_one: '{{count}} 个预算',
			valueBudgets_other: '{{count}} 个预算',
			valuePatterns_one: '{{count}} 个匹配模式',
			valuePatterns_other: '{{count}} 个匹配模式',
			valueEntries_one: '{{count}} 项',
			valueEntries_other: '{{count}} 项',
			removeBudgetValue: '移除预算 {{value}}',
			examples24h7dOr30d: '例如：24h、7d 或 30d。',
			budgetValueAmount: '预算 {{value}} 的金额',
			budgetValueUnit: '预算 {{value}} 的单位',
			budgetValueEnforcement: '预算 {{value}} 的执行方式',
			liveUsageIsUnavailable: '实时用量不可用。',
			valueOfValueUsed: '已使用 {{value}} / {{value}}',
			valueResetsValue: '{{value}}% · 将在 {{value}} 重置',
			noUsageRecordedYetValueRollingWindow: '尚无用量记录 · {{value}} 滚动窗口',
			valueOfValuePerValue: '{{value}} / {{value}}（每 {{value}}）',
			portConflict: '端口冲突',
			conflict: '冲突',
			valueListenersInvolvedInPortConflicts: '有 {{value}} 个监听器涉及端口冲突',
			qualifiedListener: '{{value}} 监听器 {{value}}',
			valueListenerValue: '{{value}} 监听器 {{value}}',
			listenerSetValueValue: 'ListenerSet {{value}}/{{value}}',
			gatewayValueValue: '网关 {{value}}/{{value}}',
			routeGroupValue: '路由组：{{value}}',
			mesh: '网格',
			direct: '直连',
			listenersAndRoutes: '{{value}} 个监听器 · {{value}} 个路由',
			theUiIsConfiguredAsReadOnlyEditingIsDisabled: 'UI 已配置为只读，编辑功能已禁用。',
			theUiIsConfiguredAsReadOnly: 'UI 已配置为只读。',
			geminiChatModelsModelGenerateContent: 'Gemini 聊天（models/{model}:generateContent）',
			geminiTokenCountModelsModelCountTokens: 'Gemini 令牌计数（models/{model}:countTokens）',
			chatCompletionsFormat: '聊天补全（/v1/chat/completions）',
			anthropicMessagesFormat: 'Anthropic 消息（/v1/messages）',
			responsesFormat: '响应（/v1/responses）',
			embeddingsFormat: '嵌入（/v1/embeddings）',
			anthropicTokenCountFormat: 'Anthropic 令牌计数（/v1/messages/count_tokens）',
			realtimeFormat: 'Realtime（/v1/realtime）',
			rerankFormat: '重排序（/v2/rerank）',
			bedrockGuardrailDetailsNotSet: '未设置 Bedrock 防护规则详情。',
			defaultModerationModel: '默认内容审核模型。',
			guardBuiltInSummary_one: '{{value}}：{{value}} 个内置检测器。',
			guardBuiltInSummary_other: '{{value}}：{{value}} 个内置检测器。',
			guardEndpointOnlySummary: '{{value}}。',
			guardEndpointSummary: '{{value}} · {{value}}。',
			guardModelSummary: '模型：{{value}}。',
			guardRegexSummary_one: '{{value}}：{{value}} 个正则表达式模式。',
			guardRegexSummary_other: '{{value}}：{{value}} 个正则表达式模式。',
			guardTargetSummary: '{{value}} · {{value}}。',
			jailbreakDetection: '越狱检测',
			mask: '屏蔽',
			modelArmorDetailsNotSet: '未设置 Model Armor 详情。',
			rawGuardYamlPreserved: '原始防护规则 YAML 已保留。如需编辑不支持的配置，请使用原始配置。',
			reject: '拒绝',
			summaryWithRejection: '{{value}}；拒绝状态：{{value}}。',
			webhookTargetNotSet: '未设置 Webhook 目标。',
			azureEndpointNotSet: '未设置 Azure 端点。',
			trajectory: '轨迹',
			turn: '轮次',
			unknownTurn: '未知轮次',
			trajectorySteps_one: '{{count}} 步',
			trajectorySteps_other: '{{count}} 步',
			trajectoryStep: '第 {{value}} 步：{{value}}',
			trajectoryStepLabel: '第 {{value}} 步 {{value}}',
			widthShowsApproximateTokens: '宽度表示近似令牌数',
			jumpToConversation: '跳转到对话',
			trajectoryToolCall: '工具调用：{{value}}',
			trajectoryToolResult: '工具结果：{{value}}',
			trajectoryMessageValue: '{{value}}：{{value}}',
			reasoningDetailsUnavailable: '无法获取推理详情',
			encrypted: '已加密',
			encryptedBytes: '已加密（{{value}} 字节）',
			histogramRepresentationToCollectNativeHistogramsAreExposedOnlyThroughThePromethe_18b5wxk:
				'要采集的直方图表示形式。原生直方图仅通过 Prometheus protobuf 格式公开。默认为 `classic`。',
			additionalRequestHeadersWhoseValuesShouldBeRedactedFromTraceAndDebugOutput:
				'其值应从追踪和调试输出中脱敏的额外请求头。',
			freeformCapabilityRoutingTagsForThisModel: '此模型的自由格式能力/路由标签。',
			disallowWriteOperationsToTheConfigFromTheUi: '禁止通过 UI 对配置执行写入操作。',
			collectClassicHistogramBucketsOnly: '仅采集经典直方图桶。',
			collectNativeHistogramBucketsOnly: '仅采集原生直方图桶。',
			collectBothClassicAndNativeHistogramBuckets: '同时采集经典和原生直方图桶。',
			scopeValuesRequestedWhenObtainingTheIdJagFromTheIdentityProviderSentSpaceDelimited:
				'从身份提供商获取 ID-JAG 时请求的 scope 值（以空格分隔）。',
			scopeValuesRequestedWhenExchangingTheIdJagForAnAccessTokenWhenUnsetInheritsScope_1veicy4:
				'在将 ID-JAG 交换为访问令牌时请求的 scope 值。未设置时继承 scopes；为空时省略 scope。',
			jwtValidationOptionsControllingWhichClaimsMustBePresentInATokenTheRequiredClaims_24lqvf:
				'用于控制令牌中必须存在哪些声明的 JWT 验证选项。',
			claimsThatMustBePresentInTheTokenBeforeValidationOnlyExpNbfAudIssSubAreEnforcedO_lq824y:
				'令牌验证前必须存在的声明。仅强制检查 exp、nbf、aud、iss、sub；其他声明（包括 iat 和 jti）会被忽略。默认为 [exp]。使用空列表时，除配置的签发者和受众所隐含的要求外，不增加任何声明要求。',
			whichPartsOfTheRequestThisGuardInspects: '此防护规则检查请求的哪些部分。',
			aCategoryOfRequestContentThatAPromptGuardCanInspect: '提示词防护规则可以检查的请求内容类别。',
			theSystemDeveloperPrompt: '系统/开发者提示词。',
			regularUserAssistantMessageText: '普通用户/助手消息文本。',
			toolCallResults: '工具调用结果。',
			toolCallArgumentsInApisThatSendToolArgumentsAsOpaqueJsonSuchAsCompletionsTheArgu_16v73q:
				'工具调用参数。在 Completions 等将工具参数作为不透明 JSON 发送的 API 中，参数会作为单个字符串整体遮蔽，因此提示词防护规则可能会将其改写为无效 JSON。',
			geminiModelsModelGenerateContentAndModelsModelStreamGenerateContent:
				'Gemini models/{model}:generateContent 和 models/{model}:streamGenerateContent。',
			geminiModelsModelCountTokens: 'Gemini models/{model}:countTokens。',
			expectedTokenIssuerTheJwtIssClaimIsRequiredAndMustMatch:
				'预期的令牌签发者。JWT iss 声明为必需项且必须匹配。',
			acceptedTokenAudiencesANonEmptyListRequiresAMatchingJwtAudClaim:
				'已接受的令牌受众。非空列表要求匹配的 JWT aud 声明。',
			optInMcpDnsRebindingProtectionHostOriginMustBeLocalhostOffByDefaultSeeHttpsGithu_eainyq:
				'选择启用 MCP DNS 重绑定保护（Host/Origin 必须为 localhost）。默认关闭；参见 https://github.com/agentgateway/agentgateway/issues/1855。',
			resolveTheDialTargetFromDownstreamTlsSniAndTheOriginalDestinationPort:
				'根据下游 TLS SNI 和原始目标端口解析拨号目标。',
			celExpressionEvaluatedAgainstTcpConnectionContextToComputeAHostPortDialTargetAva_tnefsc:
				'针对 TCP 连接上下文求值、用于计算 host:port 拨号目标的 CEL 表达式。可用字段包括 source.* 和 destination.*；对于 TLS，destination.hostname 是嗅探到的 SNI。',
			httpExternalAuthorizationPerformedOnceForEachDownstreamNetworkConnection:
				'对每个下游网络连接执行一次 HTTP 外部授权。',
			theRequestedDestinationHostnameWhenKnownForTlsConnectionsThisIsTheSniffedSni:
				'请求的目标主机名（如果已知）。对于 TLS 连接，这是嗅探到的 SNI。',
			localSpiffeWorkloadApiConfigurationWhenSetListenersAndBackendsMaySourceTheirTlsI_hdlcfx:
				'本地 SPIFFE Workload API 配置\n设置后，监听器和后端可从 SPIFFE 获取 TLS 身份。',
			spiffeWorkloadApiEndpointEGUnixRunSpireAgentSock:
				'SPIFFE Workload API 端点（例如 `unix:///run/spire/agent.sock`）。',
			maximumTimeToWaitWhenEstablishingAConnectionToAnUpstreamDefaultsTo11Seconds:
				'与上游建立连接时的最长等待时间。默认为 11 秒。',
			protocolHandlingForTheEntireBindWhenOmittedItIsInferredFromTheListeners:
				'整个绑定的协议处理方式。省略时根据监听器推断。',
			experimentalDetectsTlsPlaintextHttpOrOpaqueTcpFromTheFirstBytesOfEachConnectionA_1r6erw6:
				'实验性：根据每个连接的首字节检测 TLS、明文 HTTP 或不透明 TCP，并选择相应的监听器。尽可能使用显式协议。AUTO 要求客户端先发送数据，不透明协议可能表现得像 TLS 或 HTTP 请求。',
			certificateSourceModeStaticModeUsesCertKeyAsTheLeafCertificateDynamicCaModeUsesC_ehwfvy:
				'证书来源模式。静态模式将 cert/key 用作叶证书；动态 CA 模式将 cert/key 用作 CA，以按需签发 SNI 叶证书。\n设置 `spiffe` 时不使用。',
			pathToTheTlsCertificateFileLeafCertificateOrCaCertificateInDynamicCaModeRequired_1j2jxjv:
				'TLS 证书文件路径（叶证书；动态 CA 模式下为 CA 证书）。除非设置 `spiffe`，否则必填。',
			pathToARootCaCertificateFileUsedToValidateClientCertificatesMTlsOmitForOneWaySer_1fp8dve:
				'用于验证客户端证书（mTLS）的根 CA 证书文件路径。\n单向服务器 TLS 时可省略。设置 `spiffe` 时不使用。',
			sourceTheServingIdentityFromTheSpiffeWorkloadApiMutuallyExclusiveWithCertKeyRoot:
				'从 SPIFFE Workload API 获取服务端身份。\n不能与 `cert`/`key`/`root` 同时设置。',
			resolveSubstrateActorHostnamesForDynamicRouteBackendsOnIngress:
				'在入口侧为动态路由后端解析 Substrate actor 主机名。',
			authorizeConnectEgressUsingTheOriginatingActorSDynamicPolicy:
				'使用发起方 actor 的动态策略授权 CONNECT 出站流量。',
			getTheGatewaySClientIdentityAndTrustRootsFromTheSpiffeWorkloadApiMutuallyExclusi_1gwgyqy:
				'从 SPIFFE Workload API 获取网关的客户端身份和信任根证书。\n不能与 `cert`/`key`/`root`/`insecure`/`insecureHost` 同时设置。\n可通过 `subjectAltNames` 固定指定的上游 SPIFFE ID（例如 `spiffe://td/ns/foo/sa/bar`）；省略 `subjectAltNames` 时，接受链至 SPIFFE 信任包的任意 SVID。',
			secretValueToSendToTheBackendFileReferencesAreWatchedSoRotatingTheFileReloadsItWithoutARestart:
				'要发送到后端的机密值。会监视文件引用，因此轮换文件后无需重启即可重新加载。',
			maximumTimeAConnectionToTheBackendMayStayOpenAConnectionPastThisDurationIsNotReu_o4pwvu:
				'后端连接允许保持打开的最长时间。超过此时长的连接不会再用于新请求；系统会建立新连接，同时不会中断进行中的请求。',
			howRequestsAreSentThroughTheProxy: '请求通过代理发送的方式。',
			useConnectForTlsAndNonHttpTransportsAndAbsoluteFormRequestsForPlaintextHttp:
				'TLS 和非 HTTP 传输使用 CONNECT；明文 HTTP 使用绝对形式请求。',
			useConnectForAllTransportsIncludingPlaintextHttp: '所有传输（包括明文 HTTP）均使用 CONNECT。',
			acceptedTokenAudiencesMatchedAgainstTheJwtAudClaimIfUnsetAudienceValidationIsDisabled:
				'接受的令牌受众，与 JWT 的 `aud` 声明匹配。未设置时禁用受众验证。',
			observeModeRecordWhatTheGuardWouldHaveDoneMetricsStructuredLogButNeverBlockOrMas_tmbdrm:
				'观察模式：记录防护规则本会采取的操作（指标和结构化日志），但从不阻止或遮蔽内容——内容始终原样通过。',
			whetherToEnforceTheWebhookSVerdictOrOnlyObserveItDefaultsToRejectEnforce:
				'是否执行 Webhook 的判定，还是仅进行观察。\n默认为 `reject`（执行）。',
			actionForGuardsThatCannotMaskOnlyRejectOrObserveBedrockWebhookOpenAiModerationGo_efrlc3:
				'适用于无法遮蔽内容、只能拒绝或观察的防护规则的操作。Bedrock、Webhook、OpenAI moderation、Google Model Armor 和 Azure Content Safety 决定标记哪些内容；网关只决定是执行该判定，还是仅记录它。',
			enforceTheGuardSNativeVerdictBlockOrForBedrockAnonymizeThisIsTheDefaultAndPreser_11tq0wd:
				'执行防护规则的原生判定（阻止；对于 Bedrock，则为匿名化）。\n这是默认值，会保留原有的强制执行行为。',
			observeModeInvokeTheGuardAndRecordItsVerdictMetricsStructuredLogButNeverBlockOrM_1ny1vgr:
				'观察模式：调用防护规则并记录其判定（指标和结构化日志），但从不阻止或遮蔽内容——内容始终原样通过。',
			whetherToRejectFlaggedContentOrOnlyObserveItDefaultsToRejectEnforce:
				'是拒绝被标记的内容，还是仅进行观察。\n默认为 `reject`（执行）。',
			whetherToEnforceTheGuardrailSVerdictOrOnlyObserveItRejectTheDefaultEnforcesTheGu_16r281l:
				'是执行防护规则的判定，还是仅进行观察。\n\n`reject`（默认值）会执行防护规则判定：`BLOCKED` 评估会拒绝请求/响应，`ANONYMIZED` 评估会遮蔽匹配内容，与之前完全相同。\n\n`audit` 会记录成功的评估，但不执行其判定。即使 AWS 资源配置为 `BLOCK`/`ANONYMIZE`，`audit` 也保证网关侧不执行阻止或遮蔽。',
			keepASuccessfullyValidatedJwtInItsOriginalLocation: '将验证成功的 JWT 保留在原始位置。',
			modelPatternsThisKeyIsAllowedToAccessOmittedMeansNoAdditionalConstraintAnEmptyLi_1nw65ly:
				'此密钥允许访问的模型匹配模式。\n省略表示不增加限制；空列表表示拒绝所有模型。',
			independentBudgetsChargedAfterLlmResponsesARequestIsNotChargedWhenItsProviderDoe_7b0bcw:
				'在 LLM 响应完成后计费的独立预算。若提供商未上报预算单位所需的用量或成本，则不会对请求计费。',
			aNamedBudgetAttachedToAStandaloneApiKeyUsageIsChargedAfterAnLlmResponseWhenThePr_721r2j:
				'附加到独立 API key 的命名预算。\n\n提供商上报配置单位所需的令牌或成本后，在 LLM 响应完成时计费用量。用量不可用的请求会被记录，但无法事后计费或阻止。',
			stableNameForThisBudgetWithinItsOwningApiKey: '此预算在所属 API key 内的稳定名称。',
			maximumUsageAllowedDuringTheWindow: '该窗口内允许的最大用量。',
			rollingWindowOverWhichUsageWillBeAccumulated: '累计用量所覆盖的滚动窗口。',
			actionTakenWhenTheBudgetIsExceeded: '超出预算时采取的操作。',
			durationOfTheFixedUsageWindowForExample1h24hOr30dWindowsAreAlignedToTheUnixEpoch_cdd7lt:
				'固定用量窗口的时长，例如 `1h`、`24h` 或 `30d`。\n窗口按 Unix 纪元对齐，而不是从第一个请求开始：`1h` 遵循 UTC 整点，`24h` 从 UTC 午夜开始，`30d` 使用连续的 30 天周期，而不是日历月。',
			resolvesSubstrateActorHostnamesThroughTheAteApiForDynamicRouteBackends:
				'通过 ate-api 为动态路由后端解析 Substrate actor 主机名。',
			portOnTheResumedWorkerPodSAtunnelConnectListenerDefaultsTo8443:
				'恢复的 worker Pod 上的 atunnel CONNECT 监听器端口。默认为 8443。',
			howLongSuccessfulActorAssignmentsAreReusedDefaultsTo5s0sDisablesReuse:
				'成功的 actor 分配可复用的时长。默认为 5s；设置为 0s 可禁用复用。',
			boundedRequestParkingWhileASuspendedActorIsWaitingForWorkerCapacity:
				'已暂停 actor 等待 worker 容量期间的有界请求暂存。',
			boundsRequestsHeldWhileAnActorIsWaitingForCapacityToResume:
				'限制 actor 等待容量恢复期间可暂存的请求。',
			maximumTimeToWaitForTheActorToBecomeRoutable: '等待 actor 变为可路由状态的最长时间。',
			maximumConcurrentRequestsThatMayWaitForActorResumptionSetTo0ToDisableParking:
				'等待 actor 恢复的最大并发请求数。设为 0 可禁用请求暂存。',
			initialDelayBetweenResumeActorRetriesWhileParked:
				'请求暂存期间，重试 ResumeActor 之间的初始延迟。',
			multiplierAppliedToTheDelayAfterEachParkedRetry: '每次暂存重试后应用于延迟的倍数。',
			authorizesAnActorSEgressToTheHostnameRecoveredFromAnInternalConnectListener:
				'授权 actor 向从内部 CONNECT 监听器解析出的主机名发起出站连接。',
			maximumNumberOfInFlightHttpRequestsAcrossThisBindThisIncludesHttp1RequestsAndHtt_12ydm0j:
				'此绑定允许同时处理的 HTTP 请求数上限。包括 HTTP/1 请求和 HTTP/2 流。超过上限的请求会立即被拒绝。',
			maximumNumberOfActiveDownstreamConnectionsOnThisBindConnectionsOverTheLimitAreClosedImmediately:
				'此绑定上的活动下游连接数上限。超过上限的连接会立即关闭。',
			resolveTheDialTargetFromRequestMetadataUsingACelExpression:
				'使用 CEL 表达式从请求元数据解析拨号目标。',
			theRawSpiffeIdFirstSpiffeUriSanOfTheDownstreamClientCertificateIfPresentUnlikeId_9zald1:
				'下游客户端证书中的原始 SPIFFE ID（第一个 `spiffe://` URI SAN）（如果存在）。与 `identity` 不同，该字段适用于任意 SPIFFE ID，而不仅是 Istio `spiffe://td/ns/<ns>/sa/<sa>` 格式。',
			guardrailsContainsOneEntryPerPromptGuardGuardrailInterventionInEitherTheRequestO_6q207k:
				'`guardrails` 中每个条目对应一次 prompt-guard 防护规则干预，发生在请求或响应阶段。仅在请求完成后运行的 CEL 中提供，例如日志和指标字段。',
			recordsOnePromptGuardGuardrailIntervention: '记录一次 prompt-guard 防护规则干预。',
			thePhaseTheGuardrailIntervenedInRequestOrResponse:
				'防护规则介入的阶段：`request` 或 `response`。',
			theGuardKindThatIntervenedSuchAsBedrockGuardrails:
				'发生干预的防护规则类型，例如 `bedrockGuardrails`。',
			theActionTheGuardrailTookMaskRejectAuditFailOpen:
				'防护规则采取的操作（mask/reject/audit/failOpen）。',
			theConfiguredGuardrailIdentifier: '已配置的防护规则标识符。',
			theConfiguredGuardrailVersion: '已配置的防护规则版本。',
			theReasonTheGuardrailReportedForItsAction: '防护规则报告其操作的原因。',
			assessmentDetailReportedByTheGuardrailProviderRedactedToMetadataOnlyContentBeari_16jy1g:
				'防护规则提供商报告的评估详情，仅保留元数据；绝不会包含承载内容的字段（例如匹配到的文本）。'
		}
	}
} as const satisfies LocaleShape<typeof en>;

export default zhCN;
