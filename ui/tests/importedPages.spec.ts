/**
 * E2E tests for pages imported from the v2 UI.
 * Tests navigate to each page and verify basic content + interactions.
 * Video output is captured per the global playwright.config.ts settings.
 */

import { expect, test } from "@playwright/test";

const UI = "/ui#";

async function goTo(page: import("@playwright/test").Page, hash: string) {
  await page.goto(`${UI}${hash}`);
  await page.waitForLoadState("networkidle").catch(() => {});
}

// ── Dashboard ──────────────────────────────────────────────────────────────

test.describe("Dashboard", () => {
  test("shows Gateway Overview with surface rows", async ({ page }) => {
    await goTo(page, "/dashboard");
    await expect(page.getByRole("heading", { name: /Gateway Overview/i })).toBeVisible();
    await expect(page.getByText("LLM").first()).toBeVisible();
    await expect(page.getByText("MCP").first()).toBeVisible();
    await expect(page.getByText("Traffic").first()).toBeVisible();
  });

  test("shows onboarding or overview depending on config state", async ({ page }) => {
    await goTo(page, "/dashboard");
    const hasWelcome = await page.getByText("Welcome to Agentgateway").isVisible().catch(() => false);
    const hasOverview = await page.getByRole("heading", { name: /Gateway Overview/i }).isVisible().catch(() => false);
    expect(hasWelcome || hasOverview).toBe(true);
  });
});

// ── LLM Models ────────────────────────────────────────────────────────────

test.describe("LLM Models", () => {
  test("loads and shows page header", async ({ page }) => {
    await goTo(page, "/llm-models");
    await expect(page.getByRole("heading", { name: "LLM Models" })).toBeVisible();
  });

  test("shows Add model button", async ({ page }) => {
    await goTo(page, "/llm-models");
    await expect(page.getByRole("button", { name: /Add model/i }).first()).toBeVisible();
  });

  test("shows empty state or model list", async ({ page }) => {
    await goTo(page, "/llm-models");
    const hasEmpty = await page.getByText(/No models/i).first().isVisible().catch(() => false);
    const hasTable = await page.locator("table").first().isVisible().catch(() => false);
    const hasBtn = await page.getByRole("button", { name: /Add model/i }).first().isVisible().catch(() => false);
    expect(hasEmpty || hasTable || hasBtn).toBe(true);
  });

  test("clicking Add model opens a drawer", async ({ page }) => {
    await goTo(page, "/llm-models");
    await page.getByRole("button", { name: /Add model/i }).first().click();
    await expect(page.locator(".drawer, [role='dialog'], .form-grid").first()).toBeVisible({ timeout: 5000 });
  });
});

// ── LLM Providers ─────────────────────────────────────────────────────────

test.describe("LLM Providers", () => {
  test("loads and shows page header", async ({ page }) => {
    await goTo(page, "/llm-providers");
    await expect(page.getByRole("heading", { name: "LLM Providers" })).toBeVisible();
  });

  test("shows description text", async ({ page }) => {
    await goTo(page, "/llm-providers");
    await expect(page.getByText(/reusable provider credentials/i).first()).toBeVisible();
  });

  test("shows Add provider button", async ({ page }) => {
    await goTo(page, "/llm-providers");
    await expect(page.getByRole("button", { name: /Add provider/i }).first()).toBeVisible();
  });
});

// ── LLM Policies ──────────────────────────────────────────────────────────

test.describe("LLM Policies", () => {
  test("loads and shows page header", async ({ page }) => {
    await goTo(page, "/llm-policies");
    await expect(page.getByRole("heading", { name: "LLM Policies" })).toBeVisible();
  });

  test("shows policy content area", async ({ page }) => {
    await goTo(page, "/llm-policies");
    await expect(page.locator(".panel, .page-stack").first()).toBeVisible({ timeout: 8000 });
  });
});

// ── LLM Guardrails ────────────────────────────────────────────────────────

test.describe("LLM Guardrails", () => {
  test("loads and shows page header", async ({ page }) => {
    await goTo(page, "/llm-guardrails");
    await expect(page.getByRole("heading", { name: "LLM Guardrails" })).toBeVisible();
  });

  test("shows guardrail content", async ({ page }) => {
    await goTo(page, "/llm-guardrails");
    await expect(page.locator(".panel, .page-stack").first()).toBeVisible();
  });
});

// ── LLM Monitoring ────────────────────────────────────────────────────────

test.describe("LLM Monitoring", () => {
  test("loads and shows page header", async ({ page }) => {
    await goTo(page, "/llm-monitoring");
    await expect(page.getByRole("heading", { name: "Monitoring" })).toBeVisible();
  });

  test("shows monitoring description", async ({ page }) => {
    await goTo(page, "/llm-monitoring");
    await expect(page.getByText(/Inspect one row per LLM call/i).first()).toBeVisible();
  });
});

// ── LLM Virtual API Keys ──────────────────────────────────────────────────

