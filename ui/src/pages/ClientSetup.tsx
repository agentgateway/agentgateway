import { useMemo, useState } from "react";
import type { ReactNode } from "react";
import { Check, Clipboard, Code2, KeyRound, Terminal } from "lucide-react";
import {
  Dropdown,
  Field,
  FieldGroup,
  PageHeader,
  Panel,
  StatusBanner,
} from "../components/Primitives";
import { CatalogModelSelector } from "../components/CatalogModelSelector";
import { keyLabel, maskKey } from "../credentialDisplay";
import { gatewayOrigin } from "../gatewayUrls";
import { useGatewayConfig } from "../hooks";
import { llmModelOptions, resolveLlmModelOption } from "../llmModelOptions";
import {
  isWildcardModelName,
  modelProviderLabel,
  resolveModelName,
  wildcardModelPrefix,
  wildcardResolvedSuffix,
} from "../modelResolution";
import { ProviderIcon } from "../components/ProviderIcon";
import type { LlmModel, LlmProvider, ProviderName } from "../types";
import claudeIcon from "../assets/claude-color.svg";
import codexIcon from "../assets/codex-color.svg";
import curlIcon from "../assets/curl.svg";
import cursorIcon from "../assets/cursor.svg";
import githubCopilotIcon from "../assets/providers/copilot.svg";
import opencodeIcon from "../assets/opencode.svg";
import windsurfIcon from "../assets/windsurf.svg";

type ClientRecipe = {
  id: string;
  title: string;
  description: string;
  icon:
    | "claude"
    | "codex"
    | "curl"
    | "cursor"
    | "copilot"
    | "opencode"
    | "windsurf";
  provider?: ProviderName;
  steps?: ReactNode[];
  language: string;
  code: string;
};

