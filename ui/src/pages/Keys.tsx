import { useMemo, useState } from "react";
import {
  Check,
  Copy,
  Eye,
  EyeOff,
  KeyRound,
  Pencil,
  Plus,
  Save,
  SlidersHorizontal,
  Trash2,
  X,
} from "lucide-react";
import {
  disableApiKeyPolicy,
  getApiKeyPolicy,
  removeVirtualKey,
  upsertVirtualKey,
} from "../config";
import { EnumSelector } from "../components/EnumSelector";
import { hasKeyValue, keyValue, maskKey } from "../credentialDisplay";
import { useStickyQueryParam } from "../drawerRouteState";
import { useGatewayConfig, useUpdateConfig } from "../hooks";
import {
  ConfirmDialog,
  Drawer,
  Dropdown,
  EmptyState,
  Field,
  FieldGroup,
  PageHeader,
  Panel,
  StatusBanner,
  Tooltip,
} from "../components/Primitives";
import { headerLocationFrom } from "../policies/HeaderLocationOverride";
import {
  AdvancedSettingPanel,
  AdvancedSettingRow,
} from "../policies/PolicyLayout";
import { KeyValueEditor } from "../policies/PolicyFormControls";
import { useSchemaHelp, type SchemaHelp } from "../schemaHelp";
import type { LlmApiKeyPolicy, VirtualApiKey } from "../types";
import type { AuthorizationLocation } from "../gateway-config";

