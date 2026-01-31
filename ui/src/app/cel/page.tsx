"use client"

import React, { useEffect, useState } from "react"
import dynamic from "next/dynamic"
import { Button, Card, Badge } from "@/components/ui"
import { Play, RotateCcw } from "lucide-react"
import { toast } from "sonner"

type TemplateKey = "empty" | "http" | "llm" | "mcp"

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
}

const EXAMPLES: string[] = [
  // simple boolean check
  "request.method == 'GET' && response.status == 200",
  // access nested fields
  "envelope.payload.user.id == 42",
  // string operations
  "input.startsWith('Translate')",
  // array/length example
  "response.body.items != null ? response.body.items.length : 0",
]

export default function CELPlayground(): JSX.Element {
  const [template, setTemplate] = useState<TemplateKey>("http")
  const [expression, setExpression] = useState<string>(EXAMPLES[0])
  const [inputData, setInputData] = useState<string>(() =>
    JSON.stringify(TEMPLATES["http"], null, 2)
  )
  const [loading, setLoading] = useState<boolean>(false)
  const [result, setResult] = useState<any>(null)
  const [theme, setTheme] = useState<"vs-light" | "vs-dark">("vs-light")

  // dynamic import of Monaco Editor to avoid SSR issues
  const MonacoEditor = dynamic(() => import("@monaco-editor/react"), { ssr: false })

  useEffect(() => {
    // when template changes, load example JSON into editor
    setInputData(JSON.stringify(TEMPLATES[template], null, 2))
  }, [template])

  useEffect(() => {
    // detect color scheme preference for editor theme
    if (typeof window === "undefined" || !window.matchMedia) return
    const mq = window.matchMedia("(prefers-color-scheme: dark)")
    const update = () => setTheme(mq.matches ? "vs-dark" : "vs-light")
    update()
    try {
      mq.addEventListener("change", update)
    } catch (e) {
      // Safari fallback
      // @ts-ignore
      mq.addListener?.(update)
    }
    return () => {
      try {
        mq.removeEventListener("change", update)
      } catch (e) {
        // @ts-ignore
        mq.removeListener?.(update)
      }
    }
  }, [])

  const handleEvaluate = async () => {
    let parsed: any = {}
    if (inputData.trim().length > 0) {
      try {
        parsed = JSON.parse(inputData)
      } catch (err) {
        toast.error("Input data is not valid JSON")
        return
      }
    }

    setLoading(true)
    setResult(null)

    try {
      const res = await fetch("/cel", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ expression, data: parsed }),
      })

      if (!res.ok) {
        const text = await res.text()
        toast.error("Evaluation failed: " + res.status + " " + text)
        setResult({ error: text, status: res.status })
        return
      }

      const json = await res.json()
      setResult(json)
      toast.success("Evaluation complete")
    } catch (err: any) {
      toast.error("Request error: " + err?.message ?? String(err))
      setResult({ error: String(err) })
    } finally {
      setLoading(false)
    }
  }

  const handleReset = () => {
    setExpression(EXAMPLES[0])
    setTemplate("http")
    setInputData(JSON.stringify(TEMPLATES["http"], null, 2))
    setResult(null)
    toast("Reset to example template")
  }

  const applyExample = (ex: string) => {
    setExpression(ex)
  }

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
        </div>
      </div>

      <div className="grid grid-cols-12 gap-4">
        <section className="col-span-6">
          <label className="block text-sm font-medium mb-2">Expression</label>
          <Card className="p-4">
            {/* Monaco Editor for CEL expression (use javascript mode) */}
            <MonacoEditor
              height="150px"
              defaultLanguage="javascript"
              language="javascript"
              theme={theme}
              value={expression}
              onChange={(v) => setExpression(v ?? "")}
              options={{ minimap: { enabled: false }, lineNumbers: "on" }}
            />

            <div className="flex gap-2 mt-3 flex-wrap">
              {EXAMPLES.map((ex, idx) => (
                <button
                  key={idx}
                  onClick={() => applyExample(ex)}
                  className="text-xs px-2 py-1 rounded bg-slate-100 hover:bg-slate-200"
                  title={ex}
                >
                  {ex.length > 30 ? ex.slice(0, 30) + "…" : ex}
                </button>
              ))}
            </div>
          </Card>
        </section>

        <section className="col-span-6">
          <label className="block text-sm font-medium mb-2">Input Data (JSON)</label>
          <Card className="p-4">
            {/* Monaco Editor for JSON input */}
            <MonacoEditor
              height="300px"
              defaultLanguage="json"
              language="json"
              theme={theme}
              value={inputData}
              onChange={(v) => setInputData(v ?? "")}
              options={{ minimap: { enabled: false }, lineNumbers: "on" }}
            />
          </Card>
        </section>
      </div>

      <div className="mt-4">
        <label className="block text-sm font-medium mb-2">Result</label>
        <Card className="p-4">
          <div className="flex items-center justify-between mb-3">
            <div className="flex items-center gap-2">
              <Badge>CEL</Badge>
              <span className="text-sm text-muted-foreground">/cel POST</span>
            </div>
            <div className="text-sm text-gray-500">{loading ? "Evaluating…" : "Idle"}</div>
          </div>

          <pre className="rounded bg-slate-50 p-3 text-sm overflow-auto max-h-72">
            {result ? JSON.stringify(result, null, 2) : "(no result yet)"}
          </pre>
        </Card>
      </div>
    </div>
  )
}
