import styled from "@emotion/styled";
import { Editor, type OnMount } from "@monaco-editor/react";
import { Dropdown, type MenuProps } from "antd";
import { Settings } from "lucide-react";
import type * as monacoEditor from "monaco-editor";
import { initVimMode, type VimMode, VimMode as VimModeClass } from "monaco-vim";
import { useCallback, useEffect, useRef } from "react";
import toast from "react-hot-toast";
import { useEditorSettings } from "../../contexts/EditorSettingsContext";

interface MonacoEditorWithSettingsProps {
  value: string;
  onChange?: (value: string | undefined) => void;
  language: string;
  height?: string;
  theme: "light" | "dark";
  options?: monacoEditor.editor.IStandaloneEditorConstructionOptions;
  onMount?: OnMount;
  readOnly?: boolean;
  downloadFileName?: string;
  onSave?: () => void;
  onQuit?: () => void;
}

const EditorContainer = styled.div<{ height: string }>`
  position: relative;
  width: 100%;
  height: ${props => props.height};
  display: flex;
  flex-direction: column;
  background: transparent;
`;

const EditorWrapper = styled.div`
  flex: 1;
  position: relative;
  min-height: 0;
  background: transparent;
`;

const SettingsButton = styled.button`
  position: absolute;
  top: 8px;
  right: 8px;
  z-index: 10;
  background: var(--color-bg-elevated);
  border: 1px solid var(--color-border-base);
  border-radius: 4px;
  padding: 6px;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: all 0.2s;

  &:hover {
    background: var(--color-bg-hover);
    border-color: var(--color-primary);
  }

  svg {
    width: 16px;
    height: 16px;
    color: var(--color-text-secondary);
  }
`;

const VimStatusBar = styled.div`
  padding: 4px 8px;
  background: var(--color-bg-elevated);
  border-top: 1px solid var(--color-border-base);
  font-family: monospace;
  font-size: 12px;
  min-height: 24px;
`;

