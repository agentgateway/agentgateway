import yaml from "js-yaml";
import { useCallback, useEffect, useRef, useState } from "react";
import toast from "react-hot-toast";
import { API_BASE_URL } from "../../../api/client";
import { type Example, type ExpressionTemplate, type TemplateKey } from "./types";

export const TEMPLATES: Record<TemplateKey, string> = {
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

export const EXAMPLES: Example[] = [
  {
    name: "HTTP",
    expr: "request.method == 'GET' && response.code == 200 && request.path.startsWith('/api/')",
  },
  { name: "MCP Payload", expr: "mcp.tool.name == 'get_weather'" },
  { name: "Body Based Routing", expr: "json(request.body).model" },
  {
    name: "JWT Claims",
    expr: "jwt.iss == 'agentgateway.dev' && jwt.sub == 'test-user'",
  },
  { name: "Source IP", expr: "cidr('127.0.0.1/8').containsIP(source.address)" },
];

export const EXPRESSION_TEMPLATES: ExpressionTemplate[] = [
  {
    name: "Path Matching",
    description: "Check if request path matches pattern",
    expression: 'request.path.startsWith("/api/v1")',
    context: {
      request: { path: "/api/v1/users", method: "GET" },
    },
  },
  {
    name: "Header Validation",
    description: "Validate request headers",
    expression:
      'has(request.headers.authorization) && request.headers["content-type"] == "application/json"',
    context: {
      request: {
        headers: {
          authorization: "Bearer token",
          "content-type": "application/json",
        },
      },
    },
  },
  {
    name: "Role-Based Access",
    description: "Check user role",
    expression: 'user.role in ["admin", "moderator"] && user.active == true',
    context: {
      user: { role: "admin", active: true },
    },
  },
  {
    name: "Rate Limiting",
    description: "Rate limit by time window",
    expression: "request.count < 100 && request.window < duration('1h')",
    context: {
      request: { count: 50, window: "30m" },
    },
  },
  {
    name: "JWT Claims",
    description: "Validate JWT claims",
    expression: 'jwt.claims.sub == "user123" && jwt.claims.exp > now()',
    context: {
      jwt: {
        claims: { sub: "user123", exp: 1735689600 },
      },
    },
  },
];

export interface CELPlaygroundState {
  expression: string;
  inputData: string;
  template: TemplateKey;
  loading: boolean;
  resultValue: unknown | null;
  resultError: string | null;
  hasEvaluated: boolean;
  resultExpanded: boolean;
  editorTheme: string;
}

export interface CELPlaygroundHandlers {
  setExpression: (value: string) => void;
  setInputData: (value: string | undefined) => void;
  setTemplate: (value: TemplateKey) => void;
  setResultExpanded: (value: boolean) => void;
  handleEvaluate: () => Promise<void>;
  handleReset: () => void;
  loadTemplate: (template: ExpressionTemplate) => void;
  evaluateRef: React.MutableRefObject<() => Promise<void>>;
}

export const useCELPlayground = (): CELPlaygroundState & CELPlaygroundHandlers => {
  const [expression, setExpression] = useState<string>(EXAMPLES[0].expr);
  const [inputData, setInputDataState] = useState<string>(TEMPLATES["http"]);
  const [template, setTemplate] = useState<TemplateKey>("http");
  const [loading, setLoading] = useState<boolean>(false);
  const [resultValue, setResultValue] = useState<unknown | null>(null);
  const [resultError, setResultError] = useState<string | null>(null);
  const [hasEvaluated, setHasEvaluated] = useState<boolean>(false);
  const [resultExpanded, setResultExpanded] = useState<boolean>(false);

  const isDark = document.documentElement.getAttribute("data-theme") === "dark";
  const editorTheme = isDark ? "vs-dark" : "vs";

  useEffect(() => {
    setInputDataState(TEMPLATES[template]);
  }, [template]);

  const handleEvaluate = useCallback(async () => {
    let parsed: unknown = undefined;
    if (inputData.trim().length > 0) {
      try {
        parsed = yaml.load(inputData);
      } catch {
        toast.error("Input data is not valid YAML");
        return;
      }
    }

    setLoading(true);

    try {
      const res = await fetch(`${API_BASE_URL}/cel`, {
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
        setHasEvaluated(true);
        toast("Invalid CEL expression", { icon: "⚠️" });
        return;
      }

      const json = await res.json();
      if (json.error) {
        setResultValue(null);
        setResultError(json.error);
        setHasEvaluated(true);
        toast.error("Evaluation failed");
      } else if (json.result === false) {
        setResultValue(null);
        setResultError("Expression evaluated to false");
        setHasEvaluated(true);
        toast.error("Evaluation returned false");
      } else {
        setResultError(null);
        setResultValue(json.result);
        setHasEvaluated(true);
        toast.success("Evaluation successful");
      }
    } catch (err: unknown) {
      const message =
        err instanceof Error ? err.message : String(err);
      setResultValue(null);
      setResultError("Request error: " + message);
      setHasEvaluated(true);
      toast.error("Request failed");
    } finally {
      setLoading(false);
    }
  }, [expression, inputData]);

  const handleReset = () => {
    setExpression(EXAMPLES[0].expr);
    setTemplate("http");
    setInputDataState(TEMPLATES["http"]);
    setResultValue(null);
    setResultError(null);
    setHasEvaluated(false);
    toast("Reset to example template");
  };

  const evaluateRef = useRef(handleEvaluate);
  useEffect(() => {
    evaluateRef.current = handleEvaluate;
  }, [handleEvaluate]);

  const loadTemplate = (tpl: ExpressionTemplate) => {
    setExpression(tpl.expression);
    setInputDataState(yaml.dump(tpl.context));
    setResultValue(null);
    setResultError(null);
    setHasEvaluated(false);
  };

  const setInputData = (value: string | undefined) => {
    setInputDataState(value ?? "");
  };

  return {
    // state
    expression,
    inputData,
    template,
    loading,
    resultValue,
    resultError,
    hasEvaluated,
    resultExpanded,
    editorTheme,
    // handlers
    setExpression,
    setInputData,
    setTemplate,
    setResultExpanded,
    handleEvaluate,
    handleReset,
    loadTemplate,
    evaluateRef,
  };
};