export function ClientSetupPage() {
  const config = useGatewayConfig();
  const modelOptions = useMemo(
    () => llmModelOptions(config.data?.llm),
    [config.data],
  );
  const virtualKeys = useMemo(
    () => config.data?.llm?.policies?.apiKey?.keys ?? [],
    [config.data],
  );
  const derivedBaseUrl = gatewayOrigin(config.data?.llm?.port ?? 4000);
  const [baseUrl, setBaseUrl] = useState(derivedBaseUrl);
  const [baseUrlTouched, setBaseUrlTouched] = useState(false);
  const [model, setModel] = useState("");
  const [specificModel, setSpecificModel] = useState("");
  const [apiKeyMode, setApiKeyMode] = useState<"saved" | "raw">("saved");
  const [selectedKey, setSelectedKey] = useState("");
  const [rawKey, setRawKey] = useState("");
  const [selectedIntegration, setSelectedIntegration] = useState("curl");

  const selectedModel = modelOptions.some((item) => item.name === model)
    ? model
    : (modelOptions[0]?.name ?? "");
  const selectedModelOption = modelOptions.find(
    (item) => item.name === selectedModel,
  );
  const selectedModelConfig =
    selectedModelOption?.kind === "model"
      ? selectedModelOption.model
      : undefined;
  const wildcardPrefix =
    selectedModelConfig && isWildcardModelName(selectedModelConfig.name)
      ? wildcardModelPrefix(selectedModelConfig.name)
      : "";
  const specificModelSuffix = selectedModelConfig
    ? wildcardResolvedSuffix(
        specificModel,
        selectedModelConfig.name,
        wildcardPrefix,
      )
    : "";
  const selectedCatalogProvider = selectedModelConfig
    ? modelProviderLabel(selectedModelConfig, config.data?.llm?.providers ?? [])
    : null;
  const selectedVirtualKey =
    apiKeyMode === "saved"
      ? (virtualKeys.find((item) => item.key === selectedKey) ?? virtualKeys[0])
      : undefined;
  const apiKey = selectedVirtualKey?.key ?? rawKey;
  const effectiveBaseUrl = baseUrlTouched ? baseUrl : derivedBaseUrl;
  const requestModel = clientSetupRequestModel(
    selectedModelOption,
    specificModel,
    config.data?.llm?.providers ?? [],
  );
  const recipes = clientRecipes({
    baseUrl: effectiveBaseUrl,
    model: requestModel || "model",
    apiKey: apiKey || "agw_sk_...",
  });
  const activeRecipe =
    recipes.find((recipe) => recipe.id === selectedIntegration) ?? recipes[0];

  return (
    <div className="page-stack">
      <PageHeader
        title="Client Setup"
        description="Generate connection settings and snippets for OpenAI-compatible LLM clients."
      />
      {config.isError ? (
        <StatusBanner state="bad" title="Configuration API unavailable">
          {config.error.message}
        </StatusBanner>
      ) : null}
      {modelOptions.length === 0 && !config.isLoading ? (
        <StatusBanner state="warn" title="No models configured">
          Create an LLM model before wiring clients to the gateway.
        </StatusBanner>
      ) : null}

      <section className="client-setup-layout">
        <Panel className="client-setup-controls">
          <div className="section-heading">
            <h3>Connection</h3>
          </div>
          <Field
            label="Gateway base URL"
            hint="SDK snippets use this URL with /v1 appended."
          >
            <input
              value={effectiveBaseUrl}
              onChange={(event) => {
                setBaseUrlTouched(true);
                setBaseUrl(event.target.value);
              }}
              placeholder={derivedBaseUrl}
            />
          </Field>
          <FieldGroup label="Model">
            <Dropdown
              ariaLabel="Model"
              value={selectedModel}
              placeholder="No models"
              searchable
              options={modelOptions.map((item) => ({
                value: item.name,
                label: item.label,
                icon: item.icon,
                searchText: item.searchText,
              }))}
              onChange={(value) => {
                setModel(value);
                setSpecificModel("");
              }}
            />
          </FieldGroup>
          {selectedModelConfig &&
          isWildcardModelName(selectedModelConfig.name) ? (
            <Field
              label="Specific model"
              hint="Model uses a wildcard; specify the specific model."
            >
              <div className="target-resolved-composite">
                {wildcardPrefix ? (
                  <span className="target-prefix">{wildcardPrefix}</span>
                ) : null}
                <CatalogModelSelector
                  ariaLabel="Specific model"
                  value={specificModelSuffix}
                  provider={selectedCatalogProvider}
                  onChange={(value) =>
                    setSpecificModel(`${wildcardPrefix}${value}`)
                  }
                  placeholder="Select or type a model"
                />
              </div>
            </Field>
          ) : null}
          <FieldGroup label="Virtual API key">
            <Dropdown
              ariaLabel="Virtual API key"
              value={
                apiKeyMode === "saved" && selectedVirtualKey
                  ? selectedVirtualKey.key
                  : "__raw__"
              }
              options={[
                ...virtualKeys.map((item) => ({
                  value: item.key,
                  label: keyLabel(item),
                  icon: <KeyRound size={16} />,
                })),
                {
                  value: "__raw__",
                  label: "Raw value",
                  icon: <Code2 size={16} />,
                },
              ]}
              onChange={(value) => {
                if (value === "__raw__") {
                  setApiKeyMode("raw");
                  return;
                }
                setApiKeyMode("saved");
                setSelectedKey(value);
              }}
            />
          </FieldGroup>
          {apiKeyMode === "raw" ? (
            <Field label="Raw API key">
              <input
                value={rawKey}
                onChange={(event) => setRawKey(event.target.value)}
                placeholder="agw_sk_..."
              />
            </Field>
          ) : null}
          <div className="client-setup-summary">
            <div>
              <span>Base URL</span>
              <code>{effectiveBaseUrl.replace(/\/$/, "")}/v1</code>
            </div>
            <div>
              <span>Model</span>
              <code>{requestModel || "No model selected"}</code>
            </div>
            <div>
              <span>Auth</span>
              <code>
                Authorization: Bearer {apiKey ? maskKey(apiKey) : "..."}
              </code>
            </div>
          </div>
        </Panel>

        <ClientRecipeCard
          recipe={activeRecipe}
          recipes={recipes}
          selectedIntegration={activeRecipe.id}
          onSelectIntegration={setSelectedIntegration}
        />
      </section>
    </div>
  );
}

