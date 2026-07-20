import { tr } from "../i18n";
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
        title={tr("copy.unsupportedRateLimitShape")}
        value={props.localRateLimit}
        schema={props.help.node(["$defs", "LocalRateLimit"])}
        help={props.help}
      >
        {tr(
          "copy.thisPolicyUsesConditionalRateLimitEntriesTheVisualEditorCurrentlySupportsSimpleRateLimitsOnly",
        )}
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
          label={tr("copy.limitType")}
          tooltip={props.help.field<RateLimitSpec>("RateLimitSpec", "type")}
        >
          <EnumSelector
            ariaLabel="Limit type"
            value={type}
            options={[
              {
                value: "requests",
                label: tr("copy.requests"),
                description: tr("copy.limitByRequestCount"),
              },
              {
                value: "tokens",
                label: tr("copy.tokens"),
                description: tr("copy.limitByTokenCount"),
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
          label={tr("copy.fillInterval")}
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
          label={tr("copy.maxTokens")}
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
          label={tr("copy.tokensPerFill")}
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
