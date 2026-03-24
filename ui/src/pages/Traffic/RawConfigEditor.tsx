import styled from "@emotion/styled";
import { type OnMount } from "@monaco-editor/react";
import { Button, Space, Spin } from "antd";
import { useCallback, useEffect, useRef, useState } from "react";
import toast from "react-hot-toast";
import type * as monacoEditor from "monaco-editor";
import type { LocalConfig } from "../../api/types";
import { fetchConfig, updateConfig } from "../../api";
import { useTheme } from "../../contexts/ThemeContext";
import { MonacoEditorWithSettings } from "../../components/MonacoEditor";
import { assetUrl } from "../../utils/assetUrl";

interface RawConfigEditorProps {
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

export function RawConfigEditor({ onClose }: RawConfigEditorProps) {
  const { theme } = useTheme();
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [isLoading, setIsLoading] = useState(true);
  const [hasChanges, setHasChanges] = useState(false);
  const [configValue, setConfigValue] = useState<string>("");
  const editorRef = useRef<monacoEditor.editor.IStandaloneCodeEditor | null>(null);

  // Load config on mount
  useEffect(() => {
    async function loadConfig() {
      try {
        const config = await fetchConfig();
        const configStr = JSON.stringify(config, null, 2);
        setConfigValue(configStr);
      } catch (error) {
        console.error("Failed to load config:", error);
        toast.error("Failed to load configuration");
        setConfigValue("{}");
      } finally {
        setIsLoading(false);
      }
    }
    loadConfig();
  }, []);

  const handleEditorDidMount: OnMount = useCallback(
    async (editor, monaco) => {
      editorRef.current = editor;

      // Load JSON schema for LocalConfig
      try {
        const response = await fetch(assetUrl("/config-schema.json"));
        if (!response.ok) {
          throw new Error(`Failed to fetch config-schema.json: ${response.statusText}`);
        }
        const schema = await response.json();
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
      } catch (error) {
        console.error("Failed to load config schema:", error);
        toast.error("Failed to load configuration schema");
      }

      // Set up change detection
      editor.onDidChangeModelContent(() => {
        setHasChanges(true);
      });

      // Format on mount
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

      // Parse the JSON config
      let configObject: LocalConfig;
      try {
        configObject = JSON.parse(value);
      } catch (error) {
        toast.error("Invalid JSON format");
        setIsSubmitting(false);
        return;
      }

      await updateConfig(configObject);
      toast.success("Configuration saved successfully");
      setHasChanges(false);
    } catch (error: any) {
      console.error("Failed to save config:", error);
      toast.error(error.message || "Failed to save configuration");
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleFormat = () => {
    if (editorRef.current) {
      editorRef.current.getAction("editor.action.formatDocument")?.run();
    }
  };

  return (
    <EditorContainer>
      <ActionBar>
        <Space>
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
            language="json"
            value={configValue}
            theme={theme}
            onMount={handleEditorDidMount}
            onSave={handleSave}
            onQuit={onClose}
            downloadFileName="agentgateway-config.json"
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
