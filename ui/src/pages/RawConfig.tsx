import { useNavigate } from "@tanstack/react-router";
import { Clipboard, Download, Save, RotateCcw } from "lucide-react";
import { lazy, Suspense, useEffect, useMemo, useRef, useState } from "react";
import { validateGatewayConfig } from "../configValidation";
import { PageHeader, Panel, StatusBanner } from "../components/Primitives";
import { useConfigDumpMode, useGatewayConfig, useUpdateConfig } from "../hooks";
import { parseYamlText, toYamlText } from "../policies/policyUtils";
import type { GatewayConfig } from "../types";

const LazyRawConfigEditor = lazy(() =>
  import("../components/RawConfigEditor").then((module) => ({
    default: module.RawConfigEditor,
  })),
);

export function RawConfigPage() {
  const mode = useConfigDumpMode();
  const navigate = useNavigate();

  useEffect(() => {
    if (mode.data?.mode === "dump") void navigate({ to: "/" });
  }, [mode.data?.mode, navigate]);

  if (mode.isLoading) {
    return (
      <div className="page-stack">
        <StatusBanner state="loading" title="Detecting configuration mode" />
      </div>
    );
  }
  if (mode.data?.mode === "dump") return null;
  return <RawConfigEditorPage />;
}

function RawConfigEditorPage() {
  const config = useGatewayConfig();
  const update = useUpdateConfig();
  const initialText = useMemo(
    () => (config.data ? toYamlText(config.data) : ""),
    [config.data],
  );
  const [text, setText] = useState(initialText);
  const [error, setError] = useState<string | null>(null);
  const [savedText, setSavedText] = useState<string | null>(null);
  const previousInitialText = useRef(initialText);
  const dirty = text !== initialText;
  const showSaved = Boolean(
    savedText && text === savedText && initialText === savedText,
  );

  useEffect(() => {
    if (previousInitialText.current !== initialText) {
      if (!text || text === previousInitialText.current) setText(initialText);
      previousInitialText.current = initialText;
    }
  }, [initialText, text]);

  function updateText(next: string) {
    setText(next);
    setError(null);
    setSavedText(null);
    update.reset();
  }

  async function save() {
    setError(null);
    setSavedText(null);
    try {
      const parsed = parseYamlText(text);
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
        throw new Error("Configuration must be a YAML object.");
      }
      await validateGatewayConfig(parsed as GatewayConfig);
      await update.mutateAsync(() => parsed as GatewayConfig);
      setSavedText(text);
    } catch (err) {
      setError(
        err instanceof Error ? err.message : "Invalid configuration YAML.",
      );
    }
  }

  return (
    <div className="page-stack">
      <PageHeader
        title="Raw Configuration"
        description="Edit the full gateway YAML."
        actions={
          <div className="button-row">
            <button
              className="button"
              type="button"
              disabled={!text}
              onClick={() => void copyConfig(text)}
            >
              <Clipboard size={16} />
              Copy
            </button>
            <button
              className="button"
              type="button"
              disabled={!text}
              onClick={() => downloadConfig(text)}
            >
              <Download size={16} />
              Download
            </button>
            <button
              className="button"
              type="button"
              disabled={!dirty || update.isPending}
              onClick={() => updateText(initialText)}
            >
              <RotateCcw size={16} />
              Reset
            </button>
            <button
              className="button primary"
              type="button"
              disabled={!dirty || update.isPending}
              onClick={() => void save()}
            >
              <Save size={16} />
              Save
            </button>
          </div>
        }
      />

      {config.isError ? (
        <StatusBanner state="bad" title="Configuration API unavailable">
          {config.error.message}
        </StatusBanner>
      ) : null}
      {error ? (
        <StatusBanner state="bad" title="Save failed">
          {error}
        </StatusBanner>
      ) : null}
      {showSaved ? (
        <StatusBanner state="ok" title="Configuration saved" />
      ) : null}

      <Panel>
        <Suspense
          fallback={
            <div className="editor-wrap raw-config-editor loading-panel">
              Loading editor...
            </div>
          }
        >
          <LazyRawConfigEditor
            invalid={Boolean(error)}
            value={text}
            onChange={updateText}
            onSave={() => void save()}
          />
        </Suspense>
      </Panel>
    </div>
  );
}

async function copyConfig(value: string) {
  await navigator.clipboard.writeText(value);
}

function downloadConfig(value: string) {
  const blob = new Blob([value.endsWith("\n") ? value : `${value}\n`], {
    type: "application/yaml;charset=utf-8",
  });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = "agentgateway-config.yaml";
  anchor.click();
  URL.revokeObjectURL(url);
}
