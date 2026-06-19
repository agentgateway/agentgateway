import "../monacoWorkers";
import Editor from "@monaco-editor/react";
import {
  celEditorOptions,
  celLanguage,
  configureCelMonaco,
} from "../celMonaco";
import { configureConfigYamlMonaco } from "../configMonaco";
import { configureConfigMonacoWorkers } from "../monacoWorkers";

configureConfigMonacoWorkers();

type MiniEditorProps = {
  className?: string;
  invalid?: boolean;
  language: "cel" | "json" | "yaml";
  onChange: (value: string) => void;
  onSubmit?: () => void;
  placeholder?: string;
  value: string;
};

export function MiniMonacoEditor(props: MiniEditorProps) {
  const monacoLanguage =
    props.language === "cel" ? celLanguage : props.language;
  return (
    <div className={miniEditorClassName(props.className, props.invalid)}>
      <Editor
        beforeMount={
          props.language === "cel"
            ? configureCelMonaco
            : props.language === "yaml"
              ? configureConfigYamlMonaco
              : undefined
        }
        language={monacoLanguage}
        theme={
          document.documentElement.dataset.theme === "dark"
            ? "vs-dark"
            : "light"
        }
        value={props.value}
        onChange={(value) => props.onChange(value ?? "")}
        onMount={(editor, monaco) => {
          editor.layout();
          if (props.onSubmit) {
            editor.addCommand(
              monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter,
              () => props.onSubmit?.(),
            );
          }
        }}
        options={{
          ...(props.language === "cel" ? celEditorOptions : {}),
          padding: { top: 4 },
          automaticLayout: true,
          copyWithSyntaxHighlighting: false,
          fontSize: 13,
          glyphMargin: false,
          lineDecorationsWidth: 10,
          lineNumbers: "off",
          folding: false,
          minimap: { enabled: false },
          quickSuggestions:
            props.language === "cel"
              ? { other: true, comments: false, strings: false }
              : false,
          renderLineHighlight: "none",
          scrollBeyondLastLine: false,
          scrollbar: {
            vertical: "hidden",
            verticalScrollbarSize: 0,
            alwaysConsumeMouseWheel: false,
          },
          tabSize: 2,
          wordWrap: "on",
          placeholder: props.placeholder,
        }}
      />
    </div>
  );
}

function miniEditorClassName(
  className: string | undefined,
  invalid: boolean | undefined,
) {
  return [
    "editor-wrap",
    "mini",
    "mini-monaco-editor",
    className,
    invalid ? "invalid" : null,
  ]
    .filter(Boolean)
    .join(" ");
}
