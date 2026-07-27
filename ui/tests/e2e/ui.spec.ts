import { expect, test } from "@playwright/test";
import {
  configWithClaudeSubscriptionKey,
  emptyConfig,
  implicitDefaultGatewayConfig,
  mockGateway,
  populatedConfig,
  sameOriginGatewayConfig,
} from "./fixtures";

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
  ["/traffic/gateways", "Traffic Gateways"],
  ["/traffic/listeners", "Traffic Listeners"],
  ["/traffic/routes", "Traffic Routes"],
  ["/cel", "CEL Playground"],
  ["/settings", "UI Settings"],
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
  expect(gateway.postedConfigs[0].gateways).toMatchObject({
    public: { port: 8080 },
  });
  await expect(
    page.getByRole("heading", { name: "Welcome to Agentgateway" }),
  ).toBeVisible();
  await expect(
    page.locator(".nav-list").getByRole("link", { name: "Gateways" }),
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

test("enables traffic consistently from get started", async ({ page }) => {
  const gateway = await mockGateway(page, {});
  await page.goto("/traffic/get-started");

  await page.getByRole("button", { name: "Enable", exact: true }).click();

  await expect.poll(() => gateway.postedConfigs.length).toBe(1);
  expect(gateway.postedConfigs[0].gateways).toEqual({
    public: { port: 8080 },
  });
});

test("controls the reserved default gateway name", async ({ page }) => {
  await mockGateway(page, {
    gateways: { public: { port: 8080 } },
    llm: { models: [], providers: [], virtualModels: [] },
    mcp: { targets: [] },
    ui: {},
  });
  await page.goto("/traffic/gateways");
  await page.getByRole("button", { name: "Add gateway" }).click();

  const defaultGateway = page.getByRole("checkbox", {
    name: /Default gateway/,
  });
  const name = page.getByRole("textbox", { name: /^Name/ });
  await defaultGateway.check();
  await expect(name).toHaveValue("default");
  await expect(name).toBeDisabled();
  await expect(page.getByText(/impact LLM, UI, MCP traffic/)).toBeVisible();

  await defaultGateway.uncheck();
  await expect(name).toHaveValue("public-2");
  await name.fill("default");
  await expect(page.getByText("Default name is reserved")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Save gateway" }),
  ).toBeDisabled();
});

test("does not allow a second default gateway", async ({ page }) => {
  await mockGateway(page, {
    gateways: { default: { port: 8080 } },
  });
  await page.goto("/traffic/gateways");

  await page.getByRole("button", { name: "Edit gateway" }).click();
  await expect(page.getByText(/impact .* traffic/)).not.toBeVisible();
  await page.getByRole("button", { name: "Close" }).click();

  await page.getByRole("button", { name: "Add gateway" }).click();

  await expect(
    page.getByRole("checkbox", { name: /Default gateway/ }),
  ).toBeDisabled();
  await expect(
    page.getByText("Another gateway is already the default gateway."),
  ).toBeVisible();
});

test("hybrid settings shows file diff without applying it", async ({
  page,
}) => {
  const gateway = await mockGateway(page, {
    gateways: { public: { port: 8080 } },
  });
  await page.route("**/api/runtime", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        build: {},
        ui: { gatewayMode: "standalone", configStoreMode: "hybrid" },
      }),
    }),
  );
  await page.goto("/settings");

  await page.getByRole("combobox", { name: "Public UI gateway" }).click();
  await page.getByRole("option", { name: /public/ }).click();
  const apply = page.getByRole("button", { name: "Save UI gateway" });
  await expect(apply).toBeDisabled();

  await page.getByRole("button", { name: "View diff" }).click();
  const diff = page.locator(".drawer.nested");
  await expect(diff).toBeVisible();
  const save = diff.getByRole("button", { name: "Save" });
  await expect(save).toBeDisabled();
  await save.hover({ force: true });
  await expect(
    page.getByRole("tooltip").getByText(/read-only in hybrid mode/),
  ).toBeVisible();
  expect(gateway.postedConfigs).toHaveLength(0);

  await page.keyboard.down("Control");
  await page.keyboard.down("Shift");
  await expect(
    page.getByRole("tooltip").getByText(/Override active/),
  ).toBeVisible();
  await save.click({ force: true });
  await page.keyboard.up("Shift");
  await page.keyboard.up("Control");
  await expect.poll(() => gateway.postedConfigs.length).toBe(1);
  await expect(diff).not.toBeVisible();
});