function clientSetupRequestModel(
  option: ReturnType<typeof llmModelOptions>[number] | undefined,
  specificModel: string,
  providers: LlmProvider[],
) {
  if (!option) return "";
  if (option.kind === "virtual")
    return resolveLlmModelOption(option, specificModel, providers);
  if (!option.model) return "";
  if (!isWildcardModelName(option.model.name))
    return resolveModelName(option.model, specificModel, providers);
  const normalized = normalizedClientSpecificModel(option.model, specificModel);
  if (normalized) return resolveModelName(option.model, normalized, providers);
  const prefix = wildcardModelPrefix(option.model.name);
  return prefix ? `${prefix}<model>` : "<model>";
}

function normalizedClientSpecificModel(model: LlmModel, specificModel: string) {
  const trimmed = specificModel.trim();
  const prefix = wildcardModelPrefix(model.name);
  if (!trimmed || trimmed === prefix) return "";
  if (prefix && !trimmed.startsWith(prefix)) return "";
  return trimmed;
}

function ClientRecipeCard(props: {
  recipe: ClientRecipe;
  recipes: ClientRecipe[];
  selectedIntegration: string;
  onSelectIntegration: (value: string) => void;
}) {
  return (
    <Panel className="client-recipe-card">
      <div className="client-recipe-toolbar">
        <FieldGroup label="Integration">
          <Dropdown
            ariaLabel="Integration"
            className="client-recipe-select"
            value={props.selectedIntegration}
            options={props.recipes.map((recipe) => ({
              value: recipe.id,
              label: recipe.title,
              icon: <ClientSetupIcon recipe={recipe} compact />,
              searchText: `${recipe.title} ${recipe.description}`,
            }))}
            onChange={props.onSelectIntegration}
            searchable
          />
        </FieldGroup>
        <CopyButton value={props.recipe.code} />
      </div>
      <div className="client-recipe-header">
        <ClientSetupIcon recipe={props.recipe} />
        <div>
          <h3>{props.recipe.title}</h3>
          <p>{props.recipe.description}</p>
        </div>
      </div>
      {props.recipe.steps?.length ? (
        <ol className="client-recipe-steps">
          {props.recipe.steps.map((step, index) => (
            <li key={index}>{step}</li>
          ))}
        </ol>
      ) : null}
      <HighlightedCode
        code={props.recipe.code}
        language={props.recipe.language}
      />
    </Panel>
  );
}

function CopyButton(props: { value: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      className="button"
      type="button"
      onClick={async () => {
        await navigator.clipboard.writeText(props.value);
        setCopied(true);
        window.setTimeout(() => setCopied(false), 1200);
      }}
    >
      {copied ? <Check size={16} /> : <Clipboard size={16} />}
      {copied ? "Copied" : "Copy"}
    </button>
  );
}

