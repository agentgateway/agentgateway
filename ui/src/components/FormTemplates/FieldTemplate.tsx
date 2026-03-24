import { InfoCircleOutlined } from "@ant-design/icons";
import styled from "@emotion/styled";
import type { FieldTemplateProps } from "@rjsf/utils";
import { Form, Tooltip, Typography } from "antd";
import { useContext } from "react";
import { HideLabelContext } from "./HideLabelContext";

const { Title } = Typography;

/**
 * Wrapper for object fields that renders label as a section header
 */
const ObjectFieldWrapper = styled.div`
  margin-bottom: 16px;
`;

const ObjectFieldHeader = styled.div`
  margin-bottom: 12px;
  display: flex;
  align-items: center;
  gap: 8px;
`;

const ObjectFieldContent = styled.div`
  padding-left: 16px;
`;

/**
 * Custom FieldTemplate that provides Ant Design Form.Item styling
 * and enhanced help text display. Uses vertical layout for long labels.
 * Object fields are rendered as section headers without colons.
 */
export function FieldTemplate(props: FieldTemplateProps) {
  const {
    id,
    classNames,
    label,
    help,
    required,
    description,
    rawErrors,
    children,
    schema,
    hidden,
  } = props;

  if (hidden) {
    return <div className="hidden">{children}</div>;
  }

  // Context-based label hiding: CollapsibleObjectFieldTemplate marks field IDs
  // whose labels duplicate the section title so we skip rendering the label.
  // eslint-disable-next-line react-hooks/rules-of-hooks
  const hideLabelIds = useContext(HideLabelContext);
  const shouldHideLabel = hideLabelIds.has(id);

  // Build help text from description - handle string, ReactElement, or object
  let helpText = "";
  if (typeof description === "string") {
    helpText = description;
  } else if (typeof help === "string") {
    helpText = help;
  }

  // Check if field has a default value to show in help
  const hasDefault = schema.default !== undefined;
  const defaultValueHint = hasDefault
    ? ` (Default: ${JSON.stringify(schema.default)})`
    : "";

  const hasErrors = rawErrors && rawErrors.length > 0;

  // Check if this field represents an object (nested form section)
  const isObjectField = schema.type === "object" || schema.properties !== undefined;

  // For object fields, render as a section header without Form.Item
  if (isObjectField && !shouldHideLabel) {
    // Calculate nesting level from the ID path (count underscore separators)
    const nestingLevel = (id.match(/_/g) || []).length;

    // Map nesting level to heading level and styles
    // Level 0-1: h4 (larger, more prominent)
    // Level 2: h5 (medium)
    // Level 3+: styled div (smaller, tertiary text)
    const getHeaderStyles = () => {
      if (nestingLevel <= 1) {
        return {
          level: 4 as const,
          fontSize: "16px",
          fontWeight: 600,
          color: "var(--color-text-base)",
          marginTop: 20,
          marginBottom: 12,
        };
      } else if (nestingLevel === 2) {
        return {
          level: 5 as const,
          fontSize: "14px",
          fontWeight: 600,
          color: "var(--color-text-secondary)",
          marginTop: 16,
          marginBottom: 10,
        };
      } else {
        return {
          level: 5 as const,
          fontSize: "13px",
          fontWeight: 500,
          color: "var(--color-text-tertiary)",
          marginTop: 12,
          marginBottom: 8,
        };
      }
    };

    const headerStyles = getHeaderStyles();
    const { level, ...titleStyle } = headerStyles;

    return (
      <ObjectFieldWrapper className={classNames}>
        <ObjectFieldHeader>
          <Title level={level} style={{ margin: 0, ...titleStyle }}>
            {label}
            {required && <span style={{ color: "red", marginLeft: 4 }}>*</span>}
          </Title>
          {helpText && (
            <Tooltip title={helpText + defaultValueHint}>
              <InfoCircleOutlined
                style={{
                  color: "var(--color-text-tertiary)",
                  cursor: "help",
                  fontSize: nestingLevel <= 1 ? "15px" : "14px",
                }}
              />
            </Tooltip>
          )}
        </ObjectFieldHeader>
        <ObjectFieldContent>
          {children}
        </ObjectFieldContent>
        {hasErrors && (
          <div style={{ color: "var(--ant-error-color)", fontSize: "14px", marginTop: 4 }}>
            {rawErrors?.join(", ")}
          </div>
        )}
      </ObjectFieldWrapper>
    );
  }

  // For primitive fields, use Form.Item with colon (default Ant Design behavior)
  const labelText = typeof label === "string" ? label : "";
  const isLongLabel = labelText.length > 12;

  // For long labels: full width (vertical), for short: default horizontal
  const layoutProps = isLongLabel
    ? { labelCol: { span: 24 }, wrapperCol: { span: 24 } }
    : {};

  const displayLabel = shouldHideLabel ? undefined : (
    <span>
      {label}
      {required && <span style={{ color: "red", marginLeft: 4 }}>*</span>}
      {helpText && (
        <Tooltip title={helpText + defaultValueHint}>
          <InfoCircleOutlined
            style={{
              marginLeft: 8,
              color: "var(--color-text-tertiary)",
              cursor: "help",
            }}
          />
        </Tooltip>
      )}
    </span>
  );

  return (
    <Form.Item
      label={displayLabel}
      validateStatus={hasErrors ? "error" : undefined}
      help={hasErrors ? rawErrors : undefined}
      className={classNames}
      htmlFor={id}
      {...layoutProps}
    >
      {children}
    </Form.Item>
  );
}