test("raw configuration lists hybrid database resources with masked keys", async ({
  page,
}) => {
  await mockGateway(page, {});
  await page.route("**/api/runtime", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        build: {},
        ui: { gatewayMode: "standalone", configStoreMode: "hybrid" },
      }),
    }),
  );
  await page.route("**/api/config/resources", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        resources: [
          {
            kind: "llm.model",
            id: "model-id",
            value: { id: "model-id", name: "test-model" },
            revision: 2,
            createdAt: "2026-07-10T00:00:00Z",
            updatedAt: "2026-07-10T01:00:00Z",
          },
          {
            kind: "llm.apiKey",
            id: "key-id",
            value: {
              key: "agw_sk_supersecret123",
              metadata: { id: "key-id", name: "Test key" },
            },
            revision: 1,
            createdAt: "2026-07-10T00:00:00Z",
            updatedAt: "2026-07-10T01:00:00Z",
          },
        ],
      }),
    }),
  );
  await page.goto("/raw-config");
  await page.getByRole("tab", { name: "Database" }).click();

  await expect(page.getByRole("cell", { name: "llm.model" })).toBeVisible();
  await expect(page.getByRole("cell", { name: "llm.apiKey" })).toBeVisible();
  await page
    .getByRole("row", { name: /llm.apiKey/ })
    .getByText("View JSON")
    .click();
  await expect(page.getByText("agw_sk_...t123")).toBeVisible();
  await expect(page.locator("body")).not.toContainText("agw_sk_supersecret123");
});

test("onboards LLM and MCP onto the UI gateway when present", async ({
  page,
}) => {
  const gateway = await mockGateway(page, {
    config: {},
    gateways: {
      default: {
        port: 4000,
      },
    },
    ui: {
      gateways: "default",
    },
  });
  await page.goto("/");

  await expect(
    page.getByRole("heading", { name: "Welcome to Agentgateway" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: /APIs enabled/ }),
  ).toBeDisabled();

  await page.getByRole("button", { name: /LLM/ }).click();
  await expect.poll(() => gateway.postedConfigs.length).toBe(1);
  expect(gateway.postedConfigs[0].llm).toMatchObject({
    gateways: "default",
    models: [],
    providers: [],
    virtualModels: [],
  });
  expect(gateway.postedConfigs[0].llm).not.toHaveProperty("port");

  await page.getByRole("button", { name: /MCP/ }).click();
  await expect.poll(() => gateway.postedConfigs.length).toBe(2);
  expect(gateway.postedConfigs[1].mcp).toMatchObject({
    gateways: "default",
    targets: [],
  });
  expect(gateway.postedConfigs[1].mcp).not.toHaveProperty("port");

  await expect(
    page.getByRole("button", { name: /APIs enabled/ }),
  ).toBeVisible();
  await expect(
    page.locator(".nav-list").getByRole("link", { name: "Gateways" }),
  ).toBeVisible();
});

test("homepage treats top-level gateways as traffic", async ({ page }) => {
  await mockGateway(page, {
    gateways: {
      default: {
        port: 4000,
      },
    },
    routes: [],
  });
  await page.goto("/");

  const traffic = page
    .locator(".surface-row")
    .filter({ has: page.getByText("Traffic") });
  await expect(traffic).toContainText("Enabled");
  await expect(traffic).toContainText("1 gateway");
  await expect(traffic).not.toContainText("listener");
  await expect(traffic).not.toContainText("Not enabled");
  await expect(
    page.locator(".nav-list").getByRole("link", { name: "Listeners" }),
  ).toHaveCount(0);
});

test("attaches traffic routes to gateways", async ({ page }) => {
  const gateway = await mockGateway(page, {
    gateways: {
      default: {
        port: 4000,
      },
    },
    routes: [],
  });
  await page.goto("/traffic/routes");

  await page.getByRole("button", { name: "Add route" }).first().click();
  await page.getByPlaceholder("api").fill("api");
  await page.getByRole("button", { name: "Save route" }).click();

  await expect.poll(() => gateway.postedConfigs.length).toBe(1);
  expect(gateway.postedConfigs[0].routes).toMatchObject([
    {
      gateways: "default",
      name: "api",
      matches: [{ path: { pathPrefix: "/" } }],
    },
  ]);
});

