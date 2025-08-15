"use client";

import React, { useState } from "react";
import { Tool as McpTool } from "@modelcontextprotocol/sdk/types.js";
import type { AgentSkill } from "@a2a-js/sdk";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Checkbox } from "@/components/ui/checkbox";
import { Loader2, Send, BotMessageSquare, Wand2 } from "lucide-react";

function generateSampleJson(schema: any): any {
  if (!schema || typeof schema !== "object") {
    return null;
  }

  if (schema.type === "object" && schema.properties) {
    const sample: Record<string, any> = {};
    const required = schema.required || [];
    for (const key in schema.properties) {
      // Include required fields or just the first few optional fields for brevity
      if (required.includes(key) || Object.keys(sample).length < 3) {
        sample[key] = generateSampleJson(schema.properties[key]);
      }
    }
    // Ensure all required fields are present, even if not iterated above (e.g., schema without properties)
    required.forEach((key: string) => {
      if (!(key in sample) && schema.properties && schema.properties[key]) {
        sample[key] = generateSampleJson(schema.properties[key]);
      } else if (!(key in sample)) {
        sample[key] = null;
      }
    });
    return sample;
  } else if (schema.type === "array" && schema.items) {
    // Generate one sample item for arrays
    return [generateSampleJson(schema.items)];
  } else if (schema.type === "string") {
    return schema.enum ? schema.enum[0] : "string_value";
  } else if (schema.type === "number" || schema.type === "integer") {
    return 0;
  } else if (schema.type === "boolean") {
    return false;
  }
  return null;
}

interface ActionPanelProps {
  connectionType: "mcp" | "a2a" | null;
  mcpSelectedTool: McpTool | null;
  a2aSelectedSkill: AgentSkill | null;
  mcpParamValues: Record<string, any>;
  a2aMessage: string;
  isRequestRunning: boolean;
  onMcpParamChange: (key: string, value: any) => void;
  onA2aMessageChange: (message: string) => void;
  onRunMcpTool: () => void;
  onRunA2aSkill: () => void;
}

