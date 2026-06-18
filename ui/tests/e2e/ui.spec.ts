import { expect, test } from "@playwright/test";
import { emptyConfig, mockGateway, populatedConfig } from "./fixtures";

const pages = [
  ["/", "Gateway Overview"],
  ["/llm/models", "LLM Models"],
  ["/llm/providers", "LLM Providers"],
  ["/llm/policies", "LLM Policies"],
  ["/llm/guardrails", "LLM Guardrails"],
  ["/llm/logs", "Logs"],
  ["/llm/analytics", "Analytics"],
  ["/llm/keys", "Virtual API Keys"],
  ["/llm/playground", "LLM Playground"],
  ["/llm/client-setup", "Client Setup"],
  ["/mcp/servers", "MCP Servers"],
  ["/mcp/policies", "MCP Policies"],
  ["/mcp/playground", "MCP Playground"],
  ["/traffic/listeners", "Traffic Listeners"],
  ["/traffic/routes", "Traffic Routes"],
  ["/cel", "CEL Playground"],
] as const;

test("core pages render with mocked gateway data", async ({ page }) => {
  await mockGateway(page);

  for (const [path, heading] of pages) {
    await page.goto(path);
    await expect(page.getByRole("heading", { name: heading })).toBeVisible();
    await expect(page.locator("body")).not.toContainText(
      "Configuration API unavailable",
    );
  }
});

test("onboards all surfaces from a completely empty config", async ({
  page,
}) => {
  const gateway = await mockGateway(page, {});
  await page.goto("/");

  await expect(
    page.getByRole("heading", { name: "Welcome to Agentgateway" }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: /LLM/ })).toBeVisible();
  await expect(page.getByRole("button", { name: /MCP/ })).toBeVisible();
  await page.getByRole("button", { name: /APIs/ }).click();

  await expect.poll(() => gateway.postedConfigs.length).toBe(1);
  expect(gateway.postedConfigs[0].binds).toEqual([]);
  await expect(
    page.getByRole("heading", { name: "Welcome to Agentgateway" }),
  ).toBeVisible();
  await expect(
    page.locator(".nav-list").getByRole("link", { name: "Listeners" }),
  ).toBeVisible();

  await page.getByRole("button", { name: /LLM/ }).click();
  await expect.poll(() => gateway.postedConfigs.length).toBe(2);
  expect(gateway.postedConfigs[1].llm).toMatchObject({
    port: 4000,
    models: [],
    providers: [],
    virtualModels: [],
  });
  await expect(
    page.getByRole("heading", { name: "Welcome to Agentgateway" }),
  ).toBeVisible();

  await page.getByRole("button", { name: /MCP/ }).click();
  await expect.poll(() => gateway.postedConfigs.length).toBe(3);
  expect(gateway.postedConfigs[2].mcp).toMatchObject({
    port: 3000,
    targets: [],
  });
  await expect(
    page.getByRole("heading", { name: "Welcome to Agentgateway" }),
  ).toBeVisible();
  await expect(page.getByText("3 of 3 enabled")).toBeVisible();
  await page.getByRole("button", { name: "Continue" }).click();
  await expect(
    page.getByRole("heading", { name: "Gateway Overview" }),
  ).toBeVisible();
});