test("migrates legacy HTTP binds to gateways", async ({ page }) => {
  const gateway = await mockGateway(page, populatedConfig());
  await page.goto("/traffic/gateways");

  await page.getByRole("button", { name: "Review migration" }).click();
  await expect(
    page.getByRole("heading", { name: "Migrate binds to gateways" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Save", exact: true }).click();

  await expect.poll(() => gateway.postedConfigs.length).toBe(1);
  const saved = gateway.postedConfigs[0] as {
    binds?: Array<{ port?: number }>;
    gateways?: Record<
      string,
      {
        port?: number;
        listeners?: Array<{ name?: string; hostname?: string }>;
      }
    >;
    routes?: Array<{ gateways?: string; name?: string }>;
    tcpRoutes?: Array<{ gateways?: string; name?: string }>;
  };
  expect(saved.gateways?.["port-8080"]).toMatchObject({
    port: 8080,
    listeners: [{ name: "public-http", hostname: "example.com" }],
  });
  expect(saved.routes).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        gateways: "port-8080/public-http",
        name: "api",
      }),
      expect.objectContaining({
        gateways: "port-8080/public-http",
        name: "legacy-ai",
      }),
    ]),
  );
  expect(saved.gateways?.["port-9090"]).toMatchObject({
    port: 9090,
    listeners: [{ name: "tcp", hostname: "tcp.example.com" }],
  });
  expect(saved.tcpRoutes).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        gateways: "port-9090/tcp",
        name: "tcp-main",
      }),
    ]),
  );
  expect(saved.binds).toBeUndefined();
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

test("hybrid model edits use the unified resource API", async ({ page }) => {
  const config = emptyConfig();
  const llm = config.llm as { models: Array<Record<string, unknown>> };
  llm.models = [
    {
      name: "file-model",
      provider: "openai",
      params: { model: "gpt-file" },
    },
  ];
  delete (llm as Record<string, unknown>).providers;
  delete (llm as Record<string, unknown>).virtualModels;
  await mockGateway(page, config);
  const resourceWrites: Array<Record<string, unknown>> = [];
  const dbResource = {
    kind: "llm.model",
    id: "db-model-id",
    value: {
      id: "db-model-id",
      name: "db-model",
      provider: "openai",
      params: { model: "gpt-db" },
    },
    revision: 1,
    createdAt: "2026-07-10T00:00:00Z",
    updatedAt: "2026-07-10T00:00:00Z",
  };

  await page.route("**/api/runtime", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        build: {
          version: "test",
          gitRevision: "test",
          rustVersion: "test",
          buildProfile: "test",
          buildTarget: "test",
        },
        ui: { gatewayMode: "standalone", configStoreMode: "hybrid" },
      }),
    }),
  );
  await page.route("**/api/config/effective", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        ...config,
        llm: { ...llm, models: [...llm.models, dbResource.value] },
      }),
    }),
  );
  await page.route("**/api/config/resources**", async (route) => {
    if (route.request().method() === "GET") {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ resources: [dbResource] }),
      });
      return;
    }
    if (route.request().method() === "PUT") {
      const request = route.request().postDataJSON() as {
        value: Record<string, unknown>;
      };
      const value = request.value;
      resourceWrites.push(value);
      dbResource.value = value as typeof dbResource.value;
      dbResource.revision += 1;
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ resources: [dbResource] }),
      });
      return;
    }
    await route.fallback();
  });

  await page.goto("/llm/models");

  const fileRow = page.getByRole("row", { name: /file-model/ });
  const databaseRow = page.getByRole("row", { name: /db-model/ });
  await expect(fileRow.getByText("File", { exact: true })).toBeVisible();
  await expect(
    databaseRow.getByText("Database", { exact: true }),
  ).toBeVisible();

  await fileRow.getByRole("button", { name: "Edit model" }).click();
  await page.getByLabel("Incoming model match").fill("file-model-renamed");
  await page.getByRole("button", { name: "View diff" }).click();
  const diffDrawer = page.locator(".drawer.nested");
  await expect(
    diffDrawer.getByRole("heading", { name: "Model config diff" }),
  ).toBeVisible();
  await expect(diffDrawer.getByRole("button", { name: "Save" })).toBeDisabled();
  expect(resourceWrites).toHaveLength(0);
  await diffDrawer.getByRole("button", { name: "Close" }).first().click();
  await page.getByRole("button", { name: "Close" }).first().click();
  await page.getByRole("button", { name: "Discard changes" }).click();

  await databaseRow.getByRole("button", { name: "Edit model" }).click();
  await page.getByLabel("Incoming model match").fill("db-model-renamed");
  await page.getByRole("button", { name: "View diff" }).click();
  await expect(
    diffDrawer.getByRole("heading", { name: "Model resource diff" }),
  ).toBeVisible();
  await diffDrawer.getByRole("button", { name: "Close" }).first().click();
  await page.getByRole("button", { name: "Save model" }).click();

  await expect.poll(() => resourceWrites.length).toBe(1);
  expect(resourceWrites[0]).toMatchObject({
    id: "db-model-id",
    name: "db-model-renamed",
  });

  await page.goto("/llm/client-setup");
  await page.getByRole("combobox", { name: "Model" }).click();
  await expect(
    page.getByRole("option", { name: "db-model-renamed" }),
  ).toBeVisible();
  await page.getByRole("option", { name: "db-model-renamed" }).click();
  const clientRecipe = page.locator(".client-recipe-card");
  await expect(clientRecipe).not.toContainText("Authorization: Bearer");
  await expect(clientRecipe).not.toContainText("agw_sk_...");
  await page.getByRole("combobox", { name: "Integration" }).click();
  await page.getByRole("option", { name: "OpenAI JavaScript SDK" }).click();
  await expect(clientRecipe).toContainText("dummy_key");

  await page.goto("/llm/playground");
  await page.getByRole("combobox", { name: "Model" }).click();
  await expect(
    page.getByRole("option", { name: "db-model-renamed" }),
  ).toBeVisible();
});

