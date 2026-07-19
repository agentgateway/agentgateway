import { tr } from "../i18n";
import { useNavigate } from "@tanstack/react-router";
import { Clipboard, Download, FileText, Save, RotateCcw } from "lucide-react";
import { lazy, Suspense, useEffect, useMemo, useRef, useState } from "react";
import { validateGatewayConfig } from "../configValidation";
import { ConfigDiffDrawer } from "../components/ConfigDiffDrawer";
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
        <StatusBanner
          state="loading"
          title={tr("copy.detectingConfigurationMode")}
        />
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
  const [diffOpen, setDiffOpen] = useState(false);
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
    setDiffOpen(false);
    update.reset();
  }

  async function save() {
    setError(null);
    setSavedText(null);
    try {
      const parsed = parseYamlText(text);
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
        throw new Error(tr("copy.configurationMustBeAYamlObject"));
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
        title={tr("copy.rawConfiguration")}
        description={tr("copy.editTheFullGatewayYaml")}
        actions={
          <div className="button-row">
            <button
              className="button"
              type="button"
              disabled={!text}
              onClick={() => void copyConfig(text)}
            >
              <Clipboard size={16} />
              {tr("copy.copy")}
            </button>
            <button
              className="button"
              type="button"
              disabled={!text}
              onClick={() => downloadConfig(text)}
            >
              <Download size={16} />
              {tr("copy.download")}
            </button>
            <button
              className="button"
              type="button"
              disabled={!dirty || update.isPending}
              onClick={() => updateText(initialText)}
            >
              <RotateCcw size={16} />
              {tr("copy.reset")}
            </button>
            <button
              className="button"
              type="button"
              disabled={!dirty || update.isPending}
              onClick={() => setDiffOpen(true)}
            >
              <FileText size={16} />
              {tr("copy.viewDiff")}
            </button>
            <button
              className="button primary"
              type="button"
              disabled={!dirty || update.isPending}
              onClick={() => void save()}
            >
              <Save size={16} />
              {tr("copy.save")}
            </button>
          </div>
        }
      />

      {config.isError ? (
        <StatusBanner
          state="bad"
          title={tr("copy.configurationApiUnavailable")}
        >
          {config.error.message}
        </StatusBanner>
      ) : null}
      {error ? (
        <StatusBanner state="bad" title={tr("copy.saveFailed")}>
          {error}
        </StatusBanner>
      ) : null}
      {showSaved ? (
        <StatusBanner state="ok" title={tr("copy.configurationSaved")} />
      ) : null}

      <Panel>
        <Suspense
          fallback={
            <div className="editor-wrap raw-config-editor loading-panel">
              {tr("copy.loadingEditor")}
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
      {diffOpen ? (
        <ConfigDiffDrawer
          title={tr("copy.rawConfigurationDiff")}
          original={initialText}
          modified={text}
          saving={update.isPending}
          onClose={() => setDiffOpen(false)}
          onSave={() => void save()}
        />
      ) : null}
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
