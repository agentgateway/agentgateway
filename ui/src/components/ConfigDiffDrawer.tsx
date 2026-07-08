import "../monacoWorkers";
import { DiffEditor } from "@monaco-editor/react";
import { FileText, Save } from "lucide-react";
import { useState } from "react";
import { configureConfigYamlMonaco } from "../configMonaco";
import { cloneConfig } from "../config";
import { toYamlText } from "../policies/policyUtils";
import type { GatewayConfig } from "../types";
import { Drawer } from "./Primitives";

const configTopLevelOrder = [
  "config",
  "binds",
  "frontendPolicies",
  "policies",
  "workloads",
  "services",
  "backends",
  "routeGroups",
  "gateways",
  "routes",
  "llm",
  "mcp",
  "ui",
];

export function ConfigDiffDrawer(props: {
  title: string;
  original: string;
  modified: string;
  saving?: boolean;
  onClose: () => void;
  onSave?: () => void;
}) {
  return (
    <Drawer
      title={props.title}
      variant="nested"
      onClose={props.onClose}
      footer={
        <div className="button-row">
          <button className="button" type="button" onClick={props.onClose}>
            Close
          </button>
          {props.onSave ? (
            <button
              className="button primary"
              type="button"
              disabled={props.saving}
              onClick={props.onSave}
            >
              <Save size={16} />
              Save
            </button>
          ) : null}
        </div>
      }
    >
      <div className="editor-wrap config-diff-editor">
        <DiffEditor
          beforeMount={configureConfigYamlMonaco}
          language="yaml"
          original={props.original}
          modified={props.modified}
          theme={
            document.documentElement.dataset.theme === "dark"
              ? "vs-dark"
              : "light"
          }
          options={{
            automaticLayout: true,
            copyWithSyntaxHighlighting: false,
            fontSize: 13,
            minimap: { enabled: false },
            originalEditable: false,
            readOnly: true,
            renderSideBySide: true,
            hideUnchangedRegions: {
              enabled: true,
            },
            overviewRulerLanes: 0,
            scrollbar: {
              vertical: "hidden",
              verticalScrollbarSize: 0,
              alwaysConsumeMouseWheel: false,
            },
            scrollBeyondLastLine: false,
            wordWrap: "off",
          }}
        />
      </div>
    </Drawer>
  );
}

export function ConfigDiffSaveActions(props: {
  config?: GatewayConfig | null;
  diffTitle: string;
  saveLabel: string;
  saving?: boolean;
  saveDisabled?: boolean;
  diffDisabled?: boolean;
  onCancel?: () => void;
  onSave: () => void;
  beforeDiff?: () => boolean;
  applyDiff: (config: GatewayConfig) => void;
}) {
  const [diff, setDiff] = useState<{
    original: string;
    modified: string;
  } | null>(null);

  function viewDiff() {
    if (!props.config || props.diffDisabled || props.saveDisabled) return;
    if (props.beforeDiff && !props.beforeDiff()) return;
    const modified = cloneConfig(props.config);
    props.applyDiff(modified);
    setDiff(configDiffText(props.config, modified));
  }

  return (
    <>
      <div className="button-row">
        {props.onCancel ? (
          <button className="button" type="button" onClick={props.onCancel}>
            Cancel
          </button>
        ) : null}
        <button
          className="button"
          type="button"
          disabled={
            props.saving ||
            !props.config ||
            props.diffDisabled ||
            props.saveDisabled
          }
          onClick={viewDiff}
        >
          <FileText size={16} />
          View diff
        </button>
        <button
          className="button primary"
          type="button"
          disabled={props.saving || props.saveDisabled}
          onClick={props.onSave}
        >
          <Save size={16} />
          {props.saveLabel}
        </button>
      </div>
      {diff ? (
        <ConfigDiffDrawer
          title={props.diffTitle}
          original={diff.original}
          modified={diff.modified}
          saving={props.saving}
          onClose={() => setDiff(null)}
          onSave={props.onSave}
        />
      ) : null}
    </>
  );
}

export function configDiffText(
  original: GatewayConfig,
  modified: GatewayConfig,
) {
  return {
    original: toYamlText(original),
    modified: toYamlText(orderConfigForDiff(original, modified)),
  };
}

function orderConfigForDiff(original: GatewayConfig, modified: GatewayConfig) {
  const remaining = new Set(Object.keys(modified));
  const ordered: Record<string, unknown> = {};

  function add(key: string) {
    if (!remaining.has(key)) return;
    ordered[key] = (modified as Record<string, unknown>)[key];
    remaining.delete(key);
  }

  for (const key of Object.keys(original)) {
    add(key);
    if (key === "binds") {
      add("gateways");
      add("routes");
    }
  }
  for (const key of configTopLevelOrder) add(key);
  for (const key of Object.keys(modified)) add(key);

  return ordered as GatewayConfig;
}
