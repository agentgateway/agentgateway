import styled from "@emotion/styled";
import { Button, Card, Checkbox, Input, Select, Typography } from "antd";
import { Trash2 } from "lucide-react";
import { useEffect } from "react";
import { useConfig, useMCPConfig } from "../../../api";
import { ProviderIcon } from "../../../imported/components/ProviderIcon";
import type { Message, PlaygroundModel } from "./types";

const { Text } = Typography;

// region Styles

const SidebarSection = styled.div`
  display: flex;
  flex-direction: column;
  gap: var(--spacing-lg);
`;

const SectionLabel = styled.div`
  font-size: 12px;
  font-weight: 600;
  color: var(--color-text-secondary);
  margin-bottom: 6px;
  text-transform: uppercase;
  letter-spacing: 0.05em;
`;

const EndpointInfo = styled.div`
  font-size: 12px;
  color: var(--color-text-secondary);
  background: var(--color-bg-elevated);
  border: 1px solid var(--color-border-secondary);
  border-radius: var(--border-radius-sm);
  padding: 6px 10px;
  font-family: var(--font-family-code);
  word-break: break-all;
`;

// endregion Styles

// region Helpers

const API_KEY_MODE_KEY = "playgroundApiKeyMode";
const SELECTED_KEY_KEY = "playgroundSelectedKey";

// endregion Helpers

// region Component

interface SettingsPanelProps {
  models: PlaygroundModel[];
  providedModelName: string | null;
  selectedLabel: string | null;
  selectedModel: PlaygroundModel | null;
  modelOverride: string;
  messages: Message[];
  prompt: string;
  apiKeyMode: "none" | "managed" | "raw";
  selectedApiKey: string;
  rawApiKey: string;
  includeMcpTools: boolean;
  onSelectLabel: (label: string) => void;
  onChangeModelOverride: (value: string) => void;
  onClear: () => void;
  onApiKeyModeChange: (mode: "none" | "managed" | "raw") => void;
  onSelectedApiKeyChange: (key: string) => void;
  onRawApiKeyChange: (key: string) => void;
  onIncludeMcpToolsChange: (checked: boolean) => void;
}

export function SettingsPanel({
  models,
  providedModelName,
  selectedLabel,
  selectedModel,
  modelOverride,
  messages,
  prompt,
  apiKeyMode,
  selectedApiKey,
  rawApiKey,
  includeMcpTools,
  onSelectLabel,
  onChangeModelOverride,
  onClear,
  onApiKeyModeChange,
  onSelectedApiKeyChange,
  onRawApiKeyChange,
  onIncludeMcpToolsChange,
}: SettingsPanelProps) {
  const { data: config } = useConfig();
  const { data: mcp } = useMCPConfig();

  const apiKeys: { name: string; value: string }[] = (config?.llm as any)?.policies?.apiKey?.keys ?? [];
  const mcpTargetCount = mcp?.targets?.length ?? 0;

  useEffect(() => {
    if (!providedModelName || selectedLabel !== null) return;
    const match = models.find((m) => m.label === providedModelName || m.defaultModel === providedModelName);
    if (match) {
      onSelectLabel(match.label);
      onChangeModelOverride(match.defaultModel ?? providedModelName);
    }
  }, [providedModelName, models, selectedLabel, onSelectLabel, onChangeModelOverride]);

  const handleApiKeyModeChange = (mode: "none" | "managed" | "raw") => {
    localStorage.setItem(API_KEY_MODE_KEY, mode);
    onApiKeyModeChange(mode);
  };

  const handleSelectedApiKeyChange = (key: string) => {
    localStorage.setItem(SELECTED_KEY_KEY, key);
    onSelectedApiKeyChange(key);
  };

  return (
    <Card title="Settings" size="small">
      <SidebarSection>
        <div>
          <SectionLabel>Configuration</SectionLabel>
          {models.length === 0 ? (
            <Text type="secondary" style={{ fontSize: 13 }}>
              No models configured. Add an LLM configuration to get started.
            </Text>
          ) : (
            <Select
              style={{ width: "100%" }}
              placeholder="Select a configuration"
              value={selectedLabel}
              onChange={onSelectLabel}
              popupMatchSelectWidth={false}
              options={models.filter((m, i, arr) => arr.findIndex(x => x.label === m.label) === i).map((m) => ({
                label: (
                  <span
                    style={{
                      display: "flex",
                      alignItems: "center",
                      gap: 8,
                      overflow: "hidden",
                      minWidth: 0,
                    }}
                  >
                    <ProviderIcon provider={m.provider} />
                    {m.label}
                  </span>
                ),
                value: m.label,
              }))}
            />
          )}
        </div>

        {selectedModel && (
          <>
            <div>
              <SectionLabel>Model Name</SectionLabel>
              <Input
                placeholder="e.g., smallthinker, gpt-4"
                value={modelOverride}
                onChange={(e) => onChangeModelOverride(e.target.value)}
              />
              <Text
                type="secondary"
                style={{ fontSize: 11, marginTop: 4, display: "block" }}
              >
                The model name sent in the request body
              </Text>
            </div>

            <div>
              <SectionLabel>Endpoint</SectionLabel>
              <EndpointInfo>
                {selectedModel.baseUrl}/v1/chat/completions
              </EndpointInfo>
            </div>
          </>
        )}

        {/* Virtual API Key */}
        <div>
          <SectionLabel>Virtual API Key</SectionLabel>
          <Select
            style={{ width: "100%" }}
            value={apiKeyMode}
            onChange={handleApiKeyModeChange}
            options={[
              { label: "None", value: "none" },
              ...(apiKeys.length > 0
                ? [{ label: "Managed key", value: "managed" }]
                : []),
              { label: "Raw value", value: "raw" },
            ]}
          />
          {apiKeyMode === "managed" && apiKeys.length > 0 && (
            <Select
              style={{ width: "100%", marginTop: 6 }}
              placeholder="Select a key"
              value={selectedApiKey || undefined}
              onChange={handleSelectedApiKeyChange}
              options={apiKeys.map((k) => ({
                label: k.name || k.value.slice(0, 8) + "…",
                value: k.value,
              }))}
            />
          )}
          {apiKeyMode === "raw" && (
            <Input
              style={{ marginTop: 6 }}
              placeholder="sk-..."
              value={rawApiKey}
              onChange={(e) => onRawApiKeyChange(e.target.value)}
            />
          )}
        </div>

        {/* MCP Tools */}
        {mcpTargetCount > 0 && (
          <div>
            <Checkbox
              checked={includeMcpTools}
              onChange={(e) => onIncludeMcpToolsChange(e.target.checked)}
            >
              Include MCP tools ({mcpTargetCount} server{mcpTargetCount !== 1 ? "s" : ""})
            </Checkbox>
            <Text
              type="secondary"
              style={{ fontSize: 11, marginTop: 4, display: "block" }}
            >
              Let the model call tools exposed by the MCP gateway.
            </Text>
          </div>
        )}

        <Button
          size="small"
          icon={<Trash2 size={14} />}
          onClick={onClear}
          disabled={messages.length === 0 && !prompt}
        >
          Clear Chat
        </Button>
      </SidebarSection>
    </Card>
  );
}

// endregion Component
