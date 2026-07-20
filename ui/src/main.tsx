import { tr } from "./i18n";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  RouterProvider,
  createRootRoute,
  createRoute,
  createRouter,
} from "@tanstack/react-router";
import React from "react";
import { createRoot } from "react-dom/client";
import { useTranslation } from "react-i18next";
import "./i18n";
import { routerBasePath } from "./basePath";
import { Shell } from "./components/Shell";
import { CelPage } from "./pages/Cel";
import { ClientSetupPage } from "./pages/ClientSetup";
import { CostsPage } from "./pages/Costs";
import { DumpPoliciesPage } from "./pages/DumpPolicies";
import {
  LlmGetStartedPage,
  McpGetStartedPage,
  TrafficGetStartedPage,
} from "./pages/GetStarted";
import { GuardrailsPage } from "./pages/Guardrails";
import { HomePage } from "./pages/Home";
import { KeysPage } from "./pages/Keys";
import { AnalyticsPage, LogsPage } from "./pages/Logs";
import { McpPlaygroundPage } from "./pages/McpPlayground";
import { McpServersPage } from "./pages/McpServers";
import { ModelsPage } from "./pages/Models";
import { McpPoliciesPage, PoliciesPage } from "./pages/Policies";
import { PlaygroundPage } from "./pages/Playground";
import { ProvidersPage } from "./pages/Providers";
import { RawSettingsPage } from "./pages/RawSettings";
import { TrafficGatewaysPage } from "./pages/TrafficGateways";
import { TrafficListenersPage } from "./pages/TrafficListeners";
import { TrafficRoutesPage } from "./pages/TrafficRoutes";
import "@fontsource/geist-sans/latin-400.css";
import "@fontsource/geist-sans/latin-500.css";
import "@fontsource/geist-sans/latin-600.css";
import "@fontsource/geist-sans/latin-700.css";
import "./styles.css";
import "./styles/analytics.css";

const LazyRawConfigPage = React.lazy(() =>
  import("./pages/RawConfig").then((module) => ({
    default: module.RawConfigPage,
  })),
);

const rootRoute = createRootRoute({
  component: Shell,
});

function localizedRoute(Component: React.ComponentType) {
  return function LocalizedRoute() {
    useTranslation();
    return <Component />;
  };
}

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: localizedRoute(HomePage),
});

const dumpPoliciesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/traffic/policies",
  component: localizedRoute(DumpPoliciesPage),
});

const modelsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/llm/models",
  component: localizedRoute(ModelsPage),
});

const llmGetStartedRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/llm/get-started",
  component: localizedRoute(LlmGetStartedPage),
});

const providersRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/llm/providers",
  component: localizedRoute(ProvidersPage),
});

const logsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/llm/logs",
  component: localizedRoute(LogsPage),
});

const analyticsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/llm/analytics",
  component: localizedRoute(AnalyticsPage),
});

const policiesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/llm/policies",
  component: localizedRoute(PoliciesPage),
});

const guardrailsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/llm/guardrails",
  component: localizedRoute(GuardrailsPage),
});

const costsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/llm/costs",
  component: localizedRoute(CostsPage),
});

const keysRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/llm/keys",
  component: localizedRoute(KeysPage),
});

const playgroundRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/llm/playground",
  component: localizedRoute(PlaygroundPage),
});

const clientSetupRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/llm/client-setup",
  component: localizedRoute(ClientSetupPage),
});

const mcpServersRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/mcp/servers",
  component: localizedRoute(McpServersPage),
});

const mcpPoliciesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/mcp/policies",
  component: localizedRoute(McpPoliciesPage),
});

const mcpGetStartedRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/mcp/get-started",
  component: localizedRoute(McpGetStartedPage),
});

const mcpPlaygroundRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/mcp/playground",
  component: localizedRoute(McpPlaygroundPage),
});

const trafficListenersRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/traffic/listeners",
  component: localizedRoute(TrafficListenersPage),
});

const trafficGatewaysRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/traffic/gateways",
  component: localizedRoute(TrafficGatewaysPage),
});

const trafficGetStartedRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/traffic/get-started",
  component: localizedRoute(TrafficGetStartedPage),
});

const trafficRoutesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/traffic/routes",
  component: localizedRoute(TrafficRoutesPage),
});

const celRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/cel",
  component: localizedRoute(CelPage),
});

const rawConfigRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/raw-config",
  component: localizedRoute(RawConfigRoute),
});

const rawSettingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings",
  component: localizedRoute(RawSettingsPage),
});

function RawConfigRoute() {
  return (
    <React.Suspense
      fallback={
        <div className="page-stack">
          <p className="muted-copy">{tr("copy.loadingRawConfiguration")}</p>
        </div>
      }
    >
      <LazyRawConfigPage />
    </React.Suspense>
  );
}

const router = createRouter({
  basepath: routerBasePath(),
  routeTree: rootRoute.addChildren([
    indexRoute,
    dumpPoliciesRoute,
    llmGetStartedRoute,
    modelsRoute,
    providersRoute,
    policiesRoute,
    guardrailsRoute,
    costsRoute,
    logsRoute,
    analyticsRoute,
    keysRoute,
    playgroundRoute,
    clientSetupRoute,
    mcpGetStartedRoute,
    mcpServersRoute,
    mcpPoliciesRoute,
    mcpPlaygroundRoute,
    trafficGetStartedRoute,
    trafficGatewaysRoute,
    trafficListenersRoute,
    trafficRoutesRoute,
    celRoute,
    rawSettingsRoute,
    rawConfigRoute,
  ]),
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 5_000,
      refetchOnWindowFocus: false,
    },
  },
});

function LocalizedRouter() {
  return <RouterProvider router={router} />;
}

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <LocalizedRouter />
    </QueryClientProvider>
  </React.StrictMode>,
);
