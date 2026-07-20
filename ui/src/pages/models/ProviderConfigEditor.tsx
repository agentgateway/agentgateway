import { tr } from "../../i18n";
import { useEffect, useState } from "react";
import { Eye, EyeOff } from "lucide-react";
import {
  providerDisplayName,
  providerLabel,
  providerReferenceName,
  visibleProviderNames,
} from "../../config";
import { CloudRegionCombobox } from "../../components/CloudRegionCombobox";
import { EnumSelector } from "../../components/EnumSelector";
import { Field, FieldGroup, Dropdown } from "../../components/Primitives";
import { ProviderIcon } from "../../components/ProviderIcon";
import type { SchemaHelp } from "../../schemaHelp";
import type {
  LlmParams,
  LlmProvider,
  ModelProvider,
  CanonicalProviderAuth,
  ProviderAuth,
  ProviderName,
  SecretFromFile,
} from "../../types";
import { CustomFormats } from "./CustomFormats";
import type { LlmModel, CustomProvider } from "../../types";

export function ProviderConfigEditor(props: {
  provider: ModelProvider | null;
  params?: LlmParams;
  auth?: ProviderAuth | null;
  providers?: LlmProvider[];
  help: SchemaHelp;
  apiKeyError?: string | null;
  onProviderChange: (provider: ModelProvider, params: LlmParams) => void;
  onParamsChange: (params: LlmParams) => void;
  onAuthChange?: (auth: CanonicalProviderAuth | null) => void;
}) {
  const providerReference = providerReferenceName(props.provider);
  const provider = providerLabel(props.provider) as ProviderName;
  const providerChoices = visibleProviderNames(true);
  const selectedProviderValue = providerReference
    ? `provider:${providerReference}`
    : provider
      ? `builtin:${provider}`
      : "";
  const azureResourceType = props.params?.azureResourceType ?? "openAI";
  const options = [
    ...(props.providers ?? []).map((item) => {
      const itemProvider = providerLabel(item.provider) as ProviderName;
      const displayName = providerDisplayName(itemProvider);
      return {
        value: `provider:${item.name}`,
        label: (
          <>
            {item.name} <small className="muted">{tr("copy.configured")}</small>
          </>
        ),
        icon: <ProviderIcon provider={itemProvider} />,
        searchText: `${item.name} ${displayName}`,
      };
    }),
    ...providerChoices.map((name) => ({
      value: `builtin:${name}`,
      label: providerDisplayName(name),
      icon: <ProviderIcon provider={name} />,
      searchText: providerDisplayName(name),
    })),
  ];

  function patchParams(value: Partial<LlmParams>) {
    props.onParamsChange({ ...(props.params ?? {}), ...value });
  }

  function setProvider(nextProvider: ProviderName) {
    props.onAuthChange?.(null);
    props.onProviderChange(
      nextProvider === "custom"
        ? { custom: { formats: [{ type: "completions" }] } }
        : nextProvider,
      {
        ...(props.params ?? {}),
        apiKey: null,
        ...(nextProvider === "azure" ? { azureResourceType: "openAI" } : {}),
      },
    );
  }

  useEffect(() => {
    if (provider === "azure" && !props.params?.azureResourceType) {
      patchParams({ azureResourceType: "openAI" });
    }
  }, [provider, props.params?.azureResourceType]);

  function setProviderChoice(value: string) {
    if (value.startsWith("provider:")) {
      const reference = value.slice("provider:".length);
      props.onAuthChange?.(null);
      props.onProviderChange(
        { reference },
        props.params?.model ? { model: props.params.model } : {},
      );
      return;
    }
    setProvider(value.replace(/^builtin:/, "") as ProviderName);
  }

  return (
    <>
      <FieldGroup
        label={tr("copy.provider")}
        tooltip={props.help.field<LlmModel>("LocalLLMModels", "provider")}
      >
        <Dropdown
          ariaLabel="Provider"
          value={selectedProviderValue}
          searchable
          options={options}
          placeholder={tr("copy.selectProvider")}
          allowEmpty
          onChange={setProviderChoice}
        />
      </FieldGroup>

      {props.provider && !providerReference ? (
        <>
          {provider === "bedrock" ? (
            <AwsCredentials value={props.auth} onChange={props.onAuthChange} />
          ) : provider === "vertex" ? (
            <GcpCredentials value={props.auth} onChange={props.onAuthChange} />
          ) : provider === "azure" ? (
            <AzureCredentials
              auth={props.auth}
              apiKey={props.params?.apiKey}
              onAuthChange={props.onAuthChange}
              onApiKeyChange={(apiKey) => patchParams({ apiKey })}
            />
          ) : (
            <Field
              label={tr("copy.providerApiKey")}
              tooltip={props.help.field<LlmParams>("LocalLLMParams", "apiKey")}
              className={props.apiKeyError ? "invalid" : undefined}
              hint={props.apiKeyError ?? undefined}
            >
              <ApiKeyInput
                value={props.params?.apiKey}
                onChange={(apiKey) => patchParams({ apiKey })}
              />
            </Field>
          )}

          {provider === "vertex" ? (
            <div className="form-grid">
              <Field
                label={tr("copy.vertexProject")}
                tooltip={props.help.field<LlmParams>(
                  "LocalLLMParams",
                  "vertexProject",
                  "Google Cloud project used for Vertex AI requests.",
                )}
              >
                <input
                  value={props.params?.vertexProject ?? ""}
                  onChange={(event) =>
                    patchParams({ vertexProject: event.target.value || null })
                  }
                />
              </Field>
              <Field
                label={tr("copy.vertexRegion")}
                tooltip={props.help.field<LlmParams>(
                  "LocalLLMParams",
                  "vertexRegion",
                  "Google Cloud region used for Vertex AI requests.",
                )}
                hint={tr("copy.optionalIfUnsetVertexUsesGlobal")}
              >
                <CloudRegionCombobox
                  cloud="google"
                  ariaLabel="Vertex region"
                  value={props.params?.vertexRegion ?? ""}
                  onChange={(value) =>
                    patchParams({ vertexRegion: value || null })
                  }
                  placeholder="us-central1"
                />
              </Field>
            </div>
          ) : null}
          {provider === "bedrock" ? (
            <Field
              label={tr("copy.awsRegion")}
              tooltip={props.help.field<LlmParams>(
                "LocalLLMParams",
                "awsRegion",
                "AWS region used for Bedrock requests.",
              )}
            >
              <CloudRegionCombobox
                cloud="aws"
                ariaLabel="AWS region"
                value={props.params?.awsRegion ?? ""}
                onChange={(value) => patchParams({ awsRegion: value || null })}
                placeholder="us-west-2"
              />
            </Field>
          ) : null}
          {provider === "ollama" ? (
            <Field
              label={tr("copy.baseUrl")}
              tooltip={props.help.field<LlmParams>(
                "LocalLLMParams",
                "baseUrl",
                "Override when Ollama is hosted somewhere other than the local default.",
              )}
              hint={tr("copy.optionalDefaultsToHttpLocalhost11434V1")}
            >
              <input
                value={props.params?.baseUrl ?? ""}
                onChange={(event) =>
                  patchParams({ baseUrl: event.target.value || null })
                }
                placeholder="http://localhost:11434/v1"
              />
            </Field>
          ) : null}
          {provider === "azure" ? (
            <div className="form-grid">
              <Field
                label={tr("copy.azureResourceName")}
                tooltip={props.help.field<LlmParams>(
                  "LocalLLMParams",
                  "azureResourceName",
                )}
              >
                <input
                  value={props.params?.azureResourceName ?? ""}
                  onChange={(event) =>
                    patchParams({
                      azureResourceName: event.target.value || null,
                    })
                  }
                />
              </Field>
              <Field
                label={tr("copy.azureApiVersion")}
                tooltip={props.help.field<LlmParams>(
                  "LocalLLMParams",
                  "azureApiVersion",
                )}
                hint={tr("copy.optionalLeaveUnsetToUseTheGatewayDefault")}
              >
                <input
                  value={props.params?.azureApiVersion ?? ""}
                  onChange={(event) =>
                    patchParams({ azureApiVersion: event.target.value || null })
                  }
                />
              </Field>
              <FieldGroup
                label={tr("copy.azureResourceType")}
                tooltip={props.help.field<LlmParams>(
                  "LocalLLMParams",
                  "azureResourceType",
                )}
              >
                <EnumSelector
                  ariaLabel="Azure resource type"
                  value={azureResourceType}
                  options={[
                    { value: "openAI", label: "OpenAI" },
                    { value: "foundry", label: tr("copy.foundry") },
                  ]}
                  schema={props.help.node([
                    "$defs",
                    "LocalLLMParams",
                    "properties",
                    "azureResourceType",
                  ])}
                  onChange={(value) =>
                    patchParams({ azureResourceType: value })
                  }
                />
              </FieldGroup>
              {azureResourceType === "foundry" ? (
                <Field
                  label={tr("copy.azureProjectName")}
                  tooltip={props.help.field<LlmParams>(
                    "LocalLLMParams",
                    "azureProjectName",
                  )}
                >
                  <input
                    value={props.params?.azureProjectName ?? ""}
                    onChange={(event) =>
                      patchParams({
                        azureProjectName: event.target.value || null,
                      })
                    }
                  />
                </Field>
              ) : null}
            </div>
          ) : null}
          {provider === "custom" &&
          props.provider &&
          typeof props.provider !== "string" &&
          "custom" in props.provider ? (
            <CustomProviderSettings
              provider={props.provider}
              params={props.params}
              help={props.help}
              onProviderChange={(nextProvider) =>
                props.onProviderChange(nextProvider, props.params ?? {})
              }
              onParamsChange={props.onParamsChange}
            />
          ) : null}
        </>
      ) : null}
    </>
  );
}

