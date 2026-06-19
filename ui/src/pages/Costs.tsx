import { Plus, RefreshCw, Trash2 } from "lucide-react";
import {
  configuredCostSources,
  refreshBaseCostsAndConfigure,
  type CostCatalogSource,
} from "../costs";
import {
  EmptyState,
  PageHeader,
  Panel,
  StatusBanner,
  formatNumber,
} from "../components/Primitives";
import { useGatewayConfig, useUpdateConfig } from "../hooks";
import type { GatewayConfig } from "../types";
import { useEffect, useMemo, useState } from "react";

type CustomCostRow = {
  provider: string;
  model: string;
  input: string;
  output: string;
  cacheRead: string;
  cacheWrite: string;
};

export function CostsPage() {
  const config = useGatewayConfig();
  const updateConfig = useUpdateConfig();
  const [refreshing, setRefreshing] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const sources = configuredCostSources(config.data);
  const customRows = useMemo(() => inlineCostRows(sources), [sources]);
  const [editingCustom, setEditingCustom] = useState(false);
  const [customDraft, setCustomDraft] = useState<CustomCostRow[]>(customRows);
  const [customError, setCustomError] = useState<string | null>(null);

  useEffect(() => {
    if (!editingCustom) setCustomDraft(customRows);
  }, [customRows, editingCustom]);

  async function refreshCosts() {
    setRefreshing(true);
    setError(null);
    setMessage(null);
    try {
      const refreshed = await refreshBaseCostsAndConfigure(updateConfig);
      setMessage(
        `Base cost catalog refreshed: ${formatNumber(refreshed.models)} models from ${formatNumber(refreshed.providers)} providers.`,
      );
    } catch (err) {
      setError(
        err instanceof Error
          ? err.message
          : "Failed to refresh base cost catalog",
      );
    } finally {
      setRefreshing(false);
    }
  }

  return (
    <div className="page-stack">
      <PageHeader
        title="LLM Costs"
        description="Manage model cost catalogs used for analytics and request cost attribution."
        actions={
          <button
            className="button primary"
            type="button"
            disabled={refreshing || updateConfig.isPending}
            onClick={() => void refreshCosts()}
          >
            <RefreshCw size={16} />
            Refresh base costs
          </button>
        }
      />
      {error ? (
        <StatusBanner state="bad" title="Cost refresh failed">
          {error}
        </StatusBanner>
      ) : null}
      {message ? <StatusBanner state="ok" title={message} /> : null}
      <Panel>
        <div className="section-heading-row">
          <div>
            <h3>Catalog sources</h3>
            <p>
              Sources are merged in order. Later sources override earlier
              entries.
            </p>
          </div>
        </div>
        {sources.length ? (
          <div className="table-wrap">
            <table className="data-table">
              <thead>
                <tr>
                  <th>Source</th>
                  <th>Type</th>
                </tr>
              </thead>
              <tbody>
                {sources.map((source, index) => (
                  <tr key={index}>
                    <td>
                      <code>{sourceLabel(source)}</code>
                    </td>
                    <td>{sourceType(source)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : (
          <EmptyState
            title="No cost catalogs configured"
            description="Refresh the base catalog to add pricing data from models.dev."
          />
        )}
      </Panel>
      <Panel>
        <div className="section-heading-row">
          <div>
            <h3>Custom costs</h3>
            <p>
              Inline overrides stored in this gateway configuration. Values are
              USD per 1M tokens.
            </p>
          </div>
          <div className="button-row compact">
            {editingCustom ? (
              <>
                <button
                  className="button"
                  type="button"
                  disabled={updateConfig.isPending}
                  onClick={() => {
                    setCustomDraft(customRows);
                    setCustomError(null);
                    setEditingCustom(false);
                  }}
                >
                  Cancel
                </button>
                <button
                  className="button primary"
                  type="button"
                  disabled={updateConfig.isPending}
                  onClick={() => void saveCustomCosts()}
                >
                  Save
                </button>
              </>
            ) : (
              <button
                className="button"
                type="button"
                onClick={() => setEditingCustom(true)}
              >
                Edit
              </button>
            )}
          </div>
        </div>
        {customError ? (
          <StatusBanner state="bad" title="Invalid custom costs">
            {customError}
          </StatusBanner>
        ) : null}
        <div className="table-wrap custom-cost-table-wrap">
          <table className="data-table custom-cost-table">
            <thead>
              <tr>
                <th>Provider</th>
                <th>Model</th>
                <th>Input</th>
                <th>Output</th>
                <th>Cache read</th>
                <th>Cache write</th>
                {editingCustom ? <th aria-label="Actions" /> : null}
              </tr>
            </thead>
            <tbody>
              {(editingCustom ? customDraft : customRows).map((row, index) => (
                <tr key={index}>
                  <td>
                    {editingCustom ? (
                      <input
                        value={row.provider}
                        onChange={(event) =>
                          patchCustomRow(index, {
                            provider: event.target.value,
                          })
                        }
                        placeholder="openai"
                      />
                    ) : (
                      row.provider
                    )}
                  </td>
                  <td>
                    {editingCustom ? (
                      <input
                        value={row.model}
                        onChange={(event) =>
                          patchCustomRow(index, { model: event.target.value })
                        }
                        placeholder="gpt-5-mini"
                      />
                    ) : (
                      row.model
                    )}
                  </td>
                  <td>
                    {editingCustom ? (
                      <input
                        className="cost-rate-input"
                        value={row.input}
                        onChange={(event) =>
                          patchCustomRow(index, { input: event.target.value })
                        }
                        placeholder="0.25"
                      />
                    ) : (
                      displayRate(row.input)
                    )}
                  </td>
                  <td>
                    {editingCustom ? (
                      <input
                        className="cost-rate-input"
                        value={row.output}
                        onChange={(event) =>
                          patchCustomRow(index, { output: event.target.value })
                        }
                        placeholder="2.00"
                      />
                    ) : (
                      displayRate(row.output)
                    )}
                  </td>
                  <td>
                    {editingCustom ? (
                      <input
                        className="cost-rate-input"
                        value={row.cacheRead}
                        onChange={(event) =>
                          patchCustomRow(index, {
                            cacheRead: event.target.value,
                          })
                        }
                        placeholder="0.025"
                      />
                    ) : (
                      displayRate(row.cacheRead)
                    )}
                  </td>
                  <td>
                    {editingCustom ? (
                      <input
                        className="cost-rate-input"
                        value={row.cacheWrite}
                        onChange={(event) =>
                          patchCustomRow(index, {
                            cacheWrite: event.target.value,
                          })
                        }
                        placeholder="0.30"
                      />
                    ) : (
                      displayRate(row.cacheWrite)
                    )}
                  </td>
                  {editingCustom ? (
                    <td>
                      <button
                        className="icon-button danger"
                        type="button"
                        aria-label="Remove custom cost"
                        onClick={() =>
                          setCustomDraft((current) =>
                            current.filter(
                              (_, itemIndex) => itemIndex !== index,
                            ),
                          )
                        }
                      >
                        <Trash2 size={15} />
                      </button>
                    </td>
                  ) : null}
                </tr>
              ))}
              {editingCustom && customDraft.length === 0 ? (
                <tr>
                  <td colSpan={7}>
                    <span className="muted-copy inline">No custom costs.</span>
                  </td>
                </tr>
              ) : null}
              {!editingCustom && customRows.length === 0 ? (
                <tr>
                  <td colSpan={6}>
                    <span className="muted-copy inline">No custom costs.</span>
                  </td>
                </tr>
              ) : null}
            </tbody>
          </table>
        </div>
        {editingCustom ? (
          <div className="button-row custom-cost-actions">
            <button
              className="button"
              type="button"
              onClick={() =>
                setCustomDraft((current) => [...current, emptyCustomCostRow()])
              }
            >
              <Plus size={16} />
              Add model cost
            </button>
          </div>
        ) : null}
      </Panel>
    </div>
  );

  function patchCustomRow(index: number, patch: Partial<CustomCostRow>) {
    setCustomDraft((current) =>
      current.map((row, itemIndex) =>
        itemIndex === index ? { ...row, ...patch } : row,
      ),
    );
  }

  async function saveCustomCosts() {
    setCustomError(null);
    const validationError = validateCustomRows(customDraft);
    if (validationError) {
      setCustomError(validationError);
      return;
    }
    try {
      await updateConfig.mutateAsync((next) =>
        setInlineCostRows(next, customDraft),
      );
      setEditingCustom(false);
    } catch (err) {
      setCustomError(
        err instanceof Error ? err.message : "Failed to save custom costs",
      );
    }
  }
}

function sourceType(source: CostCatalogSource) {
  if (source.file) return "File";
  if ("inline" in source) return "Inline";
  return "Unknown";
}

function sourceLabel(source: CostCatalogSource) {
  if (source.file) return source.file;
  if ("inline" in source) return "Custom inline overlay";
  return "Unknown source";
}

function emptyCustomCostRow(): CustomCostRow {
  return {
    provider: "",
    model: "",
    input: "",
    output: "",
    cacheRead: "",
    cacheWrite: "",
  };
}

function inlineCostRows(sources: CostCatalogSource[]): CustomCostRow[] {
  const rows: CustomCostRow[] = [];
  for (const source of sources) {
    if (!("inline" in source)) continue;
    const providers = record(source.inline)?.providers;
    for (const [providerName, provider] of Object.entries(record(providers))) {
      const models = record(provider).models;
      for (const [modelName, model] of Object.entries(record(models))) {
        const rates = record(model).rates;
        rows.push({
          provider: providerName,
          model: modelName,
          input: stringValue(record(rates).input),
          output: stringValue(record(rates).output),
          cacheRead: stringValue(record(rates).cacheRead),
          cacheWrite: stringValue(record(rates).cacheWrite),
        });
      }
    }
  }
  return rows.sort(
    (a, b) =>
      a.provider.localeCompare(b.provider) || a.model.localeCompare(b.model),
  );
}

function setInlineCostRows(config: GatewayConfig, rows: CustomCostRow[]) {
  config.config = config.config ?? {};
  const existing = configuredCostSources(config);
  const withoutInline = existing.filter((source) => !("inline" in source));
  config.config.modelCatalog = [
    ...withoutInline,
    { inline: inlineCatalog(rows) },
  ] as never;
}

function inlineCatalog(rows: CustomCostRow[]) {
  const providers: Record<
    string,
    { models: Record<string, { rates: Record<string, string> }> }
  > = {};
  for (const row of rows) {
    const provider = row.provider.trim();
    const model = row.model.trim();
    if (!provider || !model) continue;
    const rates = cleanRates({
      input: row.input,
      output: row.output,
      cacheRead: row.cacheRead,
      cacheWrite: row.cacheWrite,
    });
    if (!Object.keys(rates).length) continue;
    providers[provider] = providers[provider] ?? { models: {} };
    providers[provider].models[model] = { rates };
  }
  return { providers };
}

function cleanRates(rates: Record<string, string>) {
  return Object.fromEntries(
    Object.entries(rates)
      .map(([key, value]) => [key, value.trim()])
      .filter(([, value]) => value),
  );
}

function validateCustomRows(rows: CustomCostRow[]) {
  for (const row of rows) {
    const hasAny = Object.values(row).some((value) => value.trim());
    if (!hasAny) continue;
    if (!row.provider.trim())
      return "Provider is required for every custom cost row.";
    if (!row.model.trim())
      return "Model is required for every custom cost row.";
    const rates = [row.input, row.output, row.cacheRead, row.cacheWrite].filter(
      (value) => value.trim(),
    );
    if (!rates.length)
      return `${row.provider}/${row.model} needs at least one rate.`;
    for (const rate of rates) {
      if (!/^\d+(\.\d{1,6})?$/.test(rate.trim()))
        return `Invalid rate "${rate}". Use a non-negative decimal with up to 6 decimal places.`;
    }
  }
  return null;
}

function displayRate(value: string) {
  return value || "—";
}

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function stringValue(value: unknown) {
  return typeof value === "string" || typeof value === "number"
    ? String(value)
    : "";
}
