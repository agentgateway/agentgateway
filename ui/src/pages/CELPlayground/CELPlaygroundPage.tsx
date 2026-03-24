import styled from "@emotion/styled";
import { Button, Card, Col, Row, Select, Space } from "antd";
import yaml from "js-yaml";
import { ChevronDown, ChevronUp, PlayCircle, RotateCcw } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import toast from "react-hot-toast";
import { API_BASE_URL } from "../../api/client";
import { StyledAlert } from "../../components/StyledAlert";
import { MonacoEditorComponent } from "./MonacoEditorComponent";

const Container = styled.div`
  display: flex;
  flex-direction: column;
  gap: var(--spacing-lg);
  padding: var(--spacing-xl);
`;

const EditorCard = styled(Card)`
  .ant-card-body {
    padding: 0;
  }
`;

const EditorHeader = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--spacing-md) var(--spacing-lg);
  border-bottom: 1px solid var(--color-border-secondary);
  background: var(--color-bg-container);
`;

const EditorContent = styled.div`
  padding: var(--spacing-lg);
`;

const ExpandButton = styled.button`
  position: absolute;
  bottom: 0;
  left: 50%;
  transform: translateX(-50%);
  background: transparent;
  border: none;
  padding: 2px 8px;
  cursor: pointer;
  display: flex;
  align-items: center;
  gap: 2px;
  font-size: 11px;
  color: var(--color-text-secondary);
  opacity: 0.7;
  transition: all 0.2s;

  &:hover {
    opacity: 1;
    color: var(--color-text-base);
  }

  svg {
    width: 12px;
    height: 12px;
  }
`;

const TemplateList = styled.div`
  display: flex;
  flex-direction: column;
  gap: var(--spacing-sm);
`;

const TemplateItem = styled.div`
  padding: var(--spacing-sm);
  border: 1px solid var(--color-border-secondary);
  border-radius: var(--border-radius-base);
  cursor: pointer;
  transition: all var(--transition-base) var(--transition-timing);

  &:hover {
    border-color: var(--color-primary);
    background: var(--color-bg-hover);
  }