test("database-backed MCP servers are available across hybrid UI pages", async ({
  page,
}) => {
  const config = emptyConfig();
  delete config.mcp;
  const gateway = await mockGateway(page, config);
  const resources = [
    {
      kind: "mcp.target",
      id: "db-weather",
      value: {
        name: "db-weather",
        mcp: { host: "http://weather.example/mcp" },
      },
      revision: 1,
      createdAt: "2026-07-23T00:00:00Z",
      updatedAt: "2026-07-23T00:00:00Z",
    },
    {
      kind: "mcp.settings",
      id: "default",
      value: { port: 3000 },
      revision: 1,
      createdAt: "2026-07-23T00:00:00Z",
      updatedAt: "2026-07-23T00:00:00Z",
    },
  ];
  const writes: Array<Record<string, unknown>> = [];

  await page.route("**/api/runtime", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        build: {},
        ui: { gatewayMode: "standalone", configStoreMode: "hybrid" },
      }),
    }),
  );
  await page.route("**/api/config/effective", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        ...config,
        mcp: {
          port: 3000,
          targets: [resources[0].value],
          policies: {},
        },
      }),
    }),
  );
  await page.route("**/api/config/resources**", async (route) => {
    if (route.request().method() === "GET") {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ resources }),
      });
      return;
    }
    if (
      route.request().method() === "PUT" &&
      new URL(route.request().url()).pathname.endsWith("/mcp.target")
    ) {
      const request = route.request().postDataJSON() as {
        resources: Array<{ value: Record<string, unknown> }>;
      };
      writes.push(request.resources[0].value);
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ resources: [] }),
      });
      return;
    }
    await route.fallback();
  });

  await page.goto("/mcp/playground");
  await expect(page.getByText("No MCP servers", { exact: true })).toHaveCount(
    0,
  );

  await page.goto("/");
  await expect(page.getByText("1 configured server")).toBeVisible();

  await page.goto("/mcp/servers");
  await expect(page.getByRole("row", { name: /db-weather/ })).toBeVisible();
  await page.getByRole("button", { name: "Add server" }).click();
  await page.getByLabel("Server name").fill("db-search");
  await expect(page.getByRole("button", { name: "Save server" })).toBeEnabled();
  await page.getByRole("button", { name: "Save server" }).click();

  await expect.poll(() => writes.length).toBe(1);
  expect(writes[0]).toMatchObject({ name: "db-search" });
  expect(gateway.postedConfigs).toHaveLength(0);
});

