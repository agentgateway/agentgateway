import type { Page, Route } from "@playwright/test";
import { mcpSettingsFields } from "../../src/config";

export type TestConfig = Record<string, unknown>;

export function bareConfig(): TestConfig {
  return {
    config: {
      logging: {
        database: {
          url: "sqlite:///tmp/gw-logs.db",
        },
      },
    },
    binds: [],
  };
}

export function emptyConfig(): TestConfig {
  return {
    ...bareConfig(),
    llm: {
      port: 4000,
      models: [],
      providers: [],
      virtualModels: [],
      policies: {
        cors: {
          allowOrigins: ["http://127.0.0.1:19100"],
          allowHeaders: ["*"],
          allowMethods: ["GET", "POST"],
        },
      },
    },
    mcp: {
      targets: [],
      policies: {
        cors: {
          allowOrigins: ["http://127.0.0.1:19100"],
          allowHeaders: ["*"],
          allowMethods: ["GET", "POST"],
          exposeHeaders: ["Mcp-Session-Id"],
        },
      },
    },
    binds: [],
  };
}

export function populatedConfig(): TestConfig {
  return {
    ...emptyConfig(),
    llm: {
      port: 4000,
      providers: [
        {
          name: "openai-shared",
          provider: "openai",
          params: {
            apiKey: "$OPENAI_API_KEY",
          },
        },
      ],
      models: [
        {
          name: "openai/*",
          provider: "openai",
          params: {
            apiKey: "$OPENAI_API_KEY",
          },
          transformation: {
            model: 'llmRequest.model.stripPrefix("openai/")',
          },
        },
        {
          name: "anthropic/*",
          provider: "anthropic",
          params: {
            apiKey: "$ANTHROPIC_API_KEY",
          },
          transformation: {
            model: 'llmRequest.model.stripPrefix("anthropic/")',
          },
        },
        {
          name: "fast",
          provider: {
            reference: "openai-shared",
          },
          params: {
            model: "gpt-5.4-nano",
          },
        },
      ],
      virtualModels: [
        {
          name: "resilient",
          routing: {
            failover: {
              targets: [
                { model: "openai/gpt-5.4-nano", priority: 0 },
                { model: "anthropic/claude-haiku-4-5", priority: 1 },
              ],
            },
          },
        },
      ],
      policies: {
        cors: {
          allowOrigins: ["http://127.0.0.1:19100"],
          allowHeaders: ["*"],
          allowMethods: ["GET", "POST"],
        },
        apiKey: {
          keys: [
            {
              key: "agw_sk_testkey123456789",
              metadata: {
                name: "Test key",
                owner: "platform",
              },
            },
          ],
          mode: "optional",
          location: {
            header: {
              name: "authorization",
              prefix: "Bearer ",
            },
          },
        },
      },
    },
    mcp: {
      targets: [
        {
          name: "everything",
          mcp: {
            host: "http://localhost:3001/mcp",
          },
        },
      ],
      policies: {
        cors: {
          allowOrigins: ["http://127.0.0.1:19100"],
          allowHeaders: ["*"],
          allowMethods: ["GET", "POST"],
          exposeHeaders: ["Mcp-Session-Id"],
        },
      },
    },
    binds: [
      {
        port: 8080,
        listeners: [
          {
            name: "public-http",
            hostname: "example.com",
            protocol: "HTTP",
            routes: [
              {
                name: "api",
                hostnames: ["example.com"],
                matches: [{ path: { pathPrefix: "/api" } }],
                backends: [{ host: "localhost:9000" }],
              },
              {
                name: "legacy-ai",
                hostnames: ["legacy.example.com"],
                matches: [{ path: { pathPrefix: "/" } }],
                backends: [
                  { ai: { name: "legacy", provider: { openAI: {} } } },
                ],
              },
            ],
          },
        ],
      },
      {
        port: 9090,
        listeners: [
          {
            name: "tcp",
            hostname: "tcp.example.com",
            protocol: "TCP",
            tcpRoutes: [
              {
                name: "tcp-main",
                hostnames: ["tcp.example.com"],
                backends: [{ host: "localhost:3306" }],
              },
            ],
          },
        ],
      },
    ],
  };
}

