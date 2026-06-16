import { Activity, FileText, GitBranch, Pencil, Play, Plus, Save, SlidersHorizontal, Trash2 } from "lucide-react";
import type { ReactNode } from "react";
import { useEffect, useMemo, useState } from "react";
import { Link } from "../_adapters/_router";
import { useGatewayConfig, useUpdateConfig } from "../_adapters/hooks";
import { Drawer, Dropdown, EmptyState, Field, FieldGroup, PageHeader, Panel, StatusBanner, Tooltip, YamlBlock } from "../components/Primitives";
import { ProviderIcon } from "../components/ProviderIcon";
import { makeEmptyModel, makeEmptyVirtualModel, modelWarnings, providerDisplayName, providerLabel, providerReferenceName, removeModel, removeVirtualModel, upsertModel, upsertVirtualModel } from "../config";
import { isWildcardModelName, wildcardModelPrefix, wildcardResolvedSuffix } from "../modelResolution";
import { KeyValueEditor } from "../policies/PolicyFormControls";
import { CollapsiblePolicySection } from "../policies/PolicyLayout";
import { ResultingYaml } from "../policies/ResultingYaml";
import { cleanEmpty, parseYamlText, toYamlText } from "../policies/policyUtils";
import { useSchemaHelp, type SchemaHelp } from "../schemaHelp";
import type { LlmModel, LlmProvider, LlmVirtualModel, ProviderName } from "../types";
import { ModelMatchesEditor, normalizeMatches } from "./models/ModelMatchesEditor";
import {
  HeaderModifierEditor,
  HealthPolicyEditor,
  PromptCachingEditor,
  YamlMappingEditor,
  headerModifierSummary,
  healthSummary,
  promptCachingSummary,
} from "./models/ModelPolicyEditors";
import { ProviderConfigEditor } from "./models/ProviderConfigEditor";

type ModelHash = { kind: "edit"; modelName: string };
type VirtualRoutingStrategy = "weighted" | "failover" | "conditional";
type ConditionalVirtualTarget = NonNullable<LlmVirtualModel["routing"]["conditional"]>["targets"][number];

