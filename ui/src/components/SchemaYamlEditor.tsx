import "../monacoWorkers";
import Editor from "@monaco-editor/react";
import type * as Monaco from "monaco-editor";
import { useEffect, useRef } from "react";
import {
  configureConfigYamlMonaco,
  installYamlKeySuggest,
  registerConfigYamlSchema,
} from "../configMonaco";
import { configureConfigMonacoWorkers } from "../monacoWorkers";
import { parseYamlText } from "../policies/policyUtils";

configureConfigMonacoWorkers();

export function SchemaYamlEditor(props: {
  className?: string;
  invalid?: boolean;
  onChange: (value: string) => void;
  onSave?: () => void;
  path: string;
  schema: unknown;
  showLineNumbers?: boolean;
  value: string;
}) {
  const saveRef = useRef(props.onSave);
  const schemaRef = useRef(props.schema);
  const monacoRef = useRef<typeof Monaco | null>(null);

  useEffect(() => {
    saveRef.current = props.onSave;
  }, [props.onSave]);

  useEffect(() => {
    schemaRef.current = props.schema;
    if (monacoRef.current) {
      registerConfigYamlSchema(monacoRef.current, props.path, props.schema);
    }
  }, [props.path, props.schema]);

  function beforeMount(monaco: typeof Monaco) {
    configureConfigYamlMonaco(monaco);
    registerConfigYamlSchema(monaco, props.path, schemaRef.current);
  }

  function onMount(
    editor: Monaco.editor.IStandaloneCodeEditor,
    monaco: typeof Monaco,
  ) {
    monacoRef.current = monaco;
    if (import.meta.env.DEV) {
      window.__schemaYamlEditor = editor;
      window.__schemaYamlMonaco = monaco;
    }
    registerConfigYamlSchema(monaco, props.path, schemaRef.current);
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () =>
      saveRef.current?.(),
    );
    installYamlKeySuggest(editor);
  }

  return (
    <div className={editorClassName(props.className, props.invalid)}>
      <Editor
        beforeMount={beforeMount}
        language="yaml"
        path={props.path}
        theme={
          document.documentElement.dataset.theme === "dark"
            ? "vs-dark"
            : "light"
        }
        value={props.value}
        onChange={(nextValue) => props.onChange(nextValue ?? "")}
        onMount={onMount}
        options={{
          automaticLayout: true,
          copyWithSyntaxHighlighting: false,
          fontSize: 13,
          glyphMargin: false,
          lineDecorationsWidth: props.showLineNumbers === false ? 6 : undefined,
          lineNumbers: props.showLineNumbers === false ? "off" : "on",
          minimap: { enabled: false },
          quickSuggestions: { other: true, comments: false, strings: false },
          renderLineHighlight: "none",
          scrollBeyondLastLine: false,
          tabSize: 2,
          wordWrap: "off",
        }}
      />
    </div>
  );
}

export function parseSchemaYamlEditorValue(value: string) {
  return value.trim() ? parseYamlText(value) : {};
}

function editorClassName(
  className: string | undefined,
  invalid: boolean | undefined,
) {
  return [
    "editor-wrap",
    "schema-yaml-editor",
    className,
    invalid ? "invalid" : null,
  ]
    .filter(Boolean)
    .join(" ");
}

declare global {
  interface Window {
    __schemaYamlEditor?: Monaco.editor.IStandaloneCodeEditor;
    __schemaYamlMonaco?: typeof Monaco;
  }
}
