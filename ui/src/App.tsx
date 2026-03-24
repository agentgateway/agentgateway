import { ConfigProvider, theme } from "antd";
import { HashRouter, Navigate, Route, Routes } from "react-router-dom";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { MainLayout } from "./components/Layout/MainLayout";
import { ConfirmProvider } from "./contexts/ConfirmContext";
import { EditorSettingsProvider } from "./contexts/EditorSettingsContext";
import { BackendsPage } from "./pages/Backends/BackendsPage";
import { CELPlaygroundPage } from "./pages/CELPlayground/CELPlaygroundPage";
import { DashboardPage } from "./pages/Dashboard/DashboardPage";
import { FormPage } from "./pages/FormPage";
import { ListenersPage } from "./pages/Listeners/ListenersPage";
import { LLMLogsPage } from "./pages/LLM/LLMLogsPage";
import { LLMMetricsPage } from "./pages/LLM/LLMMetricsPage";
import { LLMModelsPage } from "./pages/LLM/LLMModelsPage";
import { LLMOverviewPage } from "./pages/LLM/LLMOverviewPage";
import { LLMPlaygroundPage } from "./pages/LLM/LLMPlaygroundPage";
import { LLMPoliciesPage } from "./pages/LLM/LLMPoliciesPage";
import { MCPOverviewPage } from "./pages/MCP/MCPOverviewPage";
import {
  MCPLogsPage,
  MCPMetricsPage,
  MCPPlaygroundPage,
} from "./pages/MCP/MCPPages";
import { MCPPoliciesPage } from "./pages/MCP/MCPPoliciesPage";
import { MCPServersPage } from "./pages/MCP/MCPServersPage";
import { PlaygroundPage } from "./pages/Playground/PlaygroundPage";
import { PoliciesPage } from "./pages/Policies/PoliciesPage";
import { RoutesPage } from "./pages/Routes/RoutesPage";
import { SetupWizardPage } from "./pages/SetupWizard/SetupWizardPage";
import {
  TrafficPage,
  RawConfigPage as TrafficRawConfigPage,
} from "./pages/Traffic";

function App() {
  const isDark = document.documentElement.getAttribute("data-theme") === "dark";

  return (
    <ConfigProvider
      theme={{
        algorithm: isDark ? theme.darkAlgorithm : theme.defaultAlgorithm,
        components: {
          Menu: {
            itemHoverColor: "var(--color-text-base)",
            itemSelectedColor: "var(--color-primary)",
            itemBg: "transparent",
            itemHoverBg:
              "color-mix(in srgb, var(--color-sidebar) 10%, var(--color-bg-container))",
            itemSelectedBg:
              "color-mix(in srgb, var(--color-sidebar) 14%, var(--color-bg-container))",
          },
          Layout: {
            headerBg: "var(--color-bg-container)",
            triggerBg: "var(--color-bg-container)",
            bodyBg: "var(--color-bg-layout)",
          },
        },
      }}
    >
      <HashRouter>
        <ErrorBoundary>
          <EditorSettingsProvider>
            <ConfirmProvider>
              <MainLayout>
                <Routes>
                  <Route
                    path="/"
                    element={<Navigate to="/dashboard" replace />}
                  />

                  {/* OLD Section */}
                  <Route path="/dashboard" element={<DashboardPage />} />
                  <Route path="/listeners" element={<ListenersPage />} />
                  <Route path="/routes" element={<RoutesPage />} />
                  <Route path="/backends" element={<BackendsPage />} />
                  <Route path="/policies" element={<PoliciesPage />} />
                  <Route path="/playground" element={<PlaygroundPage />} />

                  {/* Generic Form Page */}
                  <Route path="/form" element={<FormPage />} />

                  {/* LLM Section */}
                  <Route path="/llm" element={<LLMOverviewPage />} />
                  <Route path="/llm/models" element={<LLMModelsPage />} />
                  <Route path="/llm/policies" element={<LLMPoliciesPage />} />
                  <Route path="/llm/logs" element={<LLMLogsPage />} />
                  <Route path="/llm/metrics" element={<LLMMetricsPage />} />
                  <Route
                    path="/llm/playground"
                    element={<LLMPlaygroundPage />}
                  />

                  {/* MCP Section */}
                  <Route path="/mcp" element={<MCPOverviewPage />} />
                  <Route path="/mcp/servers" element={<MCPServersPage />} />
                  <Route path="/mcp/policies" element={<MCPPoliciesPage />} />
                  <Route path="/mcp/logs" element={<MCPLogsPage />} />
                  <Route path="/mcp/metrics" element={<MCPMetricsPage />} />
                  <Route
                    path="/mcp/playground"
                    element={<MCPPlaygroundPage />}
                  />

                  {/* Traffic Section */}
                  <Route path="/traffic" element={<TrafficPage />} />
                  <Route
                    path="/traffic/raw-config"
                    element={<TrafficRawConfigPage />}
                  />
                  <Route path="/traffic/llm" element={<TrafficPage />} />
                  <Route
                    path="/traffic/llm/model/:modelIndex"
                    element={<TrafficPage />}
                  />
                  <Route path="/traffic/mcp" element={<TrafficPage />} />
                  <Route
                    path="/traffic/frontendPolicies"
                    element={<TrafficPage />}
                  />
                  <Route path="/traffic/bind/:port" element={<TrafficPage />} />
                  <Route
                    path="/traffic/bind/:port/listener/:li"
                    element={<TrafficPage />}
                  />
                  <Route
                    path="/traffic/bind/:port/listener/:li/route/:ri"
                    element={<TrafficPage />}
                  />
                  <Route
                    path="/traffic/bind/:port/listener/:li/tcproute/:ri"
                    element={<TrafficPage />}
                  />
                  <Route
                    path="/traffic/bind/:port/listener/:li/route/:ri/backend/:bi"
                    element={<TrafficPage />}
                  />
                  <Route
                    path="/traffic/bind/:port/listener/:li/tcproute/:ri/backend/:bi"
                    element={<TrafficPage />}
                  />
                  <Route
                    path="/traffic/bind/:port/listener/:li/route/:ri/policy/:policyType"
                    element={<TrafficPage />}
                  />
                  <Route
                    path="/traffic/bind/:port/listener/:li/tcproute/:ri/policy/:policyType"
                    element={<TrafficPage />}
                  />

                  {/* CEL Playground */}
                  <Route
                    path="/cel-playground"
                    element={<CELPlaygroundPage />}
                  />

                  {/* Setup Wizard */}
                  <Route path="/setup" element={<SetupWizardPage />} />

                  {/* Catch all */}
                  <Route
                    path="*"
                    element={<Navigate to="/dashboard" replace />}
                  />
                </Routes>
              </MainLayout>
            </ConfirmProvider>
          </EditorSettingsProvider>
        </ErrorBoundary>
      </HashRouter>
    </ConfigProvider>
  );
}

export default App;