test("hybrid MCP settings only lock file-owned fields", async ({ page }) => {
  const config = emptyConfig();
  config.mcp = {
    port: 3000,
    targets: [],
    policies: {},
  };
  const gateway = await mockGateway(page, config);
  const writes: Array<Record<string, unknown>> = [];

  await page.route("**/api/runtime", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        build: {},
        ui: { gatewayMode: "standalone", configStoreMode: "hybrid" },
      }),
    }),
  );
  await page.route("**/api/config/effective", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(config),
    }),
  );
  await page.route("**/api/config/resources**", async (route) => {
    if (route.request().method() === "GET") {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ resources: [] }),
      });
      return;
    }
    if (
      route.request().method() === "PUT" &&
      new URL(route.request().url()).pathname.endsWith("/mcp.settings")
    ) {
      const request = route.request().postDataJSON() as {
        resources: Array<{ value: Record<string, unknown> }>;
      };
      writes.push(request.resources[0].value);
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ resources: [] }),
      });
      return;
    }
    await route.fallback();
  });

  await page.goto("/mcp/servers");
  await page.getByRole("button", { name: "Settings" }).click();
  await expect(page.getByRole("textbox", { name: /Port/ })).toBeDisabled();
  await expect(
    page.getByRole("combobox", { name: "State mode" }),
  ).toBeEnabled();
  await page.getByRole("combobox", { name: "State mode" }).click();
  await page.getByRole("option", { name: "Stateful" }).click();
  await page.getByRole("button", { name: "Save settings" }).click();

  await expect.poll(() => writes.length).toBe(1);
  expect(writes[0]).toMatchObject({ statefulMode: "stateful" });
  expect(writes[0]).not.toHaveProperty("port");
  expect(gateway.postedConfigs).toHaveLength(0);
});

test("hybrid file-owned API key policy is read-only", async ({ page }) => {
  const config = emptyConfig();
  const policies = (config.llm as Record<string, unknown>).policies as Record<
    string,
    unknown
  >;
  policies.apiKey = { mode: "strict", keys: [] };
  const gateway = await mockGateway(page, config);
  await page.route("**/api/runtime", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        build: {},
        ui: { gatewayMode: "standalone", configStoreMode: "hybrid" },
      }),
    }),
  );

  await page.goto("/llm/keys");
  const disablePolicy = page.getByRole("button", {
    name: "Disable API Key Policy",
  });
  await expect(disablePolicy).toBeDisabled();
  await disablePolicy.hover({ force: true });
  await expect(page.getByRole("tooltip")).toContainText(
    "file-owned and cannot be modified",
  );

  await page.getByRole("button", { name: "Settings" }).click();
  const drawer = page.locator(".drawer");
  await expect(
    drawer.getByRole("button", { name: "Save policy" }),
  ).toBeDisabled();
  await expect(
    drawer.getByRole("button", { name: "Disable", exact: true }),
  ).toBeDisabled();
  expect(gateway.postedConfigs).toHaveLength(0);
});

test("new hybrid traffic gateways save as database resources", async ({
  page,
}) => {
  const gateway = await mockGateway(page, emptyConfig());
  const resources = [
    {
      kind: "traffic.gateway",
      id: "db-public",
      value: { name: "db-public", port: 8080 },
      revision: 1,
      createdAt: "2026-07-23T00:00:00Z",
      updatedAt: "2026-07-23T00:00:00Z",
    },
  ];
  const writes: Array<Record<string, unknown>> = [];

  await page.route("**/api/runtime", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        build: {},
        ui: { gatewayMode: "standalone", configStoreMode: "hybrid" },
      }),
    }),
  );
  await page.route("**/api/config/effective", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        ...emptyConfig(),
        gateways: { "db-public": { port: 8080 } },
      }),
    }),
  );
  await page.route("**/api/config/resources**", async (route) => {
    if (route.request().method() === "GET") {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ resources }),
      });
      return;
    }
    if (
      route.request().method() === "PUT" &&
      new URL(route.request().url()).pathname.endsWith("/traffic.gateway")
    ) {
      const request = route.request().postDataJSON() as {
        resources: Array<{ value: Record<string, unknown> }>;
      };
      writes.push(request.resources[0].value);
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ resources: [] }),
      });
      return;
    }
    await route.fallback();
  });

  await page.goto("/traffic/gateways");
  await expect(page.getByRole("heading", { name: "db-public" })).toBeVisible();
  await page.getByRole("button", { name: "Add gateway" }).first().click();
  const drawer = page.locator(".drawer");
  await drawer.getByLabel("Name", { exact: true }).fill("db-internal");
  await expect(
    drawer.getByRole("button", { name: "Save gateway" }),
  ).toBeEnabled();
  await drawer.getByRole("button", { name: "Save gateway" }).click();

  await expect.poll(() => writes.length).toBe(1);
  expect(writes[0]).toMatchObject({ name: "db-internal" });
  expect(gateway.postedConfigs).toHaveLength(0);
});

