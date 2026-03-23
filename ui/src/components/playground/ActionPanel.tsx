"use client";

import type { AgentSkill } from "@a2a-js/sdk";
import type { Tool as McpTool } from "@modelcontextprotocol/sdk/types.js";
import { Button, Card, Checkbox, Input } from "antd";
import { BotMessageSquare, Loader2, Send, Wand2 } from "lucide-react";
import React, { useState } from "react";

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
      <div className="flex items-center justify-center p-4 text-muted-foreground bg-card h-full">
        Select one of the available{" "}
        {connectionType === "a2a" ? "skills" : "tools"}
      </div>
    );
  }

  const toolName = isA2a ? a2aSelectedSkill?.name : mcpSelectedTool?.name;
  const description = isA2a
    ? a2aSelectedSkill?.description
    : mcpSelectedTool?.description;

  return (
    <>
      <div style={{ marginBottom: "1rem" }}>
        <div style={{ fontWeight: 600, marginBottom: "0.5rem" }}>{toolName}</div>
        <p style={{ margin: 0, color: "var(--color-text-secondary)", fontSize: "14px" }}>{description}</p>
      </div>
        <div className="space-y-4 flex-grow flex flex-col">
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
                        textareaValue = JSON.stringify(
                          mcpParamValues[key] || {},
                          null,
                          2,
                        );
                      }
                    } catch (error) {
                      console.error("Error stringifying object:", error);
                      textareaValue = "Error displaying object value";
                    }

                    const handleObjectChange = (
                      e: React.ChangeEvent<HTMLTextAreaElement>,
                    ) => {
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
                          console.error(
                            "Could not generate sample for schema:",
                            prop,
                          );
                        }
                      } catch (error) {
                        console.error("Error generating sample JSON:", error);
                      }
                    };

                    return (
                      <div key={key} className="space-y-2">
                        <label style={{ display: "block", fontWeight: 500 }}>
                          {key}
                          {isRequired && (
                            <span
                              style={{ color: "#ff4d4f", marginLeft: "4px" }}
                            >
                              *
                            </span>
                          )}
                          <span
                            style={{
                              fontSize: "12px",
                              color: "#666",
                              marginLeft: "8px",
                            }}
                          >
                            (JSON Object)
                          </span>
                        </label>
                        <div className="grid grid-cols-1 gap-4">
                          <div>
                            <Input.TextArea
                              value={textareaValue}
                              onChange={handleObjectChange}
                              style={{
                                fontFamily: "monospace",
                                fontSize: "13px",
                                minHeight: "120px",
                                borderColor:
                                  jsonErrorKey === key ? "#ff4d4f" : undefined,
                              }}
                            />
                            {jsonErrorKey === key && (
                              <p
                                style={{
                                  fontSize: "12px",
                                  color: "#ff4d4f",
                                  marginTop: "4px",
                                }}
                              >
                                Invalid JSON format.
                              </p>
                            )}
                          </div>
                          <div className="space-y-1">
                            <div className="flex justify-between items-center">
                              <label
                                style={{ fontSize: "12px", fontWeight: 500 }}
                              >
                                Expected Schema:
                              </label>
                              <Button
                                type="text"
                                size="small"
                                onClick={handleGenerateSample}
                                title="Generate Sample Input"
                              >
                                <Wand2 className="h-3 w-3 mr-1" />
                                Sample
                              </Button>
                            </div>
                            {/* Display Required Properties */}
                            {requiredProperties.length > 0 && (
                              <div
                                style={{
                                  fontSize: "12px",
                                  color: "#666",
                                  background: "#f5f5f5",
                                  padding: "4px 8px",
                                  borderRadius: "4px",
                                }}
                              >
                                <span style={{ fontWeight: 500 }}>
                                  Required:
                                </span>{" "}
                                {requiredProperties.join(", ")}
                              </div>
                            )}
                            <pre
                              style={{
                                fontSize: "12px",
                                color: "#666",
                                background: "#f0f0f0",
                                padding: "8px",
                                borderRadius: "4px",
                                overflow: "auto",
                                maxHeight: "120px",
                              }}
                            >
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
                      <label style={{ display: "block", fontWeight: 500 }}>
                        {key}
                        {isRequired && (
                          <span style={{ color: "#ff4d4f", marginLeft: "4px" }}>
                            *
                          </span>
                        )}
                      </label>
                      {prop.type === "boolean" ? (
                        <div className="flex items-center space-x-2">
                          <Checkbox
                            checked={!!mcpParamValues[key]}
                            onChange={(e) =>
                              onMcpParamChange(key, Boolean(e.target.checked))
                            }
                          />
                          <label style={{ fontSize: "14px", color: "#666" }}>
                            {prop.description || "Toggle option"}
                          </label>
                        </div>
                      ) : prop.type === "number" || prop.type === "integer" ? (
                        // Input for numbers
                        <Input
                          type="number"
                          placeholder={prop.description}
                          value={mcpParamValues[key] ?? ""}
                          onChange={(e) =>
                            onMcpParamChange(
                              key,
                              e.target.value === ""
                                ? null
                                : Number(e.target.value),
                            )
                          }
                        />
                      ) : (
                        <Input
                          placeholder={prop.description}
                          value={mcpParamValues[key] || ""}
                          onChange={(e) =>
                            onMcpParamChange(key, e.target.value)
                          }
                        />
                      )}
                    </div>
                  );
                },
              )}

            {isA2a && a2aSelectedSkill && (
              <div className="space-y-2 flex flex-col flex-grow">
                <label style={{ display: "block", fontWeight: 500 }}>
                  Message
                </label>
                <Input.TextArea
                  placeholder={`Enter message for ${a2aSelectedSkill.name}...`}
                  value={a2aMessage}
                  onChange={(e) => onA2aMessageChange(e.target.value)}
                  rows={5}
                />
              </div>
            )}
          </div>

          <Button
            onClick={isA2a ? onRunA2aSkill : onRunMcpTool}
            disabled={Boolean(
              isRequestRunning || (isA2a && !a2aMessage?.trim()),
            )}
            className="w-full mt-auto"
            type="primary"
            style={{ display: "flex", alignItems: "center", justifyContent: "center" }}
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
        </div>
    </>
  );
}
