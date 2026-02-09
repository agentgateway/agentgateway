"use client";

import React, { useCallback, useEffect, useRef, useState } from "react";
import dynamic from "next/dynamic";
import { useTheme } from "next-themes";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Play, RotateCcw } from "lucide-react";
import { toast } from "sonner";
import yaml from "js-yaml";
import { API_URL } from "@/lib/api";

// dynamic import OUTSIDE component to avoid remount on every render
const MonacoEditor = dynamic(() => import("@monaco-editor/react"), { ssr: false });

type TemplateKey = "empty" | "http";

const TEMPLATES: Record<TemplateKey, string> = {
  empty: "",
  http: `apiKey:
  key: <redacted>
  role: admin
backend:
  name: my-backend
  protocol: http
  type: service
basicAuth:
  username: alice
extauthz: {}
extproc: {}
jwt:
  exp: 1900650294
  iss: agentgateway.dev
  sub: test-user
llm:
  completion:
  - Hello
  countTokens: 10
  inputTokens: 100
  outputTokens: 50
  params:
    frequency_penalty: 0.0
    max_tokens: 1024
    presence_penalty: 0.0
    seed: 42
    temperature: 0.7
    top_p: 1.0
  provider: fake-ai
  requestModel: gpt-4
  responseModel: gpt-4-turbo
  streaming: false
  totalTokens: 150
mcp:
  tool:
    name: get_weather
    target: my-mcp-server
request:
  body: eyJtb2RlbCI6ICJmYXN0In0=
  endTime: 2000-01-01T12:00:01Z
  headers:
    accept: application/json
    foo: bar
    user-agent: example
  host: example.com
  method: GET
  path: /api/test
  scheme: http
  startTime: 2000-01-01T12:00:00Z
  uri: http://example.com/api/test
  version: HTTP/1.1
response:
  body: eyJvayI6IHRydWV9
  code: 200
  headers:
    content-type: application/json
source:
  address: 127.0.0.1
  identity: null
  issuer: ''
  port: 12345
  subject: ''
  subjectAltNames: []
  subjectCn: cn
`,
};

const EXAMPLES: { name: string; expr: string }[] = [
  {
    name: "HTTP",
    expr: "request.method == 'GET' && response.code == 200 && request.path.startsWith('/api/')",
  },
  { name: "MCP Payload", expr: "mcp.tool.name == 'get_weather'" },
  { name: "Body Based Routing", expr: "json(request.body).model" },
  { name: "JWT Claims", expr: "jwt.iss == 'agentgateway.dev' && jwt.sub == 'test-user'" },
  { name: "Source IP", expr: "cidr('127.0.0.1/8').containsIP(source.address)" },
];