`;

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
  {
    name: "JWT Claims",
    expr: "jwt.iss == 'agentgateway.dev' && jwt.sub == 'test-user'",
  },
  { name: "Source IP", expr: "cidr('127.0.0.1/8').containsIP(source.address)" },
];

export const CELPlaygroundPage = () => {
  const [expression, setExpression] = useState<string>(EXAMPLES[0].expr);
  const [inputData, setInputData] = useState<string>(TEMPLATES["http"]);
  const [template, setTemplate] = useState<TemplateKey>("http");
  const [loading, setLoading] = useState<boolean>(false);
  const [resultValue, setResultValue] = useState<unknown | null>(null);
  const [resultError, setResultError] = useState<string | null>(null);
  const [hasEvaluated, setHasEvaluated] = useState<boolean>(false);
  const [resultExpanded, setResultExpanded] = useState<boolean>(false);

  const isDark = document.documentElement.getAttribute("data-theme") === "dark";
  const editorTheme = isDark ? "vs-dark" : "vs";

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
    } catch (err: any) {
      const message = err?.message ? String(err.message) : String(err);
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
    setInputData(TEMPLATES["http"]);
    setResultValue(null);
    setResultError(null);
    setHasEvaluated(false);
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

  const evaluateRef = useRef(handleEvaluate);
  useEffect(() => {
    evaluateRef.current = handleEvaluate;
  }, [handleEvaluate]);

  const templates = [
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

  const loadTemplate = (template: (typeof templates)[0]) => {
    setExpression(template.expression);
    setInputData(yaml.dump(template.context));
    setResultValue(null);
    setResultError(null);
    setHasEvaluated(false);
  };

  return (
    <Container>
      <div>
        <h1>CEL Playground</h1>
      </div>

      <StyledAlert
        message="Common Expression Language (CEL)"
        description="Test CEL expressions used for policy evaluation, routing decisions, and request validation. CEL provides a simple, fast, and safe way to evaluate expressions."
        type="info"
        showIcon
        closable
      />

      <Row gutter={[16, 16]}>
        <Col xs={24}>
          <EditorCard style={{ position: "relative" }}>
            <EditorHeader>
              <strong>Result</strong>
            </EditorHeader>
            <EditorContent>
              {!hasEvaluated ? (
                <div
                  style={{
                    display: "flex",
                    alignItems: "center",
                    justifyContent: "center",
                    height: "70px",
                    color: "var(--color-text-secondary)",
                    fontSize: "14px",
                  }}
                >
                  Click "Evaluate" to see results
                </div>
              ) : resultError ? (
                <div
                  style={{
                    borderRadius: "6px",
                    background: "var(--color-error-bg)",
                    border: "1px solid var(--color-error-border)",
                    padding: "12px",
                    height: resultExpanded ? "300px" : "70px",
                    overflow: "auto",
                    transition: "height 0.2s ease",
                  }}
                >
                  <pre
                    style={{
                      fontSize: "13px",
                      color: "var(--color-error)",
                      whiteSpace: "pre",
                      fontFamily: "Monaco, monospace",
                      margin: 0,
                    }}
                  >
                    {resultError}
                  </pre>
                </div>
              ) : resultValue !== null ? (
                <MonacoEditorComponent
                  value={JSON.stringify(resultValue, null, 2)}
                  onChange={() => {}}
                  language="json"
                  height={resultExpanded ? "300px" : "70px"}
                  theme={editorTheme}
                  options={{
                    readOnly: true,
                    wordWrap: "off",
                  }}
                />
              ) : null}
            </EditorContent>
            {hasEvaluated && (
              <ExpandButton
                type="button"
                onClick={() => setResultExpanded(!resultExpanded)}
              >
                {resultExpanded ? (
                  <>
                    <ChevronUp size={14} />
                    Collapse
                  </>
                ) : (
                  <>
                    <ChevronDown size={14} />
                    Expand
                  </>
                )}
              </ExpandButton>
            )}
          </EditorCard>
        </Col>

        <Col xs={24} lg={16}>
          <Space direction="vertical" style={{ width: "100%" }} size="large">
            <EditorCard>
              <EditorHeader>
                <div
                  style={{
                    display: "flex",
                    justifyContent: "space-between",
                    alignItems: "center",
                    width: "100%",
                  }}
                >
                  <strong>CEL Expression</strong>
                  <Space>
                    <Button
                      onClick={handleEvaluate}
                      disabled={loading}
                      icon={<PlayCircle size={14} />}
                      type="primary"
                    >
                      Evaluate
                    </Button>
                    <Button
                      icon={<RotateCcw size={14} />}
                      onClick={handleReset}
                    >
                      Reset
                    </Button>
                  </Space>
                </div>
              </EditorHeader>
              <EditorContent>
                <MonacoEditorComponent
                  value={expression}
                  onChange={(v) => setExpression(v ?? "")}
                  language="javascript"
                  height="200px"
                  theme={editorTheme}
                  onEvaluate={() => evaluateRef.current()}
                />
                <div
                  style={{
                    display: "flex",
                    gap: "8px",
                    marginTop: "12px",
                    flexWrap: "wrap",
                  }}
                >
                  {EXAMPLES.map((ex, idx) => (
                    <button
                      key={idx}
                      type="button"
                      onClick={() => setExpression(ex.expr)}
                      style={{
                        fontSize: "12px",
                        padding: "4px 8px",
                        borderRadius: "4px",
                        background: "var(--color-bg-hover)",
                        border: "1px solid var(--color-border-secondary)",
                        cursor: "pointer",
                      }}
                      title={ex.expr}
                    >
                      {ex.name}
                    </button>
                  ))}
                </div>
              </EditorContent>
            </EditorCard>

            <EditorCard>
              <EditorHeader>
                <strong>Input Data (YAML)</strong>
                <Select
                  value={template}
                  onChange={(value) => setTemplate(value as TemplateKey)}
                  style={{ width: 120 }}
                >
                  <Select.Option value="empty">Empty</Select.Option>
                  <Select.Option value="http">HTTP</Select.Option>
                </Select>
              </EditorHeader>
              <EditorContent>
                <MonacoEditorComponent
                  value={inputData}
                  onChange={(v) => setInputData(v ?? "")}
                  language="yaml"
                  height="400px"
                  theme={editorTheme}
                />
              </EditorContent>
            </EditorCard>
          </Space>
        </Col>

        <Col xs={24} lg={8}>
          <Card title="Expression Templates">
            <TemplateList>
              {templates.map((template, index) => (
                <TemplateItem
                  key={index}
                  onClick={() => loadTemplate(template)}
                >
                  <div style={{ fontWeight: 600, marginBottom: 4 }}>
                    {template.name}
                  </div>
                  <div style={{ fontSize: "12px", color: "#999" }}>
                    {template.description}
                  </div>
                  <div
                    style={{
                      fontSize: "11px",
                      fontFamily: "Monaco, monospace",
                      marginTop: 8,
                      color: "var(--color-primary)",
                    }}
                  >
                    {template.expression.length > 50
                      ? template.expression.substring(0, 50) + "..."
                      : template.expression}
                  </div>
                </TemplateItem>
              ))}
            </TemplateList>
          </Card>
        </Col>
      </Row>
    </Container>
  );
};
