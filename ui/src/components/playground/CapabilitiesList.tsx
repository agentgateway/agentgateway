"use client";

import { Tool as McpTool } from "@modelcontextprotocol/sdk/types.js";
import type { AgentSkill, AgentCard } from "@a2a-js/sdk";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";
import { Loader2, Bot, Zap, Settings } from "lucide-react";

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
  const description = `Select a ${title.toLowerCase()} to use`;
  const noItemsMessage = `No ${title.toLowerCase()} discovered.`;
  const loadingMessage = `Loading ${title}...`;

  return (
    <Card className="flex flex-col">
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          {connectionType === "a2a" ? (
            <Bot className="h-5 w-5" />
          ) : (
            <Settings className="h-5 w-5" />
          )}
          {connectionType === "a2a" ? a2aAgentCard?.name || "Unknown Agent" : "Available Tools"}
        </CardTitle>
        <CardDescription>
          {connectionType === "a2a" ? a2aAgentCard?.description || "Unknown Agent" : description}
        </CardDescription>
      </CardHeader>
      <CardContent className="flex-grow space-y-4">
        {isLoading ? (
          <div className="flex items-center justify-center py-8 h-full">
            <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
            <span className="ml-3 text-muted-foreground">{loadingMessage}</span>
          </div>
        ) : connectionType === "a2a" && a2aAgentCard ? (
          <>
            {/* Agent Card Details */}
            <div className="space-y-4">
              <div className="grid gap-3">
                {/* Capabilities */}
                {a2aAgentCard.capabilities && Object.keys(a2aAgentCard.capabilities).length > 0 && (
                  <div>
                    <div className="flex items-center gap-2 mb-2">
                      <Zap className="h-4 w-4 text-muted-foreground" />
                      <span className="text-sm font-medium">Capabilities</span>
                    </div>
                    <div className="flex flex-wrap gap-1">
                      {Object.entries(a2aAgentCard.capabilities).map(([key, value]) => (
                        <Badge
                          key={key}
                          variant={value ? "default" : "secondary"}
                          className="text-xs"
                        >
                          {key}: {value ? "✓" : "✗"}
                        </Badge>
                      ))}
                    </div>
                  </div>
                )}

                {/* Input/Output Modes */}
                <div className="grid grid-cols-2 gap-4">
                  {a2aAgentCard.defaultInputModes && a2aAgentCard.defaultInputModes.length > 0 && (
                    <div>
                      <span className="text-sm font-medium text-muted-foreground">Input Modes</span>
                      <div className="flex flex-wrap gap-1 mt-1">
                        {a2aAgentCard.defaultInputModes.map((mode, idx) => (
                          <Badge key={idx} variant="outline" className="text-xs">
                            {mode}
                          </Badge>
                        ))}
                      </div>
                    </div>
                  )}

                  {a2aAgentCard.defaultOutputModes &&
                    a2aAgentCard.defaultOutputModes.length > 0 && (
                      <div>
                        <span className="text-sm font-medium text-muted-foreground">
                          Output Modes
                        </span>
                        <div className="flex flex-wrap gap-1 mt-1">
                          {a2aAgentCard.defaultOutputModes.map((mode, idx) => (
                            <Badge key={idx} variant="outline" className="text-xs">
                              {mode}
                            </Badge>
                          ))}
                        </div>
                      </div>
                    )}
                </div>
              </div>

              <Separator />

              {/* Skills Section */}
              <div>
                <h5 className="font-medium mb-3 flex items-center gap-2">
                  <Settings className="h-4 w-4" />
                  Available Skills ({a2aSkills.length})
                </h5>
                {a2aSkills.length === 0 ? (
                  <div className="text-center py-4">
                    <span className="text-muted-foreground text-sm">No skills available</span>
                  </div>
                ) : (
                  <div className="overflow-y-auto max-h-[300px]">
                    <Table>
                      <TableHeader>
                        <TableRow>
                          <TableHead>Name</TableHead>
                          <TableHead>Description</TableHead>
                        </TableRow>
                      </TableHeader>
                      <TableBody>
                        {a2aSkills.map((skill) => (
                          <TableRow
                            key={skill.id}
                            className={`cursor-pointer hover:bg-muted/50 ${selectedA2aSkillId === skill.id ? "bg-muted" : ""}`}
                            onClick={() => onA2aSkillSelect(skill)}
                          >
                            <TableCell className="font-medium">{skill.name}</TableCell>
                            <TableCell>{skill.description || "-"}</TableCell>
                          </TableRow>
                        ))}
                      </TableBody>
                    </Table>
                  </div>
                )}
              </div>
            </div>
          </>
        ) : (connectionType === "mcp" && mcpTools.length === 0) ||
          (connectionType === "a2a" && a2aSkills.length === 0) ? (
          <div className="flex items-center justify-center py-8 h-full">
            <span className="text-muted-foreground">{noItemsMessage}</span>
          </div>
        ) : connectionType === "mcp" ? (
          <div className="overflow-y-auto max-h-[400px]">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Name</TableHead>
                  <TableHead>Description</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {mcpTools.map((tool) => (
                  <TableRow
                    key={tool.name}
                    className={`cursor-pointer hover:bg-muted/50 ${selectedMcpToolName === tool.name ? "bg-muted" : ""}`}
                    onClick={() => onMcpToolSelect(tool)}
                  >
                    <TableCell className="font-medium">{tool.name}</TableCell>
                    <TableCell>{tool.description}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        ) : null}
      </CardContent>
    </Card>
  );
}
