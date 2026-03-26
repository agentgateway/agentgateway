import { ConfigProvider, theme } from "antd";
import { HashRouter, Navigate, Route, Routes } from "react-router-dom";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { MainLayout } from "./components/Layout/MainLayout";
import { ConfirmProvider } from "./contexts/ConfirmContext";
import { EditorSettingsProvider } from "./contexts/EditorSettingsContext";
import { CELPlaygroundPage } from "./pages/CELPlayground/Playground/CELPlaygroundPage";
import { ConfigEditorPage } from "./pages/ConfigEditor/ConfigEditorPage";
import { DashboardPage } from "./pages/Dashboard/DashboardPage";
import { LLMConfigurationPage } from "./pages/LLM/LLMConfigurationPage";
import { LLMLogsPage } from "./pages/LLM/LLMLogsPage";
import { LLMMetricsPage } from "./pages/LLM/LLMMetricsPage";
import { LLMPlaygroundPage } from "./pages/LLM/Playground/LLMPlaygroundPage";
import { MCPConfigurationPage } from "./pages/MCP/MCPConfigurationPage";
import { MCPLogsPage } from "./pages/MCP/MCPLogsPage";
import { MCPMetricsPage } from "./pages/MCP/MCPMetricsPage";
import { MCPPlaygroundPage } from "./pages/MCP/Playground/MCPPlaygroundPage";
import { SetupWizardPage } from "./pages/SetupWizard/SetupWizardPage";
import { TrafficConfigurationPage } from "./pages/Traffic/TrafficConfigurationPage";
import { TrafficLogsPage } from "./pages/Traffic/TrafficLogsPage";
import { TrafficMetricsPage } from "./pages/Traffic/TrafficMetricsPage";

function App() {
  const isDark = document.documentElement.getAttribute("data-theme") === "dark";

  return (
    <ConfigProvider
      theme={{
        algorithm: isDark ? theme.darkAlgorithm : theme.defaultAlgorithm,
        components: {
          Menu: {
            itemHoverColor: "var(--color-text-base)",
            itemSelectedColor: "var(--color-text-base)",
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

                  <Route path="/dashboard" element={<DashboardPage />} />

                  {/* LLM Section */}
                  <Route path="/llm" element={<LLMConfigurationPage />} />
                  <Route path="/llm/model/:modelIndex" element={<LLMConfigurationPage />} />
                  <Route path="/llm/policy/:policyType" element={<LLMConfigurationPage />} />
                  <Route path="/llm/raw-config" element={<ConfigEditorPage />} />
                  <Route path="/llm/logs" element={<LLMLogsPage />} />
                  <Route path="/llm/metrics" element={<LLMMetricsPage />} />
                  <Route
                    path="/llm/playground"
                    element={<LLMPlaygroundPage />}
                  />

                  {/* MCP Section */}
                  <Route path="/mcp" element={<MCPConfigurationPage />} />
                  <Route path="/mcp/target/:targetIndex" element={<MCPConfigurationPage />} />
                  <Route path="/mcp/target/:targetIndex/policy/:policyType" element={<MCPConfigurationPage />} />
                  <Route path="/mcp/policy/:policyType" element={<MCPConfigurationPage />} />
                  <Route path="/mcp/raw-config" element={<ConfigEditorPage />} />
                  <Route path="/mcp/logs" element={<MCPLogsPage />} />
                  <Route path="/mcp/metrics" element={<MCPMetricsPage />} />
                  <Route
                    path="/mcp/playground"
                    element={<MCPPlaygroundPage />}
                  />

                  {/* Traffic Section */}
                  <Route path="/traffic" element={<TrafficConfigurationPage />} />
                  <Route path="/traffic/logs" element={<TrafficLogsPage />} />
                  <Route
                    path="/traffic/metrics"
                    element={<TrafficMetricsPage />}
                  />
                  <Route
                    path="/traffic/raw-config"
                    element={<ConfigEditorPage />}
                  />
                  <Route path="/traffic/llm" element={<TrafficConfigurationPage />} />
                  <Route
                    path="/traffic/llm/model/:modelIndex"
                    element={<TrafficConfigurationPage />}
                  />
                  <Route
                    path="/traffic/llm/policy/:policyType"
                    element={<TrafficConfigurationPage />}
                  />
                  <Route path="/traffic/mcp" element={<TrafficConfigurationPage />} />
                  <Route
                    path="/traffic/mcp/target/:targetIndex"
                    element={<TrafficConfigurationPage />}
                  />
                  <Route
                    path="/traffic/mcp/target/:targetIndex/policy/:policyType"
                    element={<TrafficConfigurationPage />}
                  />
                  <Route
                    path="/traffic/mcp/policy/:policyType"
                    element={<TrafficConfigurationPage />}
                  />
                  <Route
                    path="/traffic/frontendPolicies"
                    element={<TrafficConfigurationPage />}
                  />
                  <Route path="/traffic/bind/:port" element={<TrafficConfigurationPage />} />
                  <Route
                    path="/traffic/bind/:port/listener/:li"
                    element={<TrafficConfigurationPage />}
                  />
                  <Route
                    path="/traffic/bind/:port/listener/:li/route/:ri"
                    element={<TrafficConfigurationPage />}
                  />
                  <Route
                    path="/traffic/bind/:port/listener/:li/tcproute/:ri"
                    element={<TrafficConfigurationPage />}
                  />
                  <Route
                    path="/traffic/bind/:port/listener/:li/route/:ri/backend/:bi"
                    element={<TrafficConfigurationPage />}
                  />
                  <Route
                    path="/traffic/bind/:port/listener/:li/tcproute/:ri/backend/:bi"
                    element={<TrafficConfigurationPage />}
                  />
                  <Route
                    path="/traffic/bind/:port/listener/:li/route/:ri/policy/:policyType"
                    element={<TrafficConfigurationPage />}
                  />
                  <Route
                    path="/traffic/bind/:port/listener/:li/tcproute/:ri/policy/:policyType"
                    element={<TrafficConfigurationPage />}
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