function CustomProviderSettings(props: {
  provider: Extract<ModelProvider, { custom: unknown }>;
  params?: LlmParams;
  help: SchemaHelp;
  onProviderChange: (provider: ModelProvider) => void;
  onParamsChange: (params: LlmParams) => void;
}) {
  const fakeModel: LlmModel = {
    name: "",
    provider: props.provider,
    params: props.params,
  };

  return (
    <section className="policy-form-section">
      <div className="policy-form-section-header">
        <span className="policy-form-section-icon">
          <ProviderIcon provider="custom" />
        </span>
        <div>
          <h4>{tr("copy.customProvider")}</h4>
          <p>
            {tr(
              "copy.useThisWhenTheUpstreamExposesOneOrMoreLlmCompatibleHttpApisAtYourOwnEndpoint",
            )}
          </p>
        </div>
      </div>
      <div className="policy-form-section-body">
        <Field
          label={tr("copy.baseUrl")}
          tooltip={props.help.field<LlmParams>("LocalLLMParams", "baseUrl")}
        >
          <input
            value={props.params?.baseUrl ?? ""}
            onChange={(event) =>
              props.onParamsChange({
                ...(props.params ?? {}),
                baseUrl: event.target.value || null,
              })
            }
            placeholder="https://llm.internal.example.com"
          />
        </Field>
        <div className="section-heading compact">
          <h3>{tr("copy.routeFormats")}</h3>
          <p>
            {props.help.field<CustomProvider>(
              "CustomProvider",
              "formats",
              "Select each API shape this custom provider supports. Optional path overrides are appended to the base URL.",
            )}
          </p>
        </div>
        <CustomFormats
          model={fakeModel}
          help={props.help}
          setModel={(value) => {
            const next = typeof value === "function" ? value(fakeModel) : value;
            props.onProviderChange(next.provider);
          }}
        />
      </div>
    </section>
  );
}