export function sameOriginGatewayConfig(): TestConfig {
  const config = populatedConfig();
  config.gateways = {
    public: {
      port: 8080,
    },
  };
  config.ui = {
    gateways: ["public"],
  };

  const llm = config.llm as Record<string, unknown>;
  llm.gateways = ["public"];
  delete llm.port;
  const llmPolicies = llm.policies as Record<string, unknown> | undefined;
  delete llmPolicies?.cors;

  const mcp = config.mcp as Record<string, unknown>;
  mcp.gateways = ["public"];
  delete mcp.port;
  const mcpPolicies = mcp.policies as Record<string, unknown> | undefined;
  delete mcpPolicies?.cors;

  return config;
}

export function implicitDefaultGatewayConfig(): TestConfig {
  const config = populatedConfig();
  config.gateways = {
    default: {
      port: 8080,
    },
  };
  config.ui = {};

  const llm = config.llm as Record<string, unknown>;
  delete llm.gateways;
  delete llm.port;
  delete llm.tls;
  const llmPolicies = llm.policies as Record<string, unknown> | undefined;
  delete llmPolicies?.cors;

  const mcp = config.mcp as Record<string, unknown>;
  delete mcp.gateways;
  delete mcp.port;
  const mcpPolicies = mcp.policies as Record<string, unknown> | undefined;
  delete mcpPolicies?.cors;

  return config;
}

