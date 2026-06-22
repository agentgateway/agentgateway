import { Plus, SlidersHorizontal, Trash2 } from "lucide-react";
import { Tooltip } from "../../components/Primitives";
import { CollapsiblePolicySection } from "../../policies/PolicyLayout";
import type { LlmModel } from "../../types";

type ModelMatches = NonNullable<LlmModel["matches"]>;
type ModelMatch = ModelMatches[number];
type HeaderMatch = NonNullable<ModelMatch["headers"]>[number];

export function ModelMatchesEditor(props: {
  matches: ModelMatches;
  onChange: (matches: ModelMatches) => void;
}) {
  const matches = props.matches;

  function updateMatch(index: number, match: ModelMatch) {
    props.onChange(
      matches.map((item, itemIndex) => (itemIndex === index ? match : item)),
    );
  }

  function removeMatch(index: number) {
    props.onChange(matches.filter((_, itemIndex) => itemIndex !== index));
  }

  return (
    <CollapsiblePolicySection
      icon={<SlidersHorizontal size={17} />}
      title="Matches"
      description="At least one match group must match. Within a group, every header condition must match."
      defaultOpen={matches.length > 0}
    >
      {matches.length ? (
        <div className="policy-editor-stack compact">
          {matches.map((match, index) => (
            <MatchCard
              key={index}
              index={index}
              match={match}
              onChange={(next) => updateMatch(index, next)}
              onRemove={() => removeMatch(index)}
            />
          ))}
        </div>
      ) : (
        <div className="empty-inline">No additional match conditions.</div>
      )}
      <div className="button-row">
        <button
          className="button"
          type="button"
          onClick={() =>
            props.onChange([
              ...matches,
              { headers: [{ name: "", value: { exact: "" } }] },
            ])
          }
        >
          <Plus size={16} />
          Add match
        </button>
      </div>
    </CollapsiblePolicySection>
  );
}

export function normalizeMatches(
  matches: LlmModel["matches"] | null | undefined,
) {
  const next = (matches ?? [])
    .map((match) => ({
      ...match,
      headers: (match.headers ?? []).filter((header) => header.name.trim()),
    }))
    .filter((match) => (match.headers?.length ?? 0) > 0);
  return next.length ? next : undefined;
}

function MatchCard(props: {
  index: number;
  match: ModelMatch;
  onChange: (match: ModelMatch) => void;
  onRemove: () => void;
}) {
  const headers = props.match.headers ?? [];

  function updateHeader(index: number, header: HeaderMatch) {
    props.onChange({
      ...props.match,
      headers: headers.map((item, itemIndex) =>
        itemIndex === index ? header : item,
      ),
    });
  }

  function removeHeader(index: number) {
    props.onChange({
      ...props.match,
      headers: headers.filter((_, itemIndex) => itemIndex !== index),
    });
  }

  return (
    <section className="match-card">
      <div className="match-card-header">
        <span />
        <Tooltip content="Remove match">
          <button
            className="icon-button danger"
            type="button"
            aria-label={`Remove match ${props.index + 1}`}
            onClick={props.onRemove}
          >
            <Trash2 size={15} />
          </button>
        </Tooltip>
      </div>
      <div className="match-card-body">
        {headers.length ? (
          <div className="match-header-list">
            {headers.map((header, index) => (
              <HeaderMatchRow
                key={index}
                header={header}
                onChange={(next) => updateHeader(index, next)}
                onRemove={() => removeHeader(index)}
              />
            ))}
          </div>
        ) : (
          <div className="empty-inline">No header conditions.</div>
        )}
        <button
          className="button small"
          type="button"
          onClick={() =>
            props.onChange({
              ...props.match,
              headers: [...headers, { name: "", value: { exact: "" } }],
            })
          }
        >
          <Plus size={16} />
          Add header
        </button>
      </div>
    </section>
  );
}

function HeaderMatchRow(props: {
  header: HeaderMatch;
  onChange: (header: HeaderMatch) => void;
  onRemove: () => void;
}) {
  const value = props.header.value;
  const mode =
    value && typeof value === "object" && "regex" in value ? "regex" : "exact";
  const text =
    value && typeof value === "object"
      ? "regex" in value
        ? value.regex
        : "exact" in value
          ? value.exact
          : ""
      : "";
  const setMode = (regex: boolean) =>
    props.onChange({
      ...props.header,
      value: regex ? { regex: text } : { exact: text },
    });
  const setText = (next: string) =>
    props.onChange({
      ...props.header,
      value: mode === "regex" ? { regex: next } : { exact: next },
    });

  return (
    <div className="header-match-row">
      <input
        aria-label="Header name"
        value={props.header.name}
        onChange={(event) =>
          props.onChange({ ...props.header, name: event.target.value })
        }
        placeholder="Header name"
      />
      <input
        aria-label="Header value"
        value={text}
        onChange={(event) => setText(event.target.value)}
        placeholder={mode === "regex" ? "Regex value" : "Exact value"}
      />
      <label
        className={mode === "regex" ? "regex-toggle selected" : "regex-toggle"}
      >
        <input
          type="checkbox"
          checked={mode === "regex"}
          onChange={(event) => setMode(event.target.checked)}
        />
        Regex
      </label>
      <Tooltip content="Remove header condition">
        <button
          className="icon-button danger"
          type="button"
          aria-label="Remove header condition"
          onClick={props.onRemove}
        >
          <Trash2 size={15} />
        </button>
      </Tooltip>
    </div>
  );
}