type AwsCredentialMode = "ambient" | "static";
type GcpCredentialMode = "ambient" | "file";
type AzureCredentialMode = "default" | "managedIdentity" | "apiKey";

type ProviderAuthKey = "aws" | "gcp" | "azure";
type ProviderAuthVariant<K extends ProviderAuthKey> = Extract<
  CanonicalProviderAuth,
  Record<K, unknown>
>;

function canonicalAuth<K extends ProviderAuthKey>(
  auth: ProviderAuth | null | undefined,
  key: K,
): ProviderAuthVariant<K> | null {
  return typeof auth === "object" && auth !== null && key in auth
    ? (auth as ProviderAuthVariant<K>)
    : null;
}

function AwsCredentials(props: {
  value?: ProviderAuth | null;
  onChange?: (auth: CanonicalProviderAuth | null) => void;
}) {
  const aws = canonicalAuth(props.value, "aws")?.aws ?? null;
  const staticAws = aws && "accessKeyId" in aws ? aws : null;
  const [mode, setMode] = useState<AwsCredentialMode>(
    staticAws ? "static" : "ambient",
  );
  const [accessKeyId, setAccessKeyId] = useState(staticAws?.accessKeyId ?? "");
  const [secretAccessKey, setSecretAccessKey] = useState(
    staticAws?.secretAccessKey ?? "",
  );
  const [sessionToken, setSessionToken] = useState(
    staticAws?.sessionToken ?? "",
  );
  const [showSecret, setShowSecret] = useState(false);

  function setAmbient() {
    setMode("ambient");
    props.onChange?.(null);
  }

  function saveStatic(next: {
    accessKeyId?: string;
    secretAccessKey?: string;
    sessionToken?: string | null;
  }) {
    const merged = {
      accessKeyId,
      secretAccessKey,
      sessionToken: sessionToken || null,
      ...next,
    };
    setAccessKeyId(merged.accessKeyId ?? "");
    setSecretAccessKey(merged.secretAccessKey ?? "");
    setSessionToken(merged.sessionToken ?? "");
    props.onChange?.({
      aws: {
        accessKeyId: merged.accessKeyId ?? "",
        secretAccessKey: merged.secretAccessKey ?? "",
        region: null,
        sessionToken: merged.sessionToken || null,
        serviceName: null,
      },
    });
  }

  return (
    <FieldGroup
      label={tr("copy.awsCredentials")}
      tooltip={tr(
        "copy.useAmbientAwsCredentialsOrStaticAccessKeysForBedrockSigning",
      )}
    >
      <div className="credential-row">
        <div className="segmented-control compact">
          <button
            className={mode === "ambient" ? "active" : ""}
            type="button"
            onClick={setAmbient}
          >
            {tr("copy.ambient")}
          </button>
          <button
            className={mode === "static" ? "active" : ""}
            type="button"
            onClick={() => {
              setMode("static");
              saveStatic({});
            }}
          >
            {tr("copy.static")}
          </button>
        </div>
        {mode === "static" ? (
          <div className="credential-grid">
            <input
              value={accessKeyId}
              onChange={(event) =>
                saveStatic({ accessKeyId: event.target.value })
              }
              placeholder={tr("copy.awsAccessKeyId")}
            />
            <div className="api-key-value-wrap">
              <input
                value={secretAccessKey}
                type="text"
                className={showSecret ? undefined : "masked-secret-input"}
                onChange={(event) =>
                  saveStatic({ secretAccessKey: event.target.value })
                }
                placeholder={tr("copy.awsSecretAccessKey")}
                autoComplete="off"
                autoCorrect="off"
                autoCapitalize="none"
                data-1p-ignore="true"
                data-lpignore="true"
                data-form-type="other"
                name="agw-aws-secret-access-key"
              />
              <VisibilityButton
                visible={showSecret}
                onClick={() => setShowSecret((current) => !current)}
              />
            </div>
            <input
              value={sessionToken}
              onChange={(event) =>
                saveStatic({ sessionToken: event.target.value || null })
              }
              placeholder={tr("copy.sessionTokenOptional")}
            />
          </div>
        ) : null}
      </div>
    </FieldGroup>
  );
}

