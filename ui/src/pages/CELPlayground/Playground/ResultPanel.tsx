import styled from "@emotion/styled";
import { Card } from "antd";
import { ChevronDown, ChevronUp } from "lucide-react";
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

const ExpandButton = styled.button`
  position: absolute;
  bottom: 0;
  left: 50%;
  transform: translateX(-50%);
  background: transparent;
  border: none;
  padding: 2px 8px;
  cursor: pointer;
  display: flex;
  align-items: center;
  gap: 2px;
  font-size: 11px;
  color: var(--color-text-secondary);
  opacity: 0.7;
  transition: all 0.2s;

  &:hover {
    opacity: 1;
    color: var(--color-text-base);
  }

  svg {
    width: 12px;
    height: 12px;
  }
`;

interface ResultPanelProps {
  hasEvaluated: boolean;
  resultError: string | null;
  resultValue: unknown | null;
  resultExpanded: boolean;
  editorTheme: string;
  onToggleExpanded: () => void;
}

export const ResultPanel = ({
  hasEvaluated,
  resultError,
  resultValue,
  resultExpanded,
  editorTheme,
  onToggleExpanded,
}: ResultPanelProps) => {
  return (
    <EditorCard style={{ position: "relative" }}>
      <EditorHeader>
        <strong>Result</strong>
      </EditorHeader>
      <EditorContent>
        {!hasEvaluated ? (
          <div
            style={{
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              height: "70px",
              color: "var(--color-text-secondary)",
              fontSize: "14px",
            }}
          >
            Click "Evaluate" to see results
          </div>
        ) : resultError ? (
          <div
            style={{
              borderRadius: "6px",
              background: "var(--color-error-bg)",
              border: "1px solid var(--color-error-border)",
              padding: "12px",
              height: resultExpanded ? "300px" : "70px",
              overflow: "auto",
              transition: "height 0.2s ease",
            }}
          >
            <pre
              style={{
                fontSize: "13px",
                color: "var(--color-error)",
                whiteSpace: "pre",
                fontFamily: "Monaco, monospace",
                margin: 0,
              }}
            >
              {resultError}
            </pre>
          </div>
        ) : resultValue !== null ? (
          <MonacoEditorComponent
            value={JSON.stringify(resultValue, null, 2)}
            onChange={() => {}}
            language="json"
            height={resultExpanded ? "300px" : "70px"}
            theme={editorTheme}
            options={{
              readOnly: true,
              wordWrap: "off",
            }}
          />
        ) : null}
      </EditorContent>
      {hasEvaluated && (
        <ExpandButton type="button" onClick={onToggleExpanded}>
          {resultExpanded ? (
            <>
              <ChevronUp size={14} />
              Collapse
            </>
          ) : (
            <>
              <ChevronDown size={14} />
              Expand
            </>
          )}
        </ExpandButton>
      )}
    </EditorCard>
  );
};