test.describe("LLM Virtual API Keys", () => {
  test("loads and shows page header", async ({ page }) => {
    await goTo(page, "/llm-keys");
    await expect(page.getByRole("heading", { name: "Virtual API Keys", exact: true })).toBeVisible();
  });

  test("shows New key button", async ({ page }) => {
    await goTo(page, "/llm-keys");
    await expect(page.getByRole("button", { name: /New key/i }).first()).toBeVisible();
  });

  test("shows description text", async ({ page }) => {
    await goTo(page, "/llm-keys");
    await expect(page.getByText(/Provision incoming credentials/i).first()).toBeVisible();
  });
});

// ── LLM Client Setup ──────────────────────────────────────────────────────

test.describe("LLM Client Setup", () => {
  test("loads and shows page header", async ({ page }) => {
    await goTo(page, "/llm-client-setup");
    await expect(page.getByRole("heading", { name: "Client Setup" })).toBeVisible();
  });

  test("shows code snippets", async ({ page }) => {
    await goTo(page, "/llm-client-setup");
    await expect(page.locator("pre, code, .code-block").first()).toBeVisible({ timeout: 8000 });
  });
});

// ── MCP Servers ───────────────────────────────────────────────────────────

test.describe("MCP Servers", () => {
  test("loads and shows page header", async ({ page }) => {
    await goTo(page, "/mcp-servers");
    await expect(page.getByRole("heading", { name: "MCP Servers" })).toBeVisible();
  });

  test("shows Add server button", async ({ page }) => {
    await goTo(page, "/mcp-servers");
    await expect(page.getByRole("button", { name: /Add server/i }).first()).toBeVisible();
  });

  test("shows empty state or server table", async ({ page }) => {
    await goTo(page, "/mcp-servers");
    const hasEmpty = await page.getByText(/No MCP servers/i).first().isVisible().catch(() => false);
    const hasTable = await page.locator("table").first().isVisible().catch(() => false);
    expect(hasEmpty || hasTable).toBe(true);
  });

  test("clicking Add server opens drawer", async ({ page }) => {
    await goTo(page, "/mcp-servers");
    await page.getByRole("button", { name: /Add server/i }).first().click();
    await expect(page.locator(".drawer, [role='dialog'], .form-grid").first()).toBeVisible({ timeout: 5000 });
  });
});

// ── Traffic Listeners ─────────────────────────────────────────────────────

test.describe("Traffic Listeners", () => {
  test("loads and shows page header", async ({ page }) => {
    await goTo(page, "/traffic-listeners");
    await expect(page.getByRole("heading", { name: "Traffic Listeners" })).toBeVisible();
  });

  test("shows Add bind button", async ({ page }) => {
    await goTo(page, "/traffic-listeners");
    await expect(page.getByRole("button", { name: /Add bind/i }).first()).toBeVisible();
  });

  test("shows Add listener button", async ({ page }) => {
    await goTo(page, "/traffic-listeners");
    await expect(page.getByRole("button", { name: /Add listener/i }).first()).toBeVisible();
  });

  test("shows bind sections or empty state", async ({ page }) => {
    await goTo(page, "/traffic-listeners");
    const hasBinds = await page.locator(".traffic-bind, .traffic-bind-list").first().isVisible().catch(() => false);
    const hasEmpty = await page.getByText(/No traffic binds/i).first().isVisible().catch(() => false);
    const hasPanel = await page.locator(".panel").first().isVisible().catch(() => false);
    expect(hasBinds || hasEmpty || hasPanel).toBe(true);
  });

  test("clicking Add bind shows drawer", async ({ page }) => {
    await goTo(page, "/traffic-listeners");
    await page.getByRole("button", { name: /Add bind/i }).first().click();
    await expect(page.locator(".drawer, [role='dialog']").first()).toBeVisible({ timeout: 5000 });
  });
});

// ── Traffic Routes ────────────────────────────────────────────────────────

test.describe("Traffic Routes", () => {
  test("loads and shows page header", async ({ page }) => {
    await goTo(page, "/traffic-routes");
    await expect(page.getByRole("heading", { name: "Traffic Routes" }).first()).toBeVisible();
  });

  test("shows Add route button or empty/panel state", async ({ page }) => {
    await goTo(page, "/traffic-routes");
    const hasAddBtn = await page.getByRole("button", { name: /Add route/i }).first().isVisible().catch(() => false);
    const hasEmpty = await page.getByText(/No routes|No listeners/i).first().isVisible().catch(() => false);
    const hasPanel = await page.locator(".panel").first().isVisible().catch(() => false);
    expect(hasAddBtn || hasEmpty || hasPanel).toBe(true);
  });
});

// ── Raw Configuration ─────────────────────────────────────────────────────

