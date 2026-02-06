"use client";

import React, { useCallback, useEffect, useRef, useState } from "react";
import dynamic from "next/dynamic";
import { useTheme } from "next-themes";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { Play, RotateCcw } from "lucide-react";
import { toast } from "sonner";
import yaml from "js-yaml";
import { API_URL } from "@/lib/api";

// dynamic import OUTSIDE component to avoid remount on every render
const MonacoEditor = dynamic(() => import("@monaco-editor/react"), { ssr: false });

type TemplateKey = "empty" | "http" | "llm" | "mcp";

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
  llm: `llm:
  input: "Translate the following English text to French:\\n\\nHello world"
  model: gpt-4
  maxTokens: 256`,
  mcp: `mcp:
  envelope:
    id: msg-1
    from: service-a
    to: service-b
    type: event
    payload:
      event: user.created
      user:
        id: 42
        email: a@b.com`,
};

const EXAMPLES: { name: string; expr: string }[] = [
  { name: "Method + Status", expr: "request.method == 'GET' && response.code == 200" },
  { name: "MCP Payload", expr: "mcp.tool.name == 'get_weather'" },
  { name: "LLM Input", expr: "llm.requestModel == 'gpt-4'" },
  { name: "JWT Claims", expr: "jwt.iss == 'agentgateway.dev' && jwt.sub == 'test-user'" },
  { name: "Source IP", expr: "source.address == '127.0.0.1'" },
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
  const [result, setResult] = useState<unknown | null>(null);

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
        toast.error("Evaluation failed: " + res.status + " " + text);
        setResult({ error: text, status: res.status });
        return;
      }

      const json = await res.json();
      setResult(json);
      toast.success("Evaluation complete");
    } catch (err: any) {
      const message = err?.message ? String(err.message) : String(err);
      toast.error("Request error: " + message);
      setResult({ error: message });
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
    setResult(null);
    toast("Reset to example template");
  };

  const handleCopyResult = async () => {
    try {
      const text = result ? JSON.stringify(result, null, 2) : "";
      await navigator.clipboard.writeText(text);
      toast.success("Result copied to clipboard");
    } catch (e) {
      toast.error("Failed to copy result");
    }
  };

  const handleEditorMount = useCallback((editor: any, monaco: any) => {
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter, () => {
      evaluateRef.current();
    });
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
        <section className="col-span-6">
          <label className="block text-sm font-medium mb-2">Expression</label>
          <Card>
            <CardContent className="p-4">
              <MonacoEditor
                height="150px"
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
              <option value="llm">LLM</option>
              <option value="mcp">MCP</option>
            </select>
          </div>
          <Card>
            <CardContent className="p-4">
              <MonacoEditor
                height="300px"
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

      <div className="mt-4">
        <label className="block text-sm font-medium mb-2">Result</label>
        <Card>
          <CardContent className="p-4">
            <div className="flex items-center justify-between mb-3">
              <div className="flex items-center gap-2">
                <Badge>CEL</Badge>
                <span className="text-sm text-muted-foreground">/cel POST</span>
              </div>
              <div className="flex items-center gap-2">
                <div className="text-sm text-gray-500">{loading ? "Evaluating…" : "Idle"}</div>
                <Button variant="secondary" onClick={handleCopyResult} className="text-sm">
                  Copy Result
                </Button>
              </div>
            </div>

            <MonacoEditor
              height="260px"
              defaultLanguage="json"
              language="json"
              theme={editorTheme}
              value={result ? JSON.stringify(result, null, 2) : "(no result yet)"}
              loading={<Skeleton className="h-48" />}
              options={{
                minimap: { enabled: false },
                lineNumbers: "off",
                readOnly: true,
                wordWrap: "on",
              }}
            />
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
