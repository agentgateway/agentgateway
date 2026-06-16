import { useState } from "react";
import { Save } from "lucide-react";
import type { SchemaHelp } from "../schemaHelp";
import { Dropdown, Field, FieldGroup, StatusBanner, YamlBlock } from "../components/Primitives";
import type { LocalRateLimitConfig, LocalRateLimitDraft } from "./types";

export function LocalRateLimitPolicyEditor(props: {
  localRateLimit: LocalRateLimitConfig | null | undefined;
  help: SchemaHelp;
  saving: boolean;
  onSave: (rateLimit: LocalRateLimitDraft) => void;
}) {
  if (props.localRateLimit && !Array.isArray(props.localRateLimit)) {
    return (
      <div className="policy-editor-stack">
        <StatusBanner state="warn" title="Unsupported rate limit shape">
          This policy uses conditional rate limit entries. The visual editor currently supports simple rate limits only.
        </StatusBanner>
        <YamlBlock value={props.localRateLimit} />
      </div>
    );
  }

  const first = props.localRateLimit?.[0];
  const [type, setType] = useState(first?.type ?? "requests");
  const [maxTokens, setMaxTokens] = useState(String(first?.maxTokens ?? 100));
  const [tokensPerFill, setTokensPerFill] = useState(String(first?.tokensPerFill ?? 100));
  const [fillInterval, setFillInterval] = useState(first?.fillInterval ?? "60s");

  return (
    <div>
      <div className="form-grid">
        <FieldGroup label="Limit type" tooltip={props.help.description(["$defs", "RateLimitSpec", "properties", "type"])}>
          <Dropdown
            ariaLabel="Limit type"
            value={type}
            options={[
              { value: "requests", label: "requests" },
              { value: "tokens", label: "tokens" },
            ]}
            onChange={(value) => setType(value as typeof type)}
          />
        </FieldGroup>
        <Field label="Fill interval" tooltip={props.help.description(["$defs", "RateLimitSpec", "properties", "fillInterval"])}>
          <input value={fillInterval} onChange={(event) => setFillInterval(event.target.value)} placeholder="60s" />
        </Field>
        <Field label="Max tokens" tooltip={props.help.description(["$defs", "RateLimitSpec", "properties", "maxTokens"])}>
          <input type="number" value={maxTokens} onChange={(event) => setMaxTokens(event.target.value)} />
        </Field>
        <Field label="Tokens per fill" tooltip={props.help.description(["$defs", "RateLimitSpec", "properties", "tokensPerFill"])}>
          <input type="number" value={tokensPerFill} onChange={(event) => setTokensPerFill(event.target.value)} />
        </Field>
      </div>
      <button className="button primary" type="button" disabled={props.saving || !fillInterval.trim()} onClick={() => props.onSave([{
        type,
        fillInterval,
        maxTokens: Number(maxTokens),
        tokensPerFill: Number(tokensPerFill),
      }])}>
        <Save size={16} />
        Save rate limit
      </button>
    </div>
  );
}
