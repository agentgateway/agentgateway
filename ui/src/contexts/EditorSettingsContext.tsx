import { createContext, useContext, useEffect, useState, type ReactNode } from "react";

const VIM_MODE_STORAGE_KEY = "monaco-editor-vim-mode";
const WORD_WRAP_STORAGE_KEY = "monaco-editor-word-wrap";

interface EditorSettings {
  vimEnabled: boolean;
  toggleVimMode: () => void;
  wordWrapEnabled: boolean;
  toggleWordWrap: () => void;
}

const EditorSettingsContext = createContext<EditorSettings | undefined>(undefined);

export function EditorSettingsProvider({ children }: { children: ReactNode }) {
  const [vimEnabled, setVimEnabled] = useState(() => {
    const saved = localStorage.getItem(VIM_MODE_STORAGE_KEY);
    return saved === "true";
  });

  const [wordWrapEnabled, setWordWrapEnabled] = useState(() => {
    const saved = localStorage.getItem(WORD_WRAP_STORAGE_KEY);
    return saved === "true";
  });

  const toggleVimMode = () => {
    setVimEnabled((prev) => {
      const newValue = !prev;
      localStorage.setItem(VIM_MODE_STORAGE_KEY, String(newValue));
      return newValue;
    });
  };

  const toggleWordWrap = () => {
    setWordWrapEnabled((prev) => {
      const newValue = !prev;
      localStorage.setItem(WORD_WRAP_STORAGE_KEY, String(newValue));
      return newValue;
    });
  };

  return (
    <EditorSettingsContext.Provider value={{ vimEnabled, toggleVimMode, wordWrapEnabled, toggleWordWrap }}>
      {children}
    </EditorSettingsContext.Provider>
  );
}

export function useEditorSettings() {
  const context = useContext(EditorSettingsContext);
  if (context === undefined) {
    throw new Error("useEditorSettings must be used within EditorSettingsProvider");
  }
  return context;
}
