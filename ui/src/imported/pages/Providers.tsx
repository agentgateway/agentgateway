import { Link } from "../_adapters/_router";
import { Bot, Pencil, Plus, Save, Trash2 } from "lucide-react";
import { useMemo, useState } from "react";
import {
  makeEmptyLlmProvider,
  providerDisplayName,
  providerLabel,
  removeLlmProvider,
  upsertLlmProvider,
} from "../config";
import { Drawer, EmptyState, Field, PageHeader, Panel, StatusBanner, Tooltip, YamlBlock } from "../components/Primitives";
import { ProviderIcon } from "../components/ProviderIcon";
import { useGatewayConfig, useUpdateConfig } from "../_adapters/hooks";
import { cleanEmpty } from "../policies/policyUtils";
import { useSchemaHelp, type SchemaHelp } from "../schemaHelp";
import type { LlmModel, LlmProvider, ProviderName } from "../types";
import { ProviderConfigEditor } from "./models/ProviderConfigEditor";

export function ProvidersPage() {
  const config = useGatewayConfig();
  const update = useUpdateConfig();
  const help = useSchemaHelp();
  const providers = useMemo(() => config.data?.llm?.providers ?? [], [config.data]);
  const models = useMemo(() => config.data?.llm?.models ?? [], [config.data]);
  const [editing, setEditing] = useState<{ previousName?: string; provider: LlmProvider } | null>(null);

  return (
    <div className="page-stack">
      <PageHeader
        title="LLM Providers"
        description="Define reusable provider credentials and connection settings for models."
        actions={
          <button className="button primary" type="button" onClick={() => setEditing({ provider: makeEmptyLlmProvider() })}>
            <Plus size={16} />
            Add provider
          </button>
        }
      />

      {update.isError ? <StatusBanner state="bad" title="Save failed">{update.error!.message}</StatusBanner> : null}
      {update.isSuccess ? <StatusBanner state="ok" title="Configuration saved" /> : null}

      <Panel>
        {config.isLoading ? (
          <StatusBanner state="loading" title="Loading providers" />
        ) : config.isError ? (
          <StatusBanner state="bad" title="Configuration API unavailable">{config.error!.message}</StatusBanner>
        ) : providers.length === 0 ? (
          <EmptyState
            title="No shared providers configured"
            description="Add a provider when multiple models should share the same credentials or upstream connection settings."
            action={
              <button className="button primary" type="button" onClick={() => setEditing({ provider: makeEmptyLlmProvider() })}>
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
                      <td><ProviderBadge provider={providerLabel(provider.provider) as ProviderName} /></td>
                      <td>{provider.params?.model || "incoming model"}</td>
                      <td>{usage.length ? <span className="badge ok">{usage.length} {usage.length === 1 ? "model" : "models"}</span> : <span className="badge">unused</span>}</td>
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
                          <button className="icon-button" aria-label="Edit provider" type="button" onClick={() => setEditing({ previousName: provider.name, provider: structuredClone(provider) })}>
                            <Pencil size={16} />
                          </button>
                        </Tooltip>
                        <Tooltip content={usage.length ? "Provider is referenced by models" : "Delete provider"}>
                          <button
                            className="icon-button danger"
                            aria-label="Delete provider"
                            type="button"
                            disabled={usage.length > 0}
                            onClick={() => update.mutate((next) => removeLlmProvider(next, provider.name))}
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

      {editing ? (
        <ProviderEditor
          initial={editing.provider}
          previousName={editing.previousName}
          help={help}
          saving={update.isPending}
          onCancel={() => setEditing(null)}
          onSave={(provider, previousName) => update.mutate((next) => upsertLlmProvider(next, provider, previousName), {
            onSuccess: () => setEditing(null),
          })}
        />
      ) : null}
    </div>
  );
}

function ProviderEditor(props: {
  initial: LlmProvider;
  previousName?: string;
  help: SchemaHelp;
  saving: boolean;
  onCancel: () => void;
  onSave: (provider: LlmProvider, previousName?: string) => void;
}) {
  const [provider, setProvider] = useState<LlmProvider>(props.initial);
  const preview = cleanEmpty(provider) as LlmProvider | undefined;

  return (
    <Drawer
      title={props.previousName ? "Edit provider" : "Add provider"}
      onClose={props.onCancel}
      footer={
        <div className="button-row">
          <button className="button" type="button" onClick={props.onCancel}>Cancel</button>
          <button className="button primary" type="button" disabled={props.saving || !provider.name.trim()} onClick={() => props.onSave(preview ?? provider, props.previousName)}>
            <Save size={16} />
            Save provider
          </button>
        </div>
      }
    >
      <div className="form-grid">
        <Field
          label="Provider name"
          tooltip={props.help.description(["$defs", "LocalLLMProvider", "properties", "name"], "Models reference this name from their provider field.")}
        >
          <input value={provider.name} onChange={(event) => setProvider({ ...provider, name: event.target.value })} placeholder="openai-prod" />
        </Field>
      </div>

      <ProviderConfigEditor
        provider={provider.provider}
        params={provider.params ?? undefined}
        auth={provider.defaults?.auth}
        help={props.help}
        onProviderChange={(nextProvider, params) => setProvider((current) => ({ ...current, provider: nextProvider, params }))}
        onParamsChange={(params) => setProvider((current) => ({ ...current, params }))}
        onAuthChange={(auth) => setProvider((current) => ({
          ...current,
          defaults: auth ? { ...(current.defaults ?? {}), auth } : removeProviderAuth(current.defaults),
        }))}
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
  return Object.values(next).some((value) => value !== null && value !== undefined) ? next : null;
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
  return models.filter((model) => model.provider && typeof model.provider === "object" && "reference" in model.provider && model.provider.reference === providerName);
}
