import { Link } from "@tanstack/react-router";
import { Bot, Pencil, Plus, Trash2 } from "lucide-react";
import { useState } from "react";
import {
  invalidProviderApiKey,
  isDatabaseConfigResource,
  makeEmptyLlmProvider,
  providerDisplayName,
  providerLabel,
  removeLlmProvider,
  upsertLlmProvider,
} from "../config";
import { ConfigDiffSaveActions } from "../components/ConfigDiffDrawer";
import {
  ConfirmDialog,
  Drawer,
  EmptyState,
  Field,
  PageHeader,
  Panel,
  StatusBanner,
  Tooltip,
  YamlBlock,
} from "../components/Primitives";
import { useStickyQueryParam } from "../drawerRouteState";
import { ProviderIcon } from "../components/ProviderIcon";
import {
  useDeleteConfigResource,
  useLlmConfigData,
  useUpdateConfig,
  useUpsertConfigResource,
} from "../hooks";
import { cleanEmpty } from "../policies/policyUtils";
import { useSchemaHelp, type SchemaHelp } from "../schemaHelp";
import type {
  GatewayConfig,
  LlmModel,
  LlmProvider,
  ProviderName,
} from "../types";
import { ProviderConfigEditor } from "./models/ProviderConfigEditor";

export function ProvidersPage() {
  const { config, hybrid, configResources, resources, models, providers } =
    useLlmConfigData();
  const update = useUpdateConfig();
  const upsertResource = useUpsertConfigResource();
  const deleteResource = useDeleteConfigResource();
  const help = useSchemaHelp();
  const [editing, setEditing] = useState<{
    previousName?: string;
    provider: LlmProvider;
  } | null>(null);
  const [deletingProvider, setDeletingProvider] = useState<string | null>(null);
  const [providerDrawer, setProviderDrawer] = useStickyQueryParam("provider");
  const linkedProvider =
    providerDrawer && providerDrawer !== "new"
      ? providers.find((provider) => provider.name === providerDrawer)
      : null;
  const activeEditing =
    editing ??
    (providerDrawer === "new"
      ? { provider: makeEmptyLlmProvider() }
      : linkedProvider
        ? {
            previousName: linkedProvider.name,
            provider: structuredClone(linkedProvider),
          }
        : null);
  const editingDatabaseProvider = Boolean(
    hybrid &&
    activeEditing &&
    (!activeEditing.previousName ||
      isDatabaseConfigResource(
        resources,
        "llm.provider",
        activeEditing.previousName,
      )),
  );

  function openNewProvider() {
    resetSaves();
    setEditing(null);
    setProviderDrawer("new");
  }

  function openEditProvider(provider: LlmProvider) {
    resetSaves();
    setEditing(null);
    setProviderDrawer(provider.name);
  }

  function closeProviderEditor() {
    resetSaves();
    setEditing(null);
    setProviderDrawer(null, "replace");
  }

  const saving =
    update.isPending || upsertResource.isPending || deleteResource.isPending;
  const saveError =
    update.error?.message ??
    upsertResource.error?.message ??
    deleteResource.error?.message ??
    null;
  const saved =
    update.isSuccess || upsertResource.isSuccess || deleteResource.isSuccess;

  function resetSaves() {
    update.reset();
    upsertResource.reset();
    deleteResource.reset();
  }

  function saveProvider(provider: LlmProvider, previousName?: string) {
    if (
      hybrid &&
      (!previousName ||
        isDatabaseConfigResource(resources, "llm.provider", previousName))
    ) {
      upsertResource.mutate(
        { kind: "llm.provider", value: provider, previousId: previousName },
        { onSuccess: closeProviderEditor },
      );
      return;
    }
    update.mutate((next) => upsertLlmProvider(next, provider, previousName), {
      onSuccess: closeProviderEditor,
    });
  }

  function deleteProvider(name: string) {
    if (hybrid && isDatabaseConfigResource(resources, "llm.provider", name)) {
      deleteResource.mutate(
        { kind: "llm.provider", id: name },
        { onSuccess: () => setDeletingProvider(null) },
      );
      return;
    }
    update.mutate((next) => removeLlmProvider(next, name), {
      onSuccess: () => setDeletingProvider(null),
    });
  }

  return (
    <div className="page-stack">
      <PageHeader
        title="LLM Providers"
        description="Define reusable provider credentials and connection settings for models."
        actions={
          <button
            className="button primary"
            type="button"
            onClick={openNewProvider}
          >
            <Plus size={16} />
            Add provider
          </button>
        }
      />

      {saveError ? (
        <StatusBanner state="bad" title="Save failed">
          {saveError}
        </StatusBanner>
      ) : null}
      {saved ? <StatusBanner state="ok" title="Configuration saved" /> : null}

      <Panel>
        {config.isLoading || (hybrid && configResources.isLoading) ? (
          <StatusBanner state="loading" title="Loading providers" />
        ) : config.isError || (hybrid && configResources.isError) ? (
          <StatusBanner state="bad" title="Configuration API unavailable">
            {config.error?.message ?? configResources.error?.message}
          </StatusBanner>
        ) : providers.length === 0 ? (
          <EmptyState
            title="No shared providers configured"
            description="Add a provider when multiple models should share the same credentials or upstream connection settings."
            action={
              <button
                className="button primary"
                type="button"
                onClick={openNewProvider}
              >
                <Plus size={16} />
                Add provider
              </button>
            }
          />
        ) : (
          <div className="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>Name</th>
                  {hybrid ? <th>Source</th> : null}
                  <th>Provider</th>
                  <th>Upstream model</th>
                  <th>Used by</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {providers.map((provider) => {
                  const usage = providerUsage(provider.name, models);
                  return (
                    <tr key={provider.name}>
                      <td className="strong">{provider.name}</td>
                      {hybrid ? (
                        <td>
                          <span className="badge">
                            {isDatabaseConfigResource(
                              resources,
                              "llm.provider",
                              provider.name,
                            )
                              ? "Database"
                              : "File"}
                          </span>
                        </td>
                      ) : null}
                      <td>
                        <ProviderBadge
                          provider={
                            providerLabel(provider.provider) as ProviderName
                          }
                        />
                      </td>
                      <td>{provider.params?.model || "Incoming model"}</td>
                      <td>
                        {usage.length ? (
                          <span className="badge ok">
                            {usage.length}{" "}
                            {usage.length === 1 ? "model" : "models"}
                          </span>
                        ) : (
                          <span className="badge">unused</span>
                        )}
                      </td>
                      <td className="row-actions">
                        <Tooltip content="Add model using this provider">
                          <Link
                            className="icon-button"
                            aria-label="Add model using provider"
                            to="/llm/models"
                            search={{ provider: provider.name }}
                          >
                            <Bot size={16} />
                          </Link>
                        </Tooltip>
                        <Tooltip content="Edit provider">
                          <button
                            className="icon-button"
                            aria-label="Edit provider"
                            type="button"
                            onClick={() => openEditProvider(provider)}
                          >
                            <Pencil size={16} />
                          </button>
                        </Tooltip>
                        <Tooltip
                          content={
                            usage.length
                              ? "Provider is referenced by models"
                              : "Delete provider"
                          }
                        >
                          <button
                            className="icon-button danger"
                            aria-label="Delete provider"
                            type="button"
                            disabled={usage.length > 0 || saving}
                            onClick={() => setDeletingProvider(provider.name)}
                          >
                            <Trash2 size={16} />
                          </button>
                        </Tooltip>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </Panel>

      {activeEditing ? (
        <ProviderEditor
          key={activeEditing.previousName ?? "new"}
          initial={activeEditing.provider}
          config={config.data}
          databaseBacked={editingDatabaseProvider}
          previousName={activeEditing.previousName}
          help={help}
          saving={saving}
          onCancel={closeProviderEditor}
          onSave={saveProvider}
        />
      ) : null}
      {deletingProvider ? (
        <ConfirmDialog
          title="Delete provider?"
          destructive
          confirmLabel="Delete provider"
          confirmDisabled={saving}
          onCancel={() => setDeletingProvider(null)}
          onConfirm={() => deleteProvider(deletingProvider)}
        >
          <p>
            Delete <strong>{deletingProvider}</strong>? This cannot be undone.
          </p>
        </ConfirmDialog>
      ) : null}
    </div>
  );
}

function ProviderEditor(props: {
  initial: LlmProvider;
  config?: GatewayConfig;
  databaseBacked: boolean;
  previousName?: string;
  help: SchemaHelp;
  saving: boolean;
  onCancel: () => void;
  onSave: (provider: LlmProvider, previousName?: string) => void;
}) {
  const [provider, setProvider] = useState<LlmProvider>(props.initial);
  const [initialDraft] = useState(() => JSON.stringify(props.initial));
  const [saveAttempted, setSaveAttempted] = useState(false);
  const preview = cleanEmpty(provider) as LlmProvider | undefined;
  const invalidApiKey = invalidProviderApiKey(provider.params?.apiKey);
  const providerApiKeyError =
    saveAttempted && invalidApiKey ? "Enter a value, or choose Unset." : null;

  function save() {
    setSaveAttempted(true);
    if (invalidApiKey) return;
    props.onSave(preview ?? provider, props.previousName);
  }

  function validateBeforeDiff() {
    setSaveAttempted(true);
    if (!provider.name.trim()) return false;
    if (invalidApiKey) return false;
    return true;
  }

  return (
    <Drawer
      title={props.previousName ? "Edit provider" : "Add provider"}
      onClose={props.onCancel}
      dirty={JSON.stringify(provider) !== initialDraft}
      saving={props.saving}
      footer={(requestClose) => (
        <ConfigDiffSaveActions
          config={props.config}
          resourceDiff={
            props.databaseBacked
              ? {
                  original: props.previousName ? props.initial : {},
                  modified: preview ?? {},
                }
              : undefined
          }
          diffTitle={
            props.databaseBacked
              ? "Provider resource diff"
              : "Provider config diff"
          }
          saveLabel="Save provider"
          saving={props.saving}
          saveDisabled={!provider.name.trim()}
          onCancel={requestClose}
          onSave={save}
          beforeDiff={validateBeforeDiff}
          applyDiff={(next) =>
            upsertLlmProvider(next, preview ?? provider, props.previousName)
          }
        />
      )}
    >
      <div className="form-grid">
        <Field
          label="Provider name"
          tooltip={props.help.field<LlmProvider>(
            "LocalLLMProvider",
            "name",
            "Models reference this name from their provider field.",
          )}
        >
          <input
            value={provider.name}
            onChange={(event) =>
              setProvider({ ...provider, name: event.target.value })
            }
            placeholder="openai-prod"
          />
        </Field>
      </div>

      <ProviderConfigEditor
        provider={provider.provider}
        params={provider.params}
        auth={provider.defaults?.auth}
        help={props.help}
        apiKeyError={providerApiKeyError}
        onProviderChange={(nextProvider, params) =>
          setProvider((current) => ({
            ...current,
            provider: nextProvider,
            params,
          }))
        }
        onParamsChange={(params) =>
          setProvider((current) => ({ ...current, params }))
        }
        onAuthChange={(auth) =>
          setProvider((current) => ({
            ...current,
            defaults: auth
              ? { ...(current.defaults ?? {}), auth }
              : removeProviderAuth(current.defaults),
          }))
        }
      />

      <details>
        <summary>Generated provider config</summary>
        <YamlBlock value={preview ?? {}} />
      </details>
    </Drawer>
  );
}

function removeProviderAuth(defaults: LlmProvider["defaults"]) {
  if (!defaults) return null;
  const next = { ...defaults, auth: null };
  return Object.values(next).some(
    (value) => value !== null && value !== undefined,
  )
    ? next
    : null;
}

function ProviderBadge(props: { provider: ProviderName }) {
  return (
    <span className="badge provider-badge">
      <ProviderIcon provider={props.provider} />
      {providerDisplayName(props.provider)}
    </span>
  );
}

function providerUsage(providerName: string, models: LlmModel[]) {
  return models.filter(
    (model) =>
      typeof model.provider === "object" &&
      "reference" in model.provider &&
      model.provider.reference === providerName,
  );
}
