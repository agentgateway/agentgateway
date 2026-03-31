import styled from "@emotion/styled";
import { Card } from "antd";
import { type ExpressionTemplate } from "./types";

const TemplateList = styled.div`
  display: flex;
  flex-direction: column;
  gap: var(--spacing-sm);
`;

const TemplateItem = styled.div`
  padding: var(--spacing-sm);
  border: 1px solid var(--color-border-secondary);
  border-radius: var(--border-radius-base);
  cursor: pointer;
  transition: all var(--transition-base) var(--transition-timing);

  &:hover {
    border-color: var(--color-primary);
    background: var(--color-bg-hover);
  }
`;

interface TemplatesPanelProps {
  templates: ExpressionTemplate[];
  onLoadTemplate: (template: ExpressionTemplate) => void;
}

export const TemplatesPanel = ({
  templates,
  onLoadTemplate,
}: TemplatesPanelProps) => {
  return (
    <Card title="Expression Templates">
      <TemplateList>
        {templates.map((template, index) => (
          <TemplateItem 
            data-testid={template.name.replace(/[\s_]+/g, '-').toLowerCase()} 
            key={index} 
            onClick={() => onLoadTemplate(template)}
          >
            <div style={{ fontWeight: 600, marginBottom: 4 }}>
              {template.name}
            </div>
            <div style={{ fontSize: "12px", color: "#999" }}>
              {template.description}
            </div>
            <div
              style={{
                fontSize: "11px",
                fontFamily: "Monaco, monospace",
                marginTop: 8,
                color: "var(--color-primary)",
              }}
            >
              {template.expression.length > 50
                ? template.expression.substring(0, 50) + "..."
                : template.expression}
            </div>
          </TemplateItem>
        ))}
      </TemplateList>
    </Card>
  );
};