function clientRecipes(args: {
  baseUrl: string;
  model: string;
  apiKey: string;
}): ClientRecipe[] {
  const base = args.baseUrl.replace(/\/$/, "");
  const v1 = `${base}/v1`;
  const completions = `${v1}/chat/completions`;
  return [
    {
      id: "curl",
      title: "curl",
      description:
        "Minimal raw HTTP request for debugging client connectivity.",
      icon: "curl",
      language: "bash",
      code: `curl ${JSON.stringify(completions)} \\
  -H "Authorization: Bearer ${args.apiKey}" \\
  -H "Content-Type: application/json" \\
  -d '{
    "model": "${args.model}",
    "messages": [
      { "role": "user", "content": "Hello from agentgateway" }
    ]
  }'`,
    },
    {
      id: "claude-code",
      title: "Claude Code",
      description:
        "Use the gateway URL and key with Claude-compatible model routes when configured.",
      icon: "claude",
      language: "bash",
      code: `export ANTHROPIC_AUTH_TOKEN="${args.apiKey}"
export ANTHROPIC_BASE_URL="${base}"

claude --model "${args.model}"`,
    },
    {
      id: "claude-desktop",
      title: "Claude Desktop",
      description:
        "Route Claude Desktop third-party inference through the gateway.",
      icon: "claude",
      steps: [
        <>
          Open Claude Desktop and enable developer mode: <strong>Help</strong>{" "}
          &gt; <strong>Troubleshooting</strong> &gt;{" "}
          <strong>Enable Developer Mode</strong>.
        </>,
        <>
          Fully quit and relaunch Claude Desktop. A new{" "}
          <strong>Developer</strong> menu appears in the menu bar.
        </>,
        <>
          Open <strong>Developer</strong> &gt;{" "}
          <strong>Configure Third-Party Inference</strong> &gt;{" "}
          <strong>Gateway</strong>.
        </>,
        <>
          Enter the gateway URL and virtual API key, save, then restart Claude
          Desktop.
        </>,
      ],
      language: "text",
      code: `Gateway URL: ${base}
API Key: ${args.apiKey}`,
    },
    {
      id: "codex",
      title: "Codex CLI",
      description:
        "Use OpenAI-compatible environment variables when running Codex against the gateway.",
      icon: "codex",
      language: "bash",
      code: `export OPENAI_API_KEY='${args.apiKey}'
# If Codex has an existing login it can impact functionality. Better if it's logged out.
# If you don't want to override your Codex configuration, you can set up a new dedicated configuration file.
export CODEX_HOME=/tmp/codex-gateway-home && mkdir -p $CODEX_HOME # optional
codex login --with-api-key <<<"$OPENAI_API_KEY"

codex --model "${args.model}" \\
  -c 'model_provider="gateway"' \\
  -c 'model_providers.gateway.name="Local gateway"' \\
  -c 'model_providers.gateway.base_url="${v1}"'`,
    },
    {
      id: "opencode",
      title: "OpenCode",
      description:
        "Configure OpenCode with an OpenAI-compatible gateway provider.",
      icon: "opencode",
      steps: [
        <>
          Create this <code>opencode.json</code> in your project root.
        </>,
        <>
          Run <code>opencode</code> from the same directory.
        </>,
      ],
      language: "bash",
      code: `
cat > opencode.json <<'EOF'
{
  "$schema": "https://opencode.ai/config.json",
  "model": "agentgateway/${args.model}",
  "provider": {
    "agentgateway": {
      "npm": "@ai-sdk/openai-compatible",
      "name": "Agentgateway",
      "options": {
        "baseURL": "${v1}",
        "apiKey": "{env:AGENTGATEWAY_API_KEY}"
      },
      "models": {
        "${args.model}": {
          "name": "${args.model}"
        }
      }
    }
  }
}
EOF

export AGENTGATEWAY_API_KEY='${args.apiKey}'  # Alternatively, type /connect to enter your API key.
opencode`,
    },
    {
      id: "cursor",
      title: "Cursor",
      description:
        "Use Cursor's OpenAI base URL override with a gateway model.",
      icon: "cursor",
      steps: [
        <>
          Open <strong>Cursor Settings</strong> &gt; <strong>Models</strong>.
        </>,
        <>
          Enable <strong>Override OpenAI Base URL</strong> and set it to{" "}
          <code>{base}</code>.
        </>,
        <>
          Add <code>{args.model}</code> as a custom model, then test from{" "}
          <strong>Ask</strong> or <strong>Plan</strong> mode.
        </>,
      ],
      language: "text",
      code: `Override OpenAI Base URL: ${base}
OpenAI API Key: ${args.apiKey}
Custom model: ${args.model}`,
    },
    {
      id: "github-copilot",
      title: "GitHub Copilot",
      description:
        "Configure VS Code Copilot Business or Enterprise to use the gateway proxy.",
      icon: "copilot",
      steps: [
        <>
          Open <strong>VS Code Settings</strong> and search for{" "}
          <code>github.copilot</code>.
        </>,
        <>
          Edit <code>settings.json</code> and set the advanced proxy URL.
        </>,
        <>Reload VS Code and test Copilot suggestions or chat.</>,
      ],
      language: "json",
      code: `{
  "github.copilot.advanced": {
    "debug.overrideProxyUrl": "${v1}"
  }
}`,
    },
    {
      id: "windsurf",
      title: "Windsurf",
      description:
        "Route Windsurf traffic through the gateway HTTP proxy setting.",
      icon: "windsurf",
      steps: [
        <>
          Open <strong>Windsurf Settings</strong>.
        </>,
        <>
          Search for <strong>Http: Proxy</strong>.
        </>,
        <>
          Set the proxy URL to <code>{base}</code> and save.
        </>,
      ],
      language: "text",
      code: `Http: Proxy: ${base}`,
    },
    {
      id: "openai-js",
      title: "OpenAI JavaScript SDK",
      description:
        "Use the gateway as an OpenAI-compatible chat completions endpoint.",
      icon: "codex",
      provider: "openai",
      language: "ts",
      code: `import OpenAI from "openai";

const client = new OpenAI({
  apiKey: "${args.apiKey}",
  baseURL: "${v1}",
});

const response = await client.chat.completions.create({
  model: "${args.model}",
  messages: [{ role: "user", content: "Hello from agentgateway" }],
});

console.log(response.choices[0]?.message?.content);`,
    },
    {
      id: "openai-python",
      title: "OpenAI Python SDK",
      description: "Point the Python SDK at the gateway listener.",
      icon: "codex",
      provider: "openai",
      language: "python",
      code: `from openai import OpenAI

client = OpenAI(
    api_key="${args.apiKey}",
    base_url="${v1}",
)

response = client.chat.completions.create(
    model="${args.model}",
    messages=[{"role": "user", "content": "Hello from agentgateway"}],
)

print(response.choices[0].message.content)`,
    },
  ];
}