export async function mockGateway(
  page: Page,
  initialConfig: TestConfig = populatedConfig(),
) {
  let config = structuredClone(initialConfig);
  const postedConfigs: TestConfig[] = [];
  const chatRequests: Array<Record<string, unknown>> = [];
  const chatUrls: string[] = [];
  const mcpRequests: Array<Record<string, unknown>> = [];
  const mcpUrls: string[] = [];
  const mcpHeaders: Array<Record<string, string>> = [];

  await page.route("**/api/runtime", async (route) => {
    await json(route, {
      build: {
        version: "test",
        gitRevision: "test",
        rustVersion: "test",
        buildProfile: "test",
        buildTarget: "test",
      },
      ui: { gatewayMode: "standalone", configStoreMode: "file" },
    });
  });

  await page.route("**/api/config/effective", async (route) => {
    await json(route, config);
  });

  await page.route("**/config", async (route) => {
    if (route.request().method() === "GET") {
      await json(route, config);
      return;
    }
    if (route.request().method() === "POST") {
      config = route.request().postDataJSON() as TestConfig;
      postedConfigs.push(structuredClone(config));
      await json(route, { status: "ok", message: "saved" });
      return;
    }
    await route.fallback();
  });

  await page.route("**/api/config/resources**", async (route) => {
    const path = new URL(route.request().url()).pathname;
    const suffix = path.split("/api/config/resources")[1] ?? "";
    const [kind, id] = suffix
      .split("/")
      .filter(Boolean)
      .map(decodeURIComponent);
    if (route.request().method() === "GET") {
      await json(route, { resources: [] });
      return;
    }
    if (route.request().method() === "PUT") {
      const body = route.request().postDataJSON() as {
        value?: unknown;
        resources?: Array<{ value: unknown }>;
      };
      const values = body.resources?.map((resource) => resource.value) ?? [
        body.value,
      ];
      for (const value of values) {
        if (kind && value !== undefined)
          upsertFileConfigResource(config, kind, value, id);
      }
      postedConfigs.push(structuredClone(config));
      await json(route, { resources: [] });
      return;
    }
    if (route.request().method() === "DELETE" && kind && id) {
      deleteFileConfigResource(config, kind, id);
      postedConfigs.push(structuredClone(config));
      await json(route, { status: "ok", message: "deleted" });
      return;
    }
    await route.fallback();
  });

  await page.route("**/api/logs/search", async (route) => {
    await json(route, {
      logs: [
        {
          id: "log-1",
          startedAt: new Date(Date.now() - 1500).toISOString(),
          completedAt: new Date().toISOString(),
          durationMs: 321,
          traceId: "trace-123456789",
          spanId: "span-1",
          httpStatus: 200,
          error: null,
          genAi: {
            providerName: "anthropic",
            requestModel: "resilient",
            responseModel: "claude-haiku-4-5",
          },
          usage: {
            inputTokens: 12,
            outputTokens: 18,
            totalTokens: 30,
          },
          cost: 0.0005,
          hasPayload: true,
        },
      ],
      nextCursor: null,
    });
  });

  await page.route("**/api/logs/get", async (route) => {
    await json(route, {
      log: {
        id: "log-1",
        startedAt: new Date(Date.now() - 1500).toISOString(),
        completedAt: new Date().toISOString(),
        durationMs: 321,
        traceId: "trace-123456789",
        spanId: "span-1",
        httpStatus: 200,
        error: null,
        genAi: {
          providerName: "anthropic",
          requestModel: "resilient",
          responseModel: "claude-haiku-4-5",
        },
        usage: {
          inputTokens: 12,
          outputTokens: 18,
          totalTokens: 30,
        },
        cost: 0.0005,
        hasPayload: true,
        payload: {
          requestPrompt: [{ role: "user", content: "ping" }],
          responseCompletion: "pong",
        },
      },
    });
  });

  await page.route("**/api/logs/analytics/summary", async (route) => {
    const now = new Date();
    await json(route, {
      timeRange: {
        from: new Date(now.getTime() - 60 * 60 * 1000).toISOString(),
        to: now.toISOString(),
      },
      bucketSeconds: 900,
      buckets: [
        {
          start: new Date(now.getTime() - 15 * 60 * 1000).toISOString(),
          group: { requestModel: "resilient" },
          inputTokens: 120,
          outputTokens: 220,
          totalTokens: 340,
          cost: 0.0042,
          requests: 7,
        },
      ],
      groups: [
        {
          group: { requestModel: "resilient" },
          inputTokens: 120,
          outputTokens: 220,
          totalTokens: 340,
          cost: 0.0042,
          requests: 7,
        },
      ],
    });
  });

  await page.route("**/api/logs/analytics/token-usage", async (route) => {
    await json(route, {
      groups: [
        {
          key: "resilient",
          inputTokens: 120,
          outputTokens: 220,
          totalTokens: 340,
          requests: 7,
        },
      ],
    });
  });

  await page.route("**/v1/chat/completions", async (route) => {
    chatUrls.push(route.request().url());
    chatRequests.push(
      route.request().postDataJSON() as Record<string, unknown>,
    );
    await json(route, {
      id: "chatcmpl-test",
      choices: [
        {
          message: {
            role: "assistant",
            content: "pong",
          },
        },
      ],
    });
  });

  await page.route("**/mcp", async (route) => {
    const body = route.request().postDataJSON() as { method?: string };
    mcpUrls.push(route.request().url());
    mcpRequests.push(body as Record<string, unknown>);
    mcpHeaders.push(route.request().headers());
    if (body.method === "initialize") {
      await json(
        route,
        { jsonrpc: "2.0", id: 1, result: { protocolVersion: "2025-03-26" } },
        { "Mcp-Session-Id": "session-1" },
      );
      return;
    }
    if (body.method === "tools/list") {
      await json(
        route,
        {
          jsonrpc: "2.0",
          id: 2,
          result: {
            tools: [
              {
                name: "echo",
                description: "Echoes back the input string",
                inputSchema: {
                  type: "object",
                  properties: {
                    text: { type: "string", description: "Text to echo" },
                  },
                  required: ["text"],
                },
              },
            ],
          },
        },
        { "Mcp-Session-Id": "session-1" },
      );
      return;
    }
    if (body.method === "tools/call") {
      await json(
        route,
        {
          jsonrpc: "2.0",
          id: 3,
          result: {
            content: [{ type: "text", text: "echo result" }],
          },
        },
        { "Mcp-Session-Id": "session-1" },
      );
      return;
    }
    await json(
      route,
      { jsonrpc: "2.0", result: {} },
      { "Mcp-Session-Id": "session-1" },
    );
  });

  return {
    postedConfigs,
    chatRequests,
    chatUrls,
    mcpRequests,
    mcpUrls,
    mcpHeaders,
  };
}

