import styled from "@emotion/styled";
import { Button, Card, Input, Select, Tag, Typography } from "antd";
import { Trash2 } from "lucide-react";
import { PROVIDER_COLORS } from "./constants";
import type { Message, PlaygroundModel } from "./types";

const { Text } = Typography;

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

interface SettingsPanelProps {
  models: PlaygroundModel[];
  selectedLabel: string | null;
  selectedModel: PlaygroundModel | null;
  modelOverride: string;
  messages: Message[];
  prompt: string;
  onSelectLabel: (label: string) => void;
  onChangeModelOverride: (value: string) => void;
  onClear: () => void;
}

export function SettingsPanel({
  models,
  selectedLabel,
  selectedModel,
  modelOverride,
  messages,
  prompt,
  onSelectLabel,
  onChangeModelOverride,
  onClear,
}: SettingsPanelProps) {
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
              options={models.map((m) => ({
                label: (
                  <span
                    style={{
                      display: "flex",
                      alignItems: "center",
                      gap: 8,
                    }}
                  >
                    <Tag
                      color={PROVIDER_COLORS[m.provider] ?? "default"}
                      style={{ fontSize: 10, marginRight: 0 }}
                    >
                      {m.provider}
                    </Tag>
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
