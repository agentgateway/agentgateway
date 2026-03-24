import { type OnMount } from "@monaco-editor/react";
import { configureMonacoYaml } from "monaco-yaml";
import { useCallback } from "react";
import { MonacoEditorWithSettings } from "../../components/MonacoEditor";
import { assetUrl } from "../../utils/assetUrl";

interface MonacoEditorProps {
  value: string;
  onChange: (value: string | undefined) => void;
  language: string;
  height: string;
  theme: string;
  options?: any;
  onEvaluate?: () => void;
}

export const MonacoEditorComponent = ({
  value,
  onChange,
  language,
  height,
  theme,
  options = {},
  onEvaluate,
}: MonacoEditorProps) => {
  const handleEditorMount: OnMount = useCallback(
    async (editor, monaco) => {
      if (onEvaluate) {
        editor.addCommand(
          monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter,
          onEvaluate,
        );
      }
      // Mark container so Vimium recognizes the editor as a text input
      const domNode = editor.getDomNode();
      if (domNode) {
        domNode.setAttribute("role", "textbox");
        domNode.setAttribute("aria-multiline", "true");
      }

      // Add CEL types for JavaScript language (used for CEL expressions)
      if (language === "javascript") {
        try {
          // Fetch CEL type definitions
          const response = await fetch(assetUrl("/cel.d.ts"));
          if (response.ok) {
            let celTypes = await response.text();

            // Remove 'export' keyword to make ExecutorSerde globally available
            celTypes = celTypes.replace(/export interface/g, "interface");

            // Add CEL type definitions and context variables
            // CEL variables are available globally in the expression
            monaco.languages.typescript.javascriptDefaults.addExtraLib(
              celTypes,
              "file:///cel-types.d.ts",
            );
          }
        } catch (error) {
          console.error("Failed to load CEL types:", error);
        }
      }

      // Add CEL schema for YAML language (used for input data)
      if (language === "yaml") {
        try {
          const response = await fetch(assetUrl("/cel-schema.json"));
          if (response.ok) {
            const schema = await response.json();

            // Configure YAML language with the CEL schema
            // The schema includes default values which serve as examples
            configureMonacoYaml(monaco, {
              enableSchemaRequest: true,
              hover: true,
              completion: true,
              validate: true,
              format: true,
              schemas: [
                {
                  uri: "http://agentgateway/cel-schema.json",
                  fileMatch: ["*"],
                  schema: schema,
                },
              ],
            });
          }
        } catch (error) {
          console.error("Failed to load CEL schema:", error);
        }
      }
    },
    [onEvaluate, language],
  );

  return (
    <MonacoEditorWithSettings
      height={height}
      language={language}
      theme={theme === "vs-dark" ? "dark" : "light"}
      value={value}
      onChange={onChange}
      options={{
        minimap: { enabled: false },
        lineNumbers: "off",
        wordWrap: "on",
        ...options,
      }}
      onMount={handleEditorMount}
    />
  );
};