async function json(
  route: Route,
  body: unknown,
  headers: Record<string, string> = {},
) {
  await route.fulfill({
    status: 200,
    contentType: "application/json",
    headers,
    body: JSON.stringify(body),
  });
}

export function configWithClaudeSubscriptionKey(): TestConfig {
  const config = populatedConfig();
  const llm = config.llm as {
    models: Array<Record<string, unknown>>;
    providers?: unknown[];
  };
  llm.models.push({
    name: "claude-sub",
    provider: "anthropic",
    params: {
      apiKey: "sk-ant-oat01-testkey1234567890abcdef",
    },
  });
  return config;
}

function upsertFileConfigResource(
  config: TestConfig,
  kind: string,
  input: unknown,
  previousId?: string,
) {
  const value = structuredClone(record(input));
  if (kind === "modelCatalog") {
    const configSection = ensureRecord(config, "config");
    const sources = array(configSection.modelCatalog).filter(
      (source) => record(source).inline === undefined,
    );
    if (value.custom !== undefined) sources.push({ inline: value.custom });
    configSection.modelCatalog = sources;
    return;
  }
  if (kind === "llm.provider")
    return upsertSectionList(config, "llm", "providers", value, previousId);
  if (kind === "llm.model")
    return upsertSectionList(
      config,
      "llm",
      "models",
      value,
      previousId,
      (item) => stringValue(item.id) || stringValue(item.name),
    );
  if (kind === "llm.virtualModel")
    return upsertSectionList(config, "llm", "virtualModels", value, previousId);
  if (kind === "mcp.target")
    return upsertSectionList(config, "mcp", "targets", value, previousId);
  if (kind === "llm.policy" || kind === "mcp.policy" || kind === "ui.policy") {
    if (!previousId) throw new Error(`${kind} writes require an id`);
    const sectionName = kind.split(".")[0];
    const section = ensureRecord(config, sectionName);
    const policies = ensureRecord(section, "policies");
    const policyId = previousId;
    const existingKeys =
      kind === "llm.policy" && policyId === "apiKey"
        ? record(policies.apiKey).keys
        : undefined;
    if (existingKeys !== undefined) value.keys = existingKeys;
    policies[policyId] = value;
    return;
  }
  if (kind === "llm.apiKey") {
    const policies = ensureRecord(ensureRecord(config, "llm"), "policies");
    const policy = ensureRecord(policies, "apiKey");
    const keys = ensureArray(policy, "keys");
    const index = previousId
      ? keys.findIndex((key, keyIndex) => {
          const keyValue = record(key);
          return (
            (stringValue(record(keyValue.metadata).id) ||
              `@index:${keyIndex}`) === previousId
          );
        })
      : -1;
    if (!previousId || !previousId.startsWith("@index:")) {
      value.metadata = {
        ...record(value.metadata),
        id: previousId ?? `test-key-${keys.length + 1}`,
      };
    }
    if (index >= 0) keys[index] = value;
    else keys.push(value);
    return;
  }
  if (kind === "mcp.settings") {
    const mcp = ensureRecord(config, "mcp");
    for (const field of mcpSettingsFields) {
      if (value[field] === undefined) delete mcp[field];
      else mcp[field] = value[field];
    }
    return;
  }
  if (kind === "traffic.gateway") {
    const name = stringValue(value.name);
    if (!name) throw new Error("traffic.gateway writes require a name");
    const gateways = ensureRecord(config, "gateways");
    if (previousId && previousId !== name) delete gateways[previousId];
    delete value.name;
    gateways[name] = value;
    return;
  }
  if (kind === "traffic.route" || kind === "traffic.tcpRoute") {
    const field = kind === "traffic.route" ? "routes" : "tcpRoutes";
    const values = ensureArray(config, field);
    const name = stringValue(value.name);
    if (!name) throw new Error(`${kind} writes require a name`);
    const lookup = previousId ?? name;
    const index = values.findIndex(
      (item) => stringValue(record(item).name) === lookup,
    );
    if (index >= 0) values[index] = value;
    else values.push(value);
  }
}

