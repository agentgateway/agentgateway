import { useRef, useState } from "react";
import {
  Check,
  Copy,
  Eye,
  EyeOff,
  KeyRound,
  Pencil,
  Plus,
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
import { ConfigDiffSaveActions } from "../components/ConfigDiffDrawer";
import { EnumSelector } from "../components/EnumSelector";
import { hasKeyValue, keyValue, maskKey } from "../credentialDisplay";
import { useStickyQueryParam } from "../drawerRouteState";
import {
  useDeleteConfigResource,
  useLlmConfigData,
  useUpdateConfig,
  useUpsertConfigResource,
  useUpsertPolicyResource,
} from "../hooks";
import { isDatabaseConfigResource } from "../config";
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
import {
  authorizationLocationFrom,
  authorizationLocationToValue,
  CredentialLocationSetting,
} from "../policies/AuthorizationLocation";
import { AdvancedSettingRow } from "../policies/PolicyLayout";
import { KeyValueEditor } from "../policies/PolicyFormControls";
import { randomUuid } from "../randomUuid";
import { useSchemaHelp, type SchemaHelp } from "../schemaHelp";
import type { GatewayConfig, LlmApiKeyPolicy, VirtualApiKey } from "../types";

export function KeysPage() {
  const {
    config,
    hybrid,
    resources,
    policies,
    apiKeys: keys,
    isLoading,
    error,
  } = useLlmConfigData();
  const update = useUpdateConfig();
  const upsertResource = useUpsertConfigResource();
  const upsertPolicy = useUpsertPolicyResource();
  const deleteResource = useDeleteConfigResource();
  const help = useSchemaHelp();
  const policy = (policies.apiKey ?? null) as LlmApiKeyPolicy | null;
  const filePolicyOwned = Boolean(
    config.data?.llm?.policies &&
    Object.prototype.hasOwnProperty.call(config.data.llm.policies, "apiKey"),
  );
  const databasePolicyOwned =
    hybrid && isDatabaseConfigResource(resources, "llm.policy", "apiKey");
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
    (keyDrawer === "new" && policy
      ? { key: newVirtualKey() }
      : linkedKey
        ? { previousKey: keyValue(linkedKey), key: structuredClone(linkedKey) }
        : null);
  const advancedOpen = keyDrawer === "settings";
  const saving =
    update.isPending ||
    upsertResource.isPending ||
    upsertPolicy.isPending ||
    deleteResource.isPending;
  const saveError =
    update.error?.message ??
    upsertResource.error?.message ??
    upsertPolicy.error?.message ??
    deleteResource.error?.message ??
    null;
  const unavailable = isLoading || Boolean(error);

  function databaseKeyId(key: VirtualApiKey) {
    const id = keyId(key);
    return hybrid && id && isDatabaseConfigResource(resources, "llm.apiKey", id)
      ? id
      : undefined;
  }

  function saveKey(key: VirtualApiKey, previousKey?: string) {
    const previous = previousKey
      ? keys.find((item) => keyValue(item) === previousKey)
      : undefined;
    const previousId = previous ? databaseKeyId(previous) : undefined;
    if (hybrid && (!previous || previousId)) {
      const value = structuredClone(key);
      if (value.metadata && typeof value.metadata === "object") {
        delete value.metadata.id;
      }
      upsertResource.mutate(
        { kind: "llm.apiKey", value, previousId },
        { onSuccess: closeKeyDrawer },
      );
      return;
    }
    update.mutate((next) => upsertVirtualKey(next, key, previousKey), {
      onSuccess: closeKeyDrawer,
    });
  }

  function removeKey(key: VirtualApiKey) {
    const id = databaseKeyId(key);
    if (id) {
      deleteResource.mutate(
        { kind: "llm.apiKey", id },
        { onSuccess: () => setDeleteKey(null) },
      );
      return;
    }
    update.mutate((next) => removeVirtualKey(next, keyValue(key)), {
      onSuccess: () => setDeleteKey(null),
    });
  }

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
    const onSuccess = () => {
      setDisablePolicyOpen(false);
      closeKeyDrawer();
    };
    if (databasePolicyOwned) {
      deleteResource.mutate(
        { kind: "llm.policy", id: "apiKey" },
        { onSuccess },
      );
      return;
    }
    update.mutate((next) => disableApiKeyPolicy(next), {
      onSuccess,
    });
  }

  return (
    <div className="page-stack">
      <PageHeader
        title="Virtual API Keys"
        description="Provision incoming credentials and metadata for callers."
        actions={
          <div className="button-row">
            {policy ? (
              <>
                <button
                  className="button"
                  type="button"
                  disabled={unavailable || saving}
                  onClick={() => setKeyDrawer("settings")}
                >
                  <SlidersHorizontal size={16} />
                  Settings
                </button>
                <button
                  className="button primary"
                  type="button"
                  disabled={unavailable || saving}
                  onClick={openNewKey}
                >
                  <Plus size={16} />
                  New key
                </button>
              </>
            ) : (
              <button
                className="button primary"
                type="button"
                disabled={unavailable || saving}
                onClick={() => setKeyDrawer("settings")}
              >
                <KeyRound size={16} />
                Enable API key auth
              </button>
            )}
          </div>
        }
      />

      {saveError ? (
        <StatusBanner state="bad" title="Save failed">
          {saveError}
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
        {isLoading ? (
          <StatusBanner state="loading" title="Loading keys" />
        ) : error ? (
          <StatusBanner state="bad" title="Configuration API unavailable">
            {error.message}
          </StatusBanner>
        ) : !policy ? (
          <EmptyState
            title="API key authentication is disabled"
            description="Enable API key authentication before provisioning virtual keys."
            action={
              <button
                className="button primary"
                type="button"
                disabled={saving}
                onClick={() => setKeyDrawer("settings")}
              >
                <KeyRound size={16} />
                Enable API key auth
              </button>
            }
          />
        ) : keys.length === 0 ? (
          <EmptyState
            title="No virtual API keys"
            description="Create a key so callers can authenticate without exposing provider credentials."
            action={
              <div className="button-row">
                <button
                  className="button danger"
                  type="button"
                  disabled={saving}
                  onClick={() => setDisablePolicyOpen(true)}
                >
                  <X size={16} />
                  Disable API Key Policy
                </button>
                <button
                  className="button primary"
                  type="button"
                  disabled={saving}
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
          config={config.data}
          previousKey={activeEditing.previousKey}
          help={help}
          existingKeys={keys}
          databaseBacked={
            hybrid &&
            (!activeEditing.previousKey ||
              Boolean(databaseKeyId(activeEditing.key)))
          }
          saving={saving}
          saveError={saveError}
          onCancel={closeKeyDrawer}
          onSave={saveKey}
        />
      ) : null}
      {deleteKey ? (
        <ConfirmDialog
          title="Delete virtual API key?"
          destructive
          confirmLabel="Delete key"
          confirmDisabled={saving}
          onCancel={() => setDeleteKey(null)}
          onConfirm={() => {
            removeKey(deleteKey);
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
          confirmDisabled={saving}
          onCancel={() => setDisablePolicyOpen(false)}
          onConfirm={disablePolicy}
        >
          <p>
            Disable virtual API key validation? Requests will no longer be
            validated against virtual API keys.
          </p>
        </ConfirmDialog>
      ) : null}
      {advancedOpen ? (
        <AdvancedSettingsDrawer
          config={config.data}
          policy={policy}
          databaseBacked={hybrid && !filePolicyOwned}
          keyCount={keys.length}
          help={help}
          saving={saving}
          saveError={saveError}
          onClose={closeKeyDrawer}
          onDisable={disablePolicy}
          onSave={(nextPolicy) => {
            if (hybrid && !filePolicyOwned) {
              upsertPolicy.mutate(
                {
                  kind: "llm.policy",
                  id: "apiKey",
                  value: nextPolicy,
                },
                { onSuccess: closeKeyDrawer },
              );
              return;
            }
            update.mutate(
              (next) => {
                Object.assign(getApiKeyPolicy(next), nextPolicy);
              },
              { onSuccess: closeKeyDrawer },
            );
          }}
        />
      ) : null}
    </div>
  );
}

function AdvancedSettingsDrawer(props: {
  config?: GatewayConfig | null;
  policy?: LlmApiKeyPolicy | null;
  databaseBacked?: boolean;
  keyCount: number;
  help: SchemaHelp;
  saving: boolean;
  saveError?: string | null;
  onClose: () => void;
  onDisable: () => void;
  onSave: (policy: Partial<LlmApiKeyPolicy>) => void;
}) {
  return (
    <Drawer
      title={props.policy ? "Settings" : "Enable API key auth"}
      onClose={props.onClose}
    >
      <PolicyControls
        policy={props.policy}
        databaseBacked={props.databaseBacked}
        config={props.config}
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
  config?: GatewayConfig | null;
  policy?: LlmApiKeyPolicy | null;
  databaseBacked?: boolean;
  keyCount: number;
  help: SchemaHelp;
  saving: boolean;
  onDisable: () => void;
  onSave: (policy: Partial<LlmApiKeyPolicy>) => void;
}) {
  const [mode, setMode] = useState(props.policy?.mode ?? "strict");
  const [location, setLocation] = useState(() =>
    authorizationLocationFrom(props.policy?.location),
  );
  const patch: Partial<LlmApiKeyPolicy> = {
    mode,
    location: authorizationLocationToValue(location),
  };
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
          options={[
            { value: "strict", label: "Strict" },
            { value: "optional", label: "Optional" },
            { value: "permissive", label: "Permissive" },
          ]}
          onChange={(value) =>
            setMode(value as "strict" | "optional" | "permissive")
          }
        />
      </FieldGroup>
      <CredentialLocationSetting
        help={props.help}
        value={location}
        defaultDescription={
          props.help.field<LlmApiKeyPolicy>(
            "LocalAPIKeys",
            "location",
            "By default, callers send Authorization: Bearer key.",
          ) ?? "By default, callers send Authorization: Bearer key."
        }
        description={
          props.help.definition(
            "AuthorizationLocation",
            "Customize where virtual API keys are read from the request.",
          ) ?? "Customize where virtual API keys are read from the request."
        }
        onChange={setLocation}
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
      <ConfigDiffSaveActions
        config={props.config}
        resourceDiff={
          props.databaseBacked
            ? () => ({
                original: props.policy
                  ? apiKeyPolicyResourceValue(props.policy)
                  : {},
                modified: patch,
              })
            : undefined
        }
        diffTitle={
          props.policy
            ? "API key policy config diff"
            : "Enable API key authentication"
        }
        saveLabel={props.policy ? "Save policy" : "Enable API key auth"}
        saving={props.saving}
        onSave={() => props.onSave(patch)}
        applyDiff={(next) => {
          Object.assign(getApiKeyPolicy(next), patch);
        }}
      />
    </div>
  );
}

function apiKeyPolicyResourceValue(policy: LlmApiKeyPolicy) {
  const value: Partial<LlmApiKeyPolicy> = { ...policy };
  delete value.keys;
  return value;
}

function KeyEditor(props: {
  initial: VirtualApiKey;
  config?: GatewayConfig | null;
  previousKey?: string;
  help: SchemaHelp;
  existingKeys: VirtualApiKey[];
  databaseBacked: boolean;
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
  const generatedKey = useRef<string | null>(null);
  const draft = JSON.stringify({
    name,
    keyMode,
    key,
    replaceKey,
    metadataValues,
  });
  const [initialDraft] = useState(() => draft);
  const nameRequired = isNew && !name.trim();
  const duplicateName = isNew
    ? duplicateKeyName(name, props.existingKeys)
    : false;

  function virtualKey() {
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
        ? (generatedKey.current ??= `agw_sk_${randomKey(32)}`)
        : key
      : replaceKey
        ? key
        : "";
    return isNew || replaceKey
      ? { key: nextKey, metadata }
      : { ...props.initial, metadata };
  }

  function nextVirtualKey() {
    setSubmitted(true);
    return nameRequired ? null : virtualKey();
  }

  function save() {
    const virtualKey = nextVirtualKey();
    if (!virtualKey) return;
    props.onSave(virtualKey, props.previousKey);
  }

  return (
    <Drawer
      title={props.previousKey ? "Edit virtual key" : "Create virtual key"}
      onClose={props.onCancel}
      dirty={draft !== initialDraft}
      saving={props.saving}
      footer={(requestClose) => (
        <ConfigDiffSaveActions
          config={props.config}
          resourceDiff={
            props.databaseBacked
              ? {
                  original: props.previousKey
                    ? keyResourceForDisplay(props.initial)
                    : {},
                  modified: keyResourceForDisplay(virtualKey()),
                }
              : undefined
          }
          diffTitle={
            props.databaseBacked
              ? "Virtual API key resource diff"
              : "Virtual API key config diff"
          }
          saveLabel="Save key"
          saving={props.saving}
          saveDisabled={keyMode === "custom" && !key.trim()}
          onCancel={requestClose}
          onSave={save}
          beforeDiff={() => Boolean(nextVirtualKey())}
          applyDiff={(next) => {
            const virtualKey = nextVirtualKey();
            if (virtualKey) {
              upsertVirtualKey(next, virtualKey, props.previousKey);
            }
          }}
        />
      )}
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

function keyResourceForDisplay(key: VirtualApiKey) {
  const value = structuredClone(key);
  if (value.metadata && typeof value.metadata === "object") {
    delete value.metadata.id;
  }
  return value;
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