test("hybrid LLM and UI policies are stored as individual resources", async ({
  page,
}) => {
  const config = emptyConfig();
  delete (config.llm as Record<string, unknown>).policies;
  const mcp = config.mcp as Record<string, unknown>;
  mcp.port = 3000;
  const mcpPolicies = mcp.policies as Record<string, unknown>;
  mcpPolicies.cors = { allowOrigins: [] };
  config.gateways = { default: { port: 8080 } };
  config.ui = { gateways: "default" };
  const gateway = await mockGateway(page, config);
  const resources: Array<Record<string, unknown>> = [
    {
      kind: "ui.policy",
      id: "oidc",
      value: { issuer: "https://idp.example.com" },
      revision: 1,
      createdAt: "2026-07-23T00:00:00Z",
      updatedAt: "2026-07-23T00:00:00Z",
    },
    {
      kind: "llm.policy",
      id: "apiKey",
      value: { mode: "optional" },
      revision: 1,
      createdAt: "2026-07-23T00:00:00Z",
      updatedAt: "2026-07-23T00:00:00Z",
    },
  ];
  const writes: Array<{ kind: string; id: string; value: unknown }> = [];

  await page.route("**/api/config/effective", (route) => {
    const effective = structuredClone(config);
    for (const resource of resources) {
      const kind = String(resource.kind);
      if (!kind.endsWith(".policy")) continue;
      const sectionName = kind.split(".")[0];
      const section = (effective[sectionName] ??= {}) as Record<
        string,
        unknown
      >;
      const policies = (section.policies ??= {}) as Record<string, unknown>;
      policies[String(resource.id)] = resource.value;
    }
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(effective),
    });
  });
  await page.route("**/api/runtime", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        build: {},
        ui: { gatewayMode: "standalone", configStoreMode: "hybrid" },
      }),
    }),
  );
  await page.route("**/api/config/resources**", async (route) => {
    const request = route.request();
    if (request.method() === "GET") {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ resources }),
      });
      return;
    }
    if (request.method() === "PUT") {
      const parts = new URL(request.url()).pathname.split("/");
      const kind = decodeURIComponent(parts.at(-2) ?? "");
      const id = decodeURIComponent(parts.at(-1) ?? "");
      const { value } = request.postDataJSON() as { value: unknown };
      writes.push({ kind, id, value });
      resources.push({
        kind,
        id,
        value,
        revision: 1,
        createdAt: "2026-07-23T00:00:00Z",
        updatedAt: "2026-07-23T00:00:00Z",
      });
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ resources: [resources.at(-1)] }),
      });
      return;
    }
    await route.fallback();
  });

  await page.goto("/");
  await expect(
    page.getByText("UI is exposed without authentication"),
  ).toHaveCount(0);
  await expect(
    page.getByText(
      "Virtual API key mode is optional; unauthenticated requests may be accepted.",
    ),
  ).toBeVisible();

  await page.goto("/llm/playground");
  await page.getByRole("button", { name: "Apply CORS" }).click();
  await expect.poll(() => writes.length).toBe(1);
  expect(writes[0]).toMatchObject({ kind: "llm.policy", id: "cors" });
  await expect(page.getByText("Browser access is not allowed")).toHaveCount(0);

  await page.goto("/mcp/playground");
  await expect(
    page.getByRole("link", { name: "Configure CORS" }),
  ).toHaveAttribute("href", "/mcp/policies#cors");

  await page.goto("/settings");
  await page.getByText("CORS", { exact: true }).click();
  await page.getByRole("button", { name: "Add current origin" }).click();
  await page.getByRole("button", { name: "Save policy" }).click();
  await expect.poll(() => writes.length).toBe(2);
  expect(writes[1]).toMatchObject({ kind: "ui.policy", id: "cors" });
  expect(gateway.postedConfigs).toHaveLength(0);
});

