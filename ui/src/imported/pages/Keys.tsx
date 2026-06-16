import { useMemo, useState } from "react";
import { Eye, EyeOff, KeyRound, Pencil, Plus, Save, SlidersHorizontal, Trash2, X } from "lucide-react";
import { getApiKeyPolicy, removeVirtualKey, upsertVirtualKey } from "../config";
import { maskKey } from "../credentialDisplay";
import { useGatewayConfig, useUpdateConfig } from "../_adapters/hooks";
import { Drawer, Dropdown, EmptyState, Field, FieldGroup, PageHeader, Panel, StatusBanner, Tooltip } from "../components/Primitives";
import { headerLocationFrom } from "../policies/HeaderLocationOverride";
import { AdvancedSettingPanel, AdvancedSettingRow } from "../policies/PolicyLayout";
import { enumOptionDetails, parseYamlText, toYamlText } from "../policies/policyUtils";
import type { SchemaNode } from "../policies/types";
import { useSchemaHelp, type SchemaHelp } from "../schemaHelp";
import type { LlmApiKeyPolicy, VirtualApiKey } from "../types";

export function KeysPage() {
  const config = useGatewayConfig();
  const update = useUpdateConfig();
  const help = useSchemaHelp();
  const policy = useMemo(() => config.data?.llm?.policies?.apiKey, [config.data]);
  const keys = policy?.keys ?? [];
  const [editing, setEditing] = useState<{ previousKey?: string; key: VirtualApiKey } | null>(null);
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [shownKeys, setShownKeys] = useState<Set<string>>(() => new Set());

  return (
    <div className="page-stack">
      <PageHeader
        title="Virtual API Keys"
        description="Provision incoming credentials and metadata for callers."
        actions={
          <div className="button-row">
            <button className="button" type="button" onClick={() => setAdvancedOpen(true)}>
              <SlidersHorizontal size={16} />
              Settings
            </button>
            <button className="button primary" type="button" onClick={() => setEditing({ key: newVirtualKey() })}>
              <Plus size={16} />
              New key
            </button>
          </div>
        }
      />

      {update.isError ? <StatusBanner state="bad" title="Save failed">{update.error!.message}</StatusBanner> : null}
      {policy?.mode && policy.mode !== "strict" ? (
        <StatusBanner state="warn" title={`Policy mode is ${modeLabel(policy.mode)}`}>
          Use strict mode when keys should be mandatory.
        </StatusBanner>
      ) : null}

      <Panel>
        {config.isLoading ? (
          <StatusBanner state="loading" title="Loading keys" />
        ) : config.isError ? (
          <StatusBanner state="bad" title="Configuration API unavailable">{config.error!.message}</StatusBanner>
        ) : keys.length === 0 ? (
          <EmptyState
            title="No virtual API keys"
            description="Create a key so callers can authenticate without exposing provider credentials."
            action={
              <button className="button primary" type="button" onClick={() => setEditing({ key: newVirtualKey() })}>
                <KeyRound size={16} />
                New key
              </button>
            }
          />
        ) : (
          <div className="table-wrap">
            <table className="keys-table">
              <thead>
                <tr>
                  <th>Name</th>
                  <th>Key</th>
                  <th>Metadata</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {keys.map((item) => (
                  <tr key={item.key}>
                    <td className="strong key-name-cell">{keyName(item) || "Unnamed key"}</td>
                    <td className="key-cell">
                      <code>{shownKeys.has(item.key) ? item.key : maskKey(item.key)}</code>
                    </td>
                    <td><MetadataSummary value={item.metadata} /></td>
                    <td className="key-action-cell">
                      <div className="key-actions">
                        <Tooltip content={shownKeys.has(item.key) ? "Hide full key" : "Show full key"}>
                          <button
                            className="table-action"
                            type="button"
                            aria-label={shownKeys.has(item.key) ? "Hide full key" : "Show full key"}
                            onClick={() => setShownKeys((current) => toggleSet(current, item.key))}
                          >
                            {shownKeys.has(item.key) ? <EyeOff size={14} /> : <Eye size={14} />}
                            {shownKeys.has(item.key) ? "Hide" : "Show"}
                          </button>
                        </Tooltip>
                        <Tooltip content="Edit key">
                          <button className="table-action" type="button" aria-label="Edit key" onClick={() => setEditing({ previousKey: item.key, key: structuredClone(item) })}>
                            <Pencil size={14} />
                            Edit
                          </button>
                        </Tooltip>
                        <Tooltip content="Delete key">
                          <button className="table-action danger" type="button" aria-label="Delete key" onClick={() => update.mutate((next) => removeVirtualKey(next, item.key))}>
                            <Trash2 size={14} />
                            Delete
                          </button>
                        </Tooltip>
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Panel>

      {editing ? (
        <KeyEditor
          initial={editing.key}
          previousKey={editing.previousKey}
          help={help}
          saving={update.isPending}
          saveError={update.isError ? update.error!.message : null}
          onCancel={() => setEditing(null)}
          onSave={(key, previousKey) => update.mutate((next) => upsertVirtualKey(next, key, previousKey), {
            onSuccess: () => {
              setShownKeys((current) => new Set(current).add(key.key));
              setEditing(null);
            },
          })}
        />
      ) : null}
      {advancedOpen ? (
        <AdvancedSettingsDrawer
          policy={policy}
          help={help}
          saving={update.isPending}
          saveError={update.isError ? update.error!.message : null}
          onClose={() => setAdvancedOpen(false)}
          onSave={(nextPolicy) => update.mutate((next) => {
            const apiKey = getApiKeyPolicy(next);
            Object.assign(apiKey, nextPolicy);
          }, {
            onSuccess: () => setAdvancedOpen(false),
          })}
        />
      ) : null}
    </div>
  );
}

function AdvancedSettingsDrawer(props: {
  policy?: LlmApiKeyPolicy | null;
  help: SchemaHelp;
  saving: boolean;
  saveError?: string | null;
  onClose: () => void;
  onSave: (policy: Partial<LlmApiKeyPolicy>) => void;
}) {
  return (
    <Drawer title="Settings" onClose={props.onClose}>
      <PolicyControls
        policy={props.policy}
        help={props.help}
        saving={props.saving}
        onSave={props.onSave}
      />
      {props.saveError ? <StatusBanner state="bad" title="Save failed">{props.saveError}</StatusBanner> : null}
    </Drawer>
  );
}

function PolicyControls(props: { policy?: LlmApiKeyPolicy | null; help: SchemaHelp; saving: boolean; onSave: (policy: Partial<LlmApiKeyPolicy>) => void }) {
  const [mode, setMode] = useState(props.policy?.mode ?? "strict");
  const header = headerLocationFrom(props.policy?.location);
  const [customHeaderLocation, setCustomHeaderLocation] = useState(Boolean(header));
  const [headerName, setHeaderName] = useState(header?.header.name ?? "authorization");
  const [prefix, setPrefix] = useState(header?.header.prefix ?? "Bearer ");
  const modeOptions = validationModeOptions(props.help);

  return (
    <div className="policy-controls api-key-policy-controls">
      <FieldGroup
        label="Validation mode"
        tooltip={props.help.description(["$defs", "LocalAPIKeys", "properties", "mode"], "Controls whether incoming requests must present a configured virtual API key.")}
      >
        <Dropdown
          ariaLabel="Validation mode"
          value={mode}
          options={modeOptions}
          onChange={(value) => setMode(value as "strict" | "optional" | "permissive")}
        />
      </FieldGroup>
      <ApiKeyLocationSetting
        help={props.help}
        enabled={customHeaderLocation}
        headerName={headerName}
        headerPrefix={prefix}
        onEnabledChange={setCustomHeaderLocation}
        onHeaderNameChange={setHeaderName}
        onHeaderPrefixChange={setPrefix}
      />
      <button
        className="button"
        type="button"
        disabled={props.saving}
        onClick={() => props.onSave({
          mode,
          location: customHeaderLocation ? { header: { name: headerName, prefix } } : undefined,
        })}
      >
        <Save size={16} />
        Save policy
      </button>
    </div>
  );
}

function ApiKeyLocationSetting(props: {
  help: SchemaHelp;
  enabled: boolean;
  headerName: string;
  headerPrefix: string;
  onEnabledChange: (enabled: boolean) => void;
  onHeaderNameChange: (value: string) => void;
  onHeaderPrefixChange: (value: string) => void;
}) {
  if (!props.enabled) {
    return (
      <AdvancedSettingRow
        className="api-key-location-row"
        icon={<KeyRound size={17} />}
        title="Credential location"
        description={props.help.description(["$defs", "LocalAPIKeys", "properties", "location"], "By default, callers send Authorization: Bearer key.") ?? "By default, callers send Authorization: Bearer key."}
        action={
          <button className="button compact-action" type="button" onClick={() => props.onEnabledChange(true)}>
            <SlidersHorizontal size={15} />
            Customize
          </button>
        }
      />
    );
  }

  return (
    <AdvancedSettingPanel
      className="api-key-location-panel"
      icon={<KeyRound size={17} />}
      title="Credential location"
      description={props.help.description(["$defs", "AuthorizationLocation", "oneOf", 0], "Customize the request header used to read virtual API keys.") ?? "Customize the request header used to read virtual API keys."}
      action={
        <button className="button" type="button" onClick={() => props.onEnabledChange(false)}>
          <X size={15} />
          Use default
        </button>
      }
    >
      <div className="api-key-location-fields">
        <Field label="Header name" tooltip={props.help.description(["$defs", "AuthorizationLocation", "oneOf", 0, "properties", "header", "properties", "name"])}>
          <input value={props.headerName} onChange={(event) => props.onHeaderNameChange(event.target.value)} placeholder="authorization" />
        </Field>
        <Field label="Header prefix" tooltip={props.help.description(["$defs", "AuthorizationLocation", "oneOf", 0, "properties", "header", "properties", "prefix"])}>
          <input value={props.headerPrefix} onChange={(event) => props.onHeaderPrefixChange(event.target.value)} placeholder="Bearer " />
        </Field>
      </div>
    </AdvancedSettingPanel>
  );
}

function KeyEditor(props: {
  initial: VirtualApiKey;
  previousKey?: string;
  help: SchemaHelp;
  saving: boolean;
  saveError?: string | null;
  onCancel: () => void;
  onSave: (key: VirtualApiKey, previousKey?: string) => void;
}) {
  const isNew = !props.previousKey;
  const initialMetadata = metadataObject(props.initial.metadata);
  const [name, setName] = useState(String(initialMetadata.name ?? ""));
  const [keyMode, setKeyMode] = useState<"auto" | "custom">(isNew ? "auto" : "custom");
  const [key, setKey] = useState(isNew ? "" : props.initial.key);
  const [metadataText, setMetadataText] = useState(metadataYamlText(withoutName(initialMetadata)));
  const [error, setError] = useState<string | null>(null);

  function save() {
    try {
      const metadata = {
        ...(metadataText.trim() ? parseMetadataYaml(metadataText) : {}),
        ...(name.trim() ? { name: name.trim() } : {}),
      };
      const nextKey = keyMode === "auto" ? `agw_sk_${randomKey(32)}` : key;
      setError(null);
      props.onSave({ key: nextKey, metadata }, props.previousKey);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Invalid metadata YAML");
    }
  }

  return (
    <Drawer
      title={props.previousKey ? "Edit virtual key" : "Create virtual key"}
      onClose={props.onCancel}
      footer={
        <div className="button-row">
          <button className="button" type="button" onClick={props.onCancel}>Cancel</button>
          <button className="button primary" type="button" disabled={props.saving || (keyMode === "custom" && !key.trim())} onClick={save}>
            <Save size={16} />
            Save key
          </button>
        </div>
      }
    >
      <Field label="Name">
        <input value={name} onChange={(event) => setName(event.target.value)} placeholder="Platform team" />
      </Field>
      {isNew ? (
        <FieldGroup label="Key value">
          <Dropdown
            ariaLabel="Key value"
            value={keyMode}
            options={[
              { value: "auto", label: "agw_sk_***** (auto generate)" },
              { value: "custom", label: "Use custom key" },
            ]}
            onChange={(value) => setKeyMode(value as "auto" | "custom")}
          />
        </FieldGroup>
      ) : null}
      {keyMode === "custom" ? (
        <Field label="Key value">
          <input value={key} onChange={(event) => setKey(event.target.value)} placeholder="agw_sk_..." />
        </Field>
      ) : null}
      <Field
        label="Metadata YAML"
        tooltip={props.help.description(["$defs", "LocalAPIKey", "properties", "metadata"])}
        hint="Attached to requests authenticated with this key for policies, logs, and routing context."
      >
        <textarea
          className="mono-input"
          rows={8}
          value={metadataText}
          onChange={(event) => setMetadataText(event.target.value)}
          placeholder={"owner: platform\nteam: inference\ntier: prod"}
        />
      </Field>
      {error ? <StatusBanner state="bad" title="Invalid metadata">{error}</StatusBanner> : null}
      {props.saveError ? <StatusBanner state="bad" title="Save failed">{props.saveError}</StatusBanner> : null}
    </Drawer>
  );
}

function newVirtualKey(): VirtualApiKey {
  return {
    key: "",
    metadata: { name: "" },
  };
}

function randomKey(length: number) {
  const alphabet = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
  const bytes = new Uint8Array(length);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (byte) => alphabet[byte % alphabet.length]).join("");
}

function validationModeOptions(help: SchemaHelp) {
  const details = enumOptionDetails(help.node(["$defs", "Mode3"]) as SchemaNode | undefined);
  const options = details.length ? details : [
    { value: "strict", label: "strict", description: "Require a valid API key." },
    { value: "optional", label: "optional", description: "Validate the API key when present. Allows requests without an API key." },
    { value: "permissive", label: "permissive", description: "Decode valid API keys for later policy use. Allows missing or invalid API keys." },
  ];
  return options.map((option) => ({
    value: option.value,
    label: modeLabel(option.label),
    description: option.description,
  }));
}

function modeLabel(mode: string) {
  const labels: Record<string, string> = {
    strict: "Strict",
    optional: "Optional",
    permissive: "Permissive",
  };
  return labels[mode] ?? mode;
}

function keyName(key: VirtualApiKey) {
  const metadata = metadataObject(key.metadata);
  return typeof metadata.name === "string" ? metadata.name : "";
}

function MetadataSummary(props: { value: unknown }) {
  const metadata = withoutName(metadataObject(props.value));
  const entries = Object.entries(metadata);
  if (!entries.length) return <span className="muted">none</span>;
  return (
    <div className="metadata-summary">
      {entries.slice(0, 3).map(([key, value]) => (
        <span className="badge" key={key}>{key}: {String(value)}</span>
      ))}
      {entries.length > 3 ? <span className="muted">+{entries.length - 3}</span> : null}
    </div>
  );
}

function metadataObject(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : {};
}

function withoutName(value: Record<string, unknown>) {
  const next = { ...value };
  delete next.name;
  return next;
}

function metadataYamlText(value: Record<string, unknown>) {
  return Object.keys(value).length ? toYamlText(value) : "";
}

function parseMetadataYaml(value: string) {
  const parsed = parseYamlText(value);
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("Metadata must be a YAML mapping.");
  }
  return parsed as Record<string, unknown>;
}

function toggleSet(set: Set<string>, key: string) {
  const next = new Set(set);
  if (next.has(key)) next.delete(key);
  else next.add(key);
  return next;
}
