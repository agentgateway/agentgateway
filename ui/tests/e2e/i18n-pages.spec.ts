import { expect, test, type Page, type TestInfo } from "@playwright/test";
import { emptyConfig, mockGateway, sameOriginGatewayConfig } from "./fixtures";

const populatedPagePaths = [
  "/",
  "/llm/models",
  "/llm/providers",
  "/llm/policies",
  "/llm/guardrails",
  "/llm/costs",
  "/llm/logs",
  "/llm/analytics",
  "/llm/keys",
  "/llm/playground",
  "/llm/client-setup",
  "/mcp/servers",
  "/mcp/policies",
  "/mcp/playground",
  "/traffic/gateways",
  "/traffic/listeners",
  "/traffic/routes",
  "/traffic/policies",
  "/cel",
  "/settings",
  "/raw-config",
] as const;

const setupPagePaths = [
  "/llm/get-started",
  "/mcp/get-started",
  "/traffic/get-started",
] as const;

test("every page avoids word-by-word mixed Chinese copy", async ({
  page,
}, testInfo) => {
  test.setTimeout(120_000);
  const findings: Array<{ path: string; text: string }> = [];
  await mockGateway(page, sameOriginGatewayConfig());
  for (const path of populatedPagePaths) {
    await inspectPage(page, path, testInfo, findings);
  }

  const setupPage = await page.context().newPage();
  await mockGateway(setupPage, emptyConfig());
  for (const path of setupPagePaths) {
    await inspectPage(setupPage, path, testInfo, findings);
  }
  await setupPage.close();

  await page.goto("/settings?lang=zh-CN");
  await page.getByRole("button", { name: /JWT 身份验证/ }).click();
  await expect(
    page.getByRole("heading", { name: "JWT 身份验证" }),
  ).toBeVisible();
  findings.push(
    ...(await mixedCopyFindings(page)).map((text) => ({
      path: "/settings#jwtAuth",
      text,
    })),
  );
  if (process.env.I18N_SCREENSHOT) {
    await page.screenshot({
      path: testInfo.outputPath("zh-CN-settings-jwt-auth.png"),
      fullPage: true,
    });
  }

  expect(findings).toEqual([]);
});

test("every English page avoids visible Chinese copy", async ({ page }) => {
  test.setTimeout(240_000);
  const findings: Array<{ path: string; text: string }> = [];
  await mockGateway(page, sameOriginGatewayConfig());
  for (const path of populatedPagePaths) {
    await inspectEnglishPage(page, path, findings);
  }

  const setupPage = await page.context().newPage();
  await mockGateway(setupPage, emptyConfig());
  for (const path of setupPagePaths) {
    await inspectEnglishPage(setupPage, path, findings);
  }
  await setupPage.close();

  await page.goto("/settings?lang=en");
  await page.getByRole("button", { name: /JWT auth/i }).click();
  findings.push(
    ...(await visibleChineseCopy(page)).map((text) => ({
      path: "/settings#jwtAuth",
      text,
    })),
  );

  expect(findings).toEqual([]);
});

async function inspectPage(
  page: Page,
  path: string,
  testInfo: TestInfo,
  findings: Array<{ path: string; text: string }>,
) {
  await page.goto(`${path}?lang=zh-CN`);
  await expect(page.locator("html")).toHaveAttribute("lang", "zh-CN");
  await expect(page.locator("main.content")).toBeVisible();
  await expect
    .poll(() => page.locator("body").innerText())
    .not.toMatch(/正在加载/);
  findings.push(
    ...(await mixedCopyFindings(page)).map((text) => ({ path, text })),
  );
  if (process.env.I18N_SCREENSHOT) {
    const slug = path === "/" ? "home" : path.slice(1).replaceAll("/", "-");
    await page.screenshot({
      path: testInfo.outputPath(`zh-CN-${slug}.png`),
      fullPage: true,
    });
  }
}

async function inspectEnglishPage(
  page: Page,
  path: string,
  findings: Array<{ path: string; text: string }>,
) {
  await page.goto(`${path}?lang=en`);
  await expect(page.locator("html")).toHaveAttribute("lang", "en");
  await expect(page.locator("main.content")).toBeVisible();
  findings.push(
    ...(await visibleChineseCopy(page)).map((text) => ({ path, text })),
  );
}

async function visibleChineseCopy(page: Page) {
  const bodyText = await page.locator("body").innerText();
  return [...new Set(bodyText.split("\n").map((line) => line.trim()))].filter(
    (text) => /[\p{Script=Han}]/u.test(text),
  );
}

async function mixedCopyFindings(page: Page) {
  const bodyText = await page.locator("body").innerText();
  return [...new Set(bodyText.split("\n").map((line) => line.trim()))].filter(
    isSuspiciousMixedCopy,
  );
}

function isSuspiciousMixedCopy(text: string) {
  const hanCount = text.match(/[\p{Script=Han}]/gu)?.length ?? 0;
  if (hanCount < 2) return false;
  const prose = text
    .replace(/https?:\/\/\S+/giu, "")
    .replace(
      /\b(?:Agentgateway|OpenAI|Anthropic|Claude|Gemini|Ollama|Azure|AWS|Amazon|Google|Bedrock|Guardrails|VS|Code|Copilot|Business|Enterprise|API|HTTP|HTTPS|TCP|JWT|JWKS|OIDC|OAuth|LLM|MCP|CEL|CORS|JSON|YAML|URL|SDK|CLI|TLS|OTLP|gRPC|Basic|Bearer|POST|GET|PUT|PATCH|DELETE|SQLite|WebSocket)\b/giu,
      "",
    );
  const englishWords = prose.match(/[A-Za-z][A-Za-z'-]{2,}/g) ?? [];
  return englishWords.length >= 3;
}
