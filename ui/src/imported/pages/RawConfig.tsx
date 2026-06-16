import { useState } from "react";
import { useGatewayConfig } from "../_adapters/hooks";
import { PageHeader, StatusBanner } from "../components/Primitives";
import yaml from "js-yaml";

export function RawConfigPage() {
  const config = useGatewayConfig();
  const [copied, setCopied] = useState(false);

  const yamlText = config.data ? yaml.dump(config.data, { indent: 2, lineWidth: -1 }) : "";

  function copyToClipboard() {
    void navigator.clipboard.writeText(yamlText).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    });
  }

  return (
    <div className="page-stack">
      <PageHeader
        title="Raw Configuration"
        description="Full read-only YAML view of the current gateway configuration."
        actions={
          <button className="button" type="button" onClick={copyToClipboard} disabled={!yamlText}>
            {copied ? "Copied!" : "Copy YAML"}
          </button>
        }
      />
      {config.isLoading ? (
        <StatusBanner state="loading" title="Loading configuration" />
      ) : config.isError ? (
        <StatusBanner state="bad" title="Failed to load configuration">
          {config.error?.message}
        </StatusBanner>
      ) : (
        <pre style={{
          background: "var(--color-surface, #1e1e1e)",
          color: "var(--color-text, #d4d4d4)",
          padding: "1.5rem",
          borderRadius: "0.5rem",
          overflow: "auto",
          fontSize: "0.85rem",
          lineHeight: 1.6,
          fontFamily: "monospace",
          whiteSpace: "pre",
          margin: 0,
        }}>
          {yamlText || "(empty configuration)"}
        </pre>
      )}
    </div>
  );
}
