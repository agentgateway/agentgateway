import { useEffect, useState } from "react";
import { Eye, EyeOff } from "lucide-react";
import {
  providerDisplayName,
  providerLabel,
  providerReferenceName,
  visibleProviderNames,
} from "../../config";
import { EnumSelector } from "../../components/EnumSelector";
import { FreeformCombobox } from "../../components/FreeformCombobox";
import { Field, FieldGroup, Dropdown } from "../../components/Primitives";
import { ProviderIcon } from "../../components/ProviderIcon";
import type { SchemaHelp } from "../../schemaHelp";
import type {
  LlmParams,
  LlmProvider,
  ModelProvider,
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
  onAuthChange?: (auth: ProviderAuth | null) => void;
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
            {item.name} <small className="muted">configured</small>
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
        label="Provider"
        tooltip={props.help.field<LlmModel>("LocalLLMModels", "provider")}
      >
        <Dropdown
          ariaLabel="Provider"
          value={selectedProviderValue}
          searchable
          options={options}
          placeholder="Select provider"
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
              label="Provider API key"
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
                label="Vertex project"
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
                label="Vertex region"
                tooltip={props.help.field<LlmParams>(
                  "LocalLLMParams",
                  "vertexRegion",
                  "Google Cloud region used for Vertex AI requests.",
                )}
                hint="Optional. If unset, Vertex uses global."
              >
                <FreeformCombobox
                  ariaLabel="Vertex region"
                  value={props.params?.vertexRegion ?? ""}
                  options={vertexRegions}
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
              label="AWS region"
              tooltip={props.help.field<LlmParams>(
                "LocalLLMParams",
                "awsRegion",
                "AWS region used for Bedrock requests.",
              )}
            >
              <FreeformCombobox
                ariaLabel="AWS region"
                value={props.params?.awsRegion ?? ""}
                options={awsRegions}
                onChange={(value) => patchParams({ awsRegion: value || null })}
                placeholder="us-west-2"
              />
            </Field>
          ) : null}
          {provider === "ollama" ? (
            <Field
              label="Base URL"
              tooltip={props.help.field<LlmParams>(
                "LocalLLMParams",
                "baseUrl",
                "Override when Ollama is hosted somewhere other than the local default.",
              )}
              hint="Optional. Defaults to http://localhost:11434/v1."
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
                label="Azure resource name"
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
                label="Azure API version"
                tooltip={props.help.field<LlmParams>(
                  "LocalLLMParams",
                  "azureApiVersion",
                )}
                hint="Optional. Leave unset to use the gateway default."
              >
                <input
                  value={props.params?.azureApiVersion ?? ""}
                  onChange={(event) =>
                    patchParams({ azureApiVersion: event.target.value || null })
                  }
                />
              </Field>
              <FieldGroup
                label="Azure resource type"
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
                    { value: "foundry", label: "Foundry" },
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
                  label="Azure project name"
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
          <h4>Custom provider</h4>
          <p>
            Use this when the upstream exposes one or more LLM-compatible HTTP
            APIs at your own endpoint.
          </p>
        </div>
      </div>
      <div className="policy-form-section-body">
        <Field
          label="Base URL"
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
          <h3>Route formats</h3>
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

const awsRegions = [
  "us-east-1",
  "us-east-2",
  "us-west-1",
  "us-west-2",
  "af-south-1",
  "ap-east-1",
  "ap-south-2",
  "ap-southeast-3",
  "ap-southeast-5",
  "ap-southeast-4",
  "ap-south-1",
  "ap-southeast-6",
  "ap-northeast-3",
  "ap-northeast-2",
  "ap-southeast-1",
  "ap-southeast-2",
  "ap-east-2",
  "ap-southeast-7",
  "ap-northeast-1",
  "ca-central-1",
  "ca-west-1",
  "eu-central-1",
  "eu-west-1",
  "eu-west-2",
  "eu-south-1",
  "eu-west-3",
  "eu-south-2",
  "eu-north-1",
  "eu-central-2",
  "il-central-1",
  "mx-central-1",
  "me-south-1",
  "me-central-1",
  "sa-east-1",
];

const vertexRegions = [
  "africa-south1",
  "asia-east1",
  "asia-east2",
  "asia-northeast1",
  "asia-northeast2",
  "asia-northeast3",
  "asia-south1",
  "asia-south2",
  "asia-southeast1",
  "asia-southeast2",
  "asia-southeast3",
  "australia-southeast1",
  "australia-southeast2",
  "europe-central2",
  "europe-north1",
  "europe-north2",
  "europe-southwest1",
  "europe-west1",
  "europe-west10",
  "europe-west12",
  "europe-west2",
  "europe-west3",
  "europe-west4",
  "europe-west6",
  "europe-west8",
  "europe-west9",
  "me-central1",
  "me-central2",
  "me-west1",
  "northamerica-northeast1",
  "northamerica-northeast2",
  "northamerica-south1",
  "southamerica-east1",
  "southamerica-west1",
  "us-central1",
  "us-east1",
  "us-east4",
  "us-east5",
  "us-south1",
  "us-west1",
  "us-west2",
  "us-west3",
  "us-west4",
];

function AwsCredentials(props: {
  value?: ProviderAuth | null;
  onChange?: (auth: ProviderAuth | null) => void;
}) {
  const aws =
    typeof props.value === "object" && props.value && "aws" in props.value
      ? props.value.aws
      : null;
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
      label="AWS credentials"
      tooltip="Use ambient AWS credentials or static access keys for Bedrock signing."
    >
      <div className="credential-row">
        <div className="segmented-control compact">
          <button
            className={mode === "ambient" ? "active" : ""}
            type="button"
            onClick={setAmbient}
          >
            Ambient
          </button>
          <button
            className={mode === "static" ? "active" : ""}
            type="button"
            onClick={() => {
              setMode("static");
              saveStatic({});
            }}
          >
            Static
          </button>
        </div>
        {mode === "static" ? (
          <div className="credential-grid">
            <input
              value={accessKeyId}
              onChange={(event) =>
                saveStatic({ accessKeyId: event.target.value })
              }
              placeholder="AWS access key ID"
            />
            <div className="api-key-value-wrap">
              <input
                value={secretAccessKey}
                type="text"
                className={showSecret ? undefined : "masked-secret-input"}
                onChange={(event) =>
                  saveStatic({ secretAccessKey: event.target.value })
                }
                placeholder="AWS secret access key"
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
              placeholder="Session token (optional)"
            />
          </div>
        ) : null}
      </div>
    </FieldGroup>
  );
}

function GcpCredentials(props: {
  value?: ProviderAuth | null;
  onChange?: (auth: ProviderAuth | null) => void;
}) {
  const gcp =
    typeof props.value === "object" && props.value && "gcp" in props.value
      ? props.value.gcp
      : null;
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
      label="Google credentials"
      tooltip="Use Application Default Credentials or a service account JSON file for Vertex."
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
            ADC
          </button>
          <button
            className={mode === "file" ? "active" : ""}
            type="button"
            onClick={() => {
              setMode("file");
              setFile(file);
            }}
          >
            File
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
  onAuthChange?: (auth: ProviderAuth | null) => void;
  onApiKeyChange: (apiKey: SecretFromFile | string | null) => void;
}) {
  const azure =
    typeof props.auth === "object" && props.auth && "azure" in props.auth
      ? props.auth.azure
      : null;
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
      label="Azure credentials"
      tooltip="Use Azure default credentials, managed identity, or an Azure API key."
    >
      <div className="credential-row">
        <div className="segmented-control compact">
          <button
            className={mode === "default" ? "active" : ""}
            type="button"
            onClick={setDefault}
          >
            Default
          </button>
          <button
            className={mode === "managedIdentity" ? "active" : ""}
            type="button"
            onClick={() => setManaged()}
          >
            Managed
          </button>
          <button
            className={mode === "apiKey" ? "active" : ""}
            type="button"
            onClick={setApiKeyMode}
          >
            API key
          </button>
        </div>
        {mode === "managedIdentity" ? (
          <input
            value={clientId}
            onChange={(event) => setManaged(event.target.value)}
            placeholder="Client ID (optional)"
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
          Unset
        </button>
        <button
          className={mode === "env" ? "active" : ""}
          type="button"
          onClick={() => setNextMode("env")}
        >
          Env var
        </button>
        <button
          className={mode === "key" ? "active" : ""}
          type="button"
          onClick={() => setNextMode("key")}
        >
          API key
        </button>
        <button
          className={mode === "file" ? "active" : ""}
          type="button"
          onClick={() => setNextMode("file")}
        >
          File
        </button>
      </div>
      {mode === "unset" ? (
        <span className="api-key-unset-copy">
          No provider credential configured.
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