function ClientSetupIcon(props: { recipe: ClientRecipe; compact?: boolean }) {
  const className = props.compact
    ? "client-svg-icon compact"
    : "client-svg-icon";
  if (props.recipe.provider) {
    return (
      <span className={className}>
        <ProviderIcon provider={props.recipe.provider} />
      </span>
    );
  }
  if (props.recipe.icon === "codex") {
    return (
      <span className={className}>
        <img src={codexIcon} alt="" aria-hidden="true" />
      </span>
    );
  }
  if (props.recipe.icon === "claude") {
    return (
      <span className={className}>
        <img src={claudeIcon} alt="" aria-hidden="true" />
      </span>
    );
  }
  if (props.recipe.icon === "curl") {
    return (
      <span className={className}>
        <img src={curlIcon} alt="" aria-hidden="true" />
      </span>
    );
  }
  if (props.recipe.icon === "cursor") {
    return (
      <span className={className}>
        <img src={cursorIcon} alt="" aria-hidden="true" />
      </span>
    );
  }
  if (props.recipe.icon === "copilot") {
    return (
      <span className={className}>
        <img src={githubCopilotIcon} alt="" aria-hidden="true" />
      </span>
    );
  }
  if (props.recipe.icon === "opencode") {
    return (
      <span className={className}>
        <img src={opencodeIcon} alt="" aria-hidden="true" />
      </span>
    );
  }
  if (props.recipe.icon === "windsurf") {
    return (
      <span className={className}>
        <img src={windsurfIcon} alt="" aria-hidden="true" />
      </span>
    );
  }
  return (
    <span className={className}>
      <Terminal size={20} />
    </span>
  );
}

function HighlightedCode(props: { code: string; language: string }) {
  return (
    <pre className={`client-code-block code-lang-${props.language}`}>
      <code>{highlightCode(props.code, props.language)}</code>
    </pre>
  );
}

