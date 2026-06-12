import styled from "@emotion/styled";
import { Card, Col, Row } from "antd";
import { ChevronDown, ChevronUp, FileCode, Send } from "lucide-react";
import { MonacoEditorWithSettings } from "../../../components/MonacoEditor";
import { ActionPanel } from "../../../components/playground/ActionPanel";
import { CapabilitiesList } from "../../../components/playground/CapabilitiesList";
import { useTheme } from "../../../contexts/ThemeContext";
import type { McpState, UiState } from "./types";

const SectionCard = styled(Card)`
  .ant-card-head {
    background: var(--color-bg-container);
    border-bottom: 1px solid var(--color-border-secondary);
    padding: var(--spacing-md) var(--spacing-lg);
    min-height: auto;
    display: flex;
    align-items: center;
  }

  .ant-card-head-title {
    font-weight: 600;
    font-size: 15px;
    padding: 0;
    display: flex;
    align-items: center;
    gap: 8px;

    svg {
      flex-shrink: 0;
    }
  }

  .ant-card-body {
    padding: var(--spacing-lg);
  }
`;

const ExpandButton = styled.button`
  position: absolute;
  bottom: 0;
  left: 50%;
  transform: translateX(-50%);
  background: transparent;
  border: none;
  padding: 2px 8px;
  cursor: pointer;
  display: flex;
  align-items: center;
  gap: 2px;
  font-size: 11px;
  color: var(--color-text-secondary);
  opacity: 0.7;
  transition: all 0.2s;

  &:hover {
    opacity: 1;
    color: var(--color-text-base);
  }

  svg {
    width: 12px;
    height: 12px;
  }
`;

interface ToolTesterProps {
  mcpState: McpState;
  uiState: UiState;
  resultExpanded: boolean;
  onToolSelect: (tool: any) => void;
  onParamChange: (paramName: string, value: any) => void;
  onRunTool: () => void;
  onToggleExpand: () => void;
}

export function ToolTester({
  mcpState,
  uiState,
  resultExpanded,
  onToolSelect,
  onParamChange,
  onRunTool,
  onToggleExpand,
}: ToolTesterProps) {
  const { theme } = useTheme();

  return (
    <Row gutter={[16, 16]}>
      {/* Left Column: Available Tools */}
      <Col xs={24} lg={8}>
        <CapabilitiesList
          connectionType="mcp"
          isLoading={uiState.isLoadingCapabilities}
          mcpTools={mcpState.tools}
          a2aSkills={[]}
          a2aAgentCard={null}
          selectedMcpToolName={mcpState.selectedTool?.name || null}
          selectedA2aSkillId={null}
          onMcpToolSelect={onToolSelect}
          onA2aSkillSelect={() => {}}
        />
      </Col>

      {/* Right Column: Request and Response */}
      <Col xs={24} lg={16}>
        <div
          style={{ display: "flex", flexDirection: "column", gap: "16px" }}
        >
          {/* Top: User Request */}
          <SectionCard
            title={
              <>
                <Send size={18} /> Request
              </>
            }
          >
            <ActionPanel
              connectionType="mcp"
              mcpSelectedTool={mcpState.selectedTool}
              a2aSelectedSkill={null}
              mcpParamValues={mcpState.paramValues}
              a2aMessage=""
              isRequestRunning={uiState.isRequestRunning}
              onMcpParamChange={onParamChange}
              onA2aMessageChange={() => {}}
              onRunMcpTool={onRunTool}
              onRunA2aSkill={() => {}}
            />
          </SectionCard>

          {/* Bottom: Response */}
          <div style={{ position: "relative" }}>
            <SectionCard
              title={
                <>
                  <FileCode size={18} /> Response
                </>
              }
            >
              {mcpState.response ? (
                <div style={{ padding: 0 }}>
                  <MonacoEditorWithSettings
                    value={JSON.stringify(mcpState.response, null, 2)}
                    language="json"
                    height={resultExpanded ? "300px" : "70px"}
                    theme={theme}
                    readOnly
                    options={{
                      readOnly: true,
                      minimap: { enabled: false },
                      scrollBeyondLastLine: false,
                    }}
                  />
                </div>
              ) : (
                <div
                  style={{
                    display: "flex",
                    alignItems: "center",
                    justifyContent: "center",
                    height: "70px",
                    color: "var(--color-text-secondary)",
                    fontSize: "14px",
                  }}
                >
                  Select a tool and click "Run Tool" to see the response
                </div>
              )}
            </SectionCard>
            {mcpState.response && (
              <ExpandButton type="button" onClick={onToggleExpand}>
                {resultExpanded ? (
                  <>
                    <ChevronUp size={14} />
                    Collapse
                  </>
                ) : (
                  <>
                    <ChevronDown size={14} />
                    Expand
                  </>
                )}
              </ExpandButton>
            )}
          </div>
        </div>
      </Col>
    </Row>
  );
}