export function MonacoEditorWithSettings({
  value,
  onChange,
  language,
  height = "100%",
  theme,
  options = {},
  onMount,
  readOnly = false,
  downloadFileName,
  onSave,
  onQuit,
}: MonacoEditorWithSettingsProps) {
  const { vimEnabled, toggleVimMode, wordWrapEnabled, toggleWordWrap } = useEditorSettings();
  const editorRef = useRef<monacoEditor.editor.IStandaloneCodeEditor | null>(null);
  const vimModeRef = useRef<VimMode | null>(null);
  const statusNodeRef = useRef<HTMLDivElement | null>(null);
  const onSaveRef = useRef(onSave);
  const onQuitRef = useRef(onQuit);

  // Keep onSave and onQuit refs up to date
  useEffect(() => {
    onSaveRef.current = onSave;
  }, [onSave]);

  useEffect(() => {
    onQuitRef.current = onQuit;
  }, [onQuit]);

  // Enable/disable vim mode for this editor instance
  useEffect(() => {
    if (!editorRef.current) return;

    if (vimEnabled) {
      // Enable vim mode
      if (!vimModeRef.current && statusNodeRef.current) {
        vimModeRef.current = initVimMode(editorRef.current, statusNodeRef.current);

        // Set up vim save and quit commands using VimMode.Vim
        setTimeout(() => {
          try {
            const Vim = (VimModeClass as any).Vim;
            if (Vim && Vim.defineEx) {
              if (onSaveRef.current) {
                Vim.defineEx('write', 'w', () => {
                  onSaveRef.current?.();
                });
              }
              if (onQuitRef.current) {
                Vim.defineEx('quit', 'q', () => {
                  onQuitRef.current?.();
                });
              }
              if (onSaveRef.current && onQuitRef.current) {
                Vim.defineEx('wq', 'wq', () => {
                  onSaveRef.current?.();
                  onQuitRef.current?.();
                });
              }
            }
          } catch (e) {
            console.error('Failed to setup vim commands:', e);
          }
        }, 100);
      }
    } else {
      // Disable vim mode
      if (vimModeRef.current) {
        vimModeRef.current.dispose();
        vimModeRef.current = null;
      }
    }
  }, [vimEnabled]);

  const handleEditorMount: OnMount = useCallback(
    (editor, monaco) => {
      editorRef.current = editor;

      // Initialize vim mode if enabled
      if (vimEnabled && statusNodeRef.current && !vimModeRef.current) {
        vimModeRef.current = initVimMode(editor, statusNodeRef.current);

        // Set up vim save and quit commands using VimMode.Vim
        setTimeout(() => {
          try {
            const Vim = (VimModeClass as any).Vim;
            if (Vim && Vim.defineEx) {
              if (onSaveRef.current) {
                Vim.defineEx('write', 'w', () => {
                  onSaveRef.current?.();
                });
              }
              if (onQuitRef.current) {
                Vim.defineEx('quit', 'q', () => {
                  onQuitRef.current?.();
                });
              }
              if (onSaveRef.current && onQuitRef.current) {
                Vim.defineEx('wq', 'wq', () => {
                  onSaveRef.current?.();
                  onQuitRef.current?.();
                });
              }
            }
          } catch (e) {
            console.error('Failed to setup vim commands:', e);
          }
        }, 100);
      }

      // Call custom onMount if provided
      if (onMount) {
        onMount(editor, monaco);
      }
    },
    [onMount, vimEnabled]
  );

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      vimModeRef.current?.dispose();
    };
  }, []);

  const handleCopy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(value);
      toast.success("Copied to clipboard");
    } catch (error) {
      toast.error("Failed to copy to clipboard");
    }
  }, [value]);

  const handleDownload = useCallback(() => {
    try {
      const blob = new Blob([value], { type: "text/plain" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;

      // Determine file extension from language
      let extension = "txt";
      if (language === "json") extension = "json";
      else if (language === "yaml" || language === "yml") extension = "yaml";
      else if (language === "javascript" || language === "typescript") extension = language === "typescript" ? "ts" : "js";
      else if (language === "html") extension = "html";
      else if (language === "css") extension = "css";

      a.download = downloadFileName || `content.${extension}`;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      URL.revokeObjectURL(url);
      toast.success("Downloaded successfully");
    } catch (error) {
      toast.error("Failed to download file");
    }
  }, [value, language, downloadFileName]);

  const menuItems: MenuProps["items"] = [
    {
      key: "copy",
      label: "Copy to Clipboard",
      onClick: handleCopy,
    },
    {
      key: "download",
      label: "Download",
      onClick: handleDownload,
    },
    {
      type: "divider",
    },
    {
      key: "vim",
      label: vimEnabled ? "Disable Vim Mode" : "Enable Vim Mode",
      onClick: toggleVimMode,
    },
    {
      key: "wordwrap",
      label: wordWrapEnabled ? "Disable Word Wrap" : "Enable Word Wrap",
      onClick: toggleWordWrap,
    },
  ];

  return (
    <EditorContainer height={height}>
      <EditorWrapper>
        <Dropdown menu={{ items: menuItems }} placement="bottomRight" trigger={["click"]}>
          <SettingsButton type="button" aria-label="Editor settings">
            <Settings />
          </SettingsButton>
        </Dropdown>
        <Editor
          height="100%"
          language={language}
          value={value}
          onChange={onChange}
          theme={theme === "dark" ? "vs-dark" : "vs"}
          onMount={handleEditorMount}
          options={{
            ...options,
            readOnly,
            wordWrap: wordWrapEnabled ? "on" : "off",
          }}
        />
      </EditorWrapper>
      {vimEnabled && (
        <VimStatusBar ref={statusNodeRef as any} className="monaco-vim-status" />
      )}
    </EditorContainer>
  );
}
