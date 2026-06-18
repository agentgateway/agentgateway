import { useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import {
  Activity,
  FileText,
  GitBranch,
  Pencil,
  Play,
  Plus,
  Save,
  ShieldCheck,
  SlidersHorizontal,
  Trash2,
} from "lucide-react";
import {
  invalidProviderApiKey,
  makeEmptyModel,
  makeEmptyVirtualModel,
  modelWarnings,
  providerDisplayName,
  providerLabel,
  providerReferenceName,
  removeModel,
  removeVirtualModel,
  upsertModel,
  upsertVirtualModel,
} from "../config";
import { useGatewayConfig, useUpdateConfig } from "../hooks";
import { MiniMonacoEditor } from "../components/MiniMonacoEditor";
import { CatalogModelSelector } from "../components/CatalogModelSelector";
import {
  Drawer,
  Dropdown,
  EmptyState,
  Field,
  FieldGroup,
  PageHeader,
  Panel,
  StatusBanner,
  Tooltip,
  YamlBlock,
} from "../components/Primitives";
import { ProviderIcon } from "../components/ProviderIcon";
import { KeyValueEditor } from "../policies/PolicyFormControls";
import { AuthorizationPolicyEditor } from "../policies/AuthorizationPolicyEditor";
import { CollapsiblePolicySection } from "../policies/PolicyLayout";
import { ResultingYaml } from "../policies/ResultingYaml";
import { cleanEmpty, parseYamlText, toYamlText } from "../policies/policyUtils";
import type { AuthorizationDraft } from "../policies/types";
import { useSchemaHelp, type SchemaHelp } from "../schemaHelp";
import {
  concreteModelName,
  isWildcardModelName,
  resolvedProviderLabel,
  selectedConfiguredModelName,
  wildcardModelPrefix,
  wildcardResolvedSuffix,
} from "../modelResolution";
import {
  ModelMatchesEditor,
  normalizeMatches,
} from "./models/ModelMatchesEditor";
import {
  HeaderModifierEditor,
  HealthPolicyEditor,
  PromptCachingEditor,
  YamlMappingEditor,
  healthSummary,
  headerModifierSummary,
  promptCachingSummary,
} from "./models/ModelPolicyEditors";
import { ProviderConfigEditor } from "./models/ProviderConfigEditor";
import {
  clearModelSearch,
  modelFromProviderReference,
  modelHashFromUrl,
  providerFromUrl,
  setModelHash,
  type ModelHash,
} from "./models/modelRouteState";
import {
  defaultVirtualTargetModel,
  failoverTargetGroups,
  isIncompleteWildcardTarget,
  modelTargetOptions,
  virtualModelStrategy,
  virtualModelSummary,
} from "./models/virtualModelUtils";
import type {
  LlmModel,
  LlmProvider,
  LlmVirtualModel,
  ProviderName,
} from "../types";
import type {
  LocalLLMConditionalRouting,
  LocalLLMConditionalTarget,
  LocalLLMFailoverRouting,
  LocalLLMParams,
  LocalLLMWeightedRouting,
} from "../gateway-config";

type VirtualRoutingStrategy = "weighted" | "failover" | "conditional";
type ConditionalVirtualTarget = NonNullable<
  LlmVirtualModel["routing"]["conditional"]
>["targets"][number];

export function ModelsPage() {
  const config = useGatewayConfig();
  const update = useUpdateConfig();
  const help = useSchemaHelp();
  const models = useMemo(() => config.data?.llm?.models ?? [], [config.data]);
  const virtualModels = useMemo(
    () => config.data?.llm?.virtualModels ?? [],
    [config.data],
  );
  const providers = useMemo(
    () => config.data?.llm?.providers ?? [],
    [config.data],
  );
  const [editing, setEditing] = useState<{
    previousName?: string;
    model: LlmModel;
  } | null>(() => {
    const provider = providerFromUrl();
    return provider ? { model: modelFromProviderReference(provider) } : null;
  });
  const [editingVirtual, setEditingVirtual] = useState<{
    previousName?: string;
    model: LlmVirtualModel;
  } | null>(null);
  const [modelHash, setModelHashState] = useState<ModelHash | null>(() =>
    modelHashFromUrl(),
  );
  const hashEditModel =
    modelHash?.kind === "edit"
      ? (models.find((model) => model.name === modelHash.modelName) ?? null)
      : null;
  const activeEditing =
    editing ??
    (modelHash?.kind === "add" && modelHash.type === "model"
      ? { model: makeEmptyModel() }
      : null) ??
    (hashEditModel
      ? {
          previousName: hashEditModel.name,
          model: structuredClone(hashEditModel),
        }
      : null);
  const activeVirtualEditing =
    editingVirtual ??
    (modelHash?.kind === "add" && modelHash.type === "virtual"
      ? { model: makeEmptyVirtualModel() }
      : null);
  const modelRows = useMemo(
    () => [
      ...models.map((model) => ({ kind: "model" as const, model })),
      ...virtualModels.map((model) => ({ kind: "virtual" as const, model })),
    ],
    [models, virtualModels],
  );

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
    setEditing(null);
    setModelHashState({ kind: "add", type: "model" });
    setModelHash({ kind: "add", type: "model" }, "push");
  }

  function openNewVirtualModel() {
    update.reset();
    clearModelSearch();
    setEditingVirtual(null);
    setModelHashState({ kind: "add", type: "virtual" });
    setModelHash({ kind: "add", type: "virtual" }, "push");
  }

  function closeModelEditor() {
    update.reset();
    setEditing(null);
    clearModelSearch();
    if (
      modelHash?.kind === "edit" ||
      (modelHash?.kind === "add" && modelHash.type === "model")
    ) {
      setModelHashState(null);
      setModelHash(null, "replace");
    }
  }

  function closeVirtualModelEditor() {
    update.reset();
    setEditingVirtual(null);
    if (modelHash?.kind === "add" && modelHash.type === "virtual") {
      setModelHashState(null);
      setModelHash(null, "replace");
    }
  }

  return (
    <div className="page-stack">
      <PageHeader
        title="LLM Models"
        description="Onboard provider-backed models and configure model-specific behavior."
        actions={
          <div className="button-row">
            <button
              className="button primary"
              type="button"
              onClick={openNewModel}
            >
              <Plus size={16} />
              Add model
            </button>
            <button
              className="button"
              type="button"
              onClick={openNewVirtualModel}
            >
              <GitBranch size={16} />
              Add virtual model
            </button>
          </div>
        }
      />

      {update.isError ? (
        <StatusBanner state="bad" title="Save failed">
          {update.error.message}
        </StatusBanner>
      ) : null}
      {update.isSuccess ? (
        <StatusBanner state="ok" title="Configuration saved" />
      ) : null}

      <Panel>
        {config.isLoading ? (
          <StatusBanner state="loading" title="Loading models" />
        ) : config.isError ? (
          <StatusBanner state="bad" title="Configuration API unavailable">
            {config.error.message}
          </StatusBanner>
        ) : modelRows.length === 0 ? (
          <EmptyState
            title="No models configured"
            description="Create the first model to make LLM traffic available through the gateway."
            action={
              <div className="button-row">
                <button
                  className="button primary"
                  type="button"
                  onClick={openNewModel}
                >
                  <Plus size={16} />
                  Add model
                </button>
                <button
                  className="button"
                  type="button"
                  onClick={openNewVirtualModel}
                >
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
                  <th>Outgoing model</th>
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
                        <td>
                          <span className="badge">
                            <GitBranch size={14} /> Virtual
                          </span>
                        </td>
                        <td>{virtualModelSummary(model)}</td>
                        <td>
                          <span className="badge ok">
                            {virtualModelStrategy(model)}
                          </span>
                        </td>
                        <td className="row-actions">
                          <Tooltip content="Open in playground">
                            <Link
                              className="icon-button"
                              aria-label="Open in playground"
                              to="/llm/playground"
                              search={{ model: model.name }}
                            >
                              <Play size={16} />
                            </Link>
                          </Tooltip>
                          <Tooltip content="Edit model">
                            <button
                              className="icon-button"
                              aria-label="Edit model"
                              type="button"
                              onClick={() =>
                                setEditingVirtual({
                                  previousName: model.name,
                                  model: structuredClone(model),
                                })
                              }
                            >
                              <Pencil size={16} />
                            </button>
                          </Tooltip>
                          <Tooltip content="Delete model">
                            <button
                              className="icon-button danger"
                              aria-label="Delete model"
                              type="button"
                              onClick={() => {
                                if (confirmDelete("virtual model", model.name))
                                  update.mutate((next) =>
                                    removeVirtualModel(next, model.name),
                                  );
                              }}
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
                      <td>
                        <ModelProviderBadge
                          model={model}
                          providers={providers}
                        />
                      </td>
                      <td>{model.params?.model || "Incoming model"}</td>
                      <td>
                        <ModelPolicyState
                          model={model}
                          warnings={warnings.length}
                        />
                      </td>
                      <td className="row-actions">
                        <Tooltip content="Open in playground">
                          <Link
                            className="icon-button"
                            aria-label="Open in playground"
                            to="/llm/playground"
                            search={{ model: model.name }}
                          >
                            <Play size={16} />
                          </Link>
                        </Tooltip>
                        <Tooltip content="Edit model">
                          <button
                            className="icon-button"
                            aria-label="Edit model"
                            type="button"
                            onClick={() => openModelEditor(model)}
                          >
                            <Pencil size={16} />
                          </button>
                        </Tooltip>
                        <Tooltip content="Delete model">
                          <button
                            className="icon-button danger"
                            aria-label="Delete model"
                            type="button"
                            onClick={() => {
                              if (confirmDelete("model", model.name))
                                update.mutate((next) =>
                                  removeModel(next, model.name),
                                );
                            }}
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
          saveError={update.isError ? update.error.message : null}
          onCancel={closeModelEditor}
          onSave={(model, previousName) => {
            update.mutate((next) => upsertModel(next, model, previousName), {
              onSuccess: closeModelEditor,
            });
          }}
        />
      ) : null}
      {activeVirtualEditing ? (
        <VirtualModelEditor
          key={activeVirtualEditing.previousName ?? "new"}
          previousName={activeVirtualEditing.previousName}
          initial={activeVirtualEditing.model}
          baseModels={models}
          providers={providers}
          help={help}
          saving={update.isPending}
          saveError={update.isError ? update.error.message : null}
          onCancel={closeVirtualModelEditor}
          onSave={(model, previousName) =>
            update.mutate(
              (next) => upsertVirtualModel(next, model, previousName),
              {
                onSuccess: closeVirtualModelEditor,
              },
            )
          }
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
  const [model, setModel] = useState<LlmModel>(() => {
    if (props.initial.name || !props.initial.provider) return props.initial;
    return {
      ...props.initial,
      name: defaultIncomingModelMatch(props.initial.provider),
    };
  });
  const [autoModelMatch, setAutoModelMatch] = useState(
    () => !props.initial.name,
  );
  const [upstreamMode, setUpstreamMode] = useState<UpstreamModelMode>(() =>
    initialUpstreamMode(props.initial),
  );
  const [explicitModel, setExplicitModel] = useState(
    props.initial.params?.model ?? "",
  );
  const [customModelExpression, setCustomModelExpression] = useState(
    () => props.initial.transformation?.model ?? "llmRequest.model",
  );
  const [transformation, setTransformation] = useState<Record<string, string>>(
    () => expressionMap(props.initial.transformation),
  );
  const [health, setHealth] = useState<LlmModel["health"]>(
    () => props.initial.health ?? null,
  );
  const [defaultsText, setDefaultsText] = useState(() =>
    optionalMappingYamlText(props.initial.defaults),
  );
  const [overridesText, setOverridesText] = useState(() =>
    optionalMappingYamlText(props.initial.overrides),
  );
  const [requestHeaders, setRequestHeaders] = useState<
    LlmModel["requestHeaders"]
  >(() => props.initial.requestHeaders ?? null);
  const [responseHeaders, setResponseHeaders] = useState<
    LlmModel["responseHeaders"]
  >(() => props.initial.responseHeaders ?? null);
  const [promptCaching, setPromptCaching] = useState<LlmModel["promptCaching"]>(
    () => props.initial.promptCaching ?? null,
  );
  const [authorization, setAuthorization] = useState<AuthorizationDraft | null>(
    () => (props.initial.authorization as AuthorizationDraft | null) ?? null,
  );
  const [policyError, setPolicyError] = useState<string | null>(null);
  const [saveAttempted, setSaveAttempted] = useState(false);
  const warnings = modelWarnings(model);
  const invalidApiKey = invalidProviderApiKey(model.params?.apiKey);
  const providerApiKeyError =
    saveAttempted && invalidApiKey ? "Enter a value, or choose Unset." : null;
  const policyPatch = buildModelPolicyPatch({
    transformation,
    health,
    defaultsText,
    overridesText,
    requestHeaders,
    responseHeaders,
    promptCaching,
    authorization,
  });
  const preview = cleanEmpty(
    applyUpstreamMode(
      {
        ...model,
        ...policyPatch.value,
        matches: normalizeMatches(model.matches),
      },
      upstreamMode,
      explicitModel,
      customModelExpression,
    ),
  ) as LlmModel | undefined;
  const providerSelected = Boolean(model.provider);

  function save() {
    setSaveAttempted(true);
    if (!preview?.provider) return;
    if (invalidApiKey) return;
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
          <button className="button" type="button" onClick={props.onCancel}>
            Cancel
          </button>
          <button
            className="button primary"
            type="button"
            disabled={props.saving || !model.name.trim() || !preview?.provider}
            onClick={save}
          >
            <Save size={16} />
            Save model
          </button>
        </div>
      }
    >
      <details className="schema-details model-help-details">
        <summary>Help</summary>
        <div className="model-help-copy">
          <p>
            Agentgateway routes requests by matching an incoming model name, and
            then sending it to the configured model. The outgoing model can be
            passed through from the incoming model, be transformed, or be a
            static model.
          </p>
          <p>Some examples:</p>
          <ul>
            <li>
              Match <code>fast</code> and send to <code>gpt-mini</code>.
            </li>
            <li>
              Match <code>*</code> and forward the model as-is.
            </li>
            <li>
              Match <code>openai/*</code> and strip the <code>openai/</code>{" "}
              prefix, forwarding the remaining model as-is.
            </li>
          </ul>
        </div>
      </details>

      <div className="form-grid">
        <Field
          label="Incoming model match"
          tooltip={props.help.field<LlmModel>(
            "LocalLLMModels",
            "name",
            "The model name matched from incoming requests. Use an exact name like gpt-4.1-mini or a wildcard like openai/*.",
          )}
        >
          <input
            value={model.name}
            onChange={(event) => {
              const name = event.target.value;
              setAutoModelMatch(
                !name.trim() ||
                  name === defaultIncomingModelMatch(model.provider),
              );
              setModel({ ...model, name });
            }}
            placeholder={
              model.provider
                ? defaultIncomingModelMatch(model.provider)
                : "openai/*"
            }
          />
        </Field>
      </div>

      <ProviderConfigEditor
        provider={model.provider}
        params={model.params}
        auth={model.auth}
        providers={props.providers}
        help={props.help}
        apiKeyError={providerApiKeyError}
        onProviderChange={(provider, params) =>
          setModel((current) => {
            const currentDefault = defaultIncomingModelMatch(current.provider);
            const nextDefault = defaultIncomingModelMatch(provider);
            const shouldUseDefault =
              autoModelMatch ||
              !current.name.trim() ||
              current.name === currentDefault;
            if (shouldUseDefault) setAutoModelMatch(true);
            if (
              !props.previousName &&
              shouldUseDefault &&
              stripPrefixCandidate(nextDefault)
            )
              setUpstreamMode("strip");
            return {
              ...current,
              provider,
              params,
              name: shouldUseDefault ? nextDefault : current.name,
            };
          })
        }
        onParamsChange={(params) =>
          setModel((current) => ({ ...current, params }))
        }
        onAuthChange={(auth) => setModel((current) => ({ ...current, auth }))}
      />

      {providerSelected ? (
        <>
          <UpstreamModelFields
            mode={upstreamMode}
            explicitModel={explicitModel}
            customModelExpression={customModelExpression}
            gatewayModelName={model.name}
            provider={
              model.provider
                ? resolvedProviderLabel(model.provider, props.providers)
                : null
            }
            help={props.help}
            setMode={setUpstreamMode}
            setExplicitModel={setExplicitModel}
            setCustomModelExpression={setCustomModelExpression}
          />

          <CollapsiblePolicySection
            icon={<SlidersHorizontal size={17} />}
            title="Advanced"
            description="Match conditions and model-specific policies"
          >
            <div className="policy-editor-stack">
              <ModelMatchesEditor
                matches={model.matches ?? []}
                onChange={(matches) =>
                  setModel((current) => ({ ...current, matches }))
                }
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
                authorization={authorization}
                setTransformation={setTransformation}
                setHealth={setHealth}
                setDefaultsText={setDefaultsText}
                setOverridesText={setOverridesText}
                setRequestHeaders={setRequestHeaders}
                setResponseHeaders={setResponseHeaders}
                setPromptCaching={setPromptCaching}
                setAuthorization={setAuthorization}
              />
            </div>
          </CollapsiblePolicySection>
        </>
      ) : null}

      {providerSelected && warnings.length ? (
        <div className="model-warning-block">
          <StatusBanner state="warn" title="Model warnings">
            <ul>
              {warnings.map((warning) => (
                <li key={warning}>{warning}</li>
              ))}
            </ul>
          </StatusBanner>
        </div>
      ) : null}
      {policyError ? (
        <StatusBanner state="bad" title="Invalid model policies">
          {policyError}
        </StatusBanner>
      ) : null}
      {props.saveError ? (
        <StatusBanner state="bad" title="Save failed">
          {props.saveError}
        </StatusBanner>
      ) : null}

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
  provider?: string | null;
  help: SchemaHelp;
  setMode: (mode: UpstreamModelMode) => void;
  setExplicitModel: (model: string) => void;
  setCustomModelExpression: (expression: string) => void;
}) {
  const prefix = stripPrefixCandidate(props.gatewayModelName);
  const mode = prefix || props.mode !== "strip" ? props.mode : "incoming";
  const stripLabel = prefix ? `Strip ${prefix.slice(0, -1)}/` : "Strip prefix";
  return (
    <>
      <FieldGroup
        label="Outgoing model"
        tooltip={props.help.field<LocalLLMParams>("LocalLLMParams", "model")}
      >
        <div className="segmented-control upstream-model-control">
          <button
            className={mode === "incoming" ? "active" : ""}
            type="button"
            onClick={() => props.setMode("incoming")}
          >
            Incoming model
          </button>
          <button
            className={mode === "explicit" ? "active" : ""}
            type="button"
            onClick={() => props.setMode("explicit")}
          >
            Explicit
          </button>
          {prefix ? (
            <button
              className={mode === "strip" ? "active" : ""}
              type="button"
              onClick={() => props.setMode("strip")}
            >
              {stripLabel}
            </button>
          ) : null}
          <button
            className={mode === "custom" ? "active" : ""}
            type="button"
            onClick={() => props.setMode("custom")}
          >
            Custom
          </button>
        </div>
      </FieldGroup>

      {mode === "explicit" ? (
        <Field
          label="Explicit outgoing model"
          tooltip={props.help.field<LocalLLMParams>("LocalLLMParams", "model")}
        >
          <CatalogModelSelector
            ariaLabel="Explicit outgoing model"
            value={props.explicitModel}
            provider={props.provider}
            onChange={props.setExplicitModel}
            placeholder="gpt-4.1-mini"
          />
        </Field>
      ) : null}
      {mode === "custom" ? (
        <FieldGroup
          label="Model CEL expression"
          tooltip={props.help.field<LlmModel>(
            "LocalLLMModels",
            "transformation",
          )}
        >
          <MiniMonacoEditor
            language="cel"
            value={props.customModelExpression}
            onChange={props.setCustomModelExpression}
            placeholder='llmRequest.model.stripPrefix("anthropic/")'
          />
        </FieldGroup>
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
  authorization: AuthorizationDraft | null;
  setTransformation: (value: Record<string, string>) => void;
  setHealth: (value: LlmModel["health"] | null) => void;
  setDefaultsText: (value: string) => void;
  setOverridesText: (value: string) => void;
  setRequestHeaders: (value: LlmModel["requestHeaders"] | null) => void;
  setResponseHeaders: (value: LlmModel["responseHeaders"] | null) => void;
  setPromptCaching: (value: LlmModel["promptCaching"] | null) => void;
  setAuthorization: (value: AuthorizationDraft | null) => void;
}) {
  const patch = buildModelPolicyPatch(props);
  const transformationEnabled =
    Object.keys(expressionMap(props.model.transformation)).length > 0;
  const defaultsEnabled = Boolean(
    props.model.defaults && Object.keys(props.model.defaults).length,
  );
  const overridesEnabled = Boolean(
    props.model.overrides && Object.keys(props.model.overrides).length,
  );
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
          description={
            Object.keys(props.transformation).length
              ? `${Object.keys(props.transformation).length} fields configured`
              : "No fields configured"
          }
          defaultOpen={transformationEnabled}
        >
          <KeyValueEditor
            label="LLM request fields"
            tooltip={props.help.field<LlmModel>(
              "LocalLLMModels",
              "transformation",
            )}
            values={props.transformation}
            keyPlaceholder="field name"
            valuePlaceholder="CEL expression"
            valueKind="cel"
            onChange={props.setTransformation}
          />
        </CollapsiblePolicySection>
        <CollapsiblePolicySection
          icon={<FileText size={17} />}
          title="Default request values"
          description={
            props.defaultsText.trim()
              ? "Defaults configured"
              : "No defaults configured"
          }
          defaultOpen={defaultsEnabled}
        >
          <YamlMappingEditor
            label="Defaults YAML"
            tooltip={props.help.field<LlmModel>("LocalLLMModels", "defaults")}
            value={props.defaultsText}
            onChange={props.setDefaultsText}
            placeholder="temperature: 0.2"
          />
        </CollapsiblePolicySection>
        <CollapsiblePolicySection
          icon={<FileText size={17} />}
          title="Override request values"
          description={
            props.overridesText.trim()
              ? "Overrides configured"
              : "No overrides configured"
          }
          defaultOpen={overridesEnabled}
        >
          <YamlMappingEditor
            label="Overrides YAML"
            tooltip={props.help.field<LlmModel>("LocalLLMModels", "overrides")}
            value={props.overridesText}
            onChange={props.setOverridesText}
            placeholder="stream: false"
          />
        </CollapsiblePolicySection>
        <CollapsiblePolicySection
          icon={<SlidersHorizontal size={17} />}
          title="Request headers"
          description={headerModifierSummary(props.requestHeaders, "request")}
          defaultOpen={Boolean(props.model.requestHeaders)}
        >
          <HeaderModifierEditor
            value={props.requestHeaders}
            help={props.help}
            onChange={props.setRequestHeaders}
          />
        </CollapsiblePolicySection>
        <CollapsiblePolicySection
          icon={<SlidersHorizontal size={17} />}
          title="Response headers"
          description={headerModifierSummary(props.responseHeaders, "response")}
          defaultOpen={Boolean(props.model.responseHeaders)}
        >
          <HeaderModifierEditor
            value={props.responseHeaders}
            help={props.help}
            onChange={props.setResponseHeaders}
          />
        </CollapsiblePolicySection>
        <CollapsiblePolicySection
          icon={<Activity size={17} />}
          title="Health"
          description={healthSummary(props.health)}
          defaultOpen={Boolean(props.model.health)}
        >
          <HealthPolicyEditor
            health={props.health}
            help={props.help}
            onChange={props.setHealth}
          />
        </CollapsiblePolicySection>
        <CollapsiblePolicySection
          icon={<ShieldCheck size={17} />}
          title="Authorization"
          description={authorizationSummary(props.authorization)}
          defaultOpen={Boolean(props.model.authorization)}
        >
          <div className="policy-editor-stack compact">
            <AuthorizationPolicyEditor
              key={JSON.stringify(props.authorization ?? null)}
              authorization={props.authorization}
              saving={false}
              onSave={props.setAuthorization}
            />
            {props.authorization ? (
              <button
                className="button"
                type="button"
                onClick={() => props.setAuthorization(null)}
              >
                Clear authorization
              </button>
            ) : null}
          </div>
        </CollapsiblePolicySection>
        <CollapsiblePolicySection
          icon={<SlidersHorizontal size={17} />}
          title="Prompt caching"
          description={promptCachingSummary(props.promptCaching)}
          defaultOpen={Boolean(props.model.promptCaching)}
        >
          <PromptCachingEditor
            value={props.promptCaching}
            help={props.help}
            onChange={props.setPromptCaching}
          />
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
  authorization: AuthorizationDraft | null;
}) {
  try {
    const defaults = parseOptionalYamlMapping(args.defaultsText);
    const overrides = parseOptionalYamlMapping(args.overridesText);
    const transformation = cleanEmpty(args.transformation) as
      | LlmModel["transformation"]
      | undefined;
    const health = cleanEmpty(args.health) as LlmModel["health"] | undefined;
    const requestHeaders = cleanEmpty(args.requestHeaders) as
      | LlmModel["requestHeaders"]
      | undefined;
    const responseHeaders = cleanEmpty(args.responseHeaders) as
      | LlmModel["responseHeaders"]
      | undefined;
    const promptCaching = cleanEmpty(args.promptCaching) as
      | LlmModel["promptCaching"]
      | undefined;
    const authorization = cleanEmpty(args.authorization) as
      | LlmModel["authorization"]
      | undefined;
    return {
      value: {
        defaults,
        overrides,
        transformation:
          transformation && Object.keys(transformation).length
            ? transformation
            : null,
        requestHeaders:
          requestHeaders && Object.keys(requestHeaders).length
            ? requestHeaders
            : null,
        responseHeaders:
          responseHeaders && Object.keys(responseHeaders).length
            ? responseHeaders
            : null,
        health: health && Object.keys(health).length ? health : null,
        promptCaching:
          promptCaching && Object.keys(promptCaching).length
            ? promptCaching
            : null,
        authorization:
          authorization && Object.keys(authorization).length
            ? authorization
            : null,
      } satisfies Partial<LlmModel>,
      error: null,
    };
  } catch (error) {
    return {
      value: {},
      error:
        error instanceof Error ? error.message : "Invalid policy configuration",
    };
  }
}

function modelPolicySummary(model: Partial<LlmModel>) {
  const policies = [
    model.defaults && Object.keys(model.defaults).length ? "defaults" : null,
    model.overrides && Object.keys(model.overrides).length ? "overrides" : null,
    model.transformation && Object.keys(model.transformation).length
      ? "transformation"
      : null,
    model.requestHeaders ? "request headers" : null,
    model.responseHeaders ? "response headers" : null,
    model.health ? "health" : null,
    model.authorization ? "authorization" : null,
    model.promptCaching ? "prompt caching" : null,
  ].filter(Boolean);
  return policies.length
    ? `${policies.length} configured`
    : "No model policies configured";
}

function VirtualModelEditor(props: {
  initial: LlmVirtualModel;
  previousName?: string;
  baseModels: LlmModel[];
  providers: LlmProvider[];
  help: SchemaHelp;
  saving: boolean;
  saveError?: string | null;
  onCancel: () => void;
  onSave: (model: LlmVirtualModel, previousName?: string) => void;
}) {
  const [model, setModel] = useState<LlmVirtualModel>(props.initial);
  const strategy = model.routing.conditional
    ? "conditional"
    : model.routing.failover
      ? "failover"
      : "weighted";
  const weightedTargets = model.routing.weighted?.targets ?? [];
  const failoverTargets = model.routing.failover?.targets ?? [];
  const conditionalTargets = model.routing.conditional?.targets ?? [];
  const targetOptions = modelTargetOptions(props.baseModels);
  const preview = cleanEmpty(model) as LlmVirtualModel | undefined;
  const activeTargets =
    strategy === "weighted"
      ? weightedTargets
      : strategy === "failover"
        ? failoverTargets
        : conditionalTargets;
  const hasInvalidTarget = activeTargets.some(
    (target) =>
      !target.model.trim() ||
      isIncompleteWildcardTarget(target.model, props.baseModels),
  );
  const hasInvalidConditionalFallback =
    strategy === "conditional" &&
    conditionalTargets.some(
      (target, index) =>
        !target.when?.trim() && index !== conditionalTargets.length - 1,
    );
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
              : [
                  {
                    when: 'json(request.body).route == "default"',
                    model: defaultTarget,
                  },
                  { model: defaultTarget },
                ],
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

  function updateWeighted(
    index: number,
    patch: Partial<
      NonNullable<LlmVirtualModel["routing"]["weighted"]>["targets"][number]
    >,
  ) {
    setModel((current) => {
      const targets = [...(current.routing.weighted?.targets ?? [])];
      targets[index] = { ...targets[index], ...patch };
      return { ...current, routing: { weighted: { targets } } };
    });
  }

  function updateFailoverGroups(
    groups: Array<
      Array<
        NonNullable<LlmVirtualModel["routing"]["failover"]>["targets"][number]
      >
    >,
  ) {
    setModel((current) => ({
      ...current,
      routing: {
        failover: {
          targets: groups.flatMap((group, priority) =>
            group.map((target) => ({ ...target, priority })),
          ),
        },
      },
    }));
  }

  function updateConditional(
    index: number,
    patch: Partial<ConditionalVirtualTarget>,
  ) {
    setModel((current) => {
      const targets = [...(current.routing.conditional?.targets ?? [])];
      targets[index] = cleanEmpty({
        ...targets[index],
        ...patch,
      }) as ConditionalVirtualTarget;
      return { ...current, routing: { conditional: { targets } } };
    });
  }

  return (
    <Drawer
      title={props.previousName ? "Edit virtual model" : "Add virtual model"}
      onClose={props.onCancel}
      footer={
        <div className="button-row">
          <button className="button" type="button" onClick={props.onCancel}>
            Cancel
          </button>
          <button
            className="button primary"
            type="button"
            disabled={
              props.saving ||
              !model.name.trim() ||
              activeTargets.length === 0 ||
              hasInvalidTarget ||
              hasInvalidConditionalFallback
            }
            onClick={() => props.onSave(preview ?? model, props.previousName)}
          >
            <Save size={16} />
            Save virtual model
          </button>
        </div>
      }
    >
      <Field
        label="Virtual model name"
        tooltip={props.help.field<LlmVirtualModel>(
          "LocalLLMVirtualModel",
          "name",
        )}
      >
        <input
          value={model.name}
          onChange={(event) => setModel({ ...model, name: event.target.value })}
          placeholder="resilient"
        />
      </Field>

      <FieldGroup
        label="Routing strategy"
        tooltip={props.help.field<LlmVirtualModel>(
          "LocalLLMVirtualModel",
          "routing",
        )}
      >
        <div className="segmented-control">
          <button
            className={strategy === "weighted" ? "active" : ""}
            type="button"
            onClick={() => setStrategy("weighted")}
          >
            Weighted
          </button>
          <button
            className={strategy === "failover" ? "active" : ""}
            type="button"
            onClick={() => setStrategy("failover")}
          >
            Failover
          </button>
          <button
            className={strategy === "conditional" ? "active" : ""}
            type="button"
            onClick={() => setStrategy("conditional")}
          >
            Conditional
          </button>
        </div>
      </FieldGroup>

      {strategy === "weighted" ? (
        <FieldGroup
          label="Weighted targets"
          tooltip={props.help.field<LocalLLMWeightedRouting>(
            "LocalLLMWeightedRouting",
            "targets",
          )}
        >
          <div className="target-list">
            {weightedTargets.map((target, index) => (
              <div className="target-row weighted" key={index}>
                <VirtualTargetSelector
                  label="Model"
                  targetModel={target.model}
                  baseModels={props.baseModels}
                  options={targetOptions}
                  providers={props.providers}
                  onChange={(value) => updateWeighted(index, { model: value })}
                />
                <label className="target-field">
                  <span className="target-label">Weight</span>
                  <input
                    aria-label="Weight"
                    value={target.weight ?? 1}
                    onChange={(event) =>
                      updateWeighted(index, {
                        weight: Number(event.target.value) || 1,
                      })
                    }
                    type="number"
                    min={1}
                  />
                </label>
                <button
                  className="icon-button danger"
                  type="button"
                  aria-label="Remove target"
                  onClick={() =>
                    setModel((current) => ({
                      ...current,
                      routing: {
                        weighted: {
                          targets: (
                            current.routing.weighted?.targets ?? []
                          ).filter((_, itemIndex) => itemIndex !== index),
                        },
                      },
                    }))
                  }
                >
                  <Trash2 size={16} />
                </button>
              </div>
            ))}
          </div>
          <button
            className="button"
            type="button"
            onClick={() =>
              setModel((current) => ({
                ...current,
                routing: {
                  weighted: {
                    targets: [
                      ...(current.routing.weighted?.targets ?? []),
                      { model: defaultTarget, weight: 1 },
                    ],
                  },
                },
              }))
            }
          >
            <Plus size={16} />
            Add target
          </button>
        </FieldGroup>
      ) : strategy === "failover" ? (
        <FieldGroup
          label="Failover targets"
          tooltip={props.help.field<LocalLLMFailoverRouting>(
            "LocalLLMFailoverRouting",
            "targets",
          )}
        >
          <div className="failover-group-list">
            {failoverGroups.map((group, groupIndex) => (
              <section className="match-card" key={groupIndex}>
                <div className="match-card-header">
                  <strong>
                    {groupIndex === 0
                      ? "First attempt"
                      : `Fallback group ${groupIndex + 1}`}
                  </strong>
                  <Tooltip content="Remove group">
                    <button
                      className="icon-button danger"
                      type="button"
                      aria-label={`Remove failover group ${groupIndex + 1}`}
                      onClick={() =>
                        updateFailoverGroups(
                          failoverGroups.filter(
                            (_, itemIndex) => itemIndex !== groupIndex,
                          ),
                        )
                      }
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
                          providers={props.providers}
                          onChange={(value) =>
                            updateFailoverGroups(
                              failoverGroups.map((item, itemIndex) =>
                                itemIndex === groupIndex
                                  ? item.map((groupTarget, groupTargetIndex) =>
                                      groupTargetIndex === targetIndex
                                        ? { ...groupTarget, model: value }
                                        : groupTarget,
                                    )
                                  : item,
                              ),
                            )
                          }
                        />
                        <button
                          className="icon-button danger"
                          type="button"
                          aria-label="Remove target"
                          onClick={() =>
                            updateFailoverGroups(
                              failoverGroups
                                .map((item, itemIndex) =>
                                  itemIndex === groupIndex
                                    ? item.filter(
                                        (_, groupTargetIndex) =>
                                          groupTargetIndex !== targetIndex,
                                      )
                                    : item,
                                )
                                .filter((item) => item.length > 0),
                            )
                          }
                        >
                          <Trash2 size={16} />
                        </button>
                      </div>
                    ))}
                  </div>
                  <button
                    className="button small"
                    type="button"
                    onClick={() =>
                      updateFailoverGroups(
                        failoverGroups.map((item, itemIndex) =>
                          itemIndex === groupIndex
                            ? [
                                ...item,
                                { model: defaultTarget, priority: groupIndex },
                              ]
                            : item,
                        ),
                      )
                    }
                  >
                    <Plus size={16} />
                    Add target
                  </button>
                </div>
              </section>
            ))}
          </div>
          <button
            className="button"
            type="button"
            onClick={() =>
              updateFailoverGroups([
                ...failoverGroups,
                [{ model: defaultTarget, priority: failoverGroups.length }],
              ])
            }
          >
            <Plus size={16} />
            Add fallback group
          </button>
        </FieldGroup>
      ) : (
        <FieldGroup
          label="Conditional targets"
          tooltip={props.help.field<LocalLLMConditionalRouting>(
            "LocalLLMConditionalRouting",
            "targets",
          )}
        >
          <div className="target-list">
            {conditionalTargets.map((target, index) => {
              const isFallback = !target.when?.trim();
              return (
                <div className="conditional-target-card" key={index}>
                  <div className="match-card-header">
                    <strong>
                      {isFallback ? "Fallback" : `Rule ${index + 1}`}
                    </strong>
                    <Tooltip content="Remove rule">
                      <button
                        className="icon-button danger"
                        type="button"
                        aria-label="Remove conditional target"
                        onClick={() =>
                          setModel((current) => ({
                            ...current,
                            routing: {
                              conditional: {
                                targets: (
                                  current.routing.conditional?.targets ?? []
                                ).filter((_, itemIndex) => itemIndex !== index),
                              },
                            },
                          }))
                        }
                      >
                        <Trash2 size={15} />
                      </button>
                    </Tooltip>
                  </div>
                  <div className="conditional-target-body">
                    <FieldGroup
                      label="Condition"
                      tooltip={props.help.field<LocalLLMConditionalTarget>(
                        "LocalLLMConditionalTarget",
                        "when",
                      )}
                    >
                      <MiniMonacoEditor
                        language="cel"
                        value={target.when ?? ""}
                        onChange={(value) =>
                          updateConditional(index, {
                            when: value.trim() ? value : undefined,
                          })
                        }
                        placeholder={
                          index === conditionalTargets.length - 1
                            ? "Blank final condition means fallback"
                            : 'json(request.body).route == "code"'
                        }
                      />
                    </FieldGroup>
                    <VirtualTargetSelector
                      label="Target model"
                      targetModel={target.model}
                      baseModels={props.baseModels}
                      options={targetOptions}
                      providers={props.providers}
                      onChange={(value) =>
                        updateConditional(index, { model: value })
                      }
                    />
                  </div>
                </div>
              );
            })}
          </div>
          {hasInvalidConditionalFallback ? (
            <StatusBanner
              state="warn"
              title="Only the final conditional target can omit a condition."
            />
          ) : null}
          <div className="button-row">
            <button
              className="button"
              type="button"
              onClick={() =>
                setModel((current) => ({
                  ...current,
                  routing: {
                    conditional: {
                      targets: [
                        ...(current.routing.conditional?.targets ?? []),
                        {
                          when: 'json(request.body).route == "code"',
                          model: defaultTarget,
                        },
                      ],
                    },
                  },
                }))
              }
            >
              <Plus size={16} />
              Add rule
            </button>
            <button
              className="button"
              type="button"
              onClick={() =>
                setModel((current) => ({
                  ...current,
                  routing: {
                    conditional: {
                      targets: [
                        ...(current.routing.conditional?.targets ?? []).filter(
                          (target) => target.when?.trim(),
                        ),
                        { model: defaultTarget },
                      ],
                    },
                  },
                }))
              }
            >
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
      {props.saveError ? (
        <StatusBanner state="bad" title="Save failed">
          {props.saveError}
        </StatusBanner>
      ) : null}
    </Drawer>
  );
}

function VirtualTargetSelector(props: {
  label: string;
  targetModel: string;
  baseModels: LlmModel[];
  options: Array<{
    value: string;
    label: ReactNode;
    icon?: ReactNode;
    searchText?: string;
  }>;
  providers: LlmProvider[];
  onChange: (model: string) => void;
}) {
  const selectedModelName = selectedConfiguredModelName(
    props.targetModel,
    props.baseModels,
  );
  const selectedModel = props.baseModels.find(
    (model) => model.name === selectedModelName,
  );
  const wildcard = Boolean(
    selectedModel && isWildcardModelName(selectedModel.name),
  );
  const wildcardPrefix = selectedModel
    ? wildcardModelPrefix(selectedModel.name)
    : "";
  const resolvedSuffix = wildcard
    ? wildcardResolvedSuffix(
        props.targetModel,
        selectedModelName,
        wildcardPrefix,
      )
    : "";
  const provider = selectedModel
    ? resolvedProviderLabel(selectedModel.provider, props.providers)
    : null;

  return (
    <div className="target-field">
      <span className="target-label">{props.label}</span>
      <Dropdown
        ariaLabel={props.label}
        value={selectedModelName}
        searchable
        options={props.options}
        placeholder="No configured models"
        onChange={(value) => props.onChange(concreteModelName(value, ""))}
      />
      {wildcard ? (
        <div className="target-resolved-composite">
          {wildcardPrefix ? (
            <span className="target-prefix">{wildcardPrefix}</span>
          ) : null}
          <CatalogModelSelector
            ariaLabel="Specific model"
            value={resolvedSuffix}
            provider={provider}
            onChange={(value) =>
              props.onChange(concreteModelName(selectedModelName, value))
            }
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

function ModelProviderBadge(props: {
  model: LlmModel;
  providers: LlmProvider[];
}) {
  const reference = providerReferenceName(props.model.provider);
  if (reference) {
    const shared = props.providers.find(
      (provider) => provider.name === reference,
    );
    const provider = shared ? providerLabel(shared.provider) : "custom";
    return (
      <Link
        className="badge provider-badge badge-link"
        to="/llm/providers"
        search={{ provider: reference }}
      >
        <ProviderIcon provider={provider as ProviderName} />
        {reference}
        <span className="muted">reference</span>
      </Link>
    );
  }
  return (
    <ProviderBadge
      provider={providerLabel(props.model.provider) as ProviderName}
    />
  );
}

function ModelPolicyState(props: { model: LlmModel; warnings: number }) {
  const policies = [
    props.model.defaults && Object.keys(props.model.defaults).length
      ? "defaults"
      : null,
    props.model.overrides && Object.keys(props.model.overrides).length
      ? "overrides"
      : null,
    props.model.transformation && Object.keys(props.model.transformation).length
      ? "transformation"
      : null,
    props.model.requestHeaders ? "requestHeaders" : null,
    props.model.responseHeaders ? "responseHeaders" : null,
    props.model.health ? "health" : null,
    props.model.authorization ? "authorization" : null,
    props.model.promptCaching ? "promptCaching" : null,
  ].filter(Boolean);
  if (props.warnings > 0)
    return <span className="badge warn">{props.warnings} warnings</span>;
  if (props.model.auth)
    return <span className="badge">Custom auth detected</span>;
  if (policies.length > 0)
    return (
      <span className="badge ok">
        {policies.length} {policies.length === 1 ? "policy" : "policies"}
      </span>
    );
  return <span className="badge">none</span>;
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

function optionalMappingYamlText(
  value: Record<string, unknown> | null | undefined,
) {
  return value && Object.keys(value).length ? toYamlText(value) : "";
}

function initialUpstreamMode(model: LlmModel): UpstreamModelMode {
  if (model.params?.model) return "explicit";
  const expression = model.transformation?.model;
  if (
    expression &&
    expression === stripPrefixExpression(stripPrefixCandidate(model.name))
  )
    return "strip";
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

function defaultIncomingModelMatch(provider: LlmModel["provider"]) {
  const providerName =
    providerReferenceName(provider) ?? providerLabel(provider);
  return `${providerName === "openAI" ? "openai" : providerName || "model"}/*`;
}

function confirmDelete(kind: string, name: string) {
  return window.confirm(`Delete ${kind} "${name}"? This cannot be undone.`);
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
  const prefixExpression = stripPrefixExpression(
    stripPrefixCandidate(next.name),
  );

  if (mode === "strip" && prefixExpression) {
    transformation.model = prefixExpression;
  } else if (mode === "custom" && customModelExpression.trim()) {
    transformation.model = customModelExpression.trim();
  }

  next.transformation = Object.keys(transformation).length
    ? transformation
    : null;

  if (providerReferenceName(next.provider)) {
    next.params =
      mode === "explicit" && explicitModel
        ? { model: explicitModel }
        : undefined;
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

function expressionMap(
  value: LlmModel["transformation"],
): Record<string, string> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return Object.fromEntries(
    Object.entries(value).filter(
      (entry): entry is [string, string] => typeof entry[1] === "string",
    ),
  );
}

function authorizationSummary(value: AuthorizationDraft | null | undefined) {
  const rules = Array.isArray(value?.rules) ? value.rules : [];
  if (!rules.length) return "No authorization rules configured";
  return `${rules.length} ${rules.length === 1 ? "rule" : "rules"} configured`;
}
