/**
 * Custom OneOfField
 *
 * Replaces the default RJSF tab/select switcher with a compact Radio.Group.
 * When the user picks a different option the form data is sanitised (compatible
 * fields preserved) via `schemaUtils.sanitizeDataForNewSchema`.
 *
 * The sub-schema is rendered via `registry.fields.SchemaField` so all existing
 * templates (ObjectFieldTemplate, FieldTemplate, …) continue to apply.
 */
import type { FieldProps, RJSFSchema } from "@rjsf/utils";
import { Radio, Space } from "antd";
import { useEffect, useState } from "react";

export function OneOfField(props: FieldProps) {
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

  const options = (schema.oneOf ?? []) as RJSFSchema[];
  const { schemaUtils, fields } = registry;

  // Find the first object-compatible option as a better fallback than index 0.
  // This avoids picking a sentinel like `{ type: "string", enum: ["invalid"] }`
  // when no option actually matches the current data.
  const firstObjectOptionIndex = options.findIndex(
    (opt) => (opt as RJSFSchema).type === "object" || !(opt as RJSFSchema).type,
  );
  const fallbackIndex = firstObjectOptionIndex >= 0 ? firstObjectOptionIndex : 0;

  const matchedIndex = schemaUtils.getFirstMatchingOption(formData, options);
  const initialIndex = matchedIndex >= 0 ? matchedIndex : fallbackIndex;
  const [selectedIndex, setSelectedIndex] = useState(initialIndex);

  // On mount: either seed a brand-new (undefined) item with the selected
  // option's defaults, or strip stale keys from an existing item that has
  // leftover data from previously-selected options (e.g. after a save/reload).
  useEffect(() => {
    const activeBranchKeys = new Set<string>([
      ...Object.keys(
        ((options[selectedIndex] ?? {}) as RJSFSchema).properties ?? {},
      ),
      ...(((options[selectedIndex] ?? {}) as RJSFSchema).required ?? []),
    ]);

    if (formData === undefined || formData === null) {
      // New item — seed with the active branch's defaults.
      const selectedSchema = (options[selectedIndex] ?? {}) as RJSFSchema;
      const defaultValue = schemaUtils.getDefaultFormState(
        selectedSchema,
        undefined,
      );
      if (defaultValue !== undefined && defaultValue !== null) {
        onChange(defaultValue);
      }
      return;
    }

    if (typeof formData !== "object") return;

    // Existing item — clean up any stale keys from inactive branches.
    const staleKeys = Object.keys(
      formData as Record<string, unknown>,
    ).filter((k) => allOptionOwnedKeys.has(k) && !activeBranchKeys.has(k));

    if (staleKeys.length > 0) {
      const cleanData = Object.fromEntries(
        Object.entries(formData as Record<string, unknown>).filter(
          ([k]) => !allOptionOwnedKeys.has(k) || activeBranchKeys.has(k),
        ),
      );
      onChange(cleanData);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Collect ALL property keys that are "owned" by any oneOf branch so we can
  // strip them when switching options.  Without this, stale keys from previous
  // selections accumulate in formData and trigger `unevaluatedProperties` errors.
  const allOptionOwnedKeys = new Set<string>(
    options.flatMap((opt) => [
      ...Object.keys((opt as RJSFSchema).properties ?? {}),
      ...((opt as RJSFSchema).required ?? []),
    ]),
  );

  const handleSelect = (newIndex: number) => {
    if (newIndex === selectedIndex) return;
    const nextSchema = (options[newIndex] ?? {}) as RJSFSchema;

    // Strip every key owned by any branch from the current data, keeping only
    // the outer-schema fields (e.g. weight, policies).
    const baseData =
      typeof formData === "object" && formData !== null
        ? Object.fromEntries(
            Object.entries(formData as Record<string, unknown>).filter(
              ([k]) => !allOptionOwnedKeys.has(k),
            ),
          )
        : {};

    // Seed the new branch with its own defaults so the user sees pre-filled inputs.
    const branchDefaults = schemaUtils.getDefaultFormState(
      nextSchema,
      undefined,
    ) as Record<string, unknown> | undefined;

    setSelectedIndex(newIndex);
    onChange({ ...baseData, ...(branchDefaults ?? {}) });
  };

  // Keys that belong to the currently-selected branch (vs. all other branches).
  const activeBranchKeys = new Set<string>([
    ...Object.keys(
      ((options[selectedIndex] ?? {}) as RJSFSchema).properties ?? {},
    ),
    ...(((options[selectedIndex] ?? {}) as RJSFSchema).required ?? []),
  ]);

  // Strip keys from inactive branches before passing formData to SchemaField.
  // This prevents stale values from re-entering the form state on every edit.
  const filteredFormData =
    typeof formData === "object" && formData !== null
      ? Object.fromEntries(
          Object.entries(formData as Record<string, unknown>).filter(
            ([k]) => !allOptionOwnedKeys.has(k) || activeBranchKeys.has(k),
          ),
        )
      : formData;

  // Resolve any $refs in the selected sub-schema.
  const resolvedSchema = schemaUtils.retrieveSchema(
    (options[selectedIndex] ?? {}) as RJSFSchema,
    filteredFormData,
  );

  // Strip the title from the resolved sub-schema – the radio button already
  // communicates the selection, so the section heading would be redundant.
  const schemaForField: RJSFSchema = { ...resolvedSchema, title: "" };

  const SchemaField = fields.SchemaField as React.ComponentType<FieldProps>;

  const hasMultipleOptions = options.length > 1;

  // Check whether the resolved sub-schema has any renderable content.
  const hasProperties =
    resolvedSchema.type === "object" &&
    Object.keys(resolvedSchema.properties ?? {}).length > 0;
  const isScalar =
    resolvedSchema.type === "string" ||
    resolvedSchema.type === "number" ||
    resolvedSchema.type === "integer" ||
    resolvedSchema.type === "boolean";
  const hasContent = hasProperties || isScalar || !!resolvedSchema.$ref;

  return (
    <div>
      {hasMultipleOptions && (
        <Radio.Group
          value={selectedIndex}
          onChange={(e) => handleSelect(Number(e.target.value))}
          disabled={disabled || readonly}
          style={{ marginBottom: hasContent ? 8 : 0 }}
        >
          <Space wrap>
            {options.map((opt, i) => (
              <Radio key={i} value={i}>
                {(opt as RJSFSchema).title ?? `Option ${i + 1}`}
              </Radio>
            ))}
          </Space>
        </Radio.Group>
      )}

      {hasContent && SchemaField && (
        <div style={{ paddingTop: hasMultipleOptions ? 4 : 0 }}>
          <SchemaField
            {...props}
            schema={schemaForField}
            idSchema={idSchema}
            formData={filteredFormData}
            onChange={onChange}
            onBlur={onBlur}
            onFocus={onFocus}
            errorSchema={errorSchema}
            uiSchema={uiSchema ?? {}}
            required={required}
          />
        </div>
      )}
    </div>
  );
}
