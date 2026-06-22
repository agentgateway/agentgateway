import {
  Braces,
  ListChecks,
  Pencil,
  Plus,
  Save,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import { useMemo, useState } from "react";
import {
  EnumSelector,
  type EnumSelectorOption,
} from "../components/EnumSelector";
import {
  Drawer,
  Field,
  FieldGroup,
  PageHeader,
  Panel,
  StatusBanner,
  YamlBlock,
} from "../components/Primitives";
import { getLlmGuardrails, setLlmGuardrails } from "../config";
import { useStickyQueryParam } from "../drawerRouteState";
import { useGatewayConfig, useUpdateConfig } from "../hooks";
import { cleanEmpty } from "../policies/policyUtils";
import { useSchemaHelp, type SchemaHelp } from "../schemaHelp";
import type { LlmGuardrail } from "../types";
import type {
  AnalyzeTextConfig,
  AzureContentSafety,
  BedrockGuardrails,
  DetectJailbreakConfig,
  GoogleModelArmor,
  Moderation,
  RegexRules,
  RequestRejection,
  RequestRejection1,
  Webhook,
} from "../gateway-config";
import azureIcon from "../assets/providers/azure.svg";
import bedrockIcon from "../assets/providers/bedrock.svg";
import googleCloudIcon from "../assets/providers/googlecloud.svg";
import openAiIcon from "../assets/providers/openai.svg";

type BuiltinRule = "ssn" | "creditCard" | "phoneNumber" | "email" | "caSin";
type GuardPhase = "request" | "response";
type GuardObject = NonNullable<LlmGuardrail["request"]>[number];
type GuardKind =
  | "builtin"
  | "regex"
  | "webhook"
  | "openAIModeration"
  | "bedrockGuardrails"
  | "googleModelArmor"
  | "azureContentSafety";
type GuardDraftBase = {
  kind: GuardKind;
  rejectionStatus: string;
  rejectionBody: string;
  policies?: unknown;
};
type UnsupportedGuardDraft = {
  kind: "unsupported";
  raw: GuardObject;
};
type BuiltinGuardDraft = GuardDraftBase & {
  kind: "builtin";
  action: "mask" | "reject";
  builtins: BuiltinRule[];
};
type RegexGuardDraft = GuardDraftBase & {
  kind: "regex";
  action: "mask" | "reject";
  patterns: string[];
};
type WebhookGuardDraft = GuardDraftBase & {
  kind: "webhook";
  target: string;
  failureMode: "failClosed" | "failOpen";
};
type OpenAIModerationGuardDraft = GuardDraftBase & {
  kind: "openAIModeration";
  model: string;
};
type BedrockGuardDraft = GuardDraftBase & {
  kind: "bedrockGuardrails";
  guardrailIdentifier: string;
  guardrailVersion: string;
  region: string;
};
type GoogleModelArmorGuardDraft = GuardDraftBase & {
  kind: "googleModelArmor";
  templateId: string;
  projectId: string;
  location: string;
};
type AzureContentSafetyGuardDraft = GuardDraftBase & {
  kind: "azureContentSafety";
  endpoint: string;
  severityThreshold: string;
  analyzeApiVersion: string;
  blocklistNames: string;
  haltOnBlocklistHit: boolean;
  detectJailbreak: boolean;
  jailbreakApiVersion: string;
};
type GuardDraft =
  | UnsupportedGuardDraft
  | BuiltinGuardDraft
  | RegexGuardDraft
  | WebhookGuardDraft
  | OpenAIModerationGuardDraft
  | BedrockGuardDraft
  | GoogleModelArmorGuardDraft
  | AzureContentSafetyGuardDraft;
type SupportedGuardDraft = Exclude<GuardDraft, UnsupportedGuardDraft>;
type RegexRulesShape = {
  action?: "mask" | "reject";
  rules?: Array<{ builtin?: BuiltinRule; pattern?: string }>;
};
type GuardrailDraft = {
  request: GuardDraft[];
  response: GuardDraft[];
};

const builtinOptions: Array<{ value: BuiltinRule; label: string }> = [
  { value: "email", label: "Email" },
  { value: "phoneNumber", label: "Phone" },
  { value: "creditCard", label: "Credit card" },
  { value: "ssn", label: "SSN" },
  { value: "caSin", label: "CA SIN" },
];

const requestGuardKinds: Array<EnumSelectorOption<GuardKind>> = [
  {
    value: "builtin",
    label: "Built-in detectors",
    description:
      "Detect common sensitive data types with built-in regex rules.",
    icon: <ListChecks size={16} />,
  },
  {
    value: "regex",
    label: "Custom regex",
    description: "Match and optionally mask custom regular expressions.",
    icon: <Braces size={16} />,
  },
  {
    value: "webhook",
    label: "Webhook",
    description: "Send content to an external guardrail service.",
    icon: <ShieldCheck size={16} />,
  },
  {
    value: "openAIModeration",
    label: "OpenAI Moderation",
    description: "Use OpenAI moderation checks for incoming prompts.",
    icon: <GuardrailProviderIcon src={openAiIcon} alt="" />,
  },
  {
    value: "bedrockGuardrails",
    label: "Bedrock Guardrails",
    description: "Use AWS Bedrock Guardrails.",
    icon: <GuardrailProviderIcon src={bedrockIcon} alt="" />,
  },
  {
    value: "googleModelArmor",
    label: "Google Model Armor",
    description: "Use Google Model Armor for safety checks.",
    icon: <GuardrailProviderIcon src={googleCloudIcon} alt="" />,
  },
  {
    value: "azureContentSafety",
    label: "Azure Content Safety",
    description: "Use Azure AI Content Safety.",
    icon: <GuardrailProviderIcon src={azureIcon} alt="" />,
  },
];

const responseGuardKinds: Array<EnumSelectorOption<GuardKind>> =
  requestGuardKinds.filter((kind) => kind.value !== "openAIModeration");

function GuardrailProviderIcon(props: { src: string; alt: string }) {
  return (
    <img className="guardrail-provider-icon" src={props.src} alt={props.alt} />
  );
}

export function GuardrailsPage() {
  const config = useGatewayConfig();
  const update = useUpdateConfig();
  const help = useSchemaHelp();
  const guardrails = useMemo(
    () => getLlmGuardrails(config.data),
    [config.data],
  );

  return (
    <div className="page-stack">
      <PageHeader
        title="LLM Guardrails"
        description="Apply prompt and response guardrails to all LLM models."
        actions={
          guardrails ? (
            <button
              className="button danger"
              type="button"
              onClick={() =>
                update.mutate((next) => setLlmGuardrails(next, null))
              }
            >
              <Trash2 size={16} />
              Remove
            </button>
          ) : null
        }
      />

      {update.isError ? (
        <StatusBanner state="bad" title="Save failed">
          {update.error.message}
        </StatusBanner>
      ) : null}

      <Panel>
        {config.isLoading ? (
          <StatusBanner state="loading" title="Loading guardrails" />
        ) : config.isError ? (
          <StatusBanner state="bad" title="Configuration API unavailable">
            {config.error.message}
          </StatusBanner>
        ) : (
          <GuardrailsEditor
            key={JSON.stringify(guardrails ?? emptyGuardrails())}
            initial={guardrails ?? emptyGuardrails()}
            help={help}
            saveError={update.isError ? update.error.message : null}
            onSave={(nextGuardrails) =>
              update.mutate((next) => setLlmGuardrails(next, nextGuardrails))
            }
          />
        )}
      </Panel>
    </div>
  );
}

function GuardrailsEditor(props: {
  initial: LlmGuardrail;
  help: SchemaHelp;
  saveError?: string | null;
  onSave: (guardrails: LlmGuardrail) => void;
}) {
  const [draft, setDraft] = useState<GuardrailDraft>(() =>
    draftFromGuardrails(props.initial),
  );
  const [error, setError] = useState<string | null>(null);

  function applyDraft(nextDraft: GuardrailDraft) {
    setDraft(nextDraft);
    const validationError = validateDraft(nextDraft);
    if (validationError) {
      setError(validationError);
      return;
    }
    setError(null);
    props.onSave(buildGuardrails(nextDraft));
  }

  return (
    <div className="guardrails-editor">
      {error ? (
        <StatusBanner state="bad" title="Invalid guardrails">
          {error}
        </StatusBanner>
      ) : null}
      {props.saveError ? (
        <StatusBanner state="bad" title="Save failed">
          {props.saveError}
        </StatusBanner>
      ) : null}
      <GuardrailSection
        phase="request"
        help={props.help}
        guards={draft.request}
        onChange={(request) => applyDraft({ ...draft, request })}
      />
      <GuardrailSection
        phase="response"
        help={props.help}
        guards={draft.response}
        onChange={(response) => applyDraft({ ...draft, response })}
      />
    </div>
  );
}

function emptyGuardrails(): LlmGuardrail {
  return {
    request: [],
    response: [],
  } as LlmGuardrail;
}

function GuardrailSection(props: {
  phase: GuardPhase;
  help: SchemaHelp;
  guards: GuardDraft[];
  onChange: (guards: GuardDraft[]) => void;
}) {
  const [guardDrawer, setGuardDrawer] = useStickyQueryParam("guard");
  const title =
    props.phase === "request" ? "Request guards" : "Response guards";
  const description =
    props.phase === "request"
      ? "Inspect prompts before they reach the upstream model."
      : "Inspect model output before it is returned to the caller.";
  const addOpen = guardDrawer === `${props.phase}:new`;
  const editingIndex = guardDrawerIndex(
    guardDrawer,
    props.phase,
    props.guards.length,
  );

  function closeGuardDrawer() {
    setGuardDrawer(null, "replace");
  }

  function patch(index: number, next: GuardDraft) {
    props.onChange(
      props.guards.map((guard, guardIndex) =>
        guardIndex === index ? next : guard,
      ),
    );
  }

  return (
    <section className="policy-form-section guardrail-section">
      <div className="policy-form-section-header">
        <span className="policy-form-section-icon">
          <ShieldCheck size={17} />
        </span>
        <div>
          <h4>{title}</h4>
          <p>{description}</p>
        </div>
      </div>
      <div className="policy-form-section-body">
        {props.guards.length === 0 ? (
          <p className="muted-copy">No guards configured.</p>
        ) : null}
        {props.guards.map((guard, index) => (
          <GuardCard
            key={index}
            phase={props.phase}
            guard={guard}
            index={index}
            onEdit={() => setGuardDrawer(`${props.phase}:${index}`)}
            onRemove={() =>
              props.onChange(
                props.guards.filter((_, guardIndex) => guardIndex !== index),
              )
            }
          />
        ))}
        <AddGuardButton onOpen={() => setGuardDrawer(`${props.phase}:new`)} />
        {addOpen ? (
          <AddGuardModal
            phase={props.phase}
            help={props.help}
            onClose={closeGuardDrawer}
            onAdd={(guard) => {
              props.onChange([...props.guards, guard]);
              closeGuardDrawer();
            }}
          />
        ) : null}
        {editingIndex != null && props.guards[editingIndex] ? (
          <EditGuardDrawer
            phase={props.phase}
            help={props.help}
            guard={props.guards[editingIndex]}
            onClose={closeGuardDrawer}
            onApply={(guard) => {
              patch(editingIndex, guard);
              closeGuardDrawer();
            }}
          />
        ) : null}
      </div>
    </section>
  );
}

function AddGuardButton(props: { onOpen: () => void }) {
  return (
    <button className="button" type="button" onClick={props.onOpen}>
      <Plus size={16} />
      Add guard
    </button>
  );
}

function AddGuardModal(props: {
  phase: GuardPhase;
  help: SchemaHelp;
  onClose: () => void;
  onAdd: (guard: GuardDraft) => void;
}) {
  const options =
    props.phase === "request" ? requestGuardKinds : responseGuardKinds;
  const [kind, setKind] = useState<GuardKind | "">("");
  const [guard, setGuard] = useState<SupportedGuardDraft | null>(null);

  function selectKind(value: string) {
    const nextKind = value as GuardKind;
    setKind(nextKind);
    setGuard(emptyGuardDraft(nextKind));
  }

  return (
    <Drawer
      title={`Add ${props.phase} guard`}
      onClose={props.onClose}
      footer={
        <div className="button-row">
          <button className="button" type="button" onClick={props.onClose}>
            Cancel
          </button>
          <button
            className="button primary"
            type="button"
            disabled={!guard}
            onClick={() => guard && props.onAdd(guard)}
          >
            <Plus size={16} />
            Add guard
          </button>
        </div>
      }
    >
      <FieldGroup
        label="Guard type"
        tooltip={guardTypeHelp(props.phase, props.help)}
      >
        <EnumSelector
          ariaLabel="Guard type"
          value={kind}
          options={options}
          placeholder="Select guard type"
          allowEmpty
          onChange={selectKind}
        />
      </FieldGroup>
      {guard ? (
        <section className="guardrail-rule-card compact">
          <GuardFields
            phase={props.phase}
            help={props.help}
            guard={guard}
            onChange={setGuard}
          />
        </section>
      ) : null}
    </Drawer>
  );
}

function GuardCard(props: {
  phase: GuardPhase;
  guard: GuardDraft;
  index: number;
  onEdit: () => void;
  onRemove: () => void;
}) {
  return (
    <section className="guardrail-summary-card">
      <div className="guardrail-rule-header">
        <span className="guardrail-rule-title">
          {guardKindIcon(props.guard.kind)}
          <strong>{guardKindLabel(props.guard.kind)}</strong>
        </span>
        <div className="button-row compact">
          <button className="table-action" type="button" onClick={props.onEdit}>
            <Pencil size={14} />
            Edit
          </button>
          <button
            className="table-action danger"
            type="button"
            onClick={props.onRemove}
          >
            <Trash2 size={14} />
            Remove
          </button>
        </div>
      </div>
      <p className="muted-copy">{guardSummary(props.guard)}</p>
      {props.guard.kind === "unsupported" ? (
        <details>
          <summary>Raw guard YAML</summary>
          <YamlBlock value={props.guard.raw} />
        </details>
      ) : null}
      {props.guard.kind !== "unsupported" && props.guard.policies ? (
        <details>
          <summary>Backend policies preserved</summary>
          <YamlBlock value={props.guard.policies} />
        </details>
      ) : null}
    </section>
  );
}

function EditGuardDrawer(props: {
  phase: GuardPhase;
  help: SchemaHelp;
  guard: GuardDraft;
  onClose: () => void;
  onApply: (guard: GuardDraft) => void;
}) {
  const [draft, setDraft] = useState<GuardDraft>(props.guard);
  return (
    <Drawer
      title={`Edit ${props.phase} guard`}
      onClose={props.onClose}
      footer={
        <div className="button-row">
          <button className="button" type="button" onClick={props.onClose}>
            Cancel
          </button>
          <button
            className="button primary"
            type="button"
            onClick={() => props.onApply(draft)}
          >
            <Save size={16} />
            Apply changes
          </button>
        </div>
      }
    >
      {draft.kind === "unsupported" ? (
        <>
          <Field label="Guard type">
            <input value="Unsupported raw YAML" disabled />
          </Field>
          <UnsupportedGuardFields guard={draft} />
        </>
      ) : (
        <>
          <FieldGroup
            label="Guard type"
            tooltip={guardTypeHelp(props.phase, props.help)}
          >
            <EnumSelector
              ariaLabel="Guard type"
              value={draft.kind}
              options={
                props.phase === "request"
                  ? requestGuardKinds
                  : responseGuardKinds
              }
              onChange={(value) => setDraft(emptyGuardDraft(value))}
            />
          </FieldGroup>
          <section className="guardrail-rule-card compact">
            <GuardFields
              phase={props.phase}
              help={props.help}
              guard={draft}
              onChange={(next) => setDraft(next)}
            />
          </section>
        </>
      )}
    </Drawer>
  );
}

function UnsupportedGuardFields(props: { guard: UnsupportedGuardDraft }) {
  return (
    <div className="policy-editor-stack">
      <StatusBanner state="warn" title="Unsupported guard shape">
        This guard uses a shape the visual editor does not support yet. It will
        be preserved as raw YAML.
      </StatusBanner>
      <YamlBlock value={props.guard.raw} />
    </div>
  );
}

function GuardFields(props: {
  phase: GuardPhase;
  help: SchemaHelp;
  guard: SupportedGuardDraft;
  onChange: (guard: SupportedGuardDraft) => void;
  patch?: (next: Partial<SupportedGuardDraft>) => void;
}) {
  const patch =
    props.patch ??
    ((next: Partial<SupportedGuardDraft>) =>
      props.onChange({ ...props.guard, ...next } as SupportedGuardDraft));
  return (
    <>
      {props.guard.kind === "builtin" ? (
        <BuiltinGuardFields
          phase={props.phase}
          help={props.help}
          guard={props.guard}
          onChange={props.onChange}
        />
      ) : null}
      {props.guard.kind === "regex" ? (
        <RegexGuardFields
          phase={props.phase}
          help={props.help}
          guard={props.guard}
          onChange={props.onChange}
        />
      ) : null}
      {props.guard.kind === "webhook" ? (
        <WebhookGuardFields
          help={props.help}
          guard={props.guard}
          onChange={patch}
        />
      ) : null}
      {props.guard.kind === "openAIModeration" ? (
        <OpenAIModerationFields
          help={props.help}
          guard={props.guard}
          onChange={patch}
        />
      ) : null}
      {props.guard.kind === "bedrockGuardrails" ? (
        <BedrockGuardFields
          help={props.help}
          guard={props.guard}
          onChange={patch}
        />
      ) : null}
      {props.guard.kind === "googleModelArmor" ? (
        <GoogleModelArmorFields
          help={props.help}
          guard={props.guard}
          onChange={patch}
        />
      ) : null}
      {props.guard.kind === "azureContentSafety" ? (
        <AzureContentSafetyFields
          phase={props.phase}
          help={props.help}
          guard={props.guard}
          onChange={patch}
        />
      ) : null}
      {props.guard.kind !== "builtin" && props.guard.kind !== "regex" ? (
        <RejectionFields
          phase={props.phase}
          help={props.help}
          guard={props.guard}
          onChange={patch}
        />
      ) : null}
    </>
  );
}

function BuiltinGuardFields(props: {
  phase: GuardPhase;
  help: SchemaHelp;
  guard: BuiltinGuardDraft;
  onChange: (guard: SupportedGuardDraft) => void;
}) {
  function patch(next: Partial<BuiltinGuardDraft>) {
    props.onChange({ ...props.guard, ...next });
  }
  return (
    <>
      <FieldGroup
        label="Action"
        tooltip={props.help.field<RegexRules>("RegexRules", "action")}
      >
        <EnumSelector
          ariaLabel="Action"
          value={props.guard.action}
          options={[
            {
              value: "reject",
              label: "Reject request",
              description: "Reject the request when a detector matches.",
            },
            {
              value: "mask",
              label: "Mask matched text",
              description: "Replace matched content and continue.",
            },
          ]}
          schema={props.help.node([
            "$defs",
            "RegexRules",
            "properties",
            "action",
          ])}
          onChange={(value) => patch({ action: value })}
        />
      </FieldGroup>
      <FieldGroup
        className="guardrail-builtins"
        label="Built-in detectors"
        tooltip={props.help.field<RegexRules>("RegexRules", "rules")}
      >
        <div className="method-grid">
          {builtinOptions.map((option) => {
            const selected = props.guard.builtins.includes(option.value);
            return (
              <button
                className={selected ? "choice-pill active" : "choice-pill"}
                type="button"
                key={option.value}
                onClick={() =>
                  patch({
                    builtins: selected
                      ? props.guard.builtins.filter(
                          (item) => item !== option.value,
                        )
                      : [...props.guard.builtins, option.value],
                  })
                }
              >
                {option.label}
              </button>
            );
          })}
        </div>
      </FieldGroup>
      {props.guard.action === "reject" ? (
        <RejectionFields
          phase={props.phase}
          help={props.help}
          guard={props.guard}
          onChange={(next) => patch(next as Partial<BuiltinGuardDraft>)}
        />
      ) : null}
    </>
  );
}

function RegexGuardFields(props: {
  phase: GuardPhase;
  help: SchemaHelp;
  guard: RegexGuardDraft;
  onChange: (guard: SupportedGuardDraft) => void;
}) {
  function patch(next: Partial<RegexGuardDraft>) {
    props.onChange({ ...props.guard, ...next });
  }
  return (
    <>
      <FieldGroup
        label="Action"
        tooltip={props.help.field<RegexRules>("RegexRules", "action")}
      >
        <EnumSelector
          ariaLabel="Action"
          value={props.guard.action}
          options={[
            {
              value: "reject",
              label: "Reject request",
              description: "Reject the request when a regex matches.",
            },
            {
              value: "mask",
              label: "Mask matched text",
              description: "Replace matched content and continue.",
            },
          ]}
          schema={props.help.node([
            "$defs",
            "RegexRules",
            "properties",
            "action",
          ])}
          onChange={(value) => patch({ action: value })}
        />
      </FieldGroup>
      <PatternList
        help={props.help}
        patterns={props.guard.patterns}
        onChange={(patterns) => patch({ patterns })}
      />
      {props.guard.action === "reject" ? (
        <RejectionFields
          phase={props.phase}
          help={props.help}
          guard={props.guard}
          onChange={(next) => patch(next as Partial<RegexGuardDraft>)}
        />
      ) : null}
    </>
  );
}

function WebhookGuardFields(props: {
  help: SchemaHelp;
  guard: WebhookGuardDraft;
  onChange: (next: Partial<SupportedGuardDraft>) => void;
}) {
  return (
    <>
      <Field
        label="Webhook target"
        tooltip={props.help.field<Webhook>("Webhook", "target")}
        hint="Backend host URL for guardrail checks."
      >
        <input
          value={props.guard.target}
          onChange={(event) =>
            props.onChange({
              target: event.target.value,
            } as Partial<SupportedGuardDraft>)
          }
          placeholder="http://guardrail.internal:8080"
        />
      </Field>
      <FieldGroup
        label="Failure mode"
        tooltip={props.help.field<Webhook>("Webhook", "failureMode")}
      >
        <EnumSelector
          ariaLabel="Failure mode"
          value={props.guard.failureMode}
          options={[
            {
              value: "failClosed",
              label: "Fail closed",
              description: "Reject when the webhook is unavailable or errors.",
            },
            {
              value: "failOpen",
              label: "Fail open",
              description:
                "Continue when the webhook is unavailable or errors.",
            },
          ]}
          schema={props.help.node([
            "$defs",
            "Webhook",
            "properties",
            "failureMode",
          ])}
          onChange={(value) =>
            props.onChange({
              failureMode: value,
            } as Partial<SupportedGuardDraft>)
          }
        />
      </FieldGroup>
    </>
  );
}

function OpenAIModerationFields(props: {
  help: SchemaHelp;
  guard: OpenAIModerationGuardDraft;
  onChange: (next: Partial<SupportedGuardDraft>) => void;
}) {
  return (
    <Field
      label="Moderation model"
      tooltip={props.help.field<Moderation>("Moderation", "model")}
      hint="Optional. Defaults to omni-moderation-latest."
    >
      <input
        value={props.guard.model}
        onChange={(event) =>
          props.onChange({
            model: event.target.value,
          } as Partial<SupportedGuardDraft>)
        }
        placeholder="omni-moderation-latest"
      />
    </Field>
  );
}

function BedrockGuardFields(props: {
  help: SchemaHelp;
  guard: BedrockGuardDraft;
  onChange: (next: Partial<SupportedGuardDraft>) => void;
}) {
  return (
    <div className="form-grid">
      <Field
        label="Guardrail identifier"
        tooltip={props.help.field<BedrockGuardrails>(
          "BedrockGuardrails",
          "guardrailIdentifier",
        )}
      >
        <input
          value={props.guard.guardrailIdentifier}
          onChange={(event) =>
            props.onChange({
              guardrailIdentifier: event.target.value,
            } as Partial<SupportedGuardDraft>)
          }
        />
      </Field>
      <Field
        label="Guardrail version"
        tooltip={props.help.field<BedrockGuardrails>(
          "BedrockGuardrails",
          "guardrailVersion",
        )}
      >
        <input
          value={props.guard.guardrailVersion}
          onChange={(event) =>
            props.onChange({
              guardrailVersion: event.target.value,
            } as Partial<SupportedGuardDraft>)
          }
        />
      </Field>
      <Field
        label="AWS region"
        tooltip={props.help.field<BedrockGuardrails>(
          "BedrockGuardrails",
          "region",
        )}
      >
        <input
          value={props.guard.region}
          onChange={(event) =>
            props.onChange({
              region: event.target.value,
            } as Partial<SupportedGuardDraft>)
          }
          placeholder="us-west-2"
        />
      </Field>
    </div>
  );
}

function GoogleModelArmorFields(props: {
  help: SchemaHelp;
  guard: GoogleModelArmorGuardDraft;
  onChange: (next: Partial<SupportedGuardDraft>) => void;
}) {
  return (
    <div className="form-grid">
      <Field
        label="Template ID"
        tooltip={props.help.field<GoogleModelArmor>(
          "GoogleModelArmor",
          "templateId",
        )}
      >
        <input
          value={props.guard.templateId}
          onChange={(event) =>
            props.onChange({
              templateId: event.target.value,
            } as Partial<SupportedGuardDraft>)
          }
        />
      </Field>
      <Field
        label="Project ID"
        tooltip={props.help.field<GoogleModelArmor>(
          "GoogleModelArmor",
          "projectId",
        )}
      >
        <input
          value={props.guard.projectId}
          onChange={(event) =>
            props.onChange({
              projectId: event.target.value,
            } as Partial<SupportedGuardDraft>)
          }
        />
      </Field>
      <Field
        label="Location"
        tooltip={props.help.field<GoogleModelArmor>(
          "GoogleModelArmor",
          "location",
        )}
        hint="Optional. Defaults to us-central1."
      >
        <input
          value={props.guard.location}
          onChange={(event) =>
            props.onChange({
              location: event.target.value,
            } as Partial<SupportedGuardDraft>)
          }
          placeholder="us-central1"
        />
      </Field>
    </div>
  );
}

function AzureContentSafetyFields(props: {
  phase: GuardPhase;
  help: SchemaHelp;
  guard: AzureContentSafetyGuardDraft;
  onChange: (next: Partial<SupportedGuardDraft>) => void;
}) {
  return (
    <>
      <Field
        label="Endpoint"
        tooltip={props.help.field<AzureContentSafety>(
          "AzureContentSafety",
          "endpoint",
        )}
      >
        <input
          value={props.guard.endpoint}
          onChange={(event) =>
            props.onChange({
              endpoint: event.target.value,
            } as Partial<SupportedGuardDraft>)
          }
          placeholder="resource.cognitiveservices.azure.com"
        />
      </Field>
      <div className="form-grid">
        <Field
          label="Severity threshold"
          tooltip={props.help.field<AnalyzeTextConfig>(
            "AnalyzeTextConfig",
            "severityThreshold",
          )}
          hint="Optional. 0-6; default is 2."
        >
          <input
            value={props.guard.severityThreshold}
            onChange={(event) =>
              props.onChange({
                severityThreshold: event.target.value,
              } as Partial<SupportedGuardDraft>)
            }
            placeholder="2"
          />
        </Field>
        <Field
          label="Analyze API version"
          tooltip={props.help.field<AnalyzeTextConfig>(
            "AnalyzeTextConfig",
            "apiVersion",
          )}
        >
          <input
            value={props.guard.analyzeApiVersion}
            onChange={(event) =>
              props.onChange({
                analyzeApiVersion: event.target.value,
              } as Partial<SupportedGuardDraft>)
            }
            placeholder="2024-09-01"
          />
        </Field>
        <Field
          label="Blocklists"
          tooltip={props.help.field<AnalyzeTextConfig>(
            "AnalyzeTextConfig",
            "blocklistNames",
          )}
          hint="Comma-separated names."
        >
          <input
            value={props.guard.blocklistNames}
            onChange={(event) =>
              props.onChange({
                blocklistNames: event.target.value,
              } as Partial<SupportedGuardDraft>)
            }
          />
        </Field>
      </div>
      <label className="native-toggle">
        <input
          type="checkbox"
          checked={props.guard.haltOnBlocklistHit}
          onChange={(event) =>
            props.onChange({
              haltOnBlocklistHit: event.target.checked,
            } as Partial<SupportedGuardDraft>)
          }
        />
        <span>Halt on blocklist hit</span>
      </label>
      {props.phase === "request" ? (
        <>
          <label className="native-toggle">
            <input
              type="checkbox"
              checked={props.guard.detectJailbreak}
              onChange={(event) =>
                props.onChange({
                  detectJailbreak: event.target.checked,
                } as Partial<SupportedGuardDraft>)
              }
            />
            <span>Detect jailbreak attempts</span>
          </label>
          {props.guard.detectJailbreak ? (
            <Field
              label="Jailbreak API version"
              tooltip={props.help.field<DetectJailbreakConfig>(
                "DetectJailbreakConfig",
                "apiVersion",
              )}
            >
              <input
                value={props.guard.jailbreakApiVersion}
                onChange={(event) =>
                  props.onChange({
                    jailbreakApiVersion: event.target.value,
                  } as Partial<SupportedGuardDraft>)
                }
                placeholder="2024-02-15-preview"
              />
            </Field>
          ) : null}
        </>
      ) : null}
    </>
  );
}

function RejectionFields(props: {
  phase: GuardPhase;
  help: SchemaHelp;
  guard: GuardDraftBase;
  onChange: (next: Partial<SupportedGuardDraft>) => void;
}) {
  return (
    <div className="form-grid">
      <Field
        label="Rejection status"
        tooltip={
          props.phase === "request"
            ? props.help.field<RequestRejection>("RequestRejection", "status")
            : props.help.field<RequestRejection1>("RequestRejection1", "status")
        }
      >
        <input
          value={props.guard.rejectionStatus}
          onChange={(event) =>
            props.onChange({
              rejectionStatus: event.target.value,
            } as Partial<SupportedGuardDraft>)
          }
          placeholder="403"
        />
      </Field>
      <Field
        label="Rejection body"
        tooltip={
          props.phase === "request"
            ? props.help.field<RequestRejection>(
                "RequestRejection",
                "body",
                "Response body returned when content is rejected.",
              )
            : props.help.field<RequestRejection1>(
                "RequestRejection1",
                "body",
                "Response body returned when content is rejected.",
              )
        }
      >
        <textarea
          rows={4}
          value={props.guard.rejectionBody}
          onChange={(event) =>
            props.onChange({
              rejectionBody: event.target.value,
            } as Partial<SupportedGuardDraft>)
          }
          placeholder="The request was rejected due to inappropriate content"
        />
      </Field>
    </div>
  );
}

function PatternList(props: {
  help: SchemaHelp;
  patterns: string[];
  onChange: (patterns: string[]) => void;
}) {
  return (
    <FieldGroup
      className="guardrail-pattern-list"
      label="Custom regex patterns"
      tooltip={props.help.field<RegexRules>("RegexRules", "rules")}
    >
      {props.patterns.map((pattern, index) => (
        <div className="guardrail-pattern-row" key={index}>
          <input
            className="mono-input"
            value={pattern}
            onChange={(event) =>
              props.onChange(
                props.patterns.map((item, itemIndex) =>
                  itemIndex === index ? event.target.value : item,
                ),
              )
            }
            placeholder="(?i)secret-[a-z0-9]+"
          />
          <button
            className="icon-button danger guardrail-pattern-remove"
            type="button"
            aria-label="Remove pattern"
            onClick={() =>
              props.onChange(
                props.patterns.filter((_, itemIndex) => itemIndex !== index),
              )
            }
          >
            <Trash2 size={16} />
          </button>
        </div>
      ))}
      <button
        className="button"
        type="button"
        onClick={() => props.onChange([...props.patterns, ""])}
      >
        <Plus size={16} />
        Add pattern
      </button>
    </FieldGroup>
  );
}

function draftFromGuardrails(guardrails: LlmGuardrail): GuardrailDraft {
  return {
    request: (guardrails.request ?? []).flatMap(draftsFromGuard),
    response: ((guardrails.response ?? []) as GuardObject[]).flatMap(
      draftsFromGuard,
    ),
  };
}

function draftsFromGuard(guard: GuardObject): GuardDraft[] {
  const base = rejectionDraft(guard);
  if ("regex" in guard && guard.regex && typeof guard.regex === "object") {
    const regex = guard.regex as RegexRulesShape;
    const rules = Array.isArray(regex.rules) ? regex.rules : [];
    const action = regex.action ?? "reject";
    const builtins = rules.flatMap((rule) =>
      "builtin" in rule ? [rule.builtin as BuiltinRule] : [],
    );
    const patterns = rules.flatMap((rule) =>
      "pattern" in rule ? [String(rule.pattern)] : [],
    );
    return [
      builtins.length
        ? ({
            ...base,
            kind: "builtin",
            action,
            builtins,
          } satisfies BuiltinGuardDraft)
        : null,
      patterns.length
        ? ({
            ...base,
            kind: "regex",
            action,
            patterns,
          } satisfies RegexGuardDraft)
        : null,
    ].filter(Boolean) as GuardDraft[];
  }
  if (
    "webhook" in guard &&
    guard.webhook &&
    typeof guard.webhook === "object"
  ) {
    const webhook = guard.webhook as Record<string, unknown>;
    return [
      {
        ...base,
        kind: "webhook",
        target: targetToHost(webhook.target),
        failureMode:
          webhook.failureMode === "failOpen" ? "failOpen" : "failClosed",
      },
    ];
  }
  if (
    "openAIModeration" in guard &&
    guard.openAIModeration &&
    typeof guard.openAIModeration === "object"
  ) {
    const moderation = guard.openAIModeration as Record<string, unknown>;
    return [
      {
        ...base,
        kind: "openAIModeration",
        model: String(moderation.model ?? ""),
        policies: moderation.policies,
      },
    ];
  }
  if (
    "bedrockGuardrails" in guard &&
    guard.bedrockGuardrails &&
    typeof guard.bedrockGuardrails === "object"
  ) {
    const bedrock = guard.bedrockGuardrails as Record<string, unknown>;
    return [
      {
        ...base,
        kind: "bedrockGuardrails",
        guardrailIdentifier: String(bedrock.guardrailIdentifier ?? ""),
        guardrailVersion: String(bedrock.guardrailVersion ?? ""),
        region: String(bedrock.region ?? ""),
        policies: bedrock.policies,
      },
    ];
  }
  if (
    "googleModelArmor" in guard &&
    guard.googleModelArmor &&
    typeof guard.googleModelArmor === "object"
  ) {
    const google = guard.googleModelArmor as Record<string, unknown>;
    return [
      {
        ...base,
        kind: "googleModelArmor",
        templateId: String(google.templateId ?? ""),
        projectId: String(google.projectId ?? ""),
        location: String(google.location ?? ""),
        policies: google.policies,
      },
    ];
  }
  if (
    "azureContentSafety" in guard &&
    guard.azureContentSafety &&
    typeof guard.azureContentSafety === "object"
  ) {
    const azure = guard.azureContentSafety as Record<string, unknown>;
    const analyze = valueRecord(azure.analyzeText);
    const jailbreak = valueRecord(azure.detectJailbreak);
    return [
      {
        ...base,
        kind: "azureContentSafety",
        endpoint: String(azure.endpoint ?? ""),
        severityThreshold:
          analyze.severityThreshold == null
            ? ""
            : String(analyze.severityThreshold),
        analyzeApiVersion: String(analyze.apiVersion ?? ""),
        blocklistNames: Array.isArray(analyze.blocklistNames)
          ? analyze.blocklistNames.join(", ")
          : "",
        haltOnBlocklistHit: Boolean(analyze.haltOnBlocklistHit),
        detectJailbreak: Boolean(azure.detectJailbreak),
        jailbreakApiVersion: String(jailbreak.apiVersion ?? ""),
        policies: azure.policies,
      },
    ];
  }
  return [{ kind: "unsupported", raw: guard }];
}

function rejectionDraft(
  guard: GuardObject,
): Pick<GuardDraftBase, "rejectionStatus" | "rejectionBody"> {
  return {
    rejectionStatus: guard.rejection?.status
      ? String(guard.rejection.status)
      : "",
    rejectionBody:
      typeof guard.rejection?.body === "string" ? guard.rejection.body : "",
  };
}

function buildGuardrails(draft: GuardrailDraft): LlmGuardrail {
  const request = draft.request.map(buildGuard);
  const response = draft.response.map(buildGuard);
  return cleanEmpty({
    request: request.length ? request : undefined,
    response: response.length ? response : undefined,
  }) as LlmGuardrail;
}

function buildGuard(guard: GuardDraft): GuardObject {
  switch (guard.kind) {
    case "unsupported":
      return guard.raw;
    case "builtin":
      return buildBuiltinGuard(guard);
    case "regex":
      return buildRegexGuard(guard);
    case "webhook":
      return withRejection(guard, {
        webhook: cleanEmpty({
          target: { host: guard.target.trim() },
          failureMode:
            guard.failureMode === "failClosed" ? undefined : guard.failureMode,
        }),
      });
    case "openAIModeration":
      return withRejection(guard, {
        openAIModeration: cleanEmpty({
          model: guard.model.trim() || undefined,
          policies: guard.policies,
        }),
      });
    case "bedrockGuardrails":
      return withRejection(guard, {
        bedrockGuardrails: cleanEmpty({
          guardrailIdentifier: guard.guardrailIdentifier.trim(),
          guardrailVersion: guard.guardrailVersion.trim(),
          region: guard.region.trim(),
          policies: guard.policies,
        }),
      });
    case "googleModelArmor":
      return withRejection(guard, {
        googleModelArmor: cleanEmpty({
          templateId: guard.templateId.trim(),
          projectId: guard.projectId.trim(),
          location: guard.location.trim() || undefined,
          policies: guard.policies,
        }),
      });
    case "azureContentSafety":
      return withRejection(guard, {
        azureContentSafety: cleanEmpty({
          endpoint: guard.endpoint.trim(),
          policies: guard.policies,
          analyzeText: cleanEmpty({
            severityThreshold: guard.severityThreshold.trim()
              ? Number(guard.severityThreshold)
              : undefined,
            apiVersion: guard.analyzeApiVersion.trim() || undefined,
            blocklistNames: commaList(guard.blocklistNames),
            haltOnBlocklistHit: guard.haltOnBlocklistHit || undefined,
          }),
          detectJailbreak: guard.detectJailbreak
            ? cleanEmpty({
                apiVersion: guard.jailbreakApiVersion.trim() || undefined,
              })
            : undefined,
        }),
      });
  }
}

function buildBuiltinGuard(guard: BuiltinGuardDraft): GuardObject {
  const rules = [...guard.builtins.map((builtin) => ({ builtin }))];
  return withRejection(guard, {
    regex: {
      action: guard.action,
      rules,
    },
  });
}

function buildRegexGuard(guard: RegexGuardDraft): GuardObject {
  return withRejection(guard, {
    regex: {
      action: guard.action,
      rules: guard.patterns
        .filter((pattern) => pattern.trim())
        .map((pattern) => ({ pattern: pattern.trim() })),
    },
  });
}

function withRejection(
  guard: GuardDraftBase,
  value: Record<string, unknown>,
): GuardObject {
  return cleanEmpty({
    ...value,
    rejection:
      guard.rejectionStatus.trim() || guard.rejectionBody.trim()
        ? {
            status: guard.rejectionStatus.trim()
              ? Number(guard.rejectionStatus)
              : undefined,
            body: guard.rejectionBody.trim() || undefined,
          }
        : undefined,
  }) as GuardObject;
}

function emptyGuardDraft(kind: GuardKind = "builtin"): SupportedGuardDraft {
  const base = { kind, rejectionStatus: "", rejectionBody: "" };
  switch (kind) {
    case "builtin":
      return { ...base, kind, action: "reject", builtins: ["email"] };
    case "regex":
      return { ...base, kind, action: "reject", patterns: [""] };
    case "webhook":
      return { ...base, kind, target: "", failureMode: "failClosed" };
    case "openAIModeration":
      return { ...base, kind, model: "" };
    case "bedrockGuardrails":
      return {
        ...base,
        kind,
        guardrailIdentifier: "",
        guardrailVersion: "",
        region: "",
      };
    case "googleModelArmor":
      return { ...base, kind, templateId: "", projectId: "", location: "" };
    case "azureContentSafety":
      return {
        ...base,
        kind,
        endpoint: "",
        severityThreshold: "",
        analyzeApiVersion: "",
        blocklistNames: "",
        haltOnBlocklistHit: false,
        detectJailbreak: false,
        jailbreakApiVersion: "",
      };
  }
}

function validateDraft(draft: GuardrailDraft) {
  const guards = [...draft.request, ...draft.response];
  for (const guard of guards) {
    if (guard.kind === "unsupported") continue;
    if (guard.kind === "builtin" && guard.builtins.length === 0) {
      return "Each built-in detector guard needs at least one detector.";
    }
    if (
      guard.kind === "regex" &&
      guard.patterns.every((pattern) => !pattern.trim())
    ) {
      return "Each custom regex guard needs at least one pattern.";
    }
    if (guard.kind === "webhook" && !guard.target.trim())
      return "Webhook guards require a target.";
    if (
      guard.kind === "bedrockGuardrails" &&
      (!guard.guardrailIdentifier.trim() ||
        !guard.guardrailVersion.trim() ||
        !guard.region.trim())
    )
      return "Bedrock guardrails require identifier, version, and region.";
    if (
      guard.kind === "googleModelArmor" &&
      (!guard.templateId.trim() || !guard.projectId.trim())
    )
      return "Google Model Armor requires template ID and project ID.";
    if (guard.kind === "azureContentSafety" && !guard.endpoint.trim())
      return "Azure Content Safety requires an endpoint.";
    if (
      guard.kind === "azureContentSafety" &&
      guard.severityThreshold.trim() &&
      (!Number.isInteger(Number(guard.severityThreshold)) ||
        Number(guard.severityThreshold) < 0 ||
        Number(guard.severityThreshold) > 6)
    ) {
      return "Azure severity threshold must be an integer from 0 to 6.";
    }
    if (
      guard.rejectionStatus.trim() &&
      (!Number.isInteger(Number(guard.rejectionStatus)) ||
        Number(guard.rejectionStatus) < 100 ||
        Number(guard.rejectionStatus) > 599)
    ) {
      return "Rejection status must be a valid HTTP status code.";
    }
  }
  return null;
}

function valueRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function targetToHost(value: unknown) {
  const target = valueRecord(value);
  if (typeof target.host === "string") return target.host;
  if (typeof target.backend === "string") return target.backend;
  return "";
}

function commaList(value: string) {
  const items = value
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
  return items.length ? items : undefined;
}

function guardKindLabel(kind: GuardDraft["kind"]) {
  if (kind === "unsupported") return "Unsupported guard";
  return requestGuardKinds.find((item) => item.value === kind)?.label ?? kind;
}

function guardKindIcon(kind: GuardDraft["kind"]) {
  if (kind === "unsupported") return <Braces size={16} />;
  return (
    requestGuardKinds.find((item) => item.value === kind)?.icon ?? (
      <ShieldCheck size={16} />
    )
  );
}

function guardTypeHelp(phase: GuardPhase, help: SchemaHelp) {
  return help.definition(
    phase === "request" ? "RequestGuard" : "ResponseGuard",
    "Select the guardrail integration or rule type to apply.",
  );
}

function guardDrawerIndex(
  value: string | null,
  phase: GuardPhase,
  guardCount: number,
) {
  if (!value) return null;
  const [valuePhase, rawIndex] = value.split(":");
  if (valuePhase !== phase || rawIndex === "new") return null;
  const index = Number(rawIndex);
  return Number.isInteger(index) && index >= 0 && index < guardCount
    ? index
    : null;
}

function guardSummary(guard: GuardDraft) {
  if (guard.kind === "unsupported")
    return "Raw guard YAML is preserved. Use Raw Configuration for unsupported edits.";
  const rejection = guard.rejectionStatus.trim()
    ? ` Rejects with ${guard.rejectionStatus.trim()}.`
    : "";
  switch (guard.kind) {
    case "builtin":
      return `${capitalize(guard.action)} ${guard.builtins.length} built-in detector${guard.builtins.length === 1 ? "" : "s"}.${rejection}`;
    case "regex":
      return `${capitalize(guard.action)} ${guard.patterns.filter((pattern) => pattern.trim()).length} regex pattern${guard.patterns.filter((pattern) => pattern.trim()).length === 1 ? "" : "s"}.${rejection}`;
    case "webhook":
      return guard.target.trim()
        ? `${guard.target.trim()} · ${guard.failureMode === "failOpen" ? "fail open" : "fail closed"}.${rejection}`
        : `Webhook target not set.${rejection}`;
    case "openAIModeration":
      return guard.model.trim()
        ? `Model ${guard.model.trim()}.${rejection}`
        : `Default moderation model.${rejection}`;
    case "bedrockGuardrails":
      return (
        [guard.guardrailIdentifier, guard.guardrailVersion, guard.region]
          .filter(Boolean)
          .join(" · ") || "Bedrock guardrail details not set."
      );
    case "googleModelArmor":
      return (
        [guard.templateId, guard.projectId, guard.location]
          .filter(Boolean)
          .join(" · ") || "Model Armor details not set."
      );
    case "azureContentSafety":
      return guard.endpoint.trim()
        ? `${guard.endpoint.trim()}${guard.detectJailbreak ? " · jailbreak detection" : ""}.${rejection}`
        : `Azure endpoint not set.${rejection}`;
  }
}

function capitalize(value: string) {
  return value ? value[0].toUpperCase() + value.slice(1) : value;
}
