import styled from "@emotion/styled";
import { Card, Select } from "antd";
import { type TemplateKey } from "./types";
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

interface ContextEditorProps {
  inputData: string;
  template: TemplateKey;
  editorTheme: string;
  onInputDataChange: (value: string | undefined) => void;
  onTemplateChange: (value: TemplateKey) => void;
}

export const ContextEditor = ({
  inputData,
  template,
  editorTheme,
  onInputDataChange,
  onTemplateChange,
}: ContextEditorProps) => {
  return (
    <EditorCard>
      <EditorHeader>
        <strong>Input Data (YAML)</strong>
        <Select
          value={template}
          onChange={(value) => onTemplateChange(value as TemplateKey)}
          style={{ width: 120 }}
        >
          <Select.Option value="empty">Empty</Select.Option>
          <Select.Option value="http">HTTP</Select.Option>
        </Select>
      </EditorHeader>
      <EditorContent>
        <MonacoEditorComponent
          value={inputData}
          onChange={onInputDataChange}
          language="yaml"
          height="400px"
          theme={editorTheme}
        />
      </EditorContent>
    </EditorCard>
  );
};
