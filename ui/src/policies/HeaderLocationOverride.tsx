import { tr } from "../i18n";
import { SlidersHorizontal, X } from "lucide-react";
import { Field, FieldGroup } from "../components/Primitives";

export type HeaderLocationConfig = {
  header: {
    name: string;
    prefix?: string | null;
  };
};

export function headerLocationFrom(
  value: unknown,
): HeaderLocationConfig | undefined {
  if (!value || typeof value !== "object" || Array.isArray(value))
    return undefined;
  if (!("header" in value)) return undefined;
  const header = (value as { header?: unknown }).header;
  if (!header || typeof header !== "object" || Array.isArray(header))
    return undefined;
  const name = (header as { name?: unknown }).name;
  if (typeof name !== "string") return undefined;
  const prefix = (header as { prefix?: unknown }).prefix;
  return {
    header: {
      name,
      prefix: typeof prefix === "string" ? prefix : undefined,
    },
  };
}

export function HeaderLocationOverride(props: {
  enabled: boolean;
  headerName: string;
  headerPrefix: string;
  onEnabledChange: (enabled: boolean) => void;
  onHeaderNameChange: (value: string) => void;
  onHeaderPrefixChange: (value: string) => void;
  compactWhenDisabled?: boolean;
  hideResetButton?: boolean;
  tooltip?: string;
  headerNameTooltip?: string;
  headerPrefixTooltip?: string;
}) {
  if (!props.enabled && props.compactWhenDisabled) {
    return (
      <button
        className="button compact-action"
        type="button"
        onClick={() => props.onEnabledChange(true)}
      >
        <SlidersHorizontal size={15} />
        {tr("copy.customHeaderLocation")}
      </button>
    );
  }

  return (
    <FieldGroup label={tr("copy.headerLocation")} tooltip={props.tooltip}>
      {props.enabled ? (
        <div className="location-override-panel">
          <div className="form-grid">
            <Field
              label={tr("copy.headerName_8vzq77")}
              tooltip={props.headerNameTooltip}
            >
              <input
                value={props.headerName}
                onChange={(event) =>
                  props.onHeaderNameChange(event.target.value)
                }
                placeholder="authorization"
              />
            </Field>
            <Field
              label={tr("copy.headerPrefix")}
              tooltip={props.headerPrefixTooltip}
            >
              <input
                value={props.headerPrefix}
                onChange={(event) =>
                  props.onHeaderPrefixChange(event.target.value)
                }
                placeholder="Bearer "
              />
            </Field>
          </div>
          {props.hideResetButton ? null : (
            <button
              className="button"
              type="button"
              onClick={() => props.onEnabledChange(false)}
            >
              <X size={15} />
              {tr("copy.useDefaultLocation")}
            </button>
          )}
        </div>
      ) : (
        <button
          className="button"
          type="button"
          onClick={() => props.onEnabledChange(true)}
        >
          <SlidersHorizontal size={15} />
          {tr("copy.customHeaderLocation")}
        </button>
      )}
    </FieldGroup>
  );
}
