import "../monacoWorkers";
import { DiffEditor } from "@monaco-editor/react";
import { FileText, Save } from "lucide-react";
import { type ReactNode, useState } from "react";
import { configureConfigYamlMonaco } from "../configMonaco";
import { cloneConfig } from "../config";
import {
  allowNextHybridFileWrite,
  useHybridFileWriteOverrideKeys,
  useRuntimeInfo,
} from "../hooks";
import { toYamlText } from "../policies/policyUtils";
import type { GatewayConfig } from "../types";
import { Drawer, Tooltip } from "./Primitives";

const hybridFileWriteMessage =
  "File configuration is read-only in hybrid mode. Copy this diff and update the configuration file directly.";
const hybridFileWriteOverrideMessage =
  "Override active. Click to write this change to the configuration file.";

export function ConfigSaveButton(props: {
  children: ReactNode;
  disabled?: boolean;
  allowHybridWrite?: boolean;
  hybridFileWriteMessage?: string;
  onClick: () => void;
}) {
  const runtime = useRuntimeInfo();
  const overrideKeysActive = useHybridFileWriteOverrideKeys();
  const fileWriteDisabled =
    runtime.data?.ui.configStoreMode === "hybrid" && !props.allowHybridWrite;
  const button = (
    <button
      className={`button primary${fileWriteDisabled ? " hybrid-write-disabled" : ""}${fileWriteDisabled && overrideKeysActive ? " hybrid-write-override" : ""}`}
      type="button"
      disabled={props.disabled}
      aria-disabled={fileWriteDisabled}
      onClick={(event) => {
        if (fileWriteDisabled) {
          if (!event.ctrlKey || !event.shiftKey) return;
          allowNextHybridFileWrite();
        }
        props.onClick();
      }}
    >
      {props.children}
    </button>
  );
  if (!fileWriteDisabled) return button;
  return (
    <Tooltip
      content={
        overrideKeysActive
          ? hybridFileWriteOverrideMessage
          : (props.hybridFileWriteMessage ?? hybridFileWriteMessage)
      }
    >
      {button}
    </Tooltip>
  );
}

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
  allowHybridWrite?: boolean;
  onClose: () => void;
  onSave?: () => void;
}) {
  const saveButton = props.onSave ? (
    <ConfigSaveButton
      disabled={props.saving}
      allowHybridWrite={props.allowHybridWrite}
      onClick={props.onSave}
    >
      <Save size={16} />
      Save
    </ConfigSaveButton>
  ) : null;
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
          {saveButton}
        </div>
      }
    >
      <div className="editor-wrap config-diff-editor">
        <DiffEditor
          beforeMount={configureConfigYamlMonaco}
          language="yaml"
          original={props.original}
          modified={props.modified}
          originalModelPath={`inmemory://config-diff/${encodeURIComponent(props.title)}/original.yaml`}
          modifiedModelPath={`inmemory://config-diff/${encodeURIComponent(props.title)}/modified.yaml`}
          keepCurrentOriginalModel
          keepCurrentModifiedModel
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
  resourceDiff?: { original: unknown; modified: unknown };
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
    if (
      (!props.config && !props.resourceDiff) ||
      props.diffDisabled ||
      props.saveDisabled
    )
      return;
    if (props.beforeDiff && !props.beforeDiff()) return;
    if (props.resourceDiff) {
      setDiff({
        original: toYamlText(props.resourceDiff.original),
        modified: toYamlText(props.resourceDiff.modified),
      });
      return;
    }
    if (!props.config) return;
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
            (!props.config && !props.resourceDiff) ||
            props.diffDisabled ||
            props.saveDisabled
          }
          onClick={viewDiff}
        >
          <FileText size={16} />
          View diff
        </button>
        <ConfigSaveButton
          disabled={props.saving || props.saveDisabled}
          allowHybridWrite={Boolean(props.resourceDiff)}
          onClick={props.onSave}
        >
          <Save size={16} />
          {props.saveLabel}
        </ConfigSaveButton>
      </div>
      {diff ? (
        <ConfigDiffDrawer
          title={props.diffTitle}
          original={diff.original}
          modified={diff.modified}
          saving={props.saving}
          allowHybridWrite={Boolean(props.resourceDiff)}
          onClose={() => setDiff(null)}
          onSave={() => {
            setDiff(null);
            props.onSave();
          }}
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
