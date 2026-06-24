import { useState } from "react";
import type { ReactNode } from "react";
import { FieldGroup } from "../components/Primitives";
import { appendUnique } from "./policyUtils";

export function ListEditor(props: {
  label: string;
  values: string[];
  onChange: (values: string[]) => void;
  placeholder: string;
  emptyText?: string;
  tooltip?: string;
  suggestions?: string[];
  actions?: ReactNode;
}) {
  const [draft, setDraft] = useState("");

  function add(value: string) {
    const next = value.trim();
    if (!next) return;
    props.onChange(appendUnique(props.values, next));
    setDraft("");
  }

  return (
    <FieldGroup label={props.label} tooltip={props.tooltip}>
      <div className="list-editor">
        {props.values.length > 0 ? (
          <div className="chip-list">
            {props.values.map((value) => (
              <span className="config-chip" key={value}>
                <span>{value}</span>
                <button
                  type="button"
                  aria-label={`Remove ${value}`}
                  onClick={() =>
                    props.onChange(
                      props.values.filter((item) => item !== value),
                    )
                  }
                >
                  x
                </button>
              </span>
            ))}
          </div>
        ) : (
          <div className="empty-inline">
            {props.emptyText ?? "No values configured."}
          </div>
        )}
        <div className="list-editor-row">
          <input
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                add(draft);
              }
            }}
            placeholder={props.placeholder}
          />
          <button className="button" type="button" onClick={() => add(draft)}>
            Add
          </button>
          {props.actions}
        </div>
        {props.suggestions?.length ? (
          <div className="suggestion-row">
            {props.suggestions.map((suggestion) => (
              <button
                className="table-action"
                type="button"
                key={suggestion}
                onClick={() => add(suggestion)}
              >
                {suggestion}
              </button>
            ))}
          </div>
        ) : null}
      </div>
    </FieldGroup>
  );
}