export function ActionPanel({
  connectionType,
  mcpSelectedTool,
  a2aSelectedSkill,
  mcpParamValues,
  a2aMessage,
  isRequestRunning,
  onMcpParamChange,
  onA2aMessageChange,
  onRunMcpTool,
  onRunA2aSkill,
}: ActionPanelProps) {
  const [jsonErrorKey, setJsonErrorKey] = useState<string | null>(null);
  const isMcp = connectionType === "mcp" && mcpSelectedTool;
  const isA2a = connectionType === "a2a" && a2aSelectedSkill;

  if (!isMcp && !isA2a) {
    return (
      <Card className="flex flex-col">
        <div className="flex-1 flex items-center justify-center p-4 text-muted-foreground bg-card h-full">
          Select one of the available {connectionType === "a2a" ? "skills" : "tools"}
        </div>
      </Card>
    );
  }

  const title = isA2a ? a2aSelectedSkill?.name : mcpSelectedTool?.name;
  const description = isA2a ? a2aSelectedSkill?.description : mcpSelectedTool?.description;

  return (
    <Card className="flex flex-col" data-cy="action-panel">
      <CardHeader>
        <CardTitle>{title}</CardTitle>
        <CardDescription>{description}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4 flex-grow flex flex-col">
        <div className="flex-grow space-y-4">
          {isMcp &&
            mcpSelectedTool &&
            Object.entries(mcpSelectedTool.inputSchema.properties || {}).map(
              ([key, prop]: [string, any]) => {
                const isRequired =
                  Array.isArray(mcpSelectedTool.inputSchema.required) &&
                  mcpSelectedTool.inputSchema.required.includes(key);

                // Render Textarea for object type properties
                if (prop.type === "object") {
                  const requiredProperties = prop.required || [];
                  // Attempt to stringify the current value for the Textarea
                  let textareaValue = "";
                  try {
                    // If the value is already a string (e.g., from invalid input), use it directly
                    if (typeof mcpParamValues[key] === "string") {
                      textareaValue = mcpParamValues[key];
                    } else if (mcpParamValues[key] !== undefined) {
                      textareaValue = JSON.stringify(mcpParamValues[key] || {}, null, 2);
                    }
                  } catch (error) {
                    console.error("Error stringifying object:", error);
                    textareaValue = "Error displaying object value";
                  }

                  const handleObjectChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
                    const newValue = e.target.value;
                    try {
                      // Allow empty string to clear the object
                      if (newValue.trim() === "") {
                        onMcpParamChange(key, undefined);
                        setJsonErrorKey(null);
                        return;
                      }
                      const parsed = JSON.parse(newValue);
                      onMcpParamChange(key, parsed);
                      setJsonErrorKey(null);
                    } catch (error) {
                      console.error("Invalid JSON input:", error);
                      onMcpParamChange(key, newValue);
                      setJsonErrorKey(key);
                    }
                  };

                  const handleGenerateSample = () => {
                    try {
                      const sample = generateSampleJson(prop);
                      if (sample !== null) {
                        // Update state directly with the parsed object
                        onMcpParamChange(key, sample);
                        // Clear any previous JSON errors
                        setJsonErrorKey(null);
                      } else {
                        console.error("Could not generate sample for schema:", prop);
                      }
                    } catch (error) {
                      console.error("Error generating sample JSON:", error);
                    }
                  };

                  return (
                    <div key={key} className="space-y-2">
                      <Label htmlFor={key}>
                        {key}
                        {isRequired && <span className="text-red-500 ml-1">*</span>}
                        <span className="text-xs text-muted-foreground ml-2">(JSON Object)</span>
                      </Label>
                      <div className="grid grid-cols-1 gap-4">
                        <div>
                          <Textarea
                            id={key}
                            placeholder={prop.description || `Enter JSON for ${key}`}
                            value={textareaValue}
                            onChange={handleObjectChange}
                            className={`font-mono text-sm h-40 overflow-auto ${jsonErrorKey === key ? "border-red-500 focus-visible:ring-red-500" : ""}`}
                            data-cy={`tool-parameter-${key}`}
                          />
                          {jsonErrorKey === key && (
                            <p className="text-xs text-red-500 mt-1">Invalid JSON format.</p>
                          )}
                        </div>
                        <div className="space-y-1">
                          <div className="flex justify-between items-center">
                            <Label className="text-xs font-semibold">Expected Schema:</Label>
                            <Button
                              variant="ghost"
                              size="sm"
                              onClick={handleGenerateSample}
                              title="Generate Sample Input"
                            >
                              <Wand2 className="h-3 w-3 mr-1" />
                              Sample
                            </Button>
                          </div>
                          {/* Display Required Properties */}
                          {requiredProperties.length > 0 && (
                            <div className="text-xs text-muted-foreground bg-muted/50 p-1 px-2 rounded-md border border-dashed">
                              <span className="font-medium">Required:</span>{" "}
                              {requiredProperties.join(", ")}
                            </div>
                          )}
                          <pre className="text-xs text-muted-foreground bg-muted p-2 rounded-md overflow-auto h-40">
                            {JSON.stringify(prop, null, 2)}
                          </pre>
                        </div>
                      </div>
                    </div>
                  );
                }

                // Render standard controls for other types
                return (
                  <div key={key} className="space-y-2">
                    <Label htmlFor={key}>
                      {key}
                      {isRequired && <span className="text-red-500 ml-1">*</span>}
                    </Label>
                    {prop.type === "boolean" ? (
                      <div className="flex items-center space-x-2">
                        <Checkbox
                          id={key}
                          checked={!!mcpParamValues[key]}
                          onCheckedChange={(checked: boolean) =>
                            onMcpParamChange(key, Boolean(checked))
                          }
                        />
                        <label htmlFor={key} className="text-sm text-muted-foreground">
                          {prop.description || "Toggle option"}
                        </label>
                      </div>
                    ) : prop.type === "number" || prop.type === "integer" ? (
                      // Input for numbers
                      <Input
                        type="number"
                        id={key}
                        placeholder={prop.description}
                        value={mcpParamValues[key] ?? ""}
                        onChange={(e) =>
                          onMcpParamChange(
                            key,
                            e.target.value === "" ? null : Number(e.target.value)
                          )
                        }
                        data-cy={`tool-parameter-${key}`}
                      />
                    ) : (
                      <Input
                        id={key}
                        placeholder={prop.description}
                        value={mcpParamValues[key] || ""}
                        onChange={(e) => onMcpParamChange(key, e.target.value)}
                        data-cy={`tool-parameter-${key}`}
                      />
                    )}
                  </div>
                );
              }
            )}

          {isA2a && a2aSelectedSkill && (
            <div className="space-y-2 flex flex-col flex-grow">
              <Label htmlFor="a2aMessage">Message</Label>
              <Textarea
                id="a2aMessage"
                placeholder={`Enter message for ${a2aSelectedSkill.name}...`}
                value={a2aMessage}
                onChange={(e) => onA2aMessageChange(e.target.value)}
                className="flex-grow"
                rows={5}
              />
            </div>
          )}
        </div>

        <Button
          onClick={isA2a ? onRunA2aSkill : onRunMcpTool}
          disabled={isRequestRunning || (isA2a && !a2aMessage.trim()) ? true : undefined}
          className="w-full mt-auto"
          data-cy="run-tool-button"
        >
          {isRequestRunning ? (
            <>
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              Running...
            </>
          ) : isA2a ? (
            <>
              <BotMessageSquare className="mr-2 h-4 w-4" />
              Send Task
            </>
          ) : (
            <>
              <Send className="mr-2 h-4 w-4" />
              Run Tool
            </>
          )}
        </Button>
      </CardContent>
    </Card>
  );
}
