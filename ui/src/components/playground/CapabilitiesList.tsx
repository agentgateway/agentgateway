"use client";

import { Tool as McpTool } from "@modelcontextprotocol/sdk/types.js";
import type { AgentSkill } from "@a2a-js/sdk";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Loader2 } from "lucide-react";

interface CapabilitiesListProps {
  connectionType: "mcp" | "a2a" | null;
  isLoading: boolean;
  mcpTools: McpTool[];
  a2aSkills: AgentSkill[];
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
    <Card className="flex flex-col" data-cy="capabilities-list">
      <CardHeader>
        <CardTitle>Available {title}</CardTitle>
        <CardDescription>{description}</CardDescription>
      </CardHeader>
      <CardContent className="flex-grow">
        {isLoading ? (
          <div className="flex items-center justify-center py-8 h-full">
            <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
            <span className="ml-3 text-muted-foreground">{loadingMessage}</span>
          </div>
        ) : (connectionType === "mcp" && mcpTools.length === 0) ||
          (connectionType === "a2a" && a2aSkills.length === 0) ? (
          <div className="flex items-center justify-center py-8 h-full">
            <span className="text-muted-foreground">{noItemsMessage}</span>
          </div>
        ) : (
          <div className="overflow-y-auto max-h-[400px]">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Name</TableHead>
                  <TableHead>Description</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {connectionType === "mcp" &&
                  mcpTools.map((tool) => (
                    <TableRow
                      key={tool.name}
                      className={`cursor-pointer hover:bg-muted/50 ${selectedMcpToolName === tool.name ? "bg-muted" : ""}`}
                      onClick={() => onMcpToolSelect(tool)}
                      data-cy={`tool-row-${tool.name}`}
                    >
                      <TableCell className="font-medium">{tool.name}</TableCell>
                      <TableCell>{tool.description}</TableCell>
                    </TableRow>
                  ))}
                {connectionType === "a2a" &&
                  a2aSkills.map((skill) => (
                    <TableRow
                      key={skill.id}
                      className={`cursor-pointer hover:bg-muted/50 ${selectedA2aSkillId === skill.id ? "bg-muted" : ""}`}
                      onClick={() => onA2aSkillSelect(skill)}
                      data-cy={`skill-row-${skill.id}`}
                    >
                      <TableCell className="font-medium">{skill.name}</TableCell>
                      <TableCell>{skill.description || "-"}</TableCell>
                    </TableRow>
                  ))}
              </TableBody>
            </Table>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