function deleteFileConfigResource(
  config: TestConfig,
  kind: string,
  id: string,
) {
  if (kind === "modelCatalog") {
    const configSection = record(config.config);
    configSection.modelCatalog = array(configSection.modelCatalog).filter(
      (source) => record(source).inline === undefined,
    );
    return;
  }
  if (kind === "llm.provider")
    return deleteSectionList(config, "llm", "providers", id);
  if (kind === "llm.model")
    return deleteSectionList(
      config,
      "llm",
      "models",
      id,
      (item) => stringValue(item.id) || stringValue(item.name),
    );
  if (kind === "llm.virtualModel")
    return deleteSectionList(config, "llm", "virtualModels", id);
  if (kind === "mcp.target")
    return deleteSectionList(config, "mcp", "targets", id);
  if (kind === "llm.policy" || kind === "mcp.policy" || kind === "ui.policy") {
    delete record(record(config[kind.split(".")[0]]).policies)[id];
    return;
  }
  if (kind === "llm.apiKey") {
    const policy = record(record(record(config.llm).policies).apiKey);
    policy.keys = array(policy.keys).filter((key, index) => {
      const value = record(key);
      return (
        (stringValue(record(value.metadata).id) || `@index:${index}`) !== id
      );
    });
    return;
  }
  if (kind === "mcp.settings") {
    const mcp = record(config.mcp);
    for (const field of mcpSettingsFields) delete mcp[field];
    return;
  }
  if (kind === "traffic.gateway") {
    delete record(config.gateways)[id];
    return;
  }
  if (kind === "traffic.route" || kind === "traffic.tcpRoute") {
    const field = kind === "traffic.route" ? "routes" : "tcpRoutes";
    config[field] = array(config[field]).filter(
      (item) => stringValue(record(item).name) !== id,
    );
  }
}

function upsertSectionList(
  config: TestConfig,
  sectionName: string,
  field: string,
  value: Record<string, unknown>,
  previousId?: string,
  identity: (value: Record<string, unknown>) => string = (item) =>
    stringValue(item.name),
) {
  const values = ensureArray(ensureRecord(config, sectionName), field);
  const lookup = previousId ?? identity(value);
  const index = values.findIndex((item) => identity(record(item)) === lookup);
  if (index >= 0) values[index] = value;
  else values.push(value);
}

function deleteSectionList(
  config: TestConfig,
  sectionName: string,
  field: string,
  id: string,
  identity: (value: Record<string, unknown>) => string = (item) =>
    stringValue(item.name),
) {
  const section = record(config[sectionName]);
  section[field] = array(section[field]).filter(
    (item) => identity(record(item)) !== id,
  );
}

function ensureRecord(parent: Record<string, unknown>, field: string) {
  const existing = record(parent[field]);
  parent[field] = existing;
  return existing;
}

function ensureArray(parent: Record<string, unknown>, field: string) {
  const existing = array(parent[field]);
  parent[field] = existing;
  return existing;
}

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function array(value: unknown): unknown[] {
  return Array.isArray(value) ? value : [];
}

function stringValue(value: unknown) {
  return typeof value === "string" ? value : "";
}
