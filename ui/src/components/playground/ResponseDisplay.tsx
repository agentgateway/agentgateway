"use client";

import { Card } from "antd";

interface ResponseDisplayProps {
  connectionType: "mcp" | "a2a" | null;
  mcpResponse: any;
  a2aResponse: any;
}

export function ResponseDisplay({
  connectionType,
  mcpResponse,
  a2aResponse,
}: ResponseDisplayProps) {
  const responseData = connectionType === "a2a" ? a2aResponse : mcpResponse;

  if (!responseData) {
    return null; // Don't render anything if there's no response
  }

  return (
    <Card title="Response">
      <pre
        style={{
          background: "#f5f5f5",
          padding: "1rem",
          borderRadius: "4px",
          overflow: "auto",
          maxHeight: "500px",
          fontSize: "13px",
          fontFamily: "monospace",
        }}
      >
        {JSON.stringify(responseData, null, 2)}
      </pre>
    </Card>
  );
}
