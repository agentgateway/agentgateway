import { tr } from "../i18n";
import { Link } from "@tanstack/react-router";
import { Bot, Pencil, Plus, Trash2 } from "lucide-react";
import { useMemo, useState } from "react";
import {
  invalidProviderApiKey,
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
import { useGatewayConfig, useUpdateConfig } from "../hooks";
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
  const config = useGatewayConfig();
  const update = useUpdateConfig();
  const help = useSchemaHelp();
  const providers = useMemo(
    () => config.data?.llm?.providers ?? [],
    [config.data],
  );
  const models = useMemo(() => config.data?.llm?.models ?? [], [config.data]);
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

  function openNewProvider() {
    setEditing(null);
    setProviderDrawer("new");
  }

  function openEditProvider(provider: LlmProvider) {
    setEditing(null);
    setProviderDrawer(provider.name);
  }

  function closeProviderEditor() {
    setEditing(null);
    setProviderDrawer(null, "replace");
  }

  return (
    <div className="page-stack">
      <PageHeader
        title={tr("copy.llmProviders")}
        description={tr(
          "copy.defineReusableProviderCredentialsAndConnectionSettingsForModels",
        )}
        actions={
          <button
            className="button primary"
            type="button"
            onClick={openNewProvider}
          >
            <Plus size={16} />
            {tr("copy.addProvider")}
          </button>
        }
      />

      {update.isError ? (
        <StatusBanner state="bad" title={tr("copy.saveFailed")}>
          {update.error.message}
        </StatusBanner>
      ) : null}
      {update.isSuccess ? (
        <StatusBanner state="ok" title={tr("copy.configurationSaved")} />
      ) : null}

      <Panel>
        {config.isLoading ? (
          <StatusBanner state="loading" title={tr("copy.loadingProviders")} />
        ) : config.isError ? (
          <StatusBanner
            state="bad"
            title={tr("copy.configurationApiUnavailable")}
          >
            {config.error.message}
          </StatusBanner>
        ) : providers.length === 0 ? (
          <EmptyState
            title={tr("copy.noSharedProvidersConfigured")}
            description={tr(
              "copy.addAProviderWhenMultipleModelsShouldShareTheSameCredentialsOrUpstreamConnectionSettings",
            )}
            action={
              <button
                className="button primary"
                type="button"
                onClick={openNewProvider}
              >
                <Plus size={16} />
                {tr("copy.addProvider")}
              </button>
            }
          />
        ) : (
          <div className="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>{tr("copy.name")}</th>
                  <th>{tr("copy.provider")}</th>
                  <th>{tr("copy.upstreamModel")}</th>
                  <th>{tr("copy.usedBy")}</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {providers.map((provider) => {
                  const usage = providerUsage(provider.name, models);
                  return (
                    <tr key={provider.name}>
                      <td className="strong">{provider.name}</td>
                      <td>
                        <ProviderBadge
                          provider={
                            providerLabel(provider.provider) as ProviderName
                          }
                        />
                      </td>
                      <td>
                        {provider.params?.model || tr("copy.incomingModel")}
                      </td>
                      <td>
                        {usage.length ? (
                          <span className="badge ok">
                            {tr("copy.valueModels", { count: usage.length })}
                          </span>
                        ) : (
                          <span className="badge">{tr("copy.unused")}</span>
                        )}
                      </td>
                      <td className="row-actions">
                        <Tooltip content="Add model using this provider">
                          <Link
                            className="icon-button"
                            aria-label={tr("copy.addModelUsingProvider")}
                            to="/llm/models"
                            search={{ provider: provider.name }}
                          >
                            <Bot size={16} />
                          </Link>
                        </Tooltip>
                        <Tooltip content="Edit provider">
                          <button
                            className="icon-button"
                            aria-label={tr("copy.editProvider")}
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
                            aria-label={tr("copy.deleteProvider")}
                            type="button"
                            disabled={usage.length > 0 || update.isPending}
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
          previousName={activeEditing.previousName}
          help={help}
          saving={update.isPending}
          onCancel={closeProviderEditor}
          onSave={(provider, previousName) =>
            update.mutate(
              (next) => upsertLlmProvider(next, provider, previousName),
              {
                onSuccess: closeProviderEditor,
              },
            )
          }
        />
      ) : null}
      {deletingProvider ? (
        <ConfirmDialog
          title={tr("copy.deleteProvider_1j44lo")}
          destructive
          confirmLabel={tr("copy.deleteProvider")}
          confirmDisabled={update.isPending}
          onCancel={() => setDeletingProvider(null)}
          onConfirm={() =>
            update.mutate((next) => removeLlmProvider(next, deletingProvider), {
              onSuccess: () => setDeletingProvider(null),
            })
          }
        >
          <p>
            {tr("copy.delete")}
            <strong>{deletingProvider}</strong>
            {tr("copy.thisCannotBeUndone")}
          </p>
        </ConfirmDialog>
      ) : null}
    </div>
  );
}

function ProviderEditor(props: {
  initial: LlmProvider;
  config?: GatewayConfig;
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
          diffTitle="Provider config diff"
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
          label={tr("copy.providerName")}
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
        <summary>{tr("copy.generatedProviderConfig")}</summary>
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