function GcpCredentials(props: {
  value?: ProviderAuth | null;
  onChange?: (auth: CanonicalProviderAuth | null) => void;
}) {
  const gcp = canonicalAuth(props.value, "gcp")?.gcp ?? null;
  const file =
    gcp &&
    "credential" in gcp &&
    typeof gcp.credential === "object" &&
    gcp.credential &&
    "file" in gcp.credential
      ? gcp.credential.file
      : "";
  const [mode, setMode] = useState<GcpCredentialMode>(
    file ? "file" : "ambient",
  );

  function setFile(path: string) {
    props.onChange?.({
      gcp: { credential: path.trim() ? { file: path } : null },
    });
  }

  return (
    <FieldGroup
      label={tr("copy.googleCredentials")}
      tooltip={tr(
        "copy.useApplicationDefaultCredentialsOrAServiceAccountJsonFileForVertex",
      )}
    >
      <div className="credential-row">
        <div className="segmented-control compact">
          <button
            className={mode === "ambient" ? "active" : ""}
            type="button"
            onClick={() => {
              setMode("ambient");
              props.onChange?.(null);
            }}
          >
            {tr("copy.adc")}
          </button>
          <button
            className={mode === "file" ? "active" : ""}
            type="button"
            onClick={() => {
              setMode("file");
              setFile(file);
            }}
          >
            {tr("copy.file")}
          </button>
        </div>
        {mode === "file" ? (
          <input
            value={file}
            onChange={(event) => setFile(event.target.value)}
            placeholder="$HOME/.secrets/gcp-sa.json"
          />
        ) : null}
      </div>
    </FieldGroup>
  );
}

