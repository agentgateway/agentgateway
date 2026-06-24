import "../monacoWorkers";
import Editor from "@monaco-editor/react";
import type * as Monaco from "monaco-editor";
import { useEffect, useRef } from "react";
import { getGatewayConfigValidationErrors } from "../configValidation";
import { configureConfigMonacoWorkers } from "../monacoWorkers";
import {
  configureConfigYamlMonaco,
  installYamlKeySuggest,
  rawConfigModelPath,
} from "../configMonaco";
import { parseYamlText } from "../policies/policyUtils";
import type { GatewayConfig } from "../types";

interface RawConfigEditorProps {
  invalid: boolean;
  onChange: (value: string) => void;
  onSave: () => void;
  value: string;
}

configureConfigMonacoWorkers();

export function RawConfigEditor({
  invalid,
  onChange,
  onSave,
  value,
}: RawConfigEditorProps) {
  const saveRef = useRef(onSave);
  const editorRef = useRef<Monaco.editor.IStandaloneCodeEditor | null>(null);
  const monacoRef = useRef<typeof Monaco | null>(null);
  const validationRun = useRef(0);

  useEffect(() => {
    saveRef.current = onSave;
  }, [onSave]);

  useEffect(() => {
    const editor = editorRef.current;
    const monaco = monacoRef.current;
    const model = editor?.getModel();
    if (!editor || !monaco || !model) return;

    const run = ++validationRun.current;
    validateEditorText(value, model, monaco).then((markers) => {
      if (run !== validationRun.current) return;
      monaco.editor.setModelMarkers(model, "agentgateway-config", markers);
    });
  }, [value]);

  function mountEditor(
    editor: Monaco.editor.IStandaloneCodeEditor,
    monaco: typeof Monaco,
  ) {
    editorRef.current = editor;
    monacoRef.current = monaco;
    if (import.meta.env.DEV) {
      window.__rawConfigEditor = editor;
      window.__rawConfigMonaco = monaco;
    }
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () =>
      saveRef.current(),
    );
    installYamlKeySuggest(editor);
    const model = editor.getModel();
    if (model) {
      void validateEditorText(value, model, monaco).then((markers) => {
        monaco.editor.setModelMarkers(model, "agentgateway-config", markers);
      });
    }
  }

  return (
    <div
      className={
        invalid
          ? "editor-wrap raw-config-editor invalid"
          : "editor-wrap raw-config-editor"
      }
    >
      <Editor
        beforeMount={configureConfigYamlMonaco}
        language="yaml"
        path={rawConfigModelPath}
        theme={
          document.documentElement.dataset.theme === "dark"
            ? "vs-dark"
            : "light"
        }
        value={value}
        onChange={(nextValue) => onChange(nextValue ?? "")}
        onMount={mountEditor}
        options={{
          automaticLayout: true,
          copyWithSyntaxHighlighting: false,
          fontSize: 13,
          minimap: { enabled: false },
          quickSuggestions: { other: true, comments: false, strings: false },
          scrollBeyondLastLine: false,
          tabSize: 2,
          wordWrap: "off",
        }}
      />
    </div>
  );
}

async function validateEditorText(
  value: string,
  model: Monaco.editor.ITextModel,
  monaco: typeof Monaco,
): Promise<Monaco.editor.IMarkerData[]> {
  try {
    const parsed = parseYamlText(value);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return [
        marker(monaco, model, 1, 1, "Configuration must be a YAML object."),
      ];
    }
    const errors = await getGatewayConfigValidationErrors(
      parsed as GatewayConfig,
    );
    return errors.map((error) => {
      const params = error.params as { additionalProperty?: unknown };
      const key =
        typeof params.additionalProperty === "string"
          ? params.additionalProperty
          : undefined;
      const path = key
        ? [...error.instancePath.split("/").filter(Boolean), key]
        : error.instancePath.split("/").filter(Boolean);
      const position = findYamlPathPosition(model, path);
      return marker(
        monaco,
        model,
        position.lineNumber,
        position.column,
        error.message ?? "Invalid configuration value.",
      );
    });
  } catch (err) {
    const mark = (
      err as { mark?: { line?: number; column?: number }; message?: string }
    ).mark;
    return [
      marker(
        monaco,
        model,
        typeof mark?.line === "number" ? mark.line + 1 : 1,
        typeof mark?.column === "number" ? mark.column + 1 : 1,
        err instanceof Error ? err.message : "Invalid YAML.",
      ),
    ];
  }
}

function marker(
  monaco: typeof Monaco,
  model: Monaco.editor.ITextModel,
  lineNumber: number,
  column: number,
  message: string,
): Monaco.editor.IMarkerData {
  const safeLine = Math.min(Math.max(lineNumber, 1), model.getLineCount());
  const safeColumn = Math.min(
    Math.max(column, 1),
    model.getLineMaxColumn(safeLine),
  );
  return {
    severity: monaco.MarkerSeverity.Error,
    message,
    startLineNumber: safeLine,
    startColumn: safeColumn,
    endLineNumber: safeLine,
    endColumn: Math.max(safeColumn + 1, model.getLineMaxColumn(safeLine)),
  };
}

function findYamlPathPosition(model: Monaco.editor.ITextModel, path: string[]) {
  let key: string | undefined;
  for (let index = path.length - 1; index >= 0; index -= 1) {
    if (Number.isNaN(Number(path[index]))) {
      key = path[index];
      break;
    }
  }
  if (!key) return { lineNumber: 1, column: 1 };
  const pattern = new RegExp(`^\\s*${escapeRegExp(key)}\\s*:`);
  for (
    let lineNumber = 1;
    lineNumber <= model.getLineCount();
    lineNumber += 1
  ) {
    const line = model.getLineContent(lineNumber);
    const match = line.match(pattern);
    if (match) return { lineNumber, column: line.indexOf(key) + 1 };
  }
  return { lineNumber: 1, column: 1 };
}

function escapeRegExp(value: string) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

declare global {
  interface Window {
    __rawConfigEditor?: Monaco.editor.IStandaloneCodeEditor;
    __rawConfigMonaco?: typeof Monaco;
  }
}