function highlightCode(code: string, language: string) {
  return code.split("\n").flatMap((line, lineIndex, lines) => [
    <span className="code-line" key={`line-${lineIndex}`}>
      {highlightLine(line, language, lineIndex)}
    </span>,
    lineIndex < lines.length - 1 ? "\n" : null,
  ]);
}

function highlightLine(
  line: string,
  language: string,
  lineIndex: number,
): ReactNode {
  if (language === "bash")
    return highlightWithRules(line, lineIndex, bashRules);
  if (language === "json")
    return highlightWithRules(line, lineIndex, jsonRules);
  if (language === "python")
    return highlightWithRules(line, lineIndex, pythonRules);
  if (language === "text")
    return highlightWithRules(line, lineIndex, textRules);
  return highlightWithRules(line, lineIndex, tsRules);
}

type CodeRule = {
  className: string;
  pattern: RegExp;
};

const tsRules: CodeRule[] = [
  { className: "code-comment", pattern: /\/\/.*/y },
  {
    className: "code-string",
    pattern: /"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|`(?:\\.|[^`\\])*`/y,
  },
  {
    className: "code-keyword",
    pattern: /\b(?:await|const|from|import|new)\b/y,
  },
  { className: "code-number", pattern: /\b\d+(?:\.\d+)?\b/y },
  {
    className: "code-property",
    pattern:
      /\b(?:apiKey|baseURL|client|content|messages|model|response|role)\b(?=\s*:|\.)/y,
  },
  { className: "code-function", pattern: /\b[A-Za-z_][\w]*(?=\()/y },
];

const pythonRules: CodeRule[] = [
  { className: "code-comment", pattern: /#.*/y },
  { className: "code-string", pattern: /"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'/y },
  {
    className: "code-keyword",
    pattern: /\b(?:from|import|client|response)\b/y,
  },
  { className: "code-number", pattern: /\b\d+(?:\.\d+)?\b/y },
  {
    className: "code-property",
    pattern: /\b(?:api_key|base_url|messages|model)\b(?=\s*=)/y,
  },
  { className: "code-function", pattern: /\b[A-Za-z_][\w]*(?=\()/y },
];

const bashRules: CodeRule[] = [
  { className: "code-comment", pattern: /#.*/y },
  { className: "code-string", pattern: /"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'/y },
  { className: "code-keyword", pattern: /\b(?:curl|export|claude|codex)\b/y },
  { className: "code-flag", pattern: /--?[A-Za-z][\w-]*/y },
  { className: "code-number", pattern: /\b\d+(?:\.\d+)?\b/y },
];

const jsonRules: CodeRule[] = [
  { className: "code-string", pattern: /"(?:\\.|[^"\\])*"/y },
  { className: "code-keyword", pattern: /\b(?:true|false|null)\b/y },
  { className: "code-number", pattern: /-?\b\d+(?:\.\d+)?\b/y },
];

const textRules: CodeRule[] = [
  { className: "code-property", pattern: /^[^:]+(?=:)/y },
  { className: "code-string", pattern: /https?:\/\/\S+/y },
  { className: "code-string", pattern: /\bagw_sk_[A-Za-z0-9_.-]*/y },
];

function highlightWithRules(
  line: string,
  lineIndex: number,
  rules: CodeRule[],
) {
  const nodes: ReactNode[] = [];
  let position = 0;
  while (position < line.length) {
    const match = matchRule(line, position, rules);
    if (!match) {
      nodes.push(line[position]);
      position += 1;
      continue;
    }
    nodes.push(
      <span className={match.rule.className} key={`${lineIndex}-${position}`}>
        {match.text}
      </span>,
    );
    position += match.text.length;
  }
  return nodes;
}

function matchRule(line: string, position: number, rules: CodeRule[]) {
  for (const rule of rules) {
    rule.pattern.lastIndex = position;
    const match = rule.pattern.exec(line);
    if (match?.index === position && match[0]) {
      return { rule, text: match[0] };
    }
  }
  return null;
}
