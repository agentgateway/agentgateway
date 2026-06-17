import { useState } from "react";
import type { SchemaHelp } from "../schemaHelp";
import { EnumSelector } from "../components/EnumSelector";
import { UnsupportedYamlFallback } from "../components/EditorContracts";
import { Field, FieldGroup } from "../components/Primitives";
import type { LocalRateLimitConfig, LocalRateLimitDraft } from "./types";
import type { RateLimitSpec } from "../gateway-config";
import { ResultingYaml } from "./ResultingYaml";

export function LocalRateLimitPolicyEditor(props: {
  formId?: string;
  localRateLimit: LocalRateLimitConfig | null | undefined;
  help: SchemaHelp;
  saving: boolean;
  onSave: (rateLimit: LocalRateLimitDraft) => void;
}) {
  const first = Array.isArray(props.localRateLimit)
    ? props.localRateLimit[0]
    : undefined;
  const [type, setType] = useState(first?.type ?? "requests");
  const [maxTokens, setMaxTokens] = useState(String(first?.maxTokens ?? 100));
  const [tokensPerFill, setTokensPerFill] = useState(
    String(first?.tokensPerFill ?? 100),
  );
  const [fillInterval, setFillInterval] = useState(
    first?.fillInterval ?? "60s",
  );
  const preview = [
    {
      type,
      fillInterval,
      maxTokens: Number(maxTokens),
      tokensPerFill: Number(tokensPerFill),
    },
  ] as LocalRateLimitDraft;

  if (props.localRateLimit && !Array.isArray(props.localRateLimit)) {
    return (
      <UnsupportedYamlFallback
        title="Unsupported rate limit shape"
        value={props.localRateLimit}
        schema={props.help.node(["$defs", "LocalRateLimit"])}
        help={props.help}
      >
        This policy uses conditional rate limit entries. The visual editor
        currently supports simple rate limits only.
      </UnsupportedYamlFallback>
    );
  }

  return (
    <form
      id={props.formId}
      className="policy-editor-stack"
      onSubmit={(event) => {
        event.preventDefault();
        if (!fillInterval.trim()) return;
        props.onSave(preview);
      }}
    >
      <div className="form-grid">
        <FieldGroup
          label="Limit type"
          tooltip={props.help.field<RateLimitSpec>("RateLimitSpec", "type")}
        >
          <EnumSelector
            ariaLabel="Limit type"
            value={type}
            options={[
              {
                value: "requests",
                label: "Requests",
                description: "Limit by request count.",
              },
              {
                value: "tokens",
                label: "Tokens",
                description: "Limit by token count.",
              },
            ]}
            schema={props.help.node([
              "$defs",
              "RateLimitSpec",
              "properties",
              "type",
            ])}
            onChange={setType}
          />
        </FieldGroup>
        <Field
          label="Fill interval"
          tooltip={props.help.field<RateLimitSpec>(
            "RateLimitSpec",
            "fillInterval",
          )}
        >
          <input
            value={fillInterval}
            onChange={(event) => setFillInterval(event.target.value)}
            placeholder="60s"
          />
        </Field>
        <Field
          label="Max tokens"
          tooltip={props.help.field<RateLimitSpec>(
            "RateLimitSpec",
            "maxTokens",
          )}
        >
          <input
            type="number"
            value={maxTokens}
            onChange={(event) => setMaxTokens(event.target.value)}
          />
        </Field>
        <Field
          label="Tokens per fill"
          tooltip={props.help.field<RateLimitSpec>(
            "RateLimitSpec",
            "tokensPerFill",
          )}
        >
          <input
            type="number"
            value={tokensPerFill}
            onChange={(event) => setTokensPerFill(event.target.value)}
          />
        </Field>
      </div>
      <ResultingYaml value={preview} />
    </form>
  );
}
