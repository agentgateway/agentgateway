import { DeleteOutlined } from "@ant-design/icons";
import styled from "@emotion/styled";
import type { WrapIfAdditionalTemplateProps } from "@rjsf/utils";
import { Button, Input, Space } from "antd";

/**
 * Container for the additional property (key-value pair) with visual grouping
 */
const AdditionalPropertyContainer = styled.div`
  background: var(--color-bg-elevated);
  border: 1px solid var(--color-border-secondary);
  border-radius: var(--border-radius-lg);
  padding: var(--spacing-md);
  margin-bottom: var(--spacing-sm);
  transition: all var(--transition-base) var(--transition-timing);

  &:hover {
    border-color: var(--color-border-base);
    box-shadow: var(--shadow-sm);
  }
`;

const PropertyLabel = styled.div`
  font-size: var(--font-size-sm);
  font-weight: var(--font-weight-medium);
  color: var(--color-text-secondary);
  margin-bottom: var(--spacing-xs);
  text-transform: uppercase;
  letter-spacing: 0.03em;
`;

const KeyValueRow = styled.div`
  display: flex;
  gap: var(--spacing-sm);
  align-items: flex-start;
`;

const KeyInputWrapper = styled.div`
  flex: 0 0 200px;
  min-width: 150px;
`;

const ValueWrapper = styled.div`
  flex: 1;
  min-width: 0;
`;

const DeleteButtonWrapper = styled.div`
  flex: 0 0 auto;
  padding-top: 4px;
`;

/**
 * Custom WrapIfAdditionalTemplate for better object key-value editing.
 *
 * Improvements:
 * - Contextual placeholders instead of "newKey" / "newValue"
 * - Clear visual grouping showing key-value pairs belong together
 * - Better layout and styling
 */
export function WrapIfAdditionalTemplate(props: WrapIfAdditionalTemplateProps) {
  const {
    children,
    classNames,
    disabled,
    id,
    label,
    onDropPropertyClick,
    onKeyChange,
    readonly,
    required,
    schema,
  } = props;

  // If not an additional property, render children directly
  if (!onKeyChange) {
    return <div className={classNames}>{children}</div>;
  }

  // Get contextual placeholder based on parent context
  const getKeyPlaceholder = () => {
    const schemaTitle = schema.title?.toLowerCase() || "";
    const idLower = id.toLowerCase();

    if (schemaTitle.includes("header") || idLower.includes("header")) {
      return "header-name";
    }
    if (schemaTitle.includes("metadata") || schemaTitle.includes("label") || idLower.includes("label")) {
      return "key";
    }
    if (schemaTitle.includes("env") || schemaTitle.includes("environment") || idLower.includes("env")) {
      return "ENV_VAR_NAME";
    }
    if (schemaTitle.includes("param") || schemaTitle.includes("query") || idLower.includes("param")) {
      return "param-name";
    }
    if (idLower.includes("add") || idLower.includes("set")) {
      return "key";
    }

    return "";
  };

  const getValuePlaceholder = () => {
    const schemaTitle = schema.title?.toLowerCase() || "";
    const idLower = id.toLowerCase();

    if (schemaTitle.includes("header") || idLower.includes("header")) {
      return "header-value";
    }
    if (schemaTitle.includes("env") || schemaTitle.includes("environment") || idLower.includes("env")) {
      return "value";
    }

    return "";
  };

  return (
    <AdditionalPropertyContainer className={classNames}>
      <PropertyLabel>Key-Value Pair</PropertyLabel>
      <KeyValueRow>
        <KeyInputWrapper>
          <Input
            id={`${id}-key`}
            placeholder={getKeyPlaceholder()}
            value={label}
            onChange={(event) => onKeyChange(event.target.value)}
            disabled={disabled || readonly}
            required={required}
          />
        </KeyInputWrapper>
        <ValueWrapper>{children}</ValueWrapper>
        <DeleteButtonWrapper>
          <Button
            danger
            type="text"
            icon={<DeleteOutlined />}
            onClick={onDropPropertyClick(label)}
            disabled={disabled || readonly}
            title="Remove this key-value pair"
          />
        </DeleteButtonWrapper>
      </KeyValueRow>
    </AdditionalPropertyContainer>
  );
}