export function KeysPage() {
  const config = useGatewayConfig();
  const update = useUpdateConfig();
  const help = useSchemaHelp();
  const policy = useMemo(
    () => config.data?.llm?.policies?.apiKey,
    [config.data],
  );
  const keys = policy?.keys ?? [];
  const [editing, setEditing] = useState<{
    previousKey?: string;
    key: VirtualApiKey;
  } | null>(null);
  const [deleteKey, setDeleteKey] = useState<VirtualApiKey | null>(null);
  const [disablePolicyOpen, setDisablePolicyOpen] = useState(false);
  const [keyDrawer, setKeyDrawer] = useStickyQueryParam("key");
  const linkedKey = linkedVirtualKey(keyDrawer, keys);
  const activeEditing =
    editing ??
    (keyDrawer === "new"
      ? { key: newVirtualKey() }
      : linkedKey
        ? { previousKey: keyValue(linkedKey), key: structuredClone(linkedKey) }
        : null);
  const advancedOpen = keyDrawer === "settings";

  function openNewKey() {
    setEditing(null);
    setKeyDrawer("new");
  }

  function openEditKey(key: VirtualApiKey, index: number) {
    setEditing(null);
    setKeyDrawer(virtualKeyUrlRef(key, index));
  }

  function closeKeyDrawer() {
    setEditing(null);
    setKeyDrawer(null, "replace");
  }

  function disablePolicy() {
    update.mutate((next) => disableApiKeyPolicy(next), {
      onSuccess: closeKeyDrawer,
    });
  }

  return (
    <div className="page-stack">
      <PageHeader
        title="Virtual API Keys"
        description="Provision incoming credentials and metadata for callers."
        actions={
          <div className="button-row">
            <button
              className="button"
              type="button"
              onClick={() => setKeyDrawer("settings")}
            >
              <SlidersHorizontal size={16} />
              Settings
            </button>
            <button
              className="button primary"
              type="button"
              onClick={openNewKey}
            >
              <Plus size={16} />
              New key
            </button>
          </div>
        }
      />

      {update.isError ? (
        <StatusBanner state="bad" title="Save failed">
          {update.error.message}
        </StatusBanner>
      ) : null}
      {policy?.mode && policy.mode !== "strict" ? (
        <StatusBanner
          state="warn"
          title={`Policy mode is ${modeLabel(policy.mode)}`}
        >
          Use strict mode when keys should be mandatory.
        </StatusBanner>
      ) : null}

      <Panel>
        {config.isLoading ? (
          <StatusBanner state="loading" title="Loading keys" />
        ) : config.isError ? (
          <StatusBanner state="bad" title="Configuration API unavailable">
            {config.error.message}
          </StatusBanner>
        ) : keys.length === 0 ? (
          <EmptyState
            title="No virtual API keys"
            description="Create a key so callers can authenticate without exposing provider credentials."
            action={
              <div className="button-row">
                {policy ? (
                  <button
                    className="button danger"
                    type="button"
                    disabled={update.isPending}
                    onClick={() => setDisablePolicyOpen(true)}
                  >
                    <X size={16} />
                    Disable API Key Policy
                  </button>
                ) : null}
                <button
                  className="button primary"
                  type="button"
                  onClick={openNewKey}
                >
                  <Plus size={16} />
                  New key
                </button>
              </div>
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
                {keys.map((item, index) => (
                  <tr key={keyValue(item)}>
                    <td className="strong key-name-cell">
                      {keyName(item) || "Unnamed key"}
                    </td>
                    <td className="key-cell">
                      <VirtualKeyValue value={keyValue(item)} />
                    </td>
                    <td>
                      <MetadataSummary value={item.metadata} />
                    </td>
                    <td className="key-action-cell">
                      <div className="key-actions">
                        <Tooltip content="Edit key">
                          <button
                            className="table-action"
                            type="button"
                            aria-label="Edit key"
                            onClick={() => openEditKey(item, index)}
                          >
                            <Pencil size={14} />
                            Edit
                          </button>
                        </Tooltip>
                        <Tooltip content="Delete key">
                          <button
                            className="table-action danger"
                            type="button"
                            aria-label="Delete key"
                            onClick={() => setDeleteKey(item)}
                          >
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

      {activeEditing ? (
        <KeyEditor
          key={activeEditing.previousKey ?? "new"}
          initial={activeEditing.key}
          previousKey={activeEditing.previousKey}
          help={help}
          existingKeys={keys}
          saving={update.isPending}
          saveError={update.isError ? update.error.message : null}
          onCancel={closeKeyDrawer}
          onSave={(key, previousKey) =>
            update.mutate((next) => upsertVirtualKey(next, key, previousKey), {
              onSuccess: closeKeyDrawer,
            })
          }
        />
      ) : null}
      {deleteKey ? (
        <ConfirmDialog
          title="Delete virtual API key?"
          destructive
          confirmLabel="Delete key"
          confirmDisabled={update.isPending}
          onCancel={() => setDeleteKey(null)}
          onConfirm={() => {
            const value = keyValue(deleteKey);
            update.mutate((next) => removeVirtualKey(next, value), {
              onSuccess: () => setDeleteKey(null),
            });
          }}
        >
          <p>
            Delete <strong>{virtualKeyDeleteLabel(deleteKey)}</strong>? This
            cannot be undone.
          </p>
        </ConfirmDialog>
      ) : null}
      {disablePolicyOpen ? (
        <ConfirmDialog
          title="Disable API key policy?"
          destructive
          confirmLabel="Disable API Key Policy"
          confirmDisabled={update.isPending}
          onCancel={() => setDisablePolicyOpen(false)}
          onConfirm={() => {
            update.mutate((next) => disableApiKeyPolicy(next), {
              onSuccess: () => {
                setDisablePolicyOpen(false);
                closeKeyDrawer();
              },
            });
          }}
        >
          <p>
            Disable virtual API key validation? Requests will no longer be
            validated against virtual API keys.
          </p>
        </ConfirmDialog>
      ) : null}
      {advancedOpen ? (
        <AdvancedSettingsDrawer
          policy={policy}
          keyCount={keys.length}
          help={help}
          saving={update.isPending}
          saveError={update.isError ? update.error.message : null}
          onClose={closeKeyDrawer}
          onDisable={disablePolicy}
          onSave={(nextPolicy) =>
            update.mutate(
              (next) => {
                const apiKey = getApiKeyPolicy(next);
                Object.assign(apiKey, nextPolicy);
              },
              {
                onSuccess: closeKeyDrawer,
              },
            )
          }
        />
      ) : null}
    </div>
  );
}

function AdvancedSettingsDrawer(props: {
  policy?: LlmApiKeyPolicy | null;
  keyCount: number;
  help: SchemaHelp;
  saving: boolean;
  saveError?: string | null;
  onClose: () => void;
  onDisable: () => void;
  onSave: (policy: Partial<LlmApiKeyPolicy>) => void;
}) {
  return (
    <Drawer title="Settings" onClose={props.onClose}>
      <PolicyControls
        policy={props.policy}
        keyCount={props.keyCount}
        help={props.help}
        saving={props.saving}
        onDisable={props.onDisable}
        onSave={props.onSave}
      />
      {props.saveError ? (
        <StatusBanner state="bad" title="Save failed">
          {props.saveError}
        </StatusBanner>
      ) : null}
    </Drawer>
  );
}

function PolicyControls(props: {
  policy?: LlmApiKeyPolicy | null;
  keyCount: number;
  help: SchemaHelp;
  saving: boolean;
  onDisable: () => void;
  onSave: (policy: Partial<LlmApiKeyPolicy>) => void;
}) {
  const [mode, setMode] = useState(props.policy?.mode ?? "strict");
  const header = headerLocationFrom(props.policy?.location);
  const [customHeaderLocation, setCustomHeaderLocation] = useState(
    Boolean(header),
  );
  const [headerName, setHeaderName] = useState(
    header?.header.name ?? "authorization",
  );
  const [prefix, setPrefix] = useState(header?.header.prefix ?? "Bearer ");
  return (
    <div className="policy-controls api-key-policy-controls">
      <FieldGroup
        label="Validation mode"
        tooltip={props.help.field<LlmApiKeyPolicy>(
          "LocalAPIKeys",
          "mode",
          "Controls whether incoming requests must present a configured virtual API key.",
        )}
      >
        <EnumSelector
          ariaLabel="Validation mode"
          value={mode}
          schema={props.help.node(["$defs", "Mode3"])}
          labels={{
            strict: "Strict",
            optional: "Optional",
            permissive: "Permissive",
          }}
          onChange={(value) =>
            setMode(value as "strict" | "optional" | "permissive")
          }
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
      {props.policy && props.keyCount === 0 ? (
        <AdvancedSettingRow
          className="api-key-location-row"
          icon={<X size={17} />}
          title="Disable API key policy"
          description="Remove the API key policy entirely. Requests will not be validated against virtual API keys."
          action={
            <button
              className="button danger compact-action"
              type="button"
              disabled={props.saving}
              onClick={props.onDisable}
            >
              Disable
            </button>
          }
        />
      ) : null}
      <button
        className="button"
        type="button"
        disabled={props.saving}
        onClick={() =>
          props.onSave({
            mode,
            location: customHeaderLocation
              ? { header: { name: headerName, prefix } }
              : undefined,
          })
        }
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
        description={
          props.help.field<LlmApiKeyPolicy>(
            "LocalAPIKeys",
            "location",
            "By default, callers send Authorization: Bearer key.",
          ) ?? "By default, callers send Authorization: Bearer key."
        }
        action={
          <button
            className="button compact-action"
            type="button"
            onClick={() => props.onEnabledChange(true)}
          >
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
      description={
        props.help.definition(
          "AuthorizationLocation",
          "Customize the request header used to read virtual API keys.",
        ) ?? "Customize the request header used to read virtual API keys."
      }
      action={
        <button
          className="button"
          type="button"
          onClick={() => props.onEnabledChange(false)}
        >
          <X size={15} />
          Use default
        </button>
      }
    >
      <div className="api-key-location-fields">
        <Field
          label="Header name"
          tooltip={props.help.field<AuthorizationLocation>(
            "AuthorizationLocation",
            "header.name",
          )}
        >
          <input
            value={props.headerName}
            onChange={(event) => props.onHeaderNameChange(event.target.value)}
            placeholder="authorization"
          />
        </Field>
        <Field
          label="Header prefix"
          tooltip={props.help.field<AuthorizationLocation>(
            "AuthorizationLocation",
            "header.prefix",
          )}
        >
          <input
            value={props.headerPrefix}
            onChange={(event) => props.onHeaderPrefixChange(event.target.value)}
            placeholder="Bearer "
          />
        </Field>
      </div>
    </AdvancedSettingPanel>
  );
}

function KeyEditor(props: {
  initial: VirtualApiKey;
  previousKey?: string;
  help: SchemaHelp;
  existingKeys: VirtualApiKey[];
  saving: boolean;
  saveError?: string | null;
  onCancel: () => void;
  onSave: (key: VirtualApiKey, previousKey?: string) => void;
}) {
  const isNew = !props.previousKey;
  const initialMetadata = metadataObject(props.initial.metadata);
  const [name, setName] = useState(String(initialMetadata.name ?? ""));
  const [keyMode, setKeyMode] = useState<"auto" | "custom">(
    isNew ? "auto" : "custom",
  );
  const [key, setKey] = useState(
    isNew || !hasKeyValue(props.initial) ? "" : props.initial.key,
  );
  const [replaceKey, setReplaceKey] = useState(false);
  const [metadataValues, setMetadataValues] = useState(() =>
    stringMetadata(withoutManagedMetadata(initialMetadata)),
  );
  const [submitted, setSubmitted] = useState(false);
  const nameRequired = isNew && !name.trim();
  const duplicateName = isNew
    ? duplicateKeyName(name, props.existingKeys)
    : false;

  function save() {
    setSubmitted(true);
    if (nameRequired) return;
    const metadataId =
      typeof initialMetadata.id === "string" && initialMetadata.id.trim()
        ? initialMetadata.id.trim()
        : randomUuid();
    const metadata = {
      ...metadataValues,
      id: metadataId,
      ...(name.trim() ? { name: name.trim() } : {}),
    };
    const nextKey = isNew
      ? keyMode === "auto"
        ? `agw_sk_${randomKey(32)}`
        : key
      : replaceKey
        ? key
        : "";
    props.onSave(
      isNew || replaceKey
        ? { key: nextKey, metadata }
        : { ...props.initial, metadata },
      props.previousKey,
    );
  }

  return (
    <Drawer
      title={props.previousKey ? "Edit virtual key" : "Create virtual key"}
      onClose={props.onCancel}
      footer={
        <div className="button-row">
          <button className="button" type="button" onClick={props.onCancel}>
            Cancel
          </button>
          <button
            className="button primary"
            type="button"
            disabled={props.saving || (keyMode === "custom" && !key.trim())}
            onClick={save}
          >
            <Save size={16} />
            Save key
          </button>
        </div>
      }
    >
      <Field label="Name">
        <input
          value={name}
          onChange={(event) => setName(event.target.value)}
          placeholder="Platform team"
        />
      </Field>
      {submitted && nameRequired ? (
        <StatusBanner state="bad" title="Name is required">
          Add a name before creating this virtual API key.
        </StatusBanner>
      ) : null}
      {duplicateName ? (
        <StatusBanner state="warn" title="Name already exists">
          Another virtual key already uses this name. The key will still be
          created with a unique metadata id.
        </StatusBanner>
      ) : null}
      {isNew ? (
        <FieldGroup
          label="Key value"
          tooltip={props.help.field<VirtualApiKey>("LocalAPIKey", "key")}
        >
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
      ) : (
        <FieldGroup
          label="Key value"
          tooltip={props.help.field<VirtualApiKey>("LocalAPIKey", "key")}
        >
          <div className="key-editor-value-row">
            <VirtualKeyValue value={keyValue(props.initial)} />
            <button
              className="button"
              type="button"
              onClick={() => setReplaceKey((current) => !current)}
            >
              {replaceKey ? "Keep existing" : "Replace key"}
            </button>
          </div>
        </FieldGroup>
      )}
      {(isNew && keyMode === "custom") || (!isNew && replaceKey) ? (
        <Field
          label="Key value"
          tooltip={props.help.field<VirtualApiKey>("LocalAPIKey", "key")}
        >
          <input
            value={key}
            type="text"
            className="masked-secret-input"
            autoComplete="off"
            autoCorrect="off"
            autoCapitalize="none"
            data-1p-ignore="true"
            data-lpignore="true"
            data-form-type="other"
            name="agw-virtual-api-key"
            spellCheck={false}
            onChange={(event) => setKey(event.target.value)}
            placeholder="agw_sk_..."
          />
        </Field>
      ) : null}
      <KeyValueEditor
        label="Metadata"
        tooltip={props.help.field<VirtualApiKey>("LocalAPIKey", "metadata")}
        values={metadataValues}
        quickKeys={["user", "group"]}
        keyPlaceholder="owner"
        valuePlaceholder="platform"
        onChange={setMetadataValues}
      />
      {props.saveError ? (
        <StatusBanner state="bad" title="Save failed">
          {props.saveError}
        </StatusBanner>
      ) : null}
    </Drawer>
  );
}

function newVirtualKey(): VirtualApiKey {
  return {
    key: "",
    metadata: { id: randomUuid(), name: "" },
  };
}

function randomKey(length: number) {
  const alphabet =
    "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
  const bytes = new Uint8Array(length);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (byte) => alphabet[byte % alphabet.length]).join("");
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

function virtualKeyDeleteLabel(key: VirtualApiKey) {
  const name = keyName(key).trim();
  return name || maskKey(keyValue(key));
}

function duplicateKeyName(name: string, keys: VirtualApiKey[]) {
  const normalized = normalizeKeyName(name);
  if (!normalized) return false;
  return keys.some((key) => normalizeKeyName(keyName(key)) === normalized);
}

function normalizeKeyName(name: string) {
  return name.trim().toLowerCase();
}

function keyId(key: VirtualApiKey) {
  const metadata = metadataObject(key.metadata);
  return typeof metadata.id === "string" && metadata.id.trim()
    ? metadata.id.trim()
    : "";
}

function virtualKeyUrlRef(key: VirtualApiKey, index: number) {
  const id = keyId(key);
  if (id) return `id:${id}`;
  const name = keyName(key).trim();
  return name ? `name:${name}` : `index:${index}`;
}

function linkedVirtualKey(value: string | null, keys: VirtualApiKey[]) {
  if (!value || value === "new" || value === "settings") return null;
  if (value.startsWith("id:")) {
    const id = value.slice("id:".length);
    return keys.find((key) => keyId(key) === id) ?? null;
  }
  if (value.startsWith("name:")) {
    const name = value.slice("name:".length);
    return keys.find((key) => keyName(key) === name) ?? null;
  }
  if (value.startsWith("index:")) {
    const index = Number(value.slice("index:".length));
    return Number.isInteger(index) ? (keys[index] ?? null) : null;
  }
  return null;
}

async function copyVirtualKey(key: string): Promise<boolean> {
  if (navigator.clipboard) {
    try {
      await navigator.clipboard.writeText(key);
      return true;
    } catch {
      // fall through to execCommand fallback
    }
  }
  // Fallback for non-secure contexts (HTTP, non-localhost)
  try {
    const el = document.createElement("textarea");
    el.value = key;
    el.style.cssText = "position:fixed;left:-9999px;top:0;opacity:0";
    document.body.appendChild(el);
    el.select();
    const success = document.execCommand("copy");
    document.body.removeChild(el);
    return success;
  } catch {
    return false;
  }
}

function VirtualKeyValue(props: { value: string }) {
  const [shown, setShown] = useState(false);
  const [copied, setCopied] = useState(false);
  return (
    <div className="virtual-key-value">
      <code>{shown ? props.value : maskKey(props.value)}</code>
      <div className="virtual-key-value-actions">
        <Tooltip content={shown ? "Hide full key" : "Show full key"}>
          <button
            className="table-action"
            type="button"
            aria-label={shown ? "Hide full key" : "Show full key"}
            onClick={() => setShown((current) => !current)}
          >
            {shown ? <EyeOff size={14} /> : <Eye size={14} />}
            {shown ? "Hide" : "Show"}
          </button>
        </Tooltip>
        <Tooltip content={copied ? "Copied" : "Copy key"}>
          <button
            className={copied ? "table-action copied" : "table-action"}
            type="button"
            aria-label="Copy key"
            onClick={() => {
              void copyVirtualKey(props.value).then((success) => {
                if (success) {
                  setCopied(true);
                  window.setTimeout(() => setCopied(false), 1400);
                }
              });
            }}
          >
            {copied ? <Check size={14} /> : <Copy size={14} />}
            Copy
          </button>
        </Tooltip>
      </div>
    </div>
  );
}

function MetadataSummary(props: { value: unknown }) {
  const metadata = withoutManagedMetadata(metadataObject(props.value));
  const entries = Object.entries(metadata);
  if (!entries.length) return <span className="muted">none</span>;
  return (
    <div className="metadata-summary">
      {entries.slice(0, 3).map(([key, value]) => (
        <span className="badge" key={key}>
          {key}: {String(value)}
        </span>
      ))}
      {entries.length > 3 ? (
        <span className="muted">+{entries.length - 3}</span>
      ) : null}
    </div>
  );
}

function metadataObject(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function withoutManagedMetadata(value: Record<string, unknown>) {
  const next = { ...value };
  delete next.name;
  delete next.id;
  return next;
}

function stringMetadata(value: Record<string, unknown>) {
  return Object.fromEntries(
    Object.entries(value).map(([key, item]) => [
      key,
      typeof item === "string" ? item : String(item),
    ]),
  );
}

function randomUuid() {
  return (
    crypto.randomUUID?.() ??
    [
      randomKey(8),
      randomKey(4),
      randomKey(4),
      randomKey(4),
      randomKey(12),
    ].join("-")
  );
}
