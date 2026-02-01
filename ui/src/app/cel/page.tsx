"use client";

import React, { useEffect, useState } from "react";
import dynamic from "next/dynamic";
import { useTheme } from "next-themes";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { Play, RotateCcw } from "lucide-react";
import { toast } from "sonner";
import { API_URL } from "@/lib/api";

type TemplateKey = "empty" | "http" | "llm" | "mcp";

const TEMPLATES: Record<TemplateKey, unknown> = {
  empty: {},
  http: {
    request: {
      method: "GET",
      url: "https://api.example.com/items/123",
      headers: {
        "content-type": "application/json",
        "x-trace-id": "abc-123",
      },
      body: null,
    },
    response: {
      status: 200,
      headers: { "content-type": "application/json" },
      body: { id: 123, name: "Example" },
    },
  },
  llm: {
    input: "Translate the following English text to French:\n\nHello world",
    model: "gpt-4",
    maxTokens: 256,
  },
  mcp: {
    envelope: {
      id: "msg-1",
      from: "service-a",
      to: "service-b",
      type: "event",
      payload: { event: "user.created", user: { id: 42, email: "a@b.com" } },
    },
  },
};

const EXAMPLES: string[] = [
  // simple boolean check
  "request.method == 'GET' && response.status == 200",
  // access nested fields
  "envelope.payload.user.id == 42",
  // string operations
  "input.startsWith('Translate')",
  // array/length example
  "response.body.items != null ? response.body.items.length : 0",
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
  const [expression, setExpression] = useState<string>(EXAMPLES[0]);
  const [inputData, setInputData] = useState<string>(() =>
    JSON.stringify(TEMPLATES["http"], null, 2)
  );
  const [loading, setLoading] = useState<boolean>(false);
  const [result, setResult] = useState<unknown | null>(null);

  // dynamic import of Monaco Editor to avoid SSR issues
  const MonacoEditor = dynamic(() => import("@monaco-editor/react"), { ssr: false });

  useEffect(() => {
    // when template changes, load example JSON into editor
    setInputData(JSON.stringify(TEMPLATES[template], null, 2));
  }, [template]);

  const handleEvaluate = async () => {
    let parsed: any = {};
    if (inputData.trim().length > 0) {
      try {
        parsed = JSON.parse(inputData);
      } catch (err) {
        toast.error("Input data is not valid JSON");
        return;
      }
    }

    setLoading(true);
    setResult(null);

    try {
      const res = await fetch(`${API_URL}/cel`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ expression, data: parsed }),
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
  };

  const handleReset = () => {
    setExpression(EXAMPLES[0]);
    setTemplate("http");
    setInputData(JSON.stringify(TEMPLATES["http"], null, 2));
    setResult(null);
    toast("Reset to example template");
  };

  const handleFormat = () => {
    try {
      const parsed = JSON.parse(inputData);
      setInputData(JSON.stringify(parsed, null, 2));
      toast.success("Formatted JSON");
    } catch (e) {
      toast.error("Cannot format: invalid JSON");
    }
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

  return (
    <div className="p-6">
      <div className="flex items-center justify-between gap-4 mb-4">
        <div className="flex items-center gap-3">
          <label className="text-sm font-medium">Template</label>
          <select
            value={template}
            onChange={(e) => setTemplate(e.target.value as TemplateKey)}
            className="rounded-md border px-3 py-2 text-sm bg-white"
          >
            <option value="empty">Empty</option>
            <option value="http">HTTP</option>
            <option value="llm">LLM</option>
            <option value="mcp">MCP</option>
          </select>
        </div>

        <div className="flex items-center gap-2">
          <Button onClick={handleEvaluate} disabled={loading} className="flex items-center gap-2">
            <Play size={16} />
            Evaluate
          </Button>
          <Button variant="secondary" onClick={handleReset} className="flex items-center gap-2">
            <RotateCcw size={16} />
            Reset
          </Button>
          <Button variant="secondary" onClick={handleFormat} className="flex items-center gap-2">
            Format JSON
          </Button>
        </div>
      </div>

      <div className="grid grid-cols-12 gap-4">
        <section className="col-span-6">
          <label className="block text-sm font-medium mb-2">Expression</label>
          <Card>
            <CardContent className="p-4">
              {/* Monaco Editor for CEL expression (use javascript mode) */}
              <MonacoEditor
                height="150px"
                defaultLanguage="javascript"
                language="javascript"
                theme={editorTheme}
                value={expression}
                onChange={(v) => setExpression(v ?? "")}
                loading={<Skeleton className="h-32" />}
                onMount={(editor: any, monaco: any) => {
                  try {
                    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter, () => {
                      handleEvaluate();
                    });
                  } catch (e) {
                    // ignore
                  }
                }}
                options={{ minimap: { enabled: false }, lineNumbers: "on", wordWrap: "on" }}
              />

              <div className="flex gap-2 mt-3 flex-wrap">
                {EXAMPLES.map((ex, idx) => (
                  <button
                    key={idx}
                    onClick={() => setExpression(ex)}
                    className="text-xs px-2 py-1 rounded bg-slate-100 hover:bg-slate-200"
                    title={ex}
                  >
                    {ex.length > 30 ? ex.slice(0, 30) + "…" : ex}
                  </button>
                ))}
              </div>
            </CardContent>
          </Card>
        </section>

        <section className="col-span-6">
          <label className="block text-sm font-medium mb-2">Input Data (JSON)</label>
          <Card>
            <CardContent className="p-4">
              {/* Monaco Editor for JSON input */}
              <MonacoEditor
                height="300px"
                defaultLanguage="json"
                language="json"
                theme={editorTheme}
                value={inputData}
                onChange={(v) => setInputData(v ?? "")}
                loading={<Skeleton className="h-48" />}
                onMount={(editor: any, monaco: any) => {
                  try {
                    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter, () => {
                      handleEvaluate();
                    });
                  } catch (e) {
                    // ignore
                  }
                }}
                options={{ minimap: { enabled: false }, lineNumbers: "on", wordWrap: "on" }}
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
                lineNumbers: "on",
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
