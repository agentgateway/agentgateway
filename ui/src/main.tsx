import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  RouterProvider,
  createRootRoute,
  createRoute,
  createRouter,
} from "@tanstack/react-router";
import React from "react";
import { createRoot } from "react-dom/client";
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

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: HomePage,
});

const dumpPoliciesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/traffic/policies",
  component: DumpPoliciesPage,
});

const modelsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/llm/models",
  component: ModelsPage,
});

const llmGetStartedRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/llm/get-started",
  component: LlmGetStartedPage,
});

const providersRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/llm/providers",
  component: ProvidersPage,
});

const logsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/llm/logs",
  component: LogsPage,
});

const analyticsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/llm/analytics",
  component: AnalyticsPage,
});

const policiesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/llm/policies",
  component: PoliciesPage,
});

const guardrailsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/llm/guardrails",
  component: GuardrailsPage,
});

const costsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/llm/costs",
  component: CostsPage,
});

const keysRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/llm/keys",
  component: KeysPage,
});

const playgroundRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/llm/playground",
  component: PlaygroundPage,
});

const clientSetupRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/llm/client-setup",
  component: ClientSetupPage,
});

const mcpServersRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/mcp/servers",
  component: McpServersPage,
});

const mcpPoliciesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/mcp/policies",
  component: McpPoliciesPage,
});

const mcpGetStartedRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/mcp/get-started",
  component: McpGetStartedPage,
});

const mcpPlaygroundRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/mcp/playground",
  component: McpPlaygroundPage,
});

const trafficListenersRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/traffic/listeners",
  component: TrafficListenersPage,
});

const trafficGetStartedRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/traffic/get-started",
  component: TrafficGetStartedPage,
});

const trafficRoutesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/traffic/routes",
  component: TrafficRoutesPage,
});

const celRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/cel",
  component: CelPage,
});

const rawConfigRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/raw-config",
  component: RawConfigRoute,
});

function RawConfigRoute() {
  return (
    <React.Suspense
      fallback={
        <div className="page-stack">
          <p className="muted-copy">Loading raw configuration...</p>
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
    trafficListenersRoute,
    trafficRoutesRoute,
    celRoute,
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

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>
  </React.StrictMode>,
);
