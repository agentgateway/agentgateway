"use client";

import type { AgentCard, AgentSkill } from "@a2a-js/sdk";
import type { Tool as McpTool } from "@modelcontextprotocol/sdk/types.js";
import { Card, Spin, Table, Tag } from "antd";
import { Bot, Settings, Zap } from "lucide-react";

const { Column } = Table;

interface CapabilitiesListProps {
  connectionType: "mcp" | "a2a" | null;
  isLoading: boolean;
  mcpTools: McpTool[];
  a2aSkills: AgentSkill[];
  a2aAgentCard: AgentCard | null;
  selectedMcpToolName: string | null;
  selectedA2aSkillId: string | null;
  onMcpToolSelect: (tool: McpTool) => void;
  onA2aSkillSelect: (skill: AgentSkill) => void;
}

export function CapabilitiesList({
  connectionType,
  isLoading,
  mcpTools,
  a2aSkills,
  a2aAgentCard,
  selectedMcpToolName,
  selectedA2aSkillId,
  onMcpToolSelect,
  onA2aSkillSelect,
}: CapabilitiesListProps) {
  const title = connectionType === "a2a" ? "Skills" : "Tools";
  const noItemsMessage = `No ${title.toLowerCase()} discovered.`;
  const loadingMessage = `Loading ${title}...`;

  if (isLoading) {
    return (
      <Card
        title={
          <div className="flex items-center gap-2">
            {connectionType === "a2a" ? (
              <Bot className="h-5 w-5" />
            ) : (
              <Settings className="h-5 w-5" />
            )}
            {connectionType === "a2a"
              ? a2aAgentCard?.name || "Unknown Agent"
              : "Available Tools"}
          </div>
        }
      >
        <div className="flex items-center justify-center py-8 h-full">
          <Spin size="large" />
          <span className="ml-3">{loadingMessage}</span>
        </div>
      </Card>
    );
  }

  if (connectionType === "a2a" && a2aAgentCard) {
    return (
      <Card
        title={
          <div className="flex items-center gap-2">
            <Bot className="h-5 w-5" />
            {a2aAgentCard.name || "Unknown Agent"}
          </div>
        }
      >
          <div className="space-y-4">
            {/* Agent Card Details */}
            <div className="space-y-4">
              <div className="grid gap-3">
                {/* Capabilities */}
                {a2aAgentCard.capabilities &&
                  Object.keys(a2aAgentCard.capabilities).length > 0 && (
                    <div>
                      <div className="flex items-center gap-2 mb-2">
                        <Zap className="h-4 w-4" />
                        <span className="text-sm font-medium">
                          Capabilities
                        </span>
                      </div>
                      <div className="flex flex-wrap gap-1">
                        {Object.entries(a2aAgentCard.capabilities).map(
                          ([key, value]) => (
                            <Tag key={key} color={value ? "blue" : "default"}>
                              {key}: {value ? "✓" : "✗"}
                            </Tag>
                          ),
                        )}
                      </div>
                    </div>
                  )}

                {/* Input/Output Modes */}
                <div className="grid grid-cols-2 gap-4">
                  {a2aAgentCard.defaultInputModes &&
                    a2aAgentCard.defaultInputModes.length > 0 && (
                      <div>
                        <span className="text-sm font-medium">Input Modes</span>
                        <div className="flex flex-wrap gap-1 mt-1">
                          {a2aAgentCard.defaultInputModes.map((mode, idx) => (
                            <Tag key={idx}>{mode}</Tag>
                          ))}
                        </div>
                      </div>
                    )}

                  {a2aAgentCard.defaultOutputModes &&
                    a2aAgentCard.defaultOutputModes.length > 0 && (
                      <div>
                        <span className="text-sm font-medium">
                          Output Modes
                        </span>
                        <div className="flex flex-wrap gap-1 mt-1">
                          {a2aAgentCard.defaultOutputModes.map((mode, idx) => (
                            <Tag key={idx}>{mode}</Tag>
                          ))}
                        </div>
                      </div>
                    )}
                </div>
              </div>

              {/* Skills Section */}
              <div>
                <h5 className="font-medium mb-3 flex items-center gap-2">
                  <Settings className="h-4 w-4" />
                  Available Skills ({a2aSkills.length})
                </h5>
                {a2aSkills.length === 0 ? (
                  <div className="text-center py-4">
                    <span className="text-muted-foreground text-sm">
                      No skills available
                    </span>
                  </div>
                ) : (
                  <div className="overflow-y-auto max-h-[300px]">
                    <Table
                      dataSource={a2aSkills}
                      rowKey="id"
                      size="small"
                      pagination={false}
                      onRow={(record) => ({
                        onClick: () => onA2aSkillSelect(record),
                        style: {
                          cursor: "pointer",
                          backgroundColor:
                            selectedA2aSkillId === record.id
                              ? "var(--color-bg-selected)"
                              : "transparent",
                        },
                      })}
                    >
                      <Column title="Name" dataIndex="name" key="name" />
                      <Column
                        title="Description"
                        dataIndex="description"
                        key="description"
                      />
                    </Table>
                  </div>
                )}
              </div>
            </div>
          </div>
      </Card>
    );
  }

  if (
    (connectionType === "mcp" && mcpTools.length === 0) ||
    (connectionType === "a2a" && a2aSkills.length === 0)
  ) {
    return (
      <Card
        title={
          <div className="flex items-center gap-2">
            <Settings className="h-5 w-5" />
            Available Tools
          </div>
        }
      >
        <div className="flex items-center justify-center py-8 h-full">
          <span>{noItemsMessage}</span>
        </div>
      </Card>
    );
  }

  if (connectionType === "mcp") {
    return (
      <Card
        title={
          <div className="flex items-center gap-2">
            <Settings className="h-5 w-5" />
            Available Tools
          </div>
        }
      >
          <div className="overflow-y-auto max-h-[400px]">
            <Table
              dataSource={mcpTools}
              rowKey="name"
              size="small"
              pagination={false}
              onRow={(record) => ({
                onClick: () => onMcpToolSelect(record),
                style: {
                  cursor: "pointer",
                  backgroundColor:
                    selectedMcpToolName === record.name
                      ? "var(--color-bg-selected)"
                      : "transparent",
                },
              })}
            >
              <Column title="Name" dataIndex="name" key="name" />
              <Column
                title="Description"
                dataIndex="description"
                key="description"
              />
            </Table>
          </div>
      </Card>
    );
  }

  return null;
}