test("raw configuration editor shows schema diagnostics", async ({ page }) => {
  await mockGateway(page);
  await page.goto("/raw-config");

  await expect(
    page.getByRole("heading", { name: "Raw Configuration" }),
  ).toBeVisible();
  await expect
    .poll(async () =>
      page.evaluate(() => Boolean(window.__rawConfigEditor?.getModel())),
    )
    .toBe(true);
  await expect
    .poll(async () =>
      page.evaluate(() =>
        window.__rawConfigEditor?.getModel()?.getLanguageId(),
      ),
    )
    .toBe("yaml");
  await expect
    .poll(async () =>
      page.evaluate(() =>
        window.__rawConfigMonaco?.languages
          .getLanguages()
          .some((language) => language.id === "yaml"),
      ),
    )
    .toBe(true);
  await page.evaluate(() => {
    window.__rawConfigEditor
      ?.getModel()
      ?.setValue("notARealTopLevelField: true\n");
  });

  await expect
    .poll(
      async () =>
        page.evaluate(() => {
          const monaco = window.__rawConfigMonaco;
          const model = window.__rawConfigEditor?.getModel();
          return monaco && model
            ? monaco.editor.getModelMarkers({ resource: model.uri }).length
            : 0;
        }),
      { timeout: 15_000 },
    )
    .toBeGreaterThan(0);

  await page.evaluate(() => {
    window.__rawConfigEditor?.getModel()?.setValue("");
    window.__rawConfigEditor?.setPosition({ lineNumber: 1, column: 1 });
    window.__rawConfigEditor?.focus();
    window.__rawConfigEditor?.trigger(
      "test",
      "editor.action.triggerSuggest",
      {},
    );
  });
  await expect(page.locator(".suggest-widget")).toBeVisible();
  await expect(page.locator(".suggest-widget")).toContainText("llm");
});

test("raw configuration saved banner only follows an explicit save", async ({
  page,
}) => {
  await mockGateway(page);
  await page.goto("/raw-config");

  await expect(
    page.getByRole("heading", { name: "Raw Configuration" }),
  ).toBeVisible();
  await expect
    .poll(async () =>
      page.evaluate(() => Boolean(window.__rawConfigEditor?.getModel())),
    )
    .toBe(true);
  const original = await page.evaluate(
    () => window.__rawConfigEditor?.getModel()?.getValue() ?? "",
  );

  await page.evaluate((value) => {
    window.__rawConfigEditor
      ?.getModel()
      ?.setValue(`${value}\n# temporary edit\n`);
  }, original);
  await expect(page.getByText("Configuration saved")).toHaveCount(0);

  await page.evaluate((value) => {
    window.__rawConfigEditor?.getModel()?.setValue(value);
  }, original);
  await expect(page.getByText("Configuration saved")).toHaveCount(0);
});

test("creates a weighted virtual model with a concrete wildcard target", async ({
  page,
}) => {
  const gateway = await mockGateway(page, emptyConfigWithModels());
  await page.goto("/llm/models");

  await page.getByRole("button", { name: "Add virtual model" }).click();
  await page.getByLabel("Virtual model name").fill("balanced");
  await expect(
    page.getByRole("button", { name: "Save virtual model" }),
  ).toBeDisabled();

  await page
    .getByRole("textbox", { name: "Specific model" })
    .fill("gpt-5.4-nano");
  await page.getByRole("button", { name: "Save virtual model" }).click();

  await expect.poll(() => gateway.postedConfigs.length).toBe(1);
  const latest = gateway.postedConfigs[0] as {
    llm?: {
      virtualModels?: Array<{
        name: string;
        routing: {
          weighted?: { targets: Array<{ model: string; weight?: number }> };
        };
      }>;
    };
  };
  expect(latest.llm?.virtualModels?.[0]).toMatchObject({
    name: "balanced",
    routing: {
      weighted: {
        targets: [{ model: "openai/gpt-5.4-nano", weight: 1 }],
      },
    },
  });
});

test("reveals a virtual API key explicitly", async ({ page }) => {
  await mockGateway(page);
  await page.goto("/llm/keys");

  await expect(page.getByText("agw_sk_testkey123456789")).toHaveCount(0);
  await page.getByRole("button", { name: "Show full key" }).click();
  await expect(page.getByText("agw_sk_testkey123456789")).toBeVisible();
});

test("LLM playground sends selected virtual model name", async ({ page }) => {
  const gateway = await mockGateway(page);
  await page.goto("/llm/playground");

  await page.getByRole("combobox", { name: "Model" }).click();
  await page.getByRole("option", { name: /resilient/ }).click();
  await page.getByLabel("User message").fill("ping");
  await page.getByRole("button", { name: "Send" }).click();

  await expect(
    page.locator(".chat-message.assistant .chat-bubble"),
  ).toContainText("pong");
  await expect.poll(() => gateway.chatRequests.length).toBe(1);
  expect(gateway.chatRequests[0].model).toBe("resilient");
});

