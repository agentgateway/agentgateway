import { useEffect, useState } from "react";
import { Eye, EyeOff } from "lucide-react";
import {
  providerDefaultApiKey,
  providerDisplayName,
  providerLabel,
  providerReferenceName,
  visibleProviderNames,
} from "../../config";
import { Field, FieldGroup, Dropdown } from "../../components/Primitives";
import { ProviderIcon } from "../../components/ProviderIcon";
import type { SchemaHelp } from "../../schemaHelp";
import type { LlmParams, LlmProvider, ModelProvider, ProviderAuth, ProviderName, SecretFromFile } from "../../types";
import { CustomFormats } from "./CustomFormats";
import type { LlmModel } from "../../types";

export function ProviderConfigEditor(props: {
  provider: ModelProvider | null;
  params?: LlmParams;
  auth?: ProviderAuth | null;
  providers?: LlmProvider[];
  help: SchemaHelp;
  onProviderChange: (provider: ModelProvider, params: LlmParams) => void;
  onParamsChange: (params: LlmParams) => void;
  onAuthChange?: (auth: ProviderAuth | null) => void;
}) {
  const providerReference = providerReferenceName(props.provider);
  const provider = providerLabel(props.provider) as ProviderName;
  const providerChoices = visibleProviderNames(true);
  const selectedProviderValue = providerReference ? `provider:${providerReference}` : provider ? `builtin:${provider}` : "";
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
      nextProvider === "custom" ? { custom: { formats: [{ type: "completions" }] } } : nextProvider,
      { ...(props.params ?? {}), apiKey: providerDefaultApiKey(nextProvider) },
    );
  }

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
      <FieldGroup label="Provider" tooltip={props.help.description(["$defs", "LocalLLMModels", "properties", "provider"])}>
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
              tooltip={props.help.description(["$defs", "LocalLLMParams", "properties", "apiKey"])}
              hint="Leave empty for provider auto-detection."
            >
              <ApiKeyInput value={props.params?.apiKey} onChange={(apiKey) => patchParams({ apiKey })} />
            </Field>
          )}

          {provider === "vertex" ? (
            <div className="form-grid">
              <Field label="Vertex project" tooltip={props.help.description(["$defs", "LocalLLMParams", "properties", "vertexProject"], "Google Cloud project used for Vertex AI requests.")}><input value={props.params?.vertexProject ?? ""} onChange={(event) => patchParams({ vertexProject: event.target.value || null })} /></Field>
              <Field label="Vertex region" tooltip={props.help.description(["$defs", "LocalLLMParams", "properties", "vertexRegion"], "Google Cloud region used for Vertex AI requests.")}><input value={props.params?.vertexRegion ?? ""} onChange={(event) => patchParams({ vertexRegion: event.target.value || null })} /></Field>
            </div>
          ) : null}
          {provider === "bedrock" ? (
            <Field label="AWS region" tooltip={props.help.description(["$defs", "LocalLLMParams", "properties", "awsRegion"], "AWS region used for Bedrock requests.")}><input value={props.params?.awsRegion ?? ""} onChange={(event) => patchParams({ awsRegion: event.target.value || null })} /></Field>
          ) : null}
          {provider === "ollama" ? (
            <Field
              label="Base URL"
              tooltip={props.help.description(["$defs", "LocalLLMParams", "properties", "baseUrl"], "Override when Ollama is hosted somewhere other than the local default.")}
              hint="Optional. Defaults to http://localhost:11434/v1."
            >
              <input value={props.params?.baseUrl ?? ""} onChange={(event) => patchParams({ baseUrl: event.target.value || null })} placeholder="http://localhost:11434/v1" />
            </Field>
          ) : provider !== "bedrock" && provider !== "vertex" && provider !== "azure" && provider !== "custom" ? (
            <Field
              label="Base URL"
              tooltip={props.help.description(["$defs", "LocalLLMParams", "properties", "baseUrl"], "Override the default API endpoint. Useful for proxies or OpenAI-compatible self-hosted models.")}
              hint="Optional. Leave empty to use the provider's default endpoint."
            >
              <input value={props.params?.baseUrl ?? ""} onChange={(event) => patchParams({ baseUrl: event.target.value || null })} placeholder="https://api.example.com/v1" />
            </Field>
          ) : null}
          {provider === "azure" ? (
            <div className="form-grid">
              <Field label="Azure resource name" tooltip={props.help.description(["$defs", "LocalLLMParams", "properties", "azureResourceName"])}><input value={props.params?.azureResourceName ?? ""} onChange={(event) => patchParams({ azureResourceName: event.target.value || null })} /></Field>
              <Field label="Azure API version" tooltip={props.help.description(["$defs", "LocalLLMParams", "properties", "azureApiVersion"])}><input value={props.params?.azureApiVersion ?? ""} onChange={(event) => patchParams({ azureApiVersion: event.target.value || null })} /></Field>
              <FieldGroup label="Azure resource type" tooltip={props.help.description(["$defs", "LocalLLMParams", "properties", "azureResourceType"])}>
                <Dropdown
                  ariaLabel="Azure resource type"
                  value={props.params?.azureResourceType ?? "openAI"}
                  options={[
                    { value: "openAI", label: "OpenAI" },
                    { value: "foundry", label: "Foundry" },
                  ]}
                  onChange={(value) => patchParams({ azureResourceType: value as "openAI" | "foundry" })}
                />
              </FieldGroup>
              <Field label="Azure project name" tooltip={props.help.description(["$defs", "LocalLLMParams", "properties", "azureProjectName"])}><input value={props.params?.azureProjectName ?? ""} onChange={(event) => patchParams({ azureProjectName: event.target.value || null })} /></Field>
            </div>
          ) : null}
          {provider === "custom" && props.provider && typeof props.provider !== "string" && "custom" in props.provider ? (
            <CustomProviderSettings
              provider={props.provider}
              params={props.params}
              help={props.help}
              onProviderChange={(nextProvider) => props.onProviderChange(nextProvider, props.params ?? {})}
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
        <span className="policy-form-section-icon"><ProviderIcon provider="custom" /></span>
        <div>
          <h4>Custom provider</h4>
          <p>Use this when the upstream exposes one or more LLM-compatible HTTP APIs at your own endpoint.</p>
        </div>
      </div>
      <div className="policy-form-section-body">
        <Field label="Base URL" tooltip={props.help.description(["$defs", "LocalLLMParams", "properties", "baseUrl"])}>
          <input value={props.params?.baseUrl ?? ""} onChange={(event) => props.onParamsChange({ ...(props.params ?? {}), baseUrl: event.target.value || null })} placeholder="https://llm.internal.example.com" />
        </Field>
        <div className="section-heading compact">
          <h3>Route formats</h3>
          <p>{props.help.description(["$defs", "Provider8", "properties", "formats"], "Select each API shape this custom provider supports. Optional path overrides are appended to the base URL.")}</p>
        </div>
        <CustomFormats
          model={fakeModel}
          help={props.help}
          setModel={(value) => {
            const next = typeof value === "function" ? value(fakeModel) : value;
            if (next.provider) props.onProviderChange(next.provider);
          }}
        />
      </div>
    </section>
  );
}

