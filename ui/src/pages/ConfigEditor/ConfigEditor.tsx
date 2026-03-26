import styled from "@emotion/styled";
import { type OnMount } from "@monaco-editor/react";
import { Button, Select, Space, Spin } from "antd";
import * as yaml from "js-yaml";
import type * as monacoEditor from "monaco-editor";
import { useCallback, useEffect, useRef, useState } from "react";
import toast from "react-hot-toast";
import { fetchConfig, updateConfig } from "../../api";
import type { LocalConfig } from "../../api/types";
import { MonacoEditorWithSettings } from "../../components/MonacoEditor";
import { useTheme } from "../../contexts/ThemeContext";
import { assetUrl } from "../../utils/assetUrl";

type ConfigFormat = "json" | "yaml";

const STORAGE_KEY = "agentgateway-config-format";

interface ConfigEditorProps {
  onClose: () => void;
}

const EditorContainer = styled.div`
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
`;

const EditorWrapper = styled.div`
  flex: 1;
  border: 1px solid var(--color-border-base);
  border-radius: var(--border-radius-md);
  overflow: visible;
  position: relative;

  .monaco-editor {
    padding: var(--spacing-sm);
  }
`;

const LoadingContainer = styled.div`
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
`;

const ActionBar = styled.div`
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: var(--spacing-md);
  border-bottom: 1px solid var(--color-border-secondary);
  background: var(--color-bg-container);
`;

const InfoText = styled.div`
  color: var(--color-text-secondary);
  font-size: 13px;
`;