test("hybrid pages surface configuration database errors", async ({ page }) => {
  await mockGateway(page);
  await page.route("**/api/runtime", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        build: {},
        ui: { gatewayMode: "standalone", configStoreMode: "hybrid" },
      }),
    }),
  );
  await page.route("**/api/config/resources**", (route) =>
    route.fulfill({
      status: 503,
      contentType: "application/json",
      body: JSON.stringify("configuration database unavailable"),
    }),
  );

  await page.goto("/");
  await expect(
    page.getByText("Configuration API unavailable", { exact: true }),
  ).toBeVisible();

  await page.goto("/llm/playground");
  await expect(
    page.getByText("Configuration API unavailable", { exact: true }),
  ).toBeVisible();
  await expect(page.getByText("No configured models")).toHaveCount(0);
});

test("reveals a virtual API key explicitly", async ({ page }) => {
  await mockGateway(page);
  await page.goto("/llm/keys");

  await expect(page.getByText("agw_sk_testkey123456789")).toHaveCount(0);
  await page.getByRole("button", { name: "Show full key" }).click();
  await expect(page.getByText("agw_sk_testkey123456789")).toBeVisible();
});

test("copies a virtual API key to clipboard", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await mockGateway(page);
  await page.goto("/llm/keys");

  await page.getByRole("button", { name: "Copy key" }).click();
  await expect(page.getByRole("button", { name: "Copy key" })).toHaveClass(
    /copied/,
  );
  const clipboardText = await page.evaluate(() =>
    navigator.clipboard.readText(),
  );
  expect(clipboardText).toBe("agw_sk_testkey123456789");
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

test("LLM playground uses relative requests when UI shares the gateway", async ({
  page,
}) => {
  const gateway = await mockGateway(page, sameOriginGatewayConfig());
  await page.goto("/llm/playground");

  await expect(page.getByText("Browser access is not allowed")).toHaveCount(0);
  await page.getByRole("combobox", { name: "Model" }).click();
  await page.getByRole("option", { name: /resilient/ }).click();
  await page.getByLabel("User message").fill("ping");
  await page.getByRole("button", { name: "Send" }).click();

  await expect.poll(() => gateway.chatUrls.length).toBe(1);
  const pageOrigin = await page.evaluate(() => window.location.origin);
  const requestUrl = new URL(gateway.chatUrls[0]);
  expect(requestUrl.origin).toBe(pageOrigin);
  expect(requestUrl.pathname).toBe("/v1/chat/completions");
});

test("LLM playground uses relative requests with implicit default gateway", async ({
  page,
}) => {
  const gateway = await mockGateway(page, implicitDefaultGatewayConfig());
  await page.goto("/llm/playground");

  await expect(page.getByText("Browser access is not allowed")).toHaveCount(0);
  await page.getByRole("combobox", { name: "Model" }).click();
  await page.getByRole("option", { name: /resilient/ }).click();
  await page.getByLabel("User message").fill("ping");
  await page.getByRole("button", { name: "Send" }).click();

  await expect.poll(() => gateway.chatUrls.length).toBe(1);
  const pageOrigin = await page.evaluate(() => window.location.origin);
  const requestUrl = new URL(gateway.chatUrls[0]);
  expect(requestUrl.origin).toBe(pageOrigin);
  expect(requestUrl.pathname).toBe("/v1/chat/completions");
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

test("MCP playground uses relative requests when UI shares the gateway", async ({
  page,
}) => {
  const gateway = await mockGateway(page, sameOriginGatewayConfig());
  await page.goto("/mcp/playground");

  await expect(page.getByText("Browser access is not allowed")).toHaveCount(0);
  await page.getByRole("button", { name: "Initialize", exact: true }).click();
  await expect(page.getByText("initialized")).toBeVisible();

  await expect.poll(() => gateway.mcpUrls.length).toBeGreaterThan(0);
  const pageOrigin = await page.evaluate(() => window.location.origin);
  const requestUrl = new URL(gateway.mcpUrls[0]);
  expect(requestUrl.origin).toBe(pageOrigin);
  expect(requestUrl.pathname).toBe("/mcp");
});

test("MCP playground uses relative requests with implicit default gateway", async ({
  page,
}) => {
  const gateway = await mockGateway(page, implicitDefaultGatewayConfig());
  await page.goto("/mcp/playground");

  await expect(page.getByText("Browser access is not allowed")).toHaveCount(0);
  await page.getByRole("button", { name: "Initialize", exact: true }).click();
  await expect(page.getByText("initialized")).toBeVisible();

  await expect.poll(() => gateway.mcpUrls.length).toBeGreaterThan(0);
  const pageOrigin = await page.evaluate(() => window.location.origin);
  const requestUrl = new URL(gateway.mcpUrls[0]);
  expect(requestUrl.origin).toBe(pageOrigin);
  expect(requestUrl.pathname).toBe("/mcp");
});

test("Client Setup uses implicit default gateway port", async ({ page }) => {
  await mockGateway(page, implicitDefaultGatewayConfig());
  await page.goto("/llm/client-setup");

  const hostname = await page.evaluate(() => window.location.hostname);
  const baseUrl = `http://${hostname}:8080`;
  await expect(
    page.getByRole("textbox", { name: "Gateway base URL" }),
  ).toHaveValue(baseUrl);
  await expect(page.locator(".client-setup-summary")).toContainText(
    `${baseUrl}/v1`,
  );
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

test("refreshes a reopened policy diff", async ({ page }) => {
  const config = emptyConfig();
  const llm = config.llm as {
    policies: {
      cors: {
        allowHeaders: string[];
        allowMethods: string[];
      };
    };
  };
  llm.policies.cors = {
    allowHeaders: ["authorization", "content-type"],
    allowMethods: ["GET", "POST", "DELETE"],
  };
  await mockGateway(page, config);
  await page.route("**/api/runtime", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        build: {},
        ui: { gatewayMode: "standalone", configStoreMode: "file" },
      }),
    }),
  );
  await page.goto("/llm/policies");

  await page.getByRole("button", { name: /CORS/ }).click();
  await page.getByRole("button", { name: "PUT", exact: true }).click();
  await page.getByRole("button", { name: "View diff" }).click();
  await page
    .locator(".drawer.nested")
    .getByRole("button", { name: "Close" })
    .last()
    .click();

  await page.getByRole("button", { name: "PATCH", exact: true }).click();
  await page.getByRole("button", { name: "View diff" }).click();

  const diff = page.locator(".drawer.nested");
  await expect(
    diff.locator(".view-lines").filter({ hasText: "PATCH" }),
  ).toHaveCount(1);
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
  await page.getByPlaceholder("public-http").fill("public");
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
  await page.getByPlaceholder("api").fill("new-http");
  await page.getByRole("textbox", { name: "Path" }).fill("/new");
  await page.getByRole("button", { name: "Save route" }).click();

  await expect.poll(() => gateway.postedConfigs.length).toBe(1);
  await page.getByRole("button", { name: "Add route" }).first().click();
  await page.getByRole("combobox", { name: "Listener" }).click();
  await page.getByRole("option", { name: /tcp-listener/ }).click();
  await page.getByPlaceholder("api").fill("new-tcp");
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

test("Playground shows Claude subscription key warning", async ({ page }) => {
  await mockGateway(page, configWithClaudeSubscriptionKey());
  await page.goto("/llm/playground");

  await page.getByRole("combobox", { name: "Model" }).click();
  await page.getByRole("option", { name: /claude-sub/ }).click();

  await expect(
    page.getByText("Claude subscription key detected"),
  ).toBeVisible();
  await expect(page.getByText("sk-ant-oat")).toBeVisible();
});

test("Client Setup shows Claude subscription key warning", async ({ page }) => {
  await mockGateway(page, configWithClaudeSubscriptionKey());
  await page.goto("/llm/client-setup");

  await page.getByRole("combobox", { name: "Model" }).click();
  await page.getByRole("option", { name: /claude-sub/ }).click();

  await expect(
    page.getByText("Claude subscription key detected"),
  ).toBeVisible();
  await expect(page.getByText("sk-ant-oat")).toBeVisible();
});

test("no Claude subscription warning for env-var API keys", async ({
  page,
}) => {
  await mockGateway(page);
  await page.goto("/llm/playground");

  await page.getByRole("combobox", { name: "Model" }).click();
  await page.getByRole("option", { name: /anthropic/ }).click();

  await expect(page.getByText("Claude subscription key detected")).toHaveCount(
    0,
  );
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
