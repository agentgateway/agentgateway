import "../monacoWorkers";
import Editor from "@monaco-editor/react";
import { useCallback, useEffect, useRef, useState } from "react";
import type * as Monaco from "monaco-editor";
import { ExternalLink, Play } from "lucide-react";
import yaml from "js-yaml";
import { evaluateCel } from "../api/celApi";
import {
  celEditorOptions,
  celLanguage,
  configureCelMonaco,
} from "../celMonaco";
import {
  FieldGroup,
  PageHeader,
  Panel,
  StatusBanner,
  YamlBlock,
} from "../components/Primitives";
import { configureConfigYamlMonaco } from "../configMonaco";
import { pendingCelExpression } from "../policies/AuthorizationPolicyEditor";

const sampleContext = {
  request: {
    method: "GET",
    uri: "http://example.com/api/test?k=v",
    path: "/api/test",
    pathAndQuery: "/api/test?k=v",
    host: "example.com",
    scheme: "http",
    version: "HTTP/1.1",
    headers: {
      foo: "bar",
      "user-agent": "example",
      accept: "application/json",
    },
    body: "eyJtb2RlbCI6ICJmYXN0In0=",
    startTime: "2000-01-01T12:00:00Z",
    endTime: "2000-01-01T12:00:01.12345678Z",
  },
  response: {
    code: 200,
    headers: {
      "content-type": "application/json",
    },
    body: "eyJvayI6IHRydWV9",
  },
  proxy: {
    requestProcessingDuration: "0.012s",
    upstreamDuration: "0.675s",
    responseProcessingDuration: "0.006s",
  },
  env: {
    podName: "pod-1",
    namespace: "ns-1",
    gateway: "gw-1",
  },
  source: {
    address: "127.0.0.1",
    port: 12345,
    rawAddress: "127.0.0.1",
    rawPort: 12345,
    unverifiedWorkload: {
      name: "pod-1",
      namespace: "ns-1",
      serviceAccount: "sa-1",
    },
    identity: null,
    subjectAltNames: ["san"],
    issuer: "",
    subject: "",
    subjectCn: "cn",
    certificate: null,
  },
  jwt: {
    exp: 1900650294,
    sub: "test-user",
    iss: "agentgateway.dev",
  },
  apiKey: {
    key: "<redacted>",
    role: "admin",
  },
  basicAuth: {
    username: "alice",
  },
  llm: {
    streaming: false,
    requestModel: "gpt-4",
    responseModel: "gpt-4-turbo",
    provider: "fake-ai",
    inputTokens: 100,
    inputImageTokens: 60,
    inputTextTokens: 40,
    inputAudioTokens: 5,
    cachedInputTokens: 20,
    cacheCreationInputTokens: 10,
    outputTokens: 50,
    outputImageTokens: 30,
    outputTextTokens: 20,
    outputAudioTokens: 3,
    reasoningTokens: 30,
    totalTokens: 150,
    serviceTier: "default",
    countTokens: 10,
    completion: ["Hello"],
    params: {
      temperature: 0.7,
      top_p: 1.0,
      frequency_penalty: 0.0,
      presence_penalty: 0.0,
      seed: 42,
      max_tokens: 1024,
    },
  },
  llmRequest: {
    model: "provider/model",
  },
  mcp: {
    methodName: "tools/call",
    sessionId: "session-123",
    tool: {
      target: "my-mcp-server",
      name: "get_weather",
      arguments: {
        userId: "123",
      },
      result: {
        isError: false,
        structuredContent: {
          forecast: "sunny",
          status: "ok",
        },
        content: [],
      },
    },
  },
  backend: {
    name: "my-backend",
    type: "service",
    protocol: "http",
  },
  extauthz: {},
  extproc: {},
  metadata: {},
};

export function CelPage() {
  const [expression, setExpression] = useState(
    () =>
      pendingCelExpression() ??
      'request.path.startsWith("/v1/") && metadata.tier == "prod"',
  );
  const [context, setContext] = useState(yaml.dump(sampleContext));
  const [result, setResult] = useState<unknown>(null);
  const [hasResult, setHasResult] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const runRef = useRef<() => void>(() => {});

  const run = useCallback(async () => {
    if (loading) return;
    setLoading(true);
    setError(null);
    try {
      const data = context.trim() ? yaml.load(context) : {};
      const response = await evaluateCel(expression, data);
      if (response.error) setError(response.error);
      setResult(response.result);
      setHasResult(true);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Evaluation failed");
    } finally {
      setLoading(false);
    }
  }, [context, expression, loading]);

  useEffect(() => {
    runRef.current = () => {
      void run();
    };
  }, [run]);

  function registerRunShortcut(
    editor: Monaco.editor.IStandaloneCodeEditor,
    monaco: typeof Monaco,
  ) {
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter, () =>
      runRef.current(),
    );
  }

  return (
    <div className="page-stack">
      <PageHeader
        title="CEL Playground"
        description="Evaluate policy expressions against sample or custom request context using the gateway CEL endpoint."
        actions={
          <>
            <a
              className="button"
              href="https://agentgateway.dev/docs/standalone/latest/reference/cel/"
              rel="noreferrer"
              target="_blank"
            >
              <ExternalLink size={16} /> CEL reference
            </a>
            <button
              className="button primary"
              type="button"
              disabled={loading}
              onClick={run}
            >
              <Play size={16} />
              Evaluate
            </button>
          </>
        }
      />
      {error ? (
        <StatusBanner state="bad" title="CEL error">
          {error}
        </StatusBanner>
      ) : null}
      <section className="two-column wide-left">
        <Panel>
          <FieldGroup label="Expression">
            <div className="editor-wrap short">
              <Editor
                beforeMount={configureCelMonaco}
                language={celLanguage}
                theme={
                  document.documentElement.dataset.theme === "dark"
                    ? "vs-dark"
                    : "light"
                }
                value={expression}
                onChange={(value) => setExpression(value ?? "")}
                onMount={registerRunShortcut}
                options={{
                  ...celEditorOptions,
                  lineNumbers: "off",
                }}
              />
            </div>
          </FieldGroup>
          <FieldGroup label="Request context YAML">
            <div className="editor-wrap">
              <Editor
                beforeMount={configureConfigYamlMonaco}
                language="yaml"
                path="cel-request-context.yaml"
                theme={
                  document.documentElement.dataset.theme === "dark"
                    ? "vs-dark"
                    : "light"
                }
                value={context}
                onChange={(value) => setContext(value ?? "")}
                onMount={registerRunShortcut}
                options={{
                  minimap: { enabled: false },
                  fontSize: 13,
                  renderLineHighlight: "none",
                  scrollBeyondLastLine: false,
                  copyWithSyntaxHighlighting: false,
                }}
              />
            </div>
          </FieldGroup>
        </Panel>
        <Panel>
          <div className="section-heading">
            <h3>Result</h3>
            <p>YAML value returned by CEL evaluation.</p>
          </div>
          {hasResult ? <YamlBlock value={result ?? null} /> : null}
        </Panel>
      </section>
    </div>
  );
}

declare global {
  interface Window {
    monaco?: typeof import("monaco-editor");
  }
}
