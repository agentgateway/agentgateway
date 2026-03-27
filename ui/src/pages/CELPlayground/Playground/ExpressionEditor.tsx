import styled from "@emotion/styled";
import { Button, Card, Space } from "antd";
import { PlayCircle, RotateCcw } from "lucide-react";
import { type MutableRefObject } from "react";
import { type Example } from "./types";
import { MonacoEditorComponent } from "./MonacoEditorComponent";

const EditorCard = styled(Card)`
  .ant-card-body {
    padding: 0;
  }
`;

const EditorHeader = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--spacing-md) var(--spacing-lg);
  border-bottom: 1px solid var(--color-border-secondary);
  background: var(--color-bg-container);
`;

const EditorContent = styled.div`
  padding: var(--spacing-lg);
`;

interface ExpressionEditorProps {
  expression: string;
  examples: Example[];
  loading: boolean;
  editorTheme: string;
  evaluateRef: MutableRefObject<() => Promise<void>>;
  onExpressionChange: (value: string) => void;
  onEvaluate: () => void;
  onReset: () => void;
}

export const ExpressionEditor = ({
  expression,
  examples,
  loading,
  editorTheme,
  evaluateRef,
  onExpressionChange,
  onEvaluate,
  onReset,
}: ExpressionEditorProps) => {
  return (
    <EditorCard>
      <EditorHeader>
        <div
          style={{
            display: "flex",
            justifyContent: "space-between",
            alignItems: "center",
            width: "100%",
          }}
        >
          <strong>CEL Expression</strong>
          <Space>
            <Button
              onClick={onEvaluate}
              disabled={loading}
              icon={<PlayCircle size={14} />}
              type="primary"
            >
              Evaluate
            </Button>
            <Button icon={<RotateCcw size={14} />} onClick={onReset}>
              Reset
            </Button>
          </Space>
        </div>
      </EditorHeader>
      <EditorContent>
        <MonacoEditorComponent
          value={expression}
          onChange={(v) => onExpressionChange(v ?? "")}
          language="javascript"
          height="200px"
          theme={editorTheme}
          onEvaluate={() => evaluateRef.current()}
        />
        <div
          style={{
            display: "flex",
            gap: "8px",
            marginTop: "12px",
            flexWrap: "wrap",
          }}
        >
          {examples.map((ex, idx) => (
            <button
              key={idx}
              type="button"
              onClick={() => onExpressionChange(ex.expr)}
              style={{
                fontSize: "12px",
                padding: "4px 8px",
                borderRadius: "4px",
                background: "var(--color-bg-hover)",
                border: "1px solid var(--color-border-secondary)",
                cursor: "pointer",
              }}
              title={ex.expr}
            >
              {ex.name}
            </button>
          ))}
        </div>
      </EditorContent>
    </EditorCard>
  );
};