function AzureCredentials(props: {
  auth?: ProviderAuth | null;
  apiKey?: SecretFromFile | string | null;
  onAuthChange?: (auth: CanonicalProviderAuth | null) => void;
  onApiKeyChange: (apiKey: SecretFromFile | string | null) => void;
}) {
  const azure = canonicalAuth(props.auth, "azure")?.azure ?? null;
  const managed =
    azure &&
    "explicitConfig" in azure &&
    "managedIdentity" in azure.explicitConfig
      ? azure.explicitConfig.managedIdentity
      : null;
  const [mode, setMode] = useState<AzureCredentialMode>(
    props.apiKey ? "apiKey" : managed ? "managedIdentity" : "default",
  );
  const [clientId, setClientId] = useState(
    azureManagedIdentityClientId(managed),
  );

  function setDefault() {
    setMode("default");
    props.onApiKeyChange(null);
    props.onAuthChange?.(null);
  }

  function setManaged(nextClientId = clientId) {
    setMode("managedIdentity");
    setClientId(nextClientId);
    props.onApiKeyChange(null);
    props.onAuthChange?.({
      azure: {
        explicitConfig: {
          managedIdentity: {
            userAssignedIdentity: nextClientId.trim()
              ? { clientId: nextClientId.trim() }
              : null,
          },
        },
      },
    });
  }

  function setApiKeyMode() {
    setMode("apiKey");
    props.onAuthChange?.(null);
  }

  return (
    <FieldGroup
      label={tr("copy.azureCredentials")}
      tooltip={tr(
        "copy.useAzureDefaultCredentialsManagedIdentityOrAnAzureApiKey",
      )}
    >
      <div className="credential-row">
        <div className="segmented-control compact">
          <button
            className={mode === "default" ? "active" : ""}
            type="button"
            onClick={setDefault}
          >
            {tr("copy.default")}
          </button>
          <button
            className={mode === "managedIdentity" ? "active" : ""}
            type="button"
            onClick={() => setManaged()}
          >
            {tr("copy.managed")}
          </button>
          <button
            className={mode === "apiKey" ? "active" : ""}
            type="button"
            onClick={setApiKeyMode}
          >
            {tr("copy.apiKey")}
          </button>
        </div>
        {mode === "managedIdentity" ? (
          <input
            value={clientId}
            onChange={(event) => setManaged(event.target.value)}
            placeholder={tr("copy.clientIdOptional")}
          />
        ) : mode === "apiKey" ? (
          <ApiKeyInput value={props.apiKey} onChange={props.onApiKeyChange} />
        ) : null}
      </div>
    </FieldGroup>
  );
}

type ApiKeyMode = "unset" | "env" | "key" | "file";

