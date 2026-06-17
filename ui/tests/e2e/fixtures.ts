import type { Page, Route } from "@playwright/test";

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

export async function mockGateway(
  page: Page,
  initialConfig: TestConfig = populatedConfig(),
) {
  let config = structuredClone(initialConfig);
  const postedConfigs: TestConfig[] = [];
  const chatRequests: Array<Record<string, unknown>> = [];
  const mcpRequests: Array<Record<string, unknown>> = [];
  const mcpHeaders: Array<Record<string, string>> = [];

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
    mcpRequests,
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