test("MCP playground initializes, lists tools, and calls a tool", async ({
  page,
}) => {
  const gateway = await mockGateway(page);
  await page.goto("/mcp/playground");

  await expect(page.getByRole("textbox", { name: "Bearer token" })).toHaveCount(
    0,
  );
  await page.getByText("Authorization header").click();
  await page.getByRole("textbox", { name: "Bearer token" }).fill("mcp-secret");

  await page.getByRole("button", { name: "Initialize", exact: true }).click();
  await expect(page.getByText("initialized")).toBeVisible();
  await expect(page.getByRole("combobox", { name: "Tool" })).toContainText(
    "echo",
  );

  await page.getByLabel("text *").fill("hello");
  await page.getByRole("button", { name: "Call tool" }).click();

  await expect(
    page.locator(".mcp-text-output").getByText("echo result", { exact: true }),
  ).toBeVisible();
  await expect
    .poll(() =>
      gateway.mcpRequests.some((request) => request.method === "tools/call"),
    )
    .toBe(true);
  expect(
    gateway.mcpHeaders.every(
      (headers) => headers.authorization === "Bearer mcp-secret",
    ),
  ).toBe(true);
});

test("edits top-level MCP policies", async ({ page }) => {
  const gateway = await mockGateway(page, emptyConfig());
  await page.goto("/mcp/policies");

  await page.getByRole("button", { name: /CORS/ }).click();
  await page.getByRole("button", { name: "Add current origin" }).click();
  await page
    .getByRole("dialog", { name: "CORS" })
    .getByRole("button", { name: "Save" })
    .click();

  await expect.poll(() => gateway.postedConfigs.length).toBe(1);
  const saved = gateway.postedConfigs.at(-1) as {
    mcp?: { policies?: { cors?: { allowOrigins?: string[] } } };
  };
  expect(saved.mcp?.policies?.cors?.allowOrigins).toContain(
    "http://127.0.0.1:19100",
  );
});

test("Client Setup includes virtual models in snippets", async ({ page }) => {
  await mockGateway(page);
  await page.goto("/llm/client-setup");

  await page.getByRole("combobox", { name: "Model" }).click();
  await page.getByRole("option", { name: /resilient/ }).click();

  await expect(
    page.locator(".client-setup-summary code").filter({ hasText: "resilient" }),
  ).toBeVisible();
  await expect(page.locator(".client-code-block")).toContainText(
    '"model": "resilient"',
  );
});

test("creates a traffic bind and listener", async ({ page }) => {
  const gateway = await mockGateway(page, emptyConfig());
  await page.goto("/traffic/listeners");

  await page.getByRole("button", { name: "Add bind" }).first().click();
  await page.getByRole("textbox", { name: "Port" }).fill("8181");
  await page.getByRole("button", { name: "Save bind" }).click();

  await expect.poll(() => gateway.postedConfigs.length).toBe(1);
  await page.getByRole("button", { name: "Add listener" }).first().click();
  await page.getByRole("textbox", { name: "Name", exact: true }).fill("public");
  await page.getByRole("textbox", { name: /Hostname/ }).fill("example.test");
  await page.getByRole("button", { name: "Save listener" }).click();

  await expect.poll(() => gateway.postedConfigs.length).toBe(2);
  const latest = gateway.postedConfigs.at(-1) as {
    binds?: Array<{
      port: number;
      listeners: Array<{ name?: string; routes?: unknown[] }>;
    }>;
  };
  expect(latest.binds?.[0]).toMatchObject({
    port: 8181,
    listeners: [{ name: "public", routes: [] }],
  });
});