export default function CELPlayground(): React.JSX.Element {
  const { theme, systemTheme } = useTheme();
  const editorTheme =
    theme === "system"
      ? systemTheme === "dark"
        ? "vs-dark"
        : "vs-light"
      : theme === "dark"
        ? "vs-dark"
        : "vs-light";

  const [template, setTemplate] = useState<TemplateKey>("http");
  const [expression, setExpression] = useState<string>(EXAMPLES[0].expr);
  const [inputData, setInputData] = useState<string>(TEMPLATES["http"]);
  const [loading, setLoading] = useState<boolean>(false);
  const [resultValue, setResultValue] = useState<unknown | null>();
  const [resultError, setResultError] = useState<string | null>(null);
  const hasResult = resultValue !== undefined || resultError !== null;

  useEffect(() => {
    setInputData(TEMPLATES[template]);
  }, [template]);

  const handleEvaluate = useCallback(async () => {
    let parsed: unknown = undefined;
    if (inputData.trim().length > 0) {
      try {
        parsed = yaml.load(inputData);
      } catch (err) {
        toast.error("Input data is not valid YAML");
        return;
      }
    }

    setLoading(true);

    try {
      const res = await fetch(`${API_URL}/cel`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          expression,
          data: parsed,
        }),
      });

      if (!res.ok) {
        const text = await res.text();
        setResultValue(null);
        setResultError("Evaluation failed: " + res.status + " " + text);
        return;
      }

      const json = await res.json();
      if (json.error) {
        setResultValue(null);
        setResultError(json.error);
      } else {
        setResultError(null);
        setResultValue(json.result);
      }
    } catch (err: any) {
      const message = err?.message ? String(err.message) : String(err);
      setResultValue(null);
      setResultError("Request error: " + message);
    } finally {
      setLoading(false);
    }
  }, [expression, inputData]);

  // ref to always have latest handleEvaluate for Monaco keybinding
  const evaluateRef = useRef(handleEvaluate);
  useEffect(() => {
    evaluateRef.current = handleEvaluate;
  }, [handleEvaluate]);

  const handleReset = () => {
    setExpression(EXAMPLES[0].expr);
    setTemplate("http");
    setInputData(TEMPLATES["http"]);
    setResultValue(null);
    setResultError(null);
    toast("Reset to example template");
  };

  const handleCopyResult = async () => {
    try {
      const text = resultError
        ? resultError
        : resultValue !== null
          ? JSON.stringify(resultValue, null, 2)
          : "";
      await navigator.clipboard.writeText(text);
      toast.success("Copied to clipboard");
    } catch (e) {
      toast.error("Failed to copy result");
    }
  };

  const handleEditorMount = useCallback((editor: any, monaco: any) => {
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter, () => {
      evaluateRef.current();
    });
    // Mark container so Vimium recognizes the editor as a text input
    const domNode = editor.getDomNode();
    if (domNode) {
      domNode.setAttribute("role", "textbox");
      domNode.setAttribute("aria-multiline", "true");
    }
  }, []);

  return (
    <div className="p-6">
      <div className="flex items-center justify-end gap-2 mb-4">
        <Button onClick={handleEvaluate} disabled={loading} className="flex items-center gap-2">
          <Play size={16} />
          Evaluate
        </Button>
        <Button variant="secondary" onClick={handleReset} className="flex items-center gap-2">
          <RotateCcw size={16} />
          Reset
        </Button>
      </div>

      <div className="grid grid-cols-12 gap-4">
        {/* Left column: Expression + Result */}
        <div className="col-span-6 flex flex-col gap-4">
          <section>
            <label className="block text-sm font-medium mb-2">Expression</label>
            <Card>
              <CardContent className="p-4">
                <MonacoEditor
                  height="250px"
                  defaultLanguage="javascript"
                  language="javascript"
                  theme={editorTheme}
                  value={expression}
                  onChange={(v) => setExpression(v ?? "")}
                  loading={<Skeleton className="h-32" />}
                  onMount={handleEditorMount}
                  options={{ minimap: { enabled: false }, lineNumbers: "off", wordWrap: "on" }}
                />

                <div className="flex gap-2 mt-3 flex-wrap">
                  {EXAMPLES.map((ex, idx) => (
                    <button
                      type="button"
                      key={idx}
                      onClick={() => setExpression(ex.expr)}
                      className="text-xs px-2 py-1 rounded bg-slate-100 hover:bg-slate-200 dark:bg-slate-800 dark:hover:bg-slate-700"
                      title={ex.expr}
                    >
                      {ex.name}
                    </button>
                  ))}
                </div>
              </CardContent>
            </Card>
          </section>

          <section>
            <div className="flex items-center justify-between mb-2">
              <label className="text-sm font-medium">Result</label>
              {hasResult && (
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={handleCopyResult}
                  className="text-xs h-6 px-2"
                >
                  Copy
                </Button>
              )}
            </div>
            <Card>
              <CardContent className="p-4">
                {resultError ? (
                  <div className="rounded-md bg-red-50 dark:bg-red-950 border border-red-200 dark:border-red-800 p-4 h-[250px] overflow-auto">
                    <pre className="text-sm text-red-700 dark:text-red-300 whitespace-pre-wrap font-mono">
                      {resultError}
                    </pre>
                  </div>
                ) : resultValue !== null ? (
                  <MonacoEditor
                    height="250px"
                    defaultLanguage="json"
                    language="json"
                    theme={editorTheme}
                    value={JSON.stringify(resultValue, null, 2)}
                    loading={<Skeleton className="h-[250px]" />}
                    options={{
                      minimap: { enabled: false },
                      lineNumbers: "off",
                      readOnly: true,
                      wordWrap: "on",
                    }}
                  />
                ) : (
                  <div className="h-[250px] flex items-center justify-center text-sm text-muted-foreground">
                    Press Evaluate or Ctrl+Enter to run
                  </div>
                )}
              </CardContent>
            </Card>
          </section>
        </div>

        {/* Right column: Input Data */}
        <section className="col-span-6">
          <div className="flex items-center gap-3 mb-2">
            <label className="text-sm font-medium">Input Data (YAML)</label>
            <select
              value={template}
              onChange={(e) => setTemplate(e.target.value as TemplateKey)}
              className="rounded-md border px-2 py-1 text-xs bg-background"
            >
              <option value="empty">Empty</option>
              <option value="http">HTTP</option>
            </select>
          </div>
          <Card>
            <CardContent className="p-4">
              <MonacoEditor
                height="705px"
                defaultLanguage="yaml"
                language="yaml"
                theme={editorTheme}
                value={inputData}
                onChange={(v) => setInputData(v ?? "")}
                loading={<Skeleton className="h-48" />}
                onMount={handleEditorMount}
                options={{ minimap: { enabled: false }, lineNumbers: "off", wordWrap: "on" }}
              />
            </CardContent>
          </Card>
        </section>
      </div>
    </div>
  );
}
