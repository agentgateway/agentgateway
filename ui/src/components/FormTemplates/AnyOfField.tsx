/**
 * Custom AnyOfField
 *
 * Handles two patterns found in the gateway config schema:
 *
 * 1. Nullable pattern – `anyOf: [SomeType, { type: "null" }]`
 *    Renders a Checkbox to enable/disable the optional field.  When checked the
 *    sub-schema form is shown; when unchecked the field value is null.
 *
 * 2. True anyOf – multiple non-null types.
 *    Rendered identically to OneOfField (Radio.Group) since in this schema all
 *    anyOf usages are effectively discriminated unions.
 */
import type { FieldProps, RJSFSchema } from "@rjsf/utils";
import { Checkbox, Radio, Space } from "antd";
import { useState } from "react";

export function AnyOfField(props: FieldProps) {
  const {
    schema,
    formData,
    onChange,
    registry,
    idSchema,
    uiSchema,
    disabled,
    readonly,
    errorSchema,
    required,
    onBlur,
    onFocus,
  } = props;

  const options = (schema.anyOf ?? []) as RJSFSchema[];
  const { schemaUtils, fields } = registry;

  // Detect the nullable pattern: anyOf contains exactly one { type: "null" }.
  const nullOptionIndex = options.findIndex(
    (opt) => (opt as RJSFSchema).type === "null",
  );
  const isNullablePattern = nullOptionIndex !== -1;
  const nonNullOptions = isNullablePattern
    ? options.filter((_, i) => i !== nullOptionIndex)
    : options;

  // For the non-nullable anyOf branch, track which option is selected.
  const currentIndex = schemaUtils.getFirstMatchingOption(
    formData,
    nonNullOptions,
  );
  const [selectedIndex, setSelectedIndex] = useState(
    currentIndex >= 0 ? currentIndex : 0,
  );

  const SchemaField = fields.SchemaField as React.ComponentType<FieldProps>;

  // ── Nullable pattern ──────────────────────────────────────────────────────
  if (isNullablePattern && nonNullOptions.length === 1) {
    const isEnabled = formData !== null && formData !== undefined;
    const mainSchema = schemaUtils.retrieveSchema(
      (nonNullOptions[0] ?? {}) as RJSFSchema,
      formData,
    );
    // Strip title so the section heading doesn't duplicate the field label.
    const schemaForField: RJSFSchema = { ...mainSchema, title: "" };

    const hasProperties =
      mainSchema.type === "object" &&
      Object.keys(mainSchema.properties ?? {}).length > 0;

    const handleToggle = (checked: boolean) => {
      if (checked) {
        // For scalar types (string, number, boolean) that have an enum, use the
        // first enum value as the initial default rather than letting
        // getDefaultFormState return undefined → which would then get coerced
        // to `{}` and break the widget.
        const isScalarEnum =
          (mainSchema.type === "string" ||
            mainSchema.type === "number" ||
            mainSchema.type === "integer" ||
            mainSchema.type === "boolean") &&
          Array.isArray(mainSchema.enum) &&
          mainSchema.enum.length > 0;

        if (isScalarEnum) {
          onChange(mainSchema.enum![0] as Parameters<typeof onChange>[0]);
          return;
        }

        const defaults = schemaUtils.getDefaultFormState(
          mainSchema,
          undefined,
        );
        // Fall back to empty string for scalar types with no enum, empty object
        // for object types.
        const fallback =
          mainSchema.type === "string" ||
          mainSchema.type === "number" ||
          mainSchema.type === "integer"
            ? ""
            : {};
        onChange(
          (defaults ?? fallback) as Parameters<typeof onChange>[0],
        );
      } else {
        onChange(null as Parameters<typeof onChange>[0]);
      }
    };

    return (
      <div>
        <Checkbox
          checked={isEnabled}
          onChange={(e) => handleToggle(e.target.checked)}
          disabled={disabled || readonly || required}
        >
          {isEnabled ? "Enabled" : "Enable"}
        </Checkbox>

        {isEnabled && hasProperties && SchemaField && (
          <div
            style={{
              marginTop: 8,
              paddingLeft: 16,
              borderLeft: "2px solid var(--color-border, #f0f0f0)",
            }}
          >
            <SchemaField
              {...props}
              schema={schemaForField}
              idSchema={idSchema}
              formData={formData}
              onChange={onChange}
              onBlur={onBlur}
              onFocus={onFocus}
              errorSchema={errorSchema}
              uiSchema={uiSchema ?? {}}
            />
          </div>
        )}

        {/* Scalar nullable fields – just render the widget when enabled */}
        {isEnabled && !hasProperties && SchemaField && (
          <div style={{ marginTop: 8 }}>
            <SchemaField
              {...props}
              schema={schemaForField}
              idSchema={idSchema}
              formData={formData}
              onChange={onChange}
              onBlur={onBlur}
              onFocus={onFocus}
              errorSchema={errorSchema}
              uiSchema={uiSchema ?? {}}
            />
          </div>
        )}
      </div>
    );
  }

  // ── True anyOf: multiple non-null options → Radio.Group ──────────────────
  const handleSelect = (newIndex: number) => {
    if (newIndex === selectedIndex) return;
    const prevSchema = (nonNullOptions[selectedIndex] ?? {}) as RJSFSchema;
    const nextSchema = (nonNullOptions[newIndex] ?? {}) as RJSFSchema;
    const sanitized = schemaUtils.sanitizeDataForNewSchema(
      nextSchema,
      prevSchema,
      formData,
    );
    setSelectedIndex(newIndex);
    onChange(sanitized);
  };

  const resolvedSchema = schemaUtils.retrieveSchema(
    (nonNullOptions[selectedIndex] ?? {}) as RJSFSchema,
    formData,
  );
  const schemaForField: RJSFSchema = { ...resolvedSchema, title: "" };

  return (
    <div>
      <Radio.Group
        value={selectedIndex}
        onChange={(e) => handleSelect(Number(e.target.value))}
        disabled={disabled || readonly}
        style={{ marginBottom: 8 }}
      >
        <Space wrap>
          {nonNullOptions.map((opt, i) => (
            <Radio key={i} value={i}>
              {(opt as RJSFSchema).title ?? `Option ${i + 1}`}
            </Radio>
          ))}
        </Space>
      </Radio.Group>

      {SchemaField && (
        <SchemaField
          {...props}
          schema={schemaForField}
          idSchema={idSchema}
          formData={formData}
          onChange={onChange}
          onBlur={onBlur}
          onFocus={onFocus}
          errorSchema={errorSchema}
          uiSchema={uiSchema ?? {}}
        />
      )}
    </div>
  );
}