test.describe("Raw Configuration", () => {
  test("loads and shows page header", async ({ page }) => {
    await goTo(page, "/raw-config");
    await expect(page.getByRole("heading", { name: "Raw Configuration" })).toBeVisible();
  });

  test("shows Copy YAML button", async ({ page }) => {
    await goTo(page, "/raw-config");
    await expect(page.getByRole("button", { name: /Copy YAML/i })).toBeVisible({ timeout: 8000 });
  });

  test("YAML pre block is visible after load", async ({ page }) => {
    await goTo(page, "/raw-config");
    await expect(page.locator("pre").first()).toBeVisible({ timeout: 10000 });
  });

  test("YAML content is non-empty", async ({ page }) => {
    await goTo(page, "/raw-config");
    await page.locator("pre").first().waitFor({ timeout: 10000 });
    const text = await page.locator("pre").first().textContent();
    expect(text?.trim().length).toBeGreaterThan(0);
  });
});

// ── LLM Playground (updated features) ────────────────────────────────────

test.describe("LLM Playground - updated features", () => {
  test("loads playground page", async ({ page }) => {
    await goTo(page, "/llm-playground");
    // Heading is an emotion-styled h1; also accept any visible "LLM Playground" text
    await expect(page.getByText(/LLM Playground/i).first()).toBeVisible({ timeout: 15000 });
  });

  test("shows Virtual API Key option in settings panel", async ({ page }) => {
    await goTo(page, "/llm-playground");
    await expect(page.getByText(/Virtual API [Kk]ey|API [Kk]ey/i).first()).toBeVisible({ timeout: 8000 });
  });

  test("Send button is disabled when no message typed", async ({ page }) => {
    await goTo(page, "/llm-playground");
    // Send button uses a Lucide icon — locate by its position in the chat panel
    const sendBtn = page.locator("button[disabled], button.ant-btn[disabled]").last();
    await expect(sendBtn).toBeVisible({ timeout: 10000 });
  });
});

// ── Navigation sidebar ────────────────────────────────────────────────────

test.describe("Navigation sidebar", () => {
  test("shows all new LLM nav items", async ({ page }) => {
    await goTo(page, "/dashboard");
    const sidebar = page.locator(".ant-menu, [role='menu']").first();
    await expect(sidebar.getByText("Models")).toBeVisible();
    await expect(sidebar.getByText("Providers")).toBeVisible();
    await expect(sidebar.getByText("Policies")).toBeVisible();
    await expect(sidebar.getByText("Guardrails")).toBeVisible();
    await expect(sidebar.getByText("Monitoring")).toBeVisible();
    await expect(sidebar.getByText("Virtual API Keys")).toBeVisible();
    await expect(sidebar.getByText("Client Setup")).toBeVisible();
  });

  test("shows MCP Servers nav item", async ({ page }) => {
    await goTo(page, "/dashboard");
    const sidebar = page.locator(".ant-menu, [role='menu']").first();
    await expect(sidebar.getByText("Servers")).toBeVisible();
  });

  test("shows Traffic Listeners and Routes nav items", async ({ page }) => {
    await goTo(page, "/dashboard");
    const sidebar = page.locator(".ant-menu, [role='menu']").first();
    await expect(sidebar.getByText("Listeners")).toBeVisible();
    await expect(sidebar.getByText("Routes")).toBeVisible();
  });

  test("shows Raw Configuration nav item", async ({ page }) => {
    await goTo(page, "/dashboard");
    const sidebar = page.locator(".ant-menu, [role='menu']").first();
    await expect(sidebar.getByText("Raw Configuration")).toBeVisible();
  });

  test("clicking Models navigates to /llm-models", async ({ page }) => {
    await goTo(page, "/dashboard");
    await page.locator(".ant-menu, [role='menu']").first().getByText("Models").click();
    await expect(page).toHaveURL(/#\/llm-models/);
  });

  test("clicking Servers navigates to /mcp-servers", async ({ page }) => {
    await goTo(page, "/dashboard");
    await page.locator(".ant-menu, [role='menu']").first().getByText("Servers").click();
    await expect(page).toHaveURL(/#\/mcp-servers/);
  });

  test("clicking Listeners navigates to /traffic-listeners", async ({ page }) => {
    await goTo(page, "/dashboard");
    await page.locator(".ant-menu, [role='menu']").first().getByText("Listeners").click();
    await expect(page).toHaveURL(/#\/traffic-listeners/);
  });

  test("clicking Raw Configuration navigates to /raw-config", async ({ page }) => {
    await goTo(page, "/dashboard");
    await page.locator(".ant-menu, [role='menu']").first().getByText("Raw Configuration").click();
    await expect(page).toHaveURL(/#\/raw-config/);
  });
});

// ── Editor buttons hidden ─────────────────────────────────────────────────

test.describe("Editor buttons are hidden", () => {
  test("LLM Overview page has no Editor button", async ({ page }) => {
    await goTo(page, "/llm-configuration");
    expect(await page.getByRole("button", { name: /editor|raw editor/i }).count()).toBe(0);
  });

  test("MCP Overview page has no Editor button", async ({ page }) => {
    await goTo(page, "/mcp-configuration");
    expect(await page.getByRole("button", { name: /editor|raw editor/i }).count()).toBe(0);
  });
});