type AwsCredentialMode = "ambient" | "static";
type GcpCredentialMode = "ambient" | "file";
type AzureCredentialMode = "default" | "managedIdentity" | "apiKey";

function AwsCredentials(props: {
  value?: ProviderAuth | null;
  onChange?: (auth: ProviderAuth | null) => void;
}) {
  const aws = typeof props.value === "object" && props.value && "aws" in props.value ? props.value.aws : null;
  const staticAws = aws && "accessKeyId" in aws ? aws : null;
  const [mode, setMode] = useState<AwsCredentialMode>(staticAws ? "static" : "ambient");
  const [accessKeyId, setAccessKeyId] = useState(staticAws?.accessKeyId ?? "");
  const [secretAccessKey, setSecretAccessKey] = useState(staticAws?.secretAccessKey ?? "");
  const [sessionToken, setSessionToken] = useState(staticAws?.sessionToken ?? "");
  const [showSecret, setShowSecret] = useState(false);

  function setAmbient() {
    setMode("ambient");
    props.onChange?.(null);
  }

  function saveStatic(next: { accessKeyId?: string; secretAccessKey?: string; sessionToken?: string | null }) {
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
    <FieldGroup label="AWS credentials" tooltip="Use ambient AWS credentials or static access keys for Bedrock signing.">
      <div className="credential-row">
        <div className="segmented-control compact">
          <button className={mode === "ambient" ? "active" : ""} type="button" onClick={setAmbient}>Ambient</button>
          <button className={mode === "static" ? "active" : ""} type="button" onClick={() => { setMode("static"); saveStatic({}); }}>Static</button>
        </div>
        {mode === "static" ? (
          <div className="credential-grid">
            <input value={accessKeyId} onChange={(event) => saveStatic({ accessKeyId: event.target.value })} placeholder="AWS access key ID" />
            <div className="api-key-value-wrap">
              <input
                value={secretAccessKey}
                type={showSecret ? "text" : "password"}
                onChange={(event) => saveStatic({ secretAccessKey: event.target.value })}
                placeholder="AWS secret access key"
                autoComplete="off"
              />
              <VisibilityButton visible={showSecret} onClick={() => setShowSecret((current) => !current)} />
            </div>
            <input value={sessionToken} onChange={(event) => saveStatic({ sessionToken: event.target.value || null })} placeholder="Session token (optional)" />
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
  const gcp = typeof props.value === "object" && props.value && "gcp" in props.value ? props.value.gcp : null;
  const file = gcp && "credential" in gcp && typeof gcp.credential === "object" && gcp.credential && "file" in gcp.credential ? gcp.credential.file : "";
  const [mode, setMode] = useState<GcpCredentialMode>(file ? "file" : "ambient");

  function setFile(path: string) {
    props.onChange?.({ gcp: { credential: path.trim() ? { file: path } : null } });
  }

  return (
    <FieldGroup label="Google credentials" tooltip="Use Application Default Credentials or a service account JSON file for Vertex.">
      <div className="credential-row">
        <div className="segmented-control compact">
          <button className={mode === "ambient" ? "active" : ""} type="button" onClick={() => { setMode("ambient"); props.onChange?.(null); }}>ADC</button>
          <button className={mode === "file" ? "active" : ""} type="button" onClick={() => { setMode("file"); setFile(file); }}>File</button>
        </div>
        {mode === "file" ? <input value={file} onChange={(event) => setFile(event.target.value)} placeholder="$HOME/.secrets/gcp-sa.json" /> : null}
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
  const azure = typeof props.auth === "object" && props.auth && "azure" in props.auth ? props.auth.azure : null;
  const managed = azure && "explicitConfig" in azure && "managedIdentity" in azure.explicitConfig ? azure.explicitConfig.managedIdentity : null;
  const [mode, setMode] = useState<AzureCredentialMode>(props.apiKey ? "apiKey" : managed ? "managedIdentity" : "default");
  const [clientId, setClientId] = useState(azureManagedIdentityClientId(managed));

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
            userAssignedIdentity: nextClientId.trim() ? { clientId: nextClientId.trim() } : null,
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
    <FieldGroup label="Azure credentials" tooltip="Use Azure default credentials, managed identity, or an Azure API key.">
      <div className="credential-row">
        <div className="segmented-control compact">
          <button className={mode === "default" ? "active" : ""} type="button" onClick={setDefault}>Default</button>
          <button className={mode === "managedIdentity" ? "active" : ""} type="button" onClick={() => setManaged()}>Managed</button>
          <button className={mode === "apiKey" ? "active" : ""} type="button" onClick={setApiKeyMode}>API key</button>
        </div>
        {mode === "managedIdentity" ? (
          <input value={clientId} onChange={(event) => setManaged(event.target.value)} placeholder="Client ID (optional)" />
        ) : mode === "apiKey" ? (
          <ApiKeyInput value={props.apiKey} onChange={props.onApiKeyChange} />
        ) : null}
      </div>
    </FieldGroup>
  );
}

type ApiKeyMode = "env" | "key" | "file";

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
    props.onChange(apiKeyFromInput(inputValue, nextMode));
  }

  return (
    <div className="api-key-input-row">
      <div className="segmented-control compact">
        <button className={mode === "env" ? "active" : ""} type="button" onClick={() => setNextMode("env")}>Env var</button>
        <button className={mode === "key" ? "active" : ""} type="button" onClick={() => setNextMode("key")}>API key</button>
        <button className={mode === "file" ? "active" : ""} type="button" onClick={() => setNextMode("file")}>File</button>
      </div>
      <div className="api-key-value-wrap">
        <input
          value={inputValue}
          type={mode === "key" && !showKey ? "password" : "text"}
          autoComplete="off"
          spellCheck={false}
          onChange={(event) => props.onChange(apiKeyFromInput(event.target.value, mode))}
          placeholder={mode === "env" ? "OPENAI_API_KEY" : mode === "file" ? "$HOME/.secrets/openai" : "sk-..."}
        />
        {mode === "key" ? (
          <VisibilityButton visible={showKey} onClick={() => setShowKey((current) => !current)} />
        ) : null}
      </div>
    </div>
  );
}

function azureManagedIdentityClientId(value: unknown) {
  if (!value || typeof value !== "object" || !("userAssignedIdentity" in value)) return "";
  const identity = value.userAssignedIdentity;
  if (!identity || typeof identity !== "object" || !("clientId" in identity)) return "";
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

function apiKeyMode(value: string | SecretFromFile | null | undefined): ApiKeyMode {
  if (typeof value === "object" && value && "file" in value) return "file";
  if (typeof value === "string" && value.startsWith("$")) return "env";
  if (typeof value === "string" && value.trim()) return "key";
  return "env";
}

function apiKeyInputValue(value: string | SecretFromFile | null | undefined, mode: ApiKeyMode) {
  if (!value) return "";
  if (mode === "file" && typeof value === "object" && "file" in value) return value.file;
  if (mode === "env" && typeof value === "string") return value.startsWith("$") ? value.slice(1) : value;
  if (mode === "key" && typeof value === "string") return value;
  return "";
}

function apiKeyFromInput(value: string, mode: ApiKeyMode): string | SecretFromFile | null {
  const trimmed = value.trim();
  if (!trimmed) return null;
  if (mode === "file") return { file: trimmed };
  if (mode === "env") return trimmed.startsWith("$") ? trimmed : `$${trimmed}`;
  return value;
}