function ApiKeyInput(props: {
  value: string | SecretFromFile | null | undefined;
  onChange: (value: string | SecretFromFile | null) => void;
}) {
  const [mode, setMode] = useState<ApiKeyMode>(() => apiKeyMode(props.value));
  const [showKey, setShowKey] = useState(false);

  useEffect(() => {
    setMode(apiKeyMode(props.value));
  }, [props.value]);

  const inputValue = apiKeyInputValue(props.value, mode);

  function setNextMode(nextMode: ApiKeyMode) {
    setMode(nextMode);
    setShowKey(false);
    if (nextMode === "unset") {
      props.onChange(null);
      return;
    }
    props.onChange(apiKeyFromInput(inputValue, nextMode));
  }

  return (
    <div className="api-key-input-row">
      <div className="segmented-control compact api-key-mode-control">
        <button
          className={mode === "unset" ? "active" : ""}
          type="button"
          onClick={() => setNextMode("unset")}
        >
          {tr("copy.unset")}
        </button>
        <button
          className={mode === "env" ? "active" : ""}
          type="button"
          onClick={() => setNextMode("env")}
        >
          {tr("copy.envVar")}
        </button>
        <button
          className={mode === "key" ? "active" : ""}
          type="button"
          onClick={() => setNextMode("key")}
        >
          {tr("copy.apiKey")}
        </button>
        <button
          className={mode === "file" ? "active" : ""}
          type="button"
          onClick={() => setNextMode("file")}
        >
          {tr("copy.file")}
        </button>
      </div>
      {mode === "unset" ? (
        <span className="api-key-unset-copy">
          {tr("copy.noProviderCredentialConfigured")}
        </span>
      ) : (
        <div className="api-key-value-wrap">
          <input
            value={inputValue}
            type="text"
            className={
              mode === "key" && !showKey ? "masked-secret-input" : undefined
            }
            autoComplete="off"
            autoCorrect="off"
            autoCapitalize="none"
            data-1p-ignore="true"
            data-lpignore="true"
            data-form-type="other"
            name={`agw-provider-${mode}`}
            spellCheck={false}
            onChange={(event) =>
              props.onChange(apiKeyFromInput(event.target.value, mode))
            }
            placeholder={
              mode === "env"
                ? "ENV_VAR_NAME"
                : mode === "file"
                  ? "$HOME/.secrets/provider"
                  : "sk-..."
            }
          />
          {mode === "key" ? (
            <VisibilityButton
              visible={showKey}
              onClick={() => setShowKey((current) => !current)}
            />
          ) : null}
        </div>
      )}
    </div>
  );
}

function azureManagedIdentityClientId(value: unknown) {
  if (!value || typeof value !== "object" || !("userAssignedIdentity" in value))
    return "";
  const identity = value.userAssignedIdentity;
  if (!identity || typeof identity !== "object" || !("clientId" in identity))
    return "";
  return typeof identity.clientId === "string" ? identity.clientId : "";
}

function VisibilityButton(props: { visible: boolean; onClick: () => void }) {
  return (
    <button
      className="icon-button api-key-visibility"
      type="button"
      aria-label={props.visible ? "Hide secret" : "Show secret"}
      onClick={props.onClick}
    >
      {props.visible ? <EyeOff size={16} /> : <Eye size={16} />}
    </button>
  );
}

function apiKeyMode(
  value: string | SecretFromFile | null | undefined,
): ApiKeyMode {
  if (typeof value === "object" && value && "file" in value) return "file";
  if (typeof value === "string" && value.startsWith("$")) return "env";
  if (typeof value === "string") return "key";
  return "unset";
}

function apiKeyInputValue(
  value: string | SecretFromFile | null | undefined,
  mode: ApiKeyMode,
) {
  if (!value) return "";
  if (mode === "file" && typeof value === "object" && "file" in value)
    return value.file;
  if (mode === "env" && typeof value === "string")
    return value.startsWith("$") ? value.slice(1) : value;
  if (mode === "key" && typeof value === "string") return value;
  return "";
}

function apiKeyFromInput(
  value: string,
  mode: ApiKeyMode,
): string | SecretFromFile | null {
  const trimmed = value.trim();
  if (mode === "unset") return null;
  if (mode === "file") return { file: trimmed };
  if (mode === "env") return trimmed.startsWith("$") ? trimmed : `$${trimmed}`;
  return value;
}
