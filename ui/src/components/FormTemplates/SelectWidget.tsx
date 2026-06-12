/**
 * Custom SelectWidget for enum fields.
 *
 * Replaces the default @rjsf/antd SelectWidget to fix two issues:
 *
 * 1. Non-nullable enums: the default widget adds an empty blank option for
 *    optional fields and leaves the value as undefined on first render.
 *    This widget defaults to the first enum value immediately (via useEffect)
 *    and never shows a blank option.
 *
 * 2. Nullable enums: when the field schema itself has `null` in its type or
 *    enum (i.e. the SelectWidget is called with a schema that includes null as
 *    a valid value), a "-- not set --" option is shown for the null state.
 *    This pattern is distinct from the anyOf-nullable handled by AnyOfField;
 *    it covers schemas like `{ type: ["string","null"], enum: [..., null] }`.
 */
import type { WidgetProps } from "@rjsf/utils";
import { Select } from "antd";
import { useEffect } from "react";

interface EnumOption {
  value: unknown;
  label: string;
}

export function SelectWidget(props: WidgetProps) {
  const {
    id,
    options,
    value,
    disabled,
    readonly,
    onChange,
    schema,
  } = props;

  const { enumOptions = [], enumDisabled = [] } = options as {
    enumOptions?: EnumOption[];
    enumDisabled?: unknown[];
  };

  // Detect if null is a legitimate option in THIS schema (not via anyOf – that
  // is already handled by AnyOfField + Checkbox).  Only applies when the schema
  // directly includes null in its type array or enum array.
  const schemaType = schema.type;
  const schemaEnum = schema.enum as unknown[] | undefined;
  const isNullable =
    (Array.isArray(schemaType) && schemaType.includes("null")) ||
    (Array.isArray(schemaEnum) && schemaEnum.includes(null));

  // Build the antd Select options list.
  const NULL_SENTINEL = "__null__";

  const selectOptions: Array<{ value: string; label: string; disabled?: boolean }> = [];

  if (isNullable) {
    selectOptions.push({ value: NULL_SENTINEL, label: "-- not set --" });
  }

  for (const opt of enumOptions) {
    // Skip null/undefined/empty-string entries – we represent null above.
    if (opt.value === null || opt.value === undefined || opt.value === "") {
      continue;
    }
    selectOptions.push({
      value: String(opt.value),
      label: String(opt.label || opt.value),
      disabled: (enumDisabled as unknown[]).includes(opt.value),
    });
  }

  // Normalize incoming value to a string key.
  const normalizedValue =
    value === null || value === undefined || value === ""
      ? isNullable
        ? NULL_SENTINEL   // show "-- not set --" for null state
        : undefined       // let effect below handle the default
      : String(value);

  // For non-nullable fields with no value yet, default to the first real option.
  useEffect(() => {
    if (!isNullable && (value === undefined || value === null || value === "")) {
      const first = selectOptions.find((o) => o.value !== NULL_SENTINEL);
      if (first) {
        onChange(first.value);
      }
    }
    // Only run on mount – the dep array is intentionally empty.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const handleChange = (selected: string) => {
    if (selected === NULL_SENTINEL) {
      onChange(null as Parameters<typeof onChange>[0]);
    } else {
      onChange(selected);
    }
  };

  return (
    <Select
      id={id}
      value={normalizedValue}
      options={selectOptions}
      onChange={handleChange}
      disabled={disabled || readonly}
      style={{ width: "100%" }}
      // Show placeholder only when nullable and no selection – prevents the
      // antd default of showing an empty gap at the top.
      placeholder={isNullable ? "-- not set --" : undefined}
    />
  );
}