export function ConfigEditor({ onClose }: ConfigEditorProps) {
  const { theme } = useTheme();
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [isLoading, setIsLoading] = useState(true);
  const [hasChanges, setHasChanges] = useState(false);
  const [configValue, setConfigValue] = useState<string>("");
  const [format, setFormat] = useState<ConfigFormat>(() => {
    const stored = localStorage.getItem(STORAGE_KEY);
    return (stored === "yaml" || stored === "json") ? stored : "yaml";
  });
  const editorRef = useRef<monacoEditor.editor.IStandaloneCodeEditor | null>(null);
  const originalConfigRef = useRef<LocalConfig | null>(null);
  const originalValueRef = useRef<string>("");

  const convertToFormat = useCallback(
    (config: LocalConfig, targetFormat: ConfigFormat): string => {
      if (targetFormat === "yaml") {
        return yaml.dump(config, { indent: 2, lineWidth: 120 });
      }
      return JSON.stringify(config, null, 2);
    },
    []
  );

  useEffect(() => {
    async function loadConfig() {
      try {
        const config = await fetchConfig();
        originalConfigRef.current = config;
        const configStr = convertToFormat(config, format);
        setConfigValue(configStr);
        originalValueRef.current = configStr;
      } catch (_e) {
        console.error("Failed to load config:", _e);
        toast.error("Failed to load configuration");
        const emptyValue = format === "yaml" ? "" : "{}";
        setConfigValue(emptyValue);
        originalValueRef.current = emptyValue;
      } finally {
        setIsLoading(false);
      }
    }
    loadConfig();
  }, [format, convertToFormat]);

  const handleEditorDidMount: OnMount = useCallback(
    async (editor, monaco) => {
      editorRef.current = editor;

      try {
        const response = await fetch(assetUrl("/config-schema.json"));
        if (!response.ok) {
          throw new Error(`Failed to fetch config-schema.json: ${response.statusText}`);
        }
        const schema = await response.json();
        
        // Configure JSON schema
        monaco.languages.json.jsonDefaults.setDiagnosticsOptions({
          validate: true,
          allowComments: false,
          schemas: [
            {
              uri: "http://agentgateway/config-schema.json",
              fileMatch: ["*"],
              schema: schema,
            },
          ],
          enableSchemaRequest: true,
        });

        // Configure YAML schema (if monaco-yaml is available)
        const yamlWorker = (monaco.languages as any).yaml?.yamlDefaults;
        if (yamlWorker) {
          yamlWorker.setDiagnosticsOptions({
            validate: true,
            schemas: [
              {
                uri: "http://agentgateway/config-schema.json",
                fileMatch: ["*"],
                schema: schema,
              },
            ],
          });
        }
      } catch (_e) {
        console.error("Failed to load config schema:", _e);
        toast.error("Failed to load configuration schema");
      }

      editor.onDidChangeModelContent(() => {
        const currentValue = editor.getValue();
        const hasChanged = currentValue !== originalValueRef.current;
        setHasChanges(hasChanged);
      });

      setTimeout(() => {
        editor.getAction("editor.action.formatDocument")?.run();
      }, 300);
    },
    []
  );

  const handleSave = async () => {
    if (!editorRef.current) return;

    setIsSubmitting(true);

    try {
      const value = editorRef.current.getValue();
      const model = editorRef.current.getModel();

      let configObject: LocalConfig;
      try {
        if (format === "yaml") {
          configObject = yaml.load(value) as LocalConfig;
        } else {
          configObject = JSON.parse(value);
        }
      } catch {
        toast.error(`Invalid ${format.toUpperCase()} format`);
        setIsSubmitting(false);
        return;
      }

      // Check for validation markers (schema errors/warnings)
      if (model && typeof window !== "undefined" && (window as any).monaco) {
        const monaco = (window as any).monaco;
        const markers = monaco.editor.getModelMarkers({ resource: model.uri });
        
        // Filter for errors and warnings (severity 8 = error, 4 = warning)
        const issues = markers.filter((m: any) => m.severity === 8 || m.severity === 4);
        
        if (issues.length > 0) {
          const errorCount = issues.filter((m: any) => m.severity === 8).length;
          const warningCount = issues.filter((m: any) => m.severity === 4).length;
          
          let message = "The configuration has validation issues:\n\n";
          if (errorCount > 0) {
            message += `• ${errorCount} error${errorCount !== 1 ? 's' : ''}\n`;
          }
          if (warningCount > 0) {
            message += `• ${warningCount} warning${warningCount !== 1 ? 's' : ''}\n`;
          }
          message += "\nThis may include unknown properties or schema violations.\n\nDo you want to save anyway?";
          
          const confirmed = window.confirm(message);
          if (!confirmed) {
            setIsSubmitting(false);
            return;
          }
        }
      }

      await updateConfig(configObject);
      toast.success("Configuration saved successfully");
      originalConfigRef.current = configObject;

      // Format the editor after saving
      setTimeout(() => {
        if (editorRef.current) {
          if (format === "yaml") {
            try {
              const formatted = yaml.dump(configObject, { indent: 2, lineWidth: 120 });
              editorRef.current.setValue(formatted);
              originalValueRef.current = formatted;
              setHasChanges(false);
            } catch (error) {
              console.error("Failed to format after save:", error);
              // If formatting fails, just keep the original value
              originalValueRef.current = value;
              setHasChanges(false);
            }
          } else {
            editorRef.current.getAction("editor.action.formatDocument")?.run();
            // Update original value after JSON formatting
            setTimeout(() => {
              if (editorRef.current) {
                originalValueRef.current = editorRef.current.getValue();
                setHasChanges(false);
              }
            }, 100);
          }
        }
      }, 100);
    } catch (error: any) {
      console.error("Failed to save config:", error);
      toast.error(error.message || "Failed to save configuration");
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleFormatChange = (newFormat: ConfigFormat) => {
    if (newFormat === format) return;

    if (hasChanges) {
      const confirmed = window.confirm(
        "You have unsaved changes. Switching format will discard these changes. Continue?"
      );
      if (!confirmed) return;
    }

    setFormat(newFormat);
    localStorage.setItem(STORAGE_KEY, newFormat);
    
    // Reload config in new format
    if (originalConfigRef.current) {
      const newValue = convertToFormat(originalConfigRef.current, newFormat);
      setConfigValue(newValue);
      originalValueRef.current = newValue;
      setHasChanges(false);
      
      // Format after switching
      setTimeout(() => {
        if (editorRef.current) {
          editorRef.current.getAction("editor.action.formatDocument")?.run();
        }
      }, 100);
    }
  };

  const handleFormat = () => {
    if (!editorRef.current) return;

    if (format === "yaml") {
      // Manually format YAML since Monaco might not have built-in YAML formatting
      try {
        const currentValue = editorRef.current.getValue();
        const parsed = yaml.load(currentValue);
        const formatted = yaml.dump(parsed, { indent: 2, lineWidth: 120 });
        editorRef.current.setValue(formatted);
        toast.success("YAML formatted successfully");
      } catch (error) {
        console.error("Failed to format YAML:", error);
        toast.error("Invalid YAML - cannot format");
      }
    } else {
      // Use Monaco's built-in JSON formatter
      editorRef.current.getAction("editor.action.formatDocument")?.run();
    }
  };

  return (
    <EditorContainer>
      <ActionBar>
        <Space>
          <Select
            value={format}
            onChange={handleFormatChange}
            disabled={isLoading}
            style={{ width: 100 }}
            options={[
              { label: "JSON", value: "json" },
              { label: "YAML", value: "yaml" },
            ]}
          />
          <Button onClick={handleFormat} disabled={isLoading}>
            Format
          </Button>
          <Button onClick={onClose} disabled={isSubmitting}>
            Cancel
          </Button>
          <InfoText>
            {hasChanges && "⚠️ You have unsaved changes"}
          </InfoText>
        </Space>
        <Button
          type="primary"
          onClick={handleSave}
          loading={isSubmitting}
          disabled={!hasChanges || isLoading}
        >
          Save Changes
        </Button>
      </ActionBar>

      <EditorWrapper>
        {isLoading ? (
          <LoadingContainer>
            <Spin size="large" tip="Loading configuration..." />
          </LoadingContainer>
        ) : (
          <MonacoEditorWithSettings
            height="100%"
            language={format === "yaml" ? "yaml" : "json"}
            value={configValue}
            theme={theme}
            onMount={handleEditorDidMount}
            onSave={handleSave}
            onQuit={onClose}
            downloadFileName={`agentgateway-config.${format}`}
            options={{
              minimap: { enabled: true },
              lineNumbers: "on",
              wordWrap: "on",
              fontSize: 14,
              tabSize: 2,
              formatOnPaste: true,
              formatOnType: true,
              fixedOverflowWidgets: true,
            }}
          />
        )}
      </EditorWrapper>
    </EditorContainer>
  );
}