function modelHashFromUrl(): ModelHash | null {
  const raw = decodeURIComponent(window.location.hash.replace(/^#/, ""));
  if (!raw) return null;
  if (raw.startsWith("edit=")) {
    const modelName = raw.slice("edit=".length);
    return modelName ? { kind: "edit", modelName } : null;
  }
  if (raw.startsWith("policies=")) {
    const modelName = raw.slice("policies=".length);
    return modelName ? { kind: "edit", modelName } : null;
  }
  if (raw.startsWith("model=")) {
    const modelName = raw.slice("model=".length);
    return modelName ? { kind: "edit", modelName } : null;
  }
  if (raw.startsWith("modelPolicy=")) {
    const modelName = raw.slice("modelPolicy=".length);
    return modelName ? { kind: "edit", modelName } : null;
  }
  return null;
}

function setModelHash(value: ModelHash | null, mode: "push" | "replace") {
  const hash = value ? `#${value.kind}=${encodeURIComponent(value.modelName)}` : "";
  const nextUrl = `${window.location.pathname}${window.location.search}${hash}`;
  if (nextUrl === `${window.location.pathname}${window.location.search}${window.location.hash}`) return;
  if (mode === "push") {
    window.history.pushState(null, "", nextUrl);
  } else {
    window.history.replaceState(null, "", nextUrl);
  }
}

function clearModelSearch() {
  if (!window.location.search) return;
  window.history.replaceState(null, "", `${window.location.pathname}${window.location.hash}`);
}

function providerFromUrl(): string | null {
  const provider = new URLSearchParams(window.location.search).get("provider")?.trim();
  return provider || null;
}

function modelFromProviderReference(providerName: string): LlmModel {
  return {
    ...makeEmptyModel(),
    provider: { reference: providerName },
    params: undefined,
  };
}

export function ModelsPage() {
  const config = useGatewayConfig();
  const update = useUpdateConfig();
  const help = useSchemaHelp();
  const models = useMemo(() => config.data?.llm?.models ?? [], [config.data]);
  const virtualModels = useMemo(() => config.data?.llm?.virtualModels ?? [], [config.data]);
  const providers = useMemo(() => config.data?.llm?.providers ?? [], [config.data]);
  const [editing, setEditing] = useState<{ previousName?: string; model: LlmModel } | null>(() => {
    const provider = providerFromUrl();
    return provider ? { model: modelFromProviderReference(provider) } : null;
  });
  const [editingVirtual, setEditingVirtual] = useState<{ previousName?: string; model: LlmVirtualModel } | null>(null);
  const [modelHash, setModelHashState] = useState<ModelHash | null>(() => modelHashFromUrl());
  const hashEditModel = modelHash?.kind === "edit" ? models.find((model) => model.name === modelHash.modelName) ?? null : null;
  const activeEditing = editing ?? (hashEditModel ? { previousName: hashEditModel.name, model: structuredClone(hashEditModel) } : null);
  const modelRows = useMemo(() => [
    ...models.map((model) => ({ kind: "model" as const, model })),
    ...virtualModels.map((model) => ({ kind: "virtual" as const, model })),
  ], [models, virtualModels]);

  useEffect(() => {
    function syncSelectedFromUrl() {
      update.reset();
      setEditing(null);
      setEditingVirtual(null);
      setModelHashState(modelHashFromUrl());
    }
    window.addEventListener("hashchange", syncSelectedFromUrl);
    window.addEventListener("popstate", syncSelectedFromUrl);
    return () => {
      window.removeEventListener("hashchange", syncSelectedFromUrl);
      window.removeEventListener("popstate", syncSelectedFromUrl);
    };
  }, [update]);

  function openModelEditor(model: LlmModel) {
    update.reset();
    setEditing(null);
    setModelHashState({ kind: "edit", modelName: model.name });
    setModelHash({ kind: "edit", modelName: model.name }, "push");
  }

  function openNewModel() {
    update.reset();
    clearModelSearch();
    setModelHashState(null);
    setModelHash(null, "replace");
    setEditing({ model: makeEmptyModel() });
  }

  function openNewVirtualModel() {
    update.reset();
    clearModelSearch();
    setModelHashState(null);
    setModelHash(null, "replace");
    setEditingVirtual({ model: makeEmptyVirtualModel() });
  }

  function closeModelEditor() {
    update.reset();
    setEditing(null);
    clearModelSearch();
    if (modelHash?.kind === "edit") {
      setModelHashState(null);
      setModelHash(null, "replace");
    }
  }

  function closeVirtualModelEditor() {
    update.reset();
    setEditingVirtual(null);
  }

  return (
    <div className="page-stack">
      <PageHeader
        title="LLM Models"
        description="Onboard provider-backed models and configure model-specific behavior."
        actions={
          <div className="button-row">
            <button className="button primary" type="button" onClick={openNewModel}>
              <Plus size={16} />
              Add model
            </button>
            <button className="button" type="button" onClick={openNewVirtualModel}>
              <GitBranch size={16} />
              Add virtual model
            </button>
          </div>
        }
      />

      {update.isError ? <StatusBanner state="bad" title="Save failed">{update.error!.message}</StatusBanner> : null}
      {update.isSuccess ? <StatusBanner state="ok" title="Configuration saved" /> : null}

      <Panel>
        {config.isLoading ? (
          <StatusBanner state="loading" title="Loading models" />
        ) : config.isError ? (
          <StatusBanner state="bad" title="Configuration API unavailable">{config.error!.message}</StatusBanner>
        ) : modelRows.length === 0 ? (
          <EmptyState
            title="No models configured"
            description="Create the first model to make LLM traffic available through the gateway."
            action={
              <div className="button-row">
                <button className="button primary" type="button" onClick={openNewModel}>
                  <Plus size={16} />
                  Add model
                </button>
                <button className="button" type="button" onClick={openNewVirtualModel}>
                  <GitBranch size={16} />
                  Add virtual model
                </button>
              </div>
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
                  <th>Policy state</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {modelRows.map((row) => {
                  if (row.kind === "virtual") {
                    const model = row.model;
                    return (
                      <tr key={`virtual:${model.name}`}>
                        <td className="strong">{model.name}</td>
                        <td><span className="badge"><GitBranch size={14} /> Virtual</span></td>
                        <td>{virtualModelSummary(model)}</td>
                        <td><span className="badge ok">{virtualModelStrategy(model)}</span></td>
                        <td className="row-actions">
                          <Tooltip content="Open in playground">
                            <Link
                              className="icon-button"
                              aria-label="Open in playground"
                              to={`/llm-playground?modelName=${encodeURIComponent(model.name)}`}
                              search={{ model: model.name }}
                            >
                              <Play size={16} />
                            </Link>
                          </Tooltip>
                          <Tooltip content="Edit model">
                            <button className="icon-button" aria-label="Edit model" type="button" onClick={() => setEditingVirtual({ previousName: model.name, model: structuredClone(model) })}>
                              <Pencil size={16} />
                            </button>
                          </Tooltip>
                          <Tooltip content="Delete model">
                            <button
                              className="icon-button danger"
                              aria-label="Delete model"
                              type="button"
                              onClick={() => update.mutate((next) => removeVirtualModel(next, model.name))}
                            >
                              <Trash2 size={16} />
                            </button>
                          </Tooltip>
                        </td>
                      </tr>
                    );
                  }
                  const model = row.model;
                  const warnings = modelWarnings(model);
                  return (
                    <tr key={`model:${model.name}`}>
                      <td className="strong">{model.name}</td>
                      <td><ModelProviderBadge model={model} providers={providers} /></td>
                      <td>{model.params?.model || "incoming model"}</td>
                      <td><ModelPolicyState model={model} warnings={warnings.length} /></td>
                      <td className="row-actions">
                        <Tooltip content="Open in playground">
                          <Link
                            className="icon-button"
                            aria-label="Open in playground"
                            to={`/llm-playground?modelName=${encodeURIComponent(model.name)}`}
                            search={{ model: model.name }}
                          >
                            <Play size={16} />
                          </Link>
                        </Tooltip>
                        <Tooltip content="Edit model">
                          <button className="icon-button" aria-label="Edit model" type="button" onClick={() => openModelEditor(model)}>
                            <Pencil size={16} />
                          </button>
                        </Tooltip>
                        <Tooltip content="Delete model">
                          <button
                            className="icon-button danger"
                            aria-label="Delete model"
                            type="button"
                            onClick={() => update.mutate((next) => removeModel(next, model.name))}
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
        <ModelEditor
          key={activeEditing.previousName ?? "new"}
          previousName={activeEditing.previousName}
          initial={activeEditing.model}
          providers={providers}
          help={help}
          saving={update.isPending}
          saveError={update.isError ? update.error!.message : null}
          onCancel={activeEditing.previousName ? closeModelEditor : () => setEditing(null)}
          onSave={(model, previousName) => {
            update.mutate((next) => upsertModel(next, model, previousName), {
              onSuccess: previousName ? closeModelEditor : () => setEditing(null),
            });
          }}
        />
      ) : null}
      {editingVirtual ? (
        <VirtualModelEditor
          key={editingVirtual.previousName ?? "new"}
          previousName={editingVirtual.previousName}
          initial={editingVirtual.model}
          baseModels={models}
          saving={update.isPending}
          saveError={update.isError ? update.error!.message : null}
          onCancel={closeVirtualModelEditor}
          onSave={(model, previousName) => update.mutate((next) => upsertVirtualModel(next, model, previousName), {
            onSuccess: closeVirtualModelEditor,
          })}
        />
      ) : null}
    </div>
  );
}

function ModelEditor(props: {
  initial: LlmModel;
  providers: LlmProvider[];
  previousName?: string;
  help: SchemaHelp;
  saving: boolean;
  saveError?: string | null;
  onCancel: () => void;
  onSave: (model: LlmModel, previousName?: string) => void;
}) {
  const [model, setModel] = useState<LlmModel>(props.initial);
  const [upstreamMode, setUpstreamMode] = useState<UpstreamModelMode>(() => initialUpstreamMode(props.initial));
  const [explicitModel, setExplicitModel] = useState(props.initial.params?.model ?? "");
  const [customModelExpression, setCustomModelExpression] = useState(() => props.initial.transformation?.model ?? "llmRequest.model");
  const [transformation, setTransformation] = useState<Record<string, string>>(() => expressionMap(props.initial.transformation));
  const [health, setHealth] = useState<LlmModel["health"]>(() => props.initial.health ?? null);
  const [defaultsText, setDefaultsText] = useState(() => optionalMappingYamlText(props.initial.defaults));
  const [overridesText, setOverridesText] = useState(() => optionalMappingYamlText(props.initial.overrides));
  const [requestHeaders, setRequestHeaders] = useState<LlmModel["requestHeaders"]>(() => props.initial.requestHeaders ?? null);
  const [responseHeaders, setResponseHeaders] = useState<LlmModel["responseHeaders"]>(() => props.initial.responseHeaders ?? null);
  const [promptCaching, setPromptCaching] = useState<LlmModel["promptCaching"]>(() => props.initial.promptCaching ?? null);
  const [policyError, setPolicyError] = useState<string | null>(null);
  const warnings = modelWarnings(model);
  const policyPatch = buildModelPolicyPatch({
    transformation,
    health,
    defaultsText,
    overridesText,
    requestHeaders,
    responseHeaders,
    promptCaching,
  });
  const preview = cleanEmpty(applyUpstreamMode({ ...model, ...policyPatch.value, matches: normalizeMatches(model.matches) }, upstreamMode, explicitModel, customModelExpression)) as LlmModel | undefined;
  const providerSelected = Boolean(model.provider);

  function save() {
    if (!preview?.provider) return;
    if (policyPatch.error) {
      setPolicyError(policyPatch.error);
      return;
    }
    setPolicyError(null);
    props.onSave(preview ?? model, props.previousName);
  }

  return (
    <Drawer
      title={props.previousName ? "Edit model" : "Add model"}
      onClose={props.onCancel}
      footer={
        <div className="button-row">
          <button className="button" type="button" onClick={props.onCancel}>Cancel</button>
          <button className="button primary" type="button" disabled={props.saving || !model.name.trim() || !preview?.provider} onClick={save}>
            <Save size={16} />
            Save model
          </button>
        </div>
      }
    >
      <div className="form-grid">
        <Field
          label="Gateway model name"
          tooltip={props.help.description(["$defs", "LocalLLMModels", "properties", "name"], "The model name matched from incoming requests. Use an exact name like gpt-4.1-mini or a wildcard like openai/*.")}
        >
          <input
            value={model.name}
            onChange={(event) => setModel({ ...model, name: event.target.value })}
            placeholder="openai/*"
          />
        </Field>
      </div>

      <ProviderConfigEditor
        provider={model.provider}
        params={model.params ?? undefined}
        auth={model.auth}
        providers={props.providers}
        help={props.help}
        onProviderChange={(provider, params) => setModel((current) => ({ ...current, provider, params }))}
        onParamsChange={(params) => setModel((current) => ({ ...current, params }))}
        onAuthChange={(auth) => setModel((current) => ({ ...current, auth }))}
      />

      {providerSelected ? (
        <>
          <UpstreamModelFields
            mode={upstreamMode}
            explicitModel={explicitModel}
            customModelExpression={customModelExpression}
            gatewayModelName={model.name}
            help={props.help}
            setMode={setUpstreamMode}
            setExplicitModel={setExplicitModel}
            setCustomModelExpression={setCustomModelExpression}
          />

          <ModelMatchesEditor
            matches={model.matches ?? []}
            onChange={(matches) => setModel((current) => ({ ...current, matches }))}
          />

          <ModelPoliciesInline
            model={props.initial}
            help={props.help}
            transformation={transformation}
            health={health}
            defaultsText={defaultsText}
            overridesText={overridesText}
            requestHeaders={requestHeaders}
            responseHeaders={responseHeaders}
            promptCaching={promptCaching}
            setTransformation={setTransformation}
            setHealth={setHealth}
            setDefaultsText={setDefaultsText}
            setOverridesText={setOverridesText}
            setRequestHeaders={setRequestHeaders}
            setResponseHeaders={setResponseHeaders}
            setPromptCaching={setPromptCaching}
          />
        </>
      ) : null}

      {providerSelected && warnings.length ? (
        <div className="model-warning-block">
          <StatusBanner state="warn" title="Model warnings"><ul>{warnings.map((warning) => <li key={warning}>{warning}</li>)}</ul></StatusBanner>
        </div>
      ) : null}
      {policyError ? <StatusBanner state="bad" title="Invalid model policies">{policyError}</StatusBanner> : null}
      {props.saveError ? <StatusBanner state="bad" title="Save failed">{props.saveError}</StatusBanner> : null}

      {providerSelected ? (
        <details>
          <summary>Generated model config</summary>
          <YamlBlock value={preview ?? {}} />
        </details>
      ) : null}
    </Drawer>
  );
}

type UpstreamModelMode = "incoming" | "explicit" | "strip" | "custom";

function UpstreamModelFields(props: {
  mode: UpstreamModelMode;
  explicitModel: string;
  customModelExpression: string;
  gatewayModelName: string;
  help: SchemaHelp;
  setMode: (mode: UpstreamModelMode) => void;
  setExplicitModel: (model: string) => void;
  setCustomModelExpression: (expression: string) => void;
}) {
  const prefix = stripPrefixCandidate(props.gatewayModelName);
  return (
    <>
      <FieldGroup
        label="Upstream model"
        tooltip={props.help.description(["$defs", "LocalLLMParams", "properties", "model"])}
      >
        <div className="segmented-control">
          <button className={props.mode === "incoming" ? "active" : ""} type="button" onClick={() => props.setMode("incoming")}>
            Use incoming model
          </button>
          <button className={props.mode === "explicit" ? "active" : ""} type="button" onClick={() => props.setMode("explicit")}>
            Use explicit model
          </button>
          {prefix ? (
            <button className={props.mode === "strip" ? "active" : ""} type="button" onClick={() => props.setMode("strip")}>
              Strip {prefix}
            </button>
          ) : null}
          <button className={props.mode === "custom" ? "active" : ""} type="button" onClick={() => props.setMode("custom")}>
            Custom
          </button>
        </div>
      </FieldGroup>

      {props.mode === "explicit" ? (
        <Field label="Explicit upstream model" tooltip={props.help.description(["$defs", "LocalLLMParams", "properties", "model"])}>
          <input value={props.explicitModel} onChange={(event) => props.setExplicitModel(event.target.value)} placeholder="gpt-4.1-mini" />
        </Field>
      ) : null}
      {props.mode === "custom" ? (
        <Field label="Model CEL expression" tooltip={props.help.description(["$defs", "LocalLLMModels", "properties", "transformation"])}>
          <textarea
            className="code-textarea compact"
            value={props.customModelExpression}
            onChange={(event) => props.setCustomModelExpression(event.target.value)}
            placeholder="llmRequest.model.stripPrefix(&quot;anthropic/&quot;)"
            rows={4}
          />
        </Field>
      ) : null}
    </>
  );
}

function ModelPoliciesInline(props: {
  model: LlmModel;
  help: SchemaHelp;
  transformation: Record<string, string>;
  health: LlmModel["health"];
  defaultsText: string;
  overridesText: string;
  requestHeaders: LlmModel["requestHeaders"];
  responseHeaders: LlmModel["responseHeaders"];
  promptCaching: LlmModel["promptCaching"];
  setTransformation: (value: Record<string, string>) => void;
  setHealth: (value: LlmModel["health"] | null) => void;
  setDefaultsText: (value: string) => void;
  setOverridesText: (value: string) => void;
  setRequestHeaders: (value: LlmModel["requestHeaders"] | null) => void;
  setResponseHeaders: (value: LlmModel["responseHeaders"] | null) => void;
  setPromptCaching: (value: LlmModel["promptCaching"] | null) => void;
}) {
  const patch = buildModelPolicyPatch(props);
  const transformationEnabled = Object.keys(expressionMap(props.model.transformation)).length > 0;
  const defaultsEnabled = Boolean(props.model.defaults && Object.keys(props.model.defaults).length);
  const overridesEnabled = Boolean(props.model.overrides && Object.keys(props.model.overrides).length);
  return (
    <CollapsiblePolicySection
      icon={<SlidersHorizontal size={17} />}
      title="Model policies"
      description={modelPolicySummary({ ...props.model, ...patch.value })}
    >
      <div className="policy-editor-stack">
        <CollapsiblePolicySection
          icon={<SlidersHorizontal size={17} />}
          title="Transformation"
          description={Object.keys(props.transformation).length ? `${Object.keys(props.transformation).length} fields configured` : "No fields configured"}
          defaultOpen={transformationEnabled}
        >
          <KeyValueEditor
            label="LLM request fields"
            tooltip={props.help.description(["$defs", "LocalLLMModels", "properties", "transformation"])}
            values={props.transformation}
            keyPlaceholder="field name"
            valuePlaceholder="CEL expression"
            valueKind="cel"
            onChange={props.setTransformation}
          />
        </CollapsiblePolicySection>
        <CollapsiblePolicySection icon={<FileText size={17} />} title="Default request values" description={props.defaultsText.trim() ? "Defaults configured" : "No defaults configured"} defaultOpen={defaultsEnabled}>
          <YamlMappingEditor label="Defaults YAML" tooltip={props.help.description(["$defs", "LocalLLMModels", "properties", "defaults"])} value={props.defaultsText} onChange={props.setDefaultsText} placeholder="temperature: 0.2" />
        </CollapsiblePolicySection>
        <CollapsiblePolicySection icon={<FileText size={17} />} title="Override request values" description={props.overridesText.trim() ? "Overrides configured" : "No overrides configured"} defaultOpen={overridesEnabled}>
          <YamlMappingEditor label="Overrides YAML" tooltip={props.help.description(["$defs", "LocalLLMModels", "properties", "overrides"])} value={props.overridesText} onChange={props.setOverridesText} placeholder="stream: false" />
        </CollapsiblePolicySection>
        <CollapsiblePolicySection icon={<SlidersHorizontal size={17} />} title="Request headers" description={headerModifierSummary(props.requestHeaders, "request")} defaultOpen={Boolean(props.model.requestHeaders)}>
          <HeaderModifierEditor value={props.requestHeaders} help={props.help} onChange={props.setRequestHeaders} />
        </CollapsiblePolicySection>
        <CollapsiblePolicySection icon={<SlidersHorizontal size={17} />} title="Response headers" description={headerModifierSummary(props.responseHeaders, "response")} defaultOpen={Boolean(props.model.responseHeaders)}>
          <HeaderModifierEditor value={props.responseHeaders} help={props.help} onChange={props.setResponseHeaders} />
        </CollapsiblePolicySection>
        <CollapsiblePolicySection icon={<Activity size={17} />} title="Health" description={healthSummary(props.health)} defaultOpen={Boolean(props.model.health)}>
          <HealthPolicyEditor health={props.health} help={props.help} onChange={props.setHealth} />
        </CollapsiblePolicySection>
        <CollapsiblePolicySection icon={<SlidersHorizontal size={17} />} title="Prompt caching" description={promptCachingSummary(props.promptCaching)} defaultOpen={Boolean(props.model.promptCaching)}>
          <PromptCachingEditor value={props.promptCaching} help={props.help} onChange={props.setPromptCaching} />
        </CollapsiblePolicySection>
        <ResultingYaml value={patch.value} />
      </div>
    </CollapsiblePolicySection>
  );
}

function buildModelPolicyPatch(args: {
  transformation: Record<string, string>;
  health: LlmModel["health"];
  defaultsText: string;
  overridesText: string;
  requestHeaders: LlmModel["requestHeaders"];
  responseHeaders: LlmModel["responseHeaders"];
  promptCaching: LlmModel["promptCaching"];
}) {
  try {
    const defaults = parseOptionalYamlMapping(args.defaultsText);
    const overrides = parseOptionalYamlMapping(args.overridesText);
    const transformation = cleanEmpty(args.transformation) as LlmModel["transformation"] | undefined;
    const health = cleanEmpty(args.health) as LlmModel["health"] | undefined;
    const requestHeaders = cleanEmpty(args.requestHeaders) as LlmModel["requestHeaders"] | undefined;
    const responseHeaders = cleanEmpty(args.responseHeaders) as LlmModel["responseHeaders"] | undefined;
    const promptCaching = cleanEmpty(args.promptCaching) as LlmModel["promptCaching"] | undefined;
    return {
      value: {
        defaults,
        overrides,
        transformation: transformation && Object.keys(transformation).length ? transformation : null,
        requestHeaders: requestHeaders && Object.keys(requestHeaders).length ? requestHeaders : null,
        responseHeaders: responseHeaders && Object.keys(responseHeaders).length ? responseHeaders : null,
        health: health && Object.keys(health).length ? health : null,
        promptCaching: promptCaching && Object.keys(promptCaching).length ? promptCaching : null,
      } satisfies Partial<LlmModel>,
      error: null,
    };
  } catch (error) {
    return {
      value: {},
      error: error instanceof Error ? error.message : "Invalid policy configuration",
    };
  }
}

function modelPolicySummary(model: Partial<LlmModel>) {
  const policies = [
    model.defaults && Object.keys(model.defaults).length ? "defaults" : null,
    model.overrides && Object.keys(model.overrides).length ? "overrides" : null,
    model.transformation && Object.keys(model.transformation).length ? "transformation" : null,
    model.requestHeaders ? "request headers" : null,
    model.responseHeaders ? "response headers" : null,
    model.health ? "health" : null,
    model.promptCaching ? "prompt caching" : null,
  ].filter(Boolean);
  return policies.length ? `${policies.length} configured` : "No model policies configured";
}

function VirtualModelEditor(props: {
  initial: LlmVirtualModel;
  previousName?: string;
  baseModels: LlmModel[];
  saving: boolean;
  saveError?: string | null;
  onCancel: () => void;
  onSave: (model: LlmVirtualModel, previousName?: string) => void;
}) {
  const [model, setModel] = useState<LlmVirtualModel>(props.initial);
  const strategy = model.routing.conditional ? "conditional" : model.routing.failover ? "failover" : "weighted";
  const weightedTargets = model.routing.weighted?.targets ?? [];
  const failoverTargets = model.routing.failover?.targets ?? [];
  const conditionalTargets = model.routing.conditional?.targets ?? [];
  const targetOptions = modelTargetOptions(props.baseModels);
  const preview = cleanEmpty(model) as LlmVirtualModel | undefined;
  const activeTargets = strategy === "weighted" ? weightedTargets : strategy === "failover" ? failoverTargets : conditionalTargets;
  const hasInvalidTarget = activeTargets.some((target) => !target.model.trim() || isIncompleteWildcardTarget(target.model, props.baseModels));
  const hasInvalidConditionalFallback = strategy === "conditional" && conditionalTargets.some((target, index) => !target.when?.trim() && index !== conditionalTargets.length - 1);
  const failoverGroups = failoverTargetGroups(failoverTargets);
  const defaultTarget = defaultVirtualTargetModel(props.baseModels);

  function setStrategy(next: VirtualRoutingStrategy) {
    if (next === "weighted") {
      setModel((current) => ({
        ...current,
        routing: {
          weighted: {
            targets: current.routing.weighted?.targets?.length
              ? current.routing.weighted.targets
              : [{ model: defaultTarget, weight: 1 }],
          },
        },
      }));
      return;
    }
    if (next === "conditional") {
      setModel((current) => ({
        ...current,
        routing: {
          conditional: {
            targets: current.routing.conditional?.targets?.length
              ? current.routing.conditional.targets
              : [{ when: "json(request.body).route == \"default\"", model: defaultTarget }, { model: defaultTarget }],
          },
        },
      }));
      return;
    }
    setModel((current) => ({
      ...current,
      routing: {
        failover: {
          targets: current.routing.failover?.targets?.length
            ? current.routing.failover.targets
            : [{ model: defaultTarget, priority: 0 }],
        },
      },
    }));
  }

  function updateWeighted(index: number, patch: Partial<NonNullable<LlmVirtualModel["routing"]["weighted"]>["targets"][number]>) {
    setModel((current) => {
      const targets = [...(current.routing.weighted?.targets ?? [])];
      targets[index] = { ...targets[index], ...patch };
      return { ...current, routing: { weighted: { targets } } };
    });
  }

  function updateFailoverGroups(groups: Array<Array<NonNullable<LlmVirtualModel["routing"]["failover"]>["targets"][number]>>) {
    setModel((current) => ({
      ...current,
      routing: {
        failover: {
          targets: groups.flatMap((group, priority) => group.map((target) => ({ ...target, priority }))),
        },
      },
    }));
  }

  function updateConditional(index: number, patch: Partial<ConditionalVirtualTarget>) {
    setModel((current) => {
      const targets = [...(current.routing.conditional?.targets ?? [])];
      targets[index] = cleanEmpty({ ...targets[index], ...patch }) as ConditionalVirtualTarget;
      return { ...current, routing: { conditional: { targets } } };
    });
  }

  return (
    <Drawer
      title={props.previousName ? "Edit virtual model" : "Add virtual model"}
      onClose={props.onCancel}
      footer={
        <div className="button-row">
          <button className="button" type="button" onClick={props.onCancel}>Cancel</button>
          <button className="button primary" type="button" disabled={props.saving || !model.name.trim() || activeTargets.length === 0 || hasInvalidTarget || hasInvalidConditionalFallback} onClick={() => props.onSave(preview ?? model, props.previousName)}>
            <Save size={16} />
            Save virtual model
          </button>
        </div>
      }
    >
      <Field
        label="Virtual model name"
        tooltip="The public model name clients request. Routing selects one of the target models."
      >
        <input value={model.name} onChange={(event) => setModel({ ...model, name: event.target.value })} placeholder="resilient" />
      </Field>

      <FieldGroup label="Routing strategy">
        <div className="segmented-control">
          <button className={strategy === "weighted" ? "active" : ""} type="button" onClick={() => setStrategy("weighted")}>
            Weighted
          </button>
          <button className={strategy === "failover" ? "active" : ""} type="button" onClick={() => setStrategy("failover")}>
            Failover
          </button>
          <button className={strategy === "conditional" ? "active" : ""} type="button" onClick={() => setStrategy("conditional")}>
            Conditional
          </button>
        </div>
      </FieldGroup>

      {strategy === "weighted" ? (
        <FieldGroup label="Weighted targets">
          <div className="target-list">
            {weightedTargets.map((target, index) => (
              <div className="target-row weighted" key={index}>
                <VirtualTargetSelector
                  label="Model"
                  targetModel={target.model}
                  baseModels={props.baseModels}
                  options={targetOptions}
                  onChange={(value) => updateWeighted(index, { model: value })}
                />
                <label className="target-field">
                  <span className="target-label">Weight</span>
                  <input
                    aria-label="Weight"
                    value={target.weight ?? 1}
                    onChange={(event) => updateWeighted(index, { weight: Number(event.target.value) || 1 })}
                    type="number"
                    min={1}
                  />
                </label>
                <button className="icon-button danger" type="button" aria-label="Remove target" onClick={() => setModel((current) => ({ ...current, routing: { weighted: { targets: (current.routing.weighted?.targets ?? []).filter((_, itemIndex) => itemIndex !== index) } } }))}>
                  <Trash2 size={16} />
                </button>
              </div>
            ))}
          </div>
          <button className="button" type="button" onClick={() => setModel((current) => ({ ...current, routing: { weighted: { targets: [...(current.routing.weighted?.targets ?? []), { model: defaultTarget, weight: 1 }] } } }))}>
            <Plus size={16} />
            Add target
          </button>
        </FieldGroup>
      ) : strategy === "failover" ? (
        <FieldGroup label="Failover targets">
          <div className="failover-group-list">
            {failoverGroups.map((group, groupIndex) => (
              <section className="match-card" key={groupIndex}>
                <div className="match-card-header">
                  <strong>{groupIndex === 0 ? "First attempt" : `Fallback group ${groupIndex + 1}`}</strong>
                  <Tooltip content="Remove group">
                    <button
                      className="icon-button danger"
                      type="button"
                      aria-label={`Remove failover group ${groupIndex + 1}`}
                      onClick={() => updateFailoverGroups(failoverGroups.filter((_, itemIndex) => itemIndex !== groupIndex))}
                    >
                      <Trash2 size={15} />
                    </button>
                  </Tooltip>
                </div>
                <div className="match-card-body">
                  <div className="target-list">
                    {group.map((target, targetIndex) => (
                      <div className="target-row failover" key={targetIndex}>
                        <VirtualTargetSelector
                          label="Model"
                          targetModel={target.model}
                          baseModels={props.baseModels}
                          options={targetOptions}
                          onChange={(value) => updateFailoverGroups(failoverGroups.map((item, itemIndex) => itemIndex === groupIndex
                            ? item.map((groupTarget, groupTargetIndex) => groupTargetIndex === targetIndex ? { ...groupTarget, model: value } : groupTarget)
                            : item))}
                        />
                        <button
                          className="icon-button danger"
                          type="button"
                          aria-label="Remove target"
                          onClick={() => updateFailoverGroups(failoverGroups.map((item, itemIndex) => itemIndex === groupIndex
                            ? item.filter((_, groupTargetIndex) => groupTargetIndex !== targetIndex)
                            : item).filter((item) => item.length > 0))}
                        >
                          <Trash2 size={16} />
                        </button>
                      </div>
                    ))}
                  </div>
                  <button
                    className="button small"
                    type="button"
                    onClick={() => updateFailoverGroups(failoverGroups.map((item, itemIndex) => itemIndex === groupIndex ? [...item, { model: defaultTarget, priority: groupIndex }] : item))}
                  >
                    <Plus size={16} />
                    Add target
                  </button>
                </div>
              </section>
            ))}
          </div>
          <button className="button" type="button" onClick={() => updateFailoverGroups([...failoverGroups, [{ model: defaultTarget, priority: failoverGroups.length }]])}>
            <Plus size={16} />
            Add fallback group
          </button>
        </FieldGroup>
      ) : (
        <FieldGroup label="Conditional targets" tooltip="Rules are evaluated in order. The first matching condition selects the model. Leave the final condition blank for fallback.">
          <div className="target-list">
            {conditionalTargets.map((target, index) => {
              const isFallback = !target.when?.trim();
              return (
                <div className="conditional-target-card" key={index}>
                  <div className="match-card-header">
                    <strong>{isFallback ? "Fallback" : `Rule ${index + 1}`}</strong>
                    <Tooltip content="Remove rule">
                      <button
                        className="icon-button danger"
                        type="button"
                        aria-label="Remove conditional target"
                        onClick={() => setModel((current) => ({ ...current, routing: { conditional: { targets: (current.routing.conditional?.targets ?? []).filter((_, itemIndex) => itemIndex !== index) } } }))}
                      >
                        <Trash2 size={15} />
                      </button>
                    </Tooltip>
                  </div>
                  <div className="conditional-target-body">
                    <Field label="Condition">
                      <textarea
                        className="mono-input compact"
                        aria-label={`Condition ${index + 1}`}
                        rows={3}
                        value={target.when ?? ""}
                        onChange={(event) => updateConditional(index, { when: event.target.value.trim() ? event.target.value : undefined })}
                        placeholder={index === conditionalTargets.length - 1 ? "Blank final condition means fallback" : "json(request.body).route == \"code\""}
                      />
                    </Field>
                    <VirtualTargetSelector
                      label="Target model"
                      targetModel={target.model}
                      baseModels={props.baseModels}
                      options={targetOptions}
                      onChange={(value) => updateConditional(index, { model: value })}
                    />
                  </div>
                </div>
              );
            })}
          </div>
          {hasInvalidConditionalFallback ? <StatusBanner state="warn" title="Only the final conditional target can omit a condition." /> : null}
          <div className="button-row">
            <button className="button" type="button" onClick={() => setModel((current) => ({ ...current, routing: { conditional: { targets: [...(current.routing.conditional?.targets ?? []), { when: "json(request.body).route == \"code\"", model: defaultTarget }] } } }))}>
              <Plus size={16} />
              Add rule
            </button>
            <button className="button" type="button" onClick={() => setModel((current) => ({ ...current, routing: { conditional: { targets: [...(current.routing.conditional?.targets ?? []).filter((target) => target.when?.trim()), { model: defaultTarget }] } } }))}>
              <Plus size={16} />
              Add fallback
            </button>
          </div>
        </FieldGroup>
      )}

      <details>
        <summary>Generated virtual model config</summary>
        <YamlBlock value={preview ?? {}} />
      </details>
      {props.saveError ? <StatusBanner state="bad" title="Save failed">{props.saveError}</StatusBanner> : null}
    </Drawer>
  );
}

function VirtualTargetSelector(props: {
  label: string;
  targetModel: string;
  baseModels: LlmModel[];
  options: Array<{ value: string; label: ReactNode; icon?: ReactNode; searchText?: string }>;
  onChange: (model: string) => void;
}) {
  const selectedModelName = selectedConfiguredTargetName(props.targetModel, props.baseModels);
  const selectedModel = props.baseModels.find((model) => model.name === selectedModelName);
  const wildcard = Boolean(selectedModel && isWildcardModelName(selectedModel.name));
  const wildcardPrefix = selectedModel ? wildcardModelPrefix(selectedModel.name) : "";
  const resolvedSuffix = wildcard ? wildcardResolvedSuffix(props.targetModel, selectedModelName, wildcardPrefix) : "";

  return (
    <div className="target-field">
      <span className="target-label">{props.label}</span>
      <Dropdown
        ariaLabel={props.label}
        value={selectedModelName}
        searchable
        options={props.options}
        placeholder="No configured models"
        onChange={(value) => props.onChange(isWildcardModelName(value) ? wildcardModelPrefix(value) : value)}
      />
      {wildcard ? (
        <div className="target-resolved-composite">
          {wildcardPrefix ? <span className="target-prefix">{wildcardPrefix}</span> : null}
          <input
            className="target-resolved-input"
            aria-label="Resolved model"
            value={resolvedSuffix}
            onChange={(event) => props.onChange(`${wildcardPrefix}${event.target.value}`)}
            placeholder="claude-haiku-4-5"
          />
        </div>
      ) : null}
    </div>
  );
}

function ProviderBadge(props: { provider: ProviderName }) {
  return (
    <span className="badge provider-badge">
      <ProviderIcon provider={props.provider} />
      {providerDisplayName(props.provider)}
    </span>
  );
}

function ModelProviderBadge(props: { model: LlmModel; providers: LlmProvider[] }) {
  const reference = providerReferenceName(props.model.provider);
  if (reference) {
    const shared = props.providers.find((provider) => provider.name === reference);
    const provider = shared ? providerLabel(shared.provider) : "custom";
    return (
      <span className="badge provider-badge">
        <ProviderIcon provider={provider as ProviderName} />
        {reference}
        <span className="muted">reference</span>
      </span>
    );
  }
  return <ProviderBadge provider={providerLabel(props.model.provider) as ProviderName} />;
}

function ModelPolicyState(props: { model: LlmModel; warnings: number }) {
  const policies = [
    props.model.defaults && Object.keys(props.model.defaults).length ? "defaults" : null,
    props.model.overrides && Object.keys(props.model.overrides).length ? "overrides" : null,
    props.model.transformation && Object.keys(props.model.transformation).length ? "transformation" : null,
    props.model.requestHeaders ? "requestHeaders" : null,
    props.model.responseHeaders ? "responseHeaders" : null,
    props.model.health ? "health" : null,
    props.model.promptCaching ? "promptCaching" : null,
  ].filter(Boolean);
  if (props.warnings > 0) return <span className="badge warn">{props.warnings} warnings</span>;
  if (props.model.auth) return <span className="badge">Custom auth detected</span>;
  if (policies.length > 0) return <span className="badge ok">{policies.length} {policies.length === 1 ? "policy" : "policies"}</span>;
  return <span className="badge">none</span>;
}

function modelTargetOptions(models: LlmModel[]) {
  return models.map((model) => ({
    value: model.name,
    label: model.name,
    icon: <ProviderIcon provider={providerLabel(model.provider) as ProviderName} />,
    searchText: `${model.name} ${providerLabel(model.provider)}`,
  }));
}

function defaultVirtualTargetModel(models: LlmModel[]) {
  const model = models[0];
  if (!model) return "";
  return isWildcardModelName(model.name) ? wildcardModelPrefix(model.name) : model.name;
}

function selectedConfiguredTargetName(targetModel: string, baseModels: LlmModel[]) {
  const exact = baseModels.find((model) => model.name === targetModel);
  if (exact) return exact.name;
  const wildcard = baseModels.find((model) => isWildcardModelName(model.name) && wildcardMatchesModel(model.name, targetModel));
  return wildcard?.name ?? baseModels[0]?.name ?? "";
}

function wildcardMatchesModel(pattern: string, model: string) {
  if (pattern === "*") return Boolean(model.trim());
  const wildcardIndex = pattern.indexOf("*");
  if (wildcardIndex < 0) return pattern === model;
  const prefix = pattern.slice(0, wildcardIndex);
  const suffix = pattern.slice(wildcardIndex + 1);
  return model.startsWith(prefix) && model.endsWith(suffix) && model.length > prefix.length + suffix.length;
}

function isIncompleteWildcardTarget(targetModel: string, baseModels: LlmModel[]) {
  const selected = selectedConfiguredTargetName(targetModel, baseModels);
  if (!selected || !isWildcardModelName(selected)) return false;
  return targetModel === selected || targetModel === wildcardModelPrefix(selected);
}

function failoverTargetGroups(targets: NonNullable<LlmVirtualModel["routing"]["failover"]>["targets"]) {
  const priorities = [...new Set(targets.map((target) => target.priority ?? 0))].sort((left, right) => left - right);
  return priorities.map((priority) => targets.filter((target) => (target.priority ?? 0) === priority));
}

function virtualModelStrategy(model: LlmVirtualModel) {
  return model.routing.failover ? "failover" : "weighted";
}

function virtualModelSummary(model: LlmVirtualModel) {
  if (model.routing.failover) {
    const targets = model.routing.failover.targets ?? [];
    const priorities = new Set(targets.map((target) => target.priority)).size;
    return `${priorities} ${priorities === 1 ? "priority" : "priorities"}, ${targets.length} ${targets.length === 1 ? "target" : "targets"}`;
  }
  const targets = model.routing.weighted?.targets ?? [];
  return `${targets.length} weighted ${targets.length === 1 ? "target" : "targets"}`;
}

function parseOptionalYamlMapping(text: string) {
  const trimmed = text.trim();
  if (!trimmed || trimmed === "{}") return null;
  const parsed = parseYamlText(trimmed);
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("Expected a YAML mapping.");
  }
  return parsed as Record<string, unknown>;
}

function optionalMappingYamlText(value: Record<string, unknown> | null | undefined) {
  return value && Object.keys(value).length ? toYamlText(value) : "";
}

function initialUpstreamMode(model: LlmModel): UpstreamModelMode {
  if (model.params?.model) return "explicit";
  const expression = model.transformation?.model;
  if (expression && expression === stripPrefixExpression(stripPrefixCandidate(model.name))) return "strip";
  if (expression) return "custom";
  return "incoming";
}

function stripPrefixCandidate(name: string) {
  const slash = name.indexOf("/");
  if (slash < 0) return null;
  return name.slice(0, slash + 1);
}

function stripPrefixExpression(prefix: string | null) {
  if (!prefix) return null;
  return `llmRequest.model.stripPrefix("${prefix}")`;
}

function applyUpstreamMode(
  model: LlmModel,
  mode: UpstreamModelMode,
  explicitModel: string,
  customModelExpression: string,
): LlmModel {
  const next: LlmModel = structuredClone(model);
  const transformation = { ...(next.transformation ?? {}) };
  delete transformation.model;
  const prefixExpression = stripPrefixExpression(stripPrefixCandidate(next.name));

  if (mode === "strip" && prefixExpression) {
    transformation.model = prefixExpression;
  } else if (mode === "custom" && customModelExpression.trim()) {
    transformation.model = customModelExpression.trim();
  }

  next.transformation = Object.keys(transformation).length ? transformation : null;

  if (providerReferenceName(next.provider)) {
    next.params = mode === "explicit" && explicitModel ? { model: explicitModel } : undefined;
    return next;
  }
  next.params = { ...(next.params ?? {}) };

  if (mode === "explicit") {
    next.params.model = explicitModel || null;
  } else {
    next.params.model = null;
  }

  return next;
}

function expressionMap(value: LlmModel["transformation"]): Record<string, string> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return Object.fromEntries(
    Object.entries(value)
      .filter((entry): entry is [string, string] => typeof entry[1] === "string"),
  );
}
