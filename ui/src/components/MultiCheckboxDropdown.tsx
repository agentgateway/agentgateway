import { ChevronDown, Funnel, Layers3 } from "lucide-react";
import { useState } from "react";
import { useDismissiblePopover } from "./Primitives";

export function MultiCheckboxDropdown(props: {
  kind: "group" | "filter";
  label: string;
  options: Array<{ value: string; label: string }>;
  values: string[];
  placeholder?: string;
  allLabel?: string;
  onChange: (values: string[]) => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useDismissiblePopover<HTMLDivElement>(open, () => setOpen(false));
  const selected = props.options.filter((option) =>
    props.values.includes(option.value),
  );
  const summary = selected.length
    ? selected[0].label
    : (props.placeholder ?? "Select");
  const hasExplicitSelection =
    selected.length > 0 && summary !== props.placeholder;
  const extraCount = selected.length > 1 ? selected.length - 1 : 0;
  const Icon = props.kind === "group" ? Layers3 : Funnel;

  function toggle(value: string) {
    props.onChange(
      props.values.includes(value)
        ? props.values.filter((item) => item !== value)
        : [...props.values, value],
    );
  }

  return (
    <div className="multi-check" ref={ref}>
      <span>{props.label}</span>
      <button
        className={
          hasExplicitSelection
            ? "button multi-check-trigger has-selection"
            : "button multi-check-trigger"
        }
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((current) => !current)}
      >
        <span className="multi-check-trigger-main">
          <Icon size={14} />
          <span>{summary}</span>
        </span>
        {extraCount ? (
          <span className="multi-check-count">+{extraCount}</span>
        ) : null}
        <ChevronDown size={15} />
      </button>
      {open ? (
        <div className="multi-check-popover">
          {props.allLabel ? (
            <label className="multi-check-option all">
              <input
                type="checkbox"
                checked={props.values.length === 0}
                onChange={() => props.onChange([])}
              />
              <span>{props.allLabel}</span>
            </label>
          ) : null}
          {props.options.length ? (
            props.options.map((option) => (
              <label className="multi-check-option" key={option.value}>
                <input
                  type="checkbox"
                  checked={props.values.includes(option.value)}
                  onChange={() => toggle(option.value)}
                />
                <span>{option.label}</span>
              </label>
            ))
          ) : (
            <p className="muted-copy">No values found.</p>
          )}
        </div>
      ) : null}
    </div>
  );
}