test("creates HTTP and TCP traffic routes", async ({ page }) => {
  const gateway = await mockGateway(page, trafficBaseConfig());
  await page.goto("/traffic/routes");

  await page.getByRole("button", { name: "Add route" }).first().click();
  await page
    .getByRole("textbox", { name: "Name", exact: true })
    .fill("new-http");
  await page.getByRole("textbox", { name: "Path" }).fill("/new");
  await page.getByRole("button", { name: "Save route" }).click();

  await expect.poll(() => gateway.postedConfigs.length).toBe(1);
  await page.getByRole("button", { name: "Add route" }).first().click();
  await page.getByRole("combobox", { name: "Listener" }).click();
  await page.getByRole("option", { name: /tcp-listener/ }).click();
  await page
    .getByRole("textbox", { name: "Name", exact: true })
    .fill("new-tcp");
  await page.getByRole("button", { name: "Save route" }).click();

  await expect.poll(() => gateway.postedConfigs.length).toBe(2);
  const latest = gateway.postedConfigs.at(-1) as {
    binds?: Array<{
      listeners: Array<{ routes?: unknown[]; tcpRoutes?: unknown[] }>;
    }>;
  };
  expect(latest.binds?.[0].listeners[0].routes).toHaveLength(1);
  expect(latest.binds?.[1].listeners[0].tcpRoutes).toHaveLength(1);
});

test("edits listener and route policies from traffic drawers", async ({
  page,
}) => {
  const gateway = await mockGateway(page);
  await page.goto("/traffic/listeners");

  await page
    .getByRole("row", { name: /public-http/ })
    .getByRole("button", { name: "Edit listener" })
    .click();
  await page.getByText("Listener policies").click();
  await page.getByRole("button", { name: /CORS/ }).click();
  await page.getByRole("button", { name: "Add current origin" }).click();
  await page
    .locator(".drawer.nested")
    .last()
    .getByRole("button", { name: "Save" })
    .click();
  await page
    .locator(".drawer.nested")
    .last()
    .getByRole("button", { name: "Close" })
    .click();
  await page.getByRole("button", { name: "Save listener" }).click();

  await expect.poll(() => gateway.postedConfigs.length).toBe(1);
  const listenerPolicy = gateway.postedConfigs.at(-1) as {
    binds?: Array<{
      listeners: Array<{ policies?: { cors?: { allowOrigins?: string[] } } }>;
    }>;
  };
  expect(
    listenerPolicy.binds?.[0].listeners[0].policies?.cors?.allowOrigins,
  ).toContain("http://127.0.0.1:19100");

  await page.goto("/traffic/routes");
  await page
    .getByRole("row", { name: /api/ })
    .getByRole("button", { name: "Edit route" })
    .click();
  await page.getByText("Route policies").click();
  await page.getByRole("button", { name: /CORS/ }).click();
  await page.getByRole("button", { name: "Add current origin" }).click();
  await page
    .locator(".drawer.nested")
    .last()
    .getByRole("button", { name: "Save" })
    .click();
  await page
    .locator(".drawer.nested")
    .last()
    .getByRole("button", { name: "Close" })
    .click();
  await page.getByRole("button", { name: "Save route" }).click();

  await expect.poll(() => gateway.postedConfigs.length).toBe(2);
  const routePolicy = gateway.postedConfigs.at(-1) as {
    binds?: Array<{
      listeners: Array<{
        routes?: Array<{ policies?: { cors?: { allowOrigins?: string[] } } }>;
      }>;
    }>;
  };
  expect(
    routePolicy.binds?.[0].listeners[0].routes?.[0].policies?.cors
      ?.allowOrigins,
  ).toContain("http://127.0.0.1:19100");
});

function emptyConfigWithModels() {
  const config = populatedConfig();
  const llm = config.llm as { virtualModels?: unknown[] };
  llm.virtualModels = [];
  return config;
}

function trafficBaseConfig() {
  const config = emptyConfig();
  config.binds = [
    {
      port: 8080,
      listeners: [{ name: "http-listener", protocol: "HTTP", routes: [] }],
    },
    {
      port: 9090,
      listeners: [{ name: "tcp-listener", protocol: "TCP", tcpRoutes: [] }],
    },
  ];
  return config;
}
