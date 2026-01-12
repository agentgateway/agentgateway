import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Checkbox } from "@/components/ui/checkbox";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Trash2, Plus } from "lucide-react";
import { formatArrayForInput, handleNumberArrayInput } from "@/lib/policy-utils";
import { McpAuthorizationRule } from "@/lib/types";
import {
  ArrayInput,
  TargetInput,
  KeyValueManager,
  HeaderPairList,
  StringList,
} from "./form-components";

interface FormRendererProps {
  data: any;
  onChange: (data: any) => void;
}

export function renderJwtAuthForm({ data, onChange }: FormRendererProps) {
  const getCurrentJwksValue = () => {
    if (typeof data.jwks === "object") {
      return data.jwks.file || data.jwks.url || "";
    }
    return data.jwks || "";
  };

  const handleJwksChange = (value: string) => {
    if (!value.trim()) {
      onChange({ ...data, jwks: { url: "" } });
      return;
    }

    // Detect if it's a URL (starts with http/https) or a file path
    const isUrl = value.trim().startsWith("http://") || value.trim().startsWith("https://");

    if (isUrl) {
      onChange({ ...data, jwks: { url: value } });
    } else {
      onChange({ ...data, jwks: { file: value } });
    }
  };

  return (
    <div className="space-y-6">
      <div className="space-y-3">
        <Label htmlFor="issuer">Issuer *</Label>
        <Input
          id="issuer"
          value={data.issuer || ""}
          onChange={(e) => onChange({ ...data, issuer: e.target.value })}
          placeholder="https://example.auth0.com/"
        />
      </div>
      <ArrayInput
        id="audiences"
        label="Audiences (comma-separated) *"
        value={data.audiences}
        onChange={(audiences) => onChange({ ...data, audiences })}
        placeholder="audience1, audience2"
      />
      <div className="space-y-3">
        <Label htmlFor="jwks">JWKS URL or File Path *</Label>
        <Input
          id="jwks"
          value={getCurrentJwksValue()}
          onChange={(e) => handleJwksChange(e.target.value)}
          placeholder="https://example.auth0.com/.well-known/jwks.json"
        />
        <p className="text-xs text-muted-foreground">
          Enter a URL (https://...) for remote JWKS or a file path for local JWKS
        </p>
      </div>
    </div>
  );
}

export function renderCorsForm({ data, onChange }: FormRendererProps) {
  return (
    <div className="space-y-6">
      <div className="flex items-center space-x-2">
        <Checkbox
          id="allowCredentials"
          checked={data.allowCredentials || false}
          onCheckedChange={(checked: boolean) => onChange({ ...data, allowCredentials: checked })}
        />
        <Label htmlFor="allowCredentials">Allow Credentials</Label>
      </div>

      <ArrayInput
        id="allowOrigins"
        label="Allow Origins (comma-separated)"
        value={data.allowOrigins}
        onChange={(allowOrigins) => onChange({ ...data, allowOrigins })}
        placeholder="https://example.com, https://app.example.com"
      />

      <ArrayInput
        id="allowMethods"
        label="Allow Methods (comma-separated)"
        value={data.allowMethods}
        onChange={(allowMethods) => onChange({ ...data, allowMethods })}
        placeholder="GET, POST, PUT, DELETE"
      />

      <ArrayInput
        id="allowHeaders"
        label="Allow Headers (comma-separated)"
        value={data.allowHeaders}
        onChange={(allowHeaders) => onChange({ ...data, allowHeaders })}
        placeholder="Content-Type, Authorization"
      />

      <ArrayInput
        id="exposeHeaders"
        label="Expose Headers (comma-separated)"
        value={data.exposeHeaders}
        onChange={(exposeHeaders) => onChange({ ...data, exposeHeaders })}
        placeholder="X-Custom-Header"
      />

      <div className="space-y-3">
        <Label htmlFor="maxAge">Max Age (seconds)</Label>
        <Input
          id="maxAge"
          type="number"
          value={data.maxAge || ""}
          onChange={(e) =>
            onChange({ ...data, maxAge: e.target.value ? parseInt(e.target.value) : null })
          }
          placeholder="3600"
        />
      </div>
    </div>
  );
}

export function renderTimeoutForm({ data, onChange }: FormRendererProps) {
  return (
    <div className="space-y-6">
      <div className="space-y-3">
        <Label htmlFor="requestTimeout">
          Request Timeout (e.g., &quot;30s&quot;, &quot;1m&quot;)
        </Label>
        <Input
          id="requestTimeout"
          value={data.requestTimeout || ""}
          onChange={(e) => onChange({ ...data, requestTimeout: e.target.value })}
          placeholder="30s"
        />
      </div>
      <div className="space-y-3">
        <Label htmlFor="backendRequestTimeout">
          Backend Request Timeout (e.g., &quot;30s&quot;, &quot;1m&quot;)
        </Label>
        <Input
          id="backendRequestTimeout"
          value={data.backendRequestTimeout || ""}
          onChange={(e) => onChange({ ...data, backendRequestTimeout: e.target.value })}
          placeholder="15s"
        />
      </div>
    </div>
  );
}

export function renderRetryForm({ data, onChange }: FormRendererProps) {
  return (
    <div className="space-y-6">
      <div className="space-y-3">
        <Label htmlFor="attempts">Max Attempts</Label>
        <Input
          id="attempts"
          type="number"
          min="1"
          max="255"
          value={data.attempts || 1}
          onChange={(e) => onChange({ ...data, attempts: parseInt(e.target.value) || 1 })}
        />
      </div>
      <div className="space-y-3">
        <Label htmlFor="backoff">Backoff Duration (e.g., &quot;100ms&quot;, &quot;1s&quot;)</Label>
        <Input
          id="backoff"
          value={data.backoff || ""}
          onChange={(e) => onChange({ ...data, backoff: e.target.value })}
          placeholder="100ms"
        />
      </div>
      <div className="space-y-3">
        <Label htmlFor="codes">Retry on HTTP Status Codes (comma-separated)</Label>
        <Input
          id="codes"
          value={formatArrayForInput(data.codes)}
          onChange={(e) => onChange({ ...data, codes: e.target.value })}
          onBlur={(e) => onChange({ ...data, codes: handleNumberArrayInput(e.target.value) })}
          placeholder="500, 502, 503, 504"
        />
      </div>
    </div>
  );
}

export function renderDirectResponseForm({ data, onChange }: FormRendererProps) {
  return (
    <div className="space-y-6">
      <div className="space-y-3">
        <Label htmlFor="status">HTTP Status Code</Label>
        <Input
          id="status"
          type="number"
          min="100"
          max="599"
          value={data.status || 200}
          onChange={(e) => onChange({ ...data, status: parseInt(e.target.value || "200", 10) })}
        />
      </div>
      <div className="space-y-3">
        <Label htmlFor="body">Response Body</Label>
        <Textarea
          id="body"
          value={data.body || ""}
          onChange={(e) => onChange({ ...data, body: e.target.value })}
          placeholder="Response content"
          rows={4}
        />
      </div>
    </div>
  );
}

export function renderRemoteRateLimitForm({ data, onChange }: FormRendererProps) {
  const handleDescriptorChange = (key: string, field: string, value: any) => {
    const newDescriptors = { ...data.descriptors };
    if (field === "key") {
      const oldValue = newDescriptors[key];
      delete newDescriptors[key];
      newDescriptors[value] = oldValue;
    } else if (field === "type") {
      if (value === "static") {
        newDescriptors[key] = { static: "" };
      } else if (value === "requestHeader") {
        newDescriptors[key] = "";
      }
    } else if (field === "value") {
      const descriptor = newDescriptors[key];
      const isStatic =
        typeof descriptor === "object" && descriptor !== null && "static" in descriptor;
      if (isStatic) {
        newDescriptors[key] = { static: value };
      } else {
        newDescriptors[key] = value;
      }
    }
    onChange({ ...data, descriptors: newDescriptors });
  };

  const addDescriptor = () => {
    const newDescriptors = { ...data.descriptors };
    const newKey = `descriptor${Object.keys(newDescriptors).length + 1}`;
    newDescriptors[newKey] = "";
    onChange({ ...data, descriptors: newDescriptors });
  };

  const removeDescriptor = (key: string) => {
    const newDescriptors = { ...data.descriptors };
    delete newDescriptors[key];
    onChange({ ...data, descriptors: newDescriptors });
  };

  return (
    <div className="space-y-6">
      <TargetInput
        id="target"
        label="Target (host:port)"
        value={data.target}
        onChange={(target) => onChange({ ...data, target })}
        placeholder="ratelimit-service.example.com:8080"
        required
      />

      <div className="space-y-3">
        <Label>Rate Limit Descriptors</Label>
        <p className="text-sm text-muted-foreground">
          Configure descriptors that identify rate limit keys. Each descriptor can use a request
          header or a static value.
        </p>
        {Object.entries(data.descriptors || {}).map(([key, descriptor], index) => {
          const isStatic =
            typeof descriptor === "object" && descriptor !== null && "static" in descriptor;
          const isRequestHeader = typeof descriptor === "string";

          return (
            <div key={index} className="border rounded-lg p-4 space-y-3">
              <div className="flex items-center justify-between">
                <Label htmlFor={`desc-key-${index}`}>Descriptor {index + 1}</Label>
                <Button variant="ghost" size="sm" onClick={() => removeDescriptor(key)}>
                  <Trash2 className="h-4 w-4" />
                </Button>
              </div>

              <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
                <div className="space-y-2">
                  <Label htmlFor={`desc-key-${index}`}>Key</Label>
                  <Input
                    id={`desc-key-${index}`}
                    value={key}
                    onChange={(e) => handleDescriptorChange(key, "key", e.target.value)}
                    placeholder="user_type"
                  />
                </div>

                <div className="space-y-2">
                  <Label htmlFor={`desc-type-${index}`}>Type</Label>
                  <Select
                    value={isStatic ? "static" : isRequestHeader ? "requestHeader" : ""}
                    onValueChange={(value) => handleDescriptorChange(key, "type", value)}
                  >
                    <SelectTrigger>
                      <SelectValue placeholder="Select type" />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="requestHeader">Request Header</SelectItem>
                      <SelectItem value="static">Static Value</SelectItem>
                    </SelectContent>
                  </Select>
                </div>

                <div className="space-y-2">
                  <Label htmlFor={`desc-value-${index}`}>
                    {isStatic ? "Static Value" : "Header Name"}
                  </Label>
                  <Input
                    id={`desc-value-${index}`}
                    value={
                      isStatic
                        ? (descriptor as any).static || ""
                        : isRequestHeader
                          ? (descriptor as string)
                          : ""
                    }
                    onChange={(e) => handleDescriptorChange(key, "value", e.target.value)}
                    placeholder={isStatic ? "premium" : "x-user-type"}
                  />
                </div>
              </div>
            </div>
          );
        })}

        <Button variant="outline" size="sm" onClick={addDescriptor}>
          <Plus className="h-4 w-4 mr-2" />
          Add Descriptor
        </Button>
      </div>
    </div>
  );
}

export function renderExtAuthzForm({ data, onChange }: FormRendererProps) {
  return (
    <div className="space-y-6">
      <TargetInput
        id="host"
        label="Target (host:port)"
        value={data.host}
        onChange={(host) => onChange({ ...data, host })}
        placeholder="auth-service.example.com:8080"
        required
      />

      <KeyValueManager
        title="Context Extensions"
        description="Additional context key-value pairs sent to the authorization service"
        data={data.context}
        onChange={(context) => onChange({ ...data, context })}
        addButtonText="Add Context"
      />
    </div>
  );
}

export function renderHeaderModifierForm({ data, onChange }: FormRendererProps) {
  return (
    <div className="space-y-6">
      <HeaderPairList
        title="Add Headers"
        headers={data.add || []}
        onChange={(add) => onChange({ ...data, add })}
        buttonText="Add Header"
      />

      <HeaderPairList
        title="Set Headers"
        headers={data.set || []}
        onChange={(set) => onChange({ ...data, set })}
        buttonText="Set Header"
      />

      <StringList
        title="Remove Headers"
        items={data.remove || []}
        onChange={(remove) => onChange({ ...data, remove })}
        buttonText="Remove Header"
        placeholder="Header name to remove"
      />
    </div>
  );
}

export function renderBackendTLSForm({ data, onChange }: FormRendererProps) {
  return (
    <div className="space-y-6">
      <div className="space-y-3">
        <Label htmlFor="cert">Certificate Path</Label>
        <Input
          id="cert"
          value={data.cert || ""}
          onChange={(e) => onChange({ ...data, cert: e.target.value })}
          placeholder="/path/to/cert.pem"
        />
      </div>
      <div className="space-y-3">
        <Label htmlFor="key">Private Key Path</Label>
        <Input
          id="key"
          value={data.key || ""}
          onChange={(e) => onChange({ ...data, key: e.target.value })}
          placeholder="/path/to/key.pem"
        />
      </div>
      <div className="space-y-3">
        <Label htmlFor="root">CA Certificate Path</Label>
        <Input
          id="root"
          value={data.root || ""}
          onChange={(e) => onChange({ ...data, root: e.target.value })}
          placeholder="/path/to/ca.pem"
        />
      </div>
      <div className="flex items-center space-x-2">
        <Checkbox
          id="insecure"
          checked={data.insecure || false}
          onCheckedChange={(checked: boolean) => onChange({ ...data, insecure: checked })}
        />
        <Label htmlFor="insecure">Skip Certificate Verification</Label>
      </div>
      <div className="flex items-center space-x-2">
        <Checkbox
          id="insecureHost"
          checked={data.insecureHost || false}
          onCheckedChange={(checked: boolean) => onChange({ ...data, insecureHost: checked })}
        />
        <Label htmlFor="insecureHost">Skip Hostname Verification</Label>
      </div>
    </div>
  );
}

export function renderLocalRateLimitForm({ data, onChange }: FormRendererProps) {
  return (
    <div className="space-y-6">
      <div className="space-y-3">
        <Label>Rate Limit Rules</Label>
        <div className="space-y-4">
          {Array.isArray(data) && data.length > 0 ? (
            data.map((rule: any, index: number) => (
              <div key={index} className="border rounded-lg p-4 space-y-4">
                <div className="flex items-center justify-between">
                  <span className="font-medium">Rule {index + 1}</span>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => {
                      const newRules = [...data];
                      newRules.splice(index, 1);
                      onChange(newRules);
                    }}
                  >
                    <Trash2 className="h-4 w-4" />
                  </Button>
                </div>
                <div className="grid grid-cols-2 gap-4">
                  <div className="space-y-2">
                    <Label htmlFor={`maxTokens-${index}`}>Max Tokens</Label>
                    <Input
                      id={`maxTokens-${index}`}
                      type="number"
                      min="1"
                      value={rule.maxTokens || ""}
                      onChange={(e) => {
                        const newRules = [...data];
                        newRules[index] = { ...rule, maxTokens: parseInt(e.target.value) || 0 };
                        onChange(newRules);
                      }}
                      placeholder="100"
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor={`tokensPerFill-${index}`}>Tokens per Fill</Label>
                    <Input
                      id={`tokensPerFill-${index}`}
                      type="number"
                      min="1"
                      value={rule.tokensPerFill || ""}
                      onChange={(e) => {
                        const newRules = [...data];
                        newRules[index] = {
                          ...rule,
                          tokensPerFill: parseInt(e.target.value) || 0,
                        };
                        onChange(newRules);
                      }}
                      placeholder="10"
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor={`fillInterval-${index}`}>Fill Interval</Label>
                    <Input
                      id={`fillInterval-${index}`}
                      value={rule.fillInterval || ""}
                      onChange={(e) => {
                        const newRules = [...data];
                        newRules[index] = { ...rule, fillInterval: e.target.value };
                        onChange(newRules);
                      }}
                      placeholder="1s"
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor={`type-${index}`}>Type</Label>
                    <Select
                      value={rule.type || "requests"}
                      onValueChange={(value) => {
                        const newRules = [...data];
                        newRules[index] = { ...rule, type: value };
                        onChange(newRules);
                      }}
                    >
                      <SelectTrigger>
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="requests">Requests</SelectItem>
                        <SelectItem value="tokens">Tokens</SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                </div>
              </div>
            ))
          ) : (
            <div className="text-center py-4 text-muted-foreground">
              No rate limit rules configured.
            </div>
          )}
        </div>
        <Button
          variant="outline"
          onClick={() => {
            const newRule = {
              maxTokens: 100,
              tokensPerFill: 10,
              fillInterval: "1s",
              type: "requests",
            };
            onChange([...(Array.isArray(data) ? data : []), newRule]);
          }}
        >
          <Plus className="h-4 w-4 mr-2" />
          Add Rate Limit Rule
        </Button>
      </div>
    </div>
  );
}

export function renderMcpAuthenticationForm({ data, onChange }: FormRendererProps) {
  return (
    <div className="space-y-6">
      <div className="space-y-3">
        <Label htmlFor="issuer">Issuer *</Label>
        <Input
          id="issuer"
          value={data.issuer || ""}
          onChange={(e) => onChange({ ...data, issuer: e.target.value })}
          placeholder="https://example.auth0.com/"
        />
      </div>
      <ArrayInput
        id="scopes"
        label="Scopes (comma-separated) *"
        value={data.scopes}
        onChange={(scopes) => onChange({ ...data, scopes })}
        placeholder="read:tools, write:tools"
      />
      <div className="space-y-3">
        <Label htmlFor="audience">Audience *</Label>
        <Input
          id="audience"
          value={data.audience || ""}
          onChange={(e) => onChange({ ...data, audience: e.target.value })}
          placeholder="mcp-api"
        />
      </div>
      <div className="space-y-3">
        <Label htmlFor="provider">Provider</Label>
        <Select
          value={data.provider ? (data.provider.auth0 ? "auth0" : "keycloak") : ""}
          onValueChange={(value) => {
            const provider =
              value === "auth0" ? { auth0: {} } : value === "keycloak" ? { keycloak: {} } : null;
            onChange({ ...data, provider });
          }}
        >
          <SelectTrigger>
            <SelectValue placeholder="Select a provider" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="auth0">Auth0</SelectItem>
            <SelectItem value="keycloak">Keycloak</SelectItem>
          </SelectContent>
        </Select>
      </div>
    </div>
  );
}

export function renderMcpAuthorizationForm({ data, onChange }: FormRendererProps) {
  type McpRule = string | McpAuthorizationRule;
  type RuleType = "string" | "allow" | "deny";

  const isRuleObject = (rule: McpRule): rule is McpAuthorizationRule => {
    return typeof rule === "object" && rule !== null;
  };

  const getRuleType = (rule: McpRule): RuleType => {
    if (typeof rule === "string") return "string";
    if (isRuleObject(rule)) {
      if ("allow" in rule) return "allow";
      if ("deny" in rule) return "deny";
    }
    return "string";
  };

  const getRuleValue = (rule: McpRule): string => {
    if (typeof rule === "string") return rule;
    if (isRuleObject(rule)) {
      return rule.allow ?? rule.deny ?? "";
    }
    return "";
  };

  const updateRule = (index: number, type: RuleType, value: string) => {
    const newRules: McpRule[] = [...(data.rules || [])];
    if (type === "string") {
      newRules[index] = value;
    } else if (type === "allow") {
      newRules[index] = { allow: value };
    } else if (type === "deny") {
      newRules[index] = { deny: value };
    }
    onChange({ ...data, rules: newRules });
  };

  return (
    <div className="space-y-6">
      <div className="space-y-3">
        <Label>CEL Authorization Rules</Label>
        <p className="text-sm text-muted-foreground">
          Define CEL policy rules for MCP authorization. Rules can be simple expressions or explicit
          allow/deny policies.
        </p>
        {(data.rules || []).map((rule: McpRule, index: number) => {
          const ruleType = getRuleType(rule);
          const ruleValue = getRuleValue(rule);

          return (
            <div key={index} className="border rounded-lg p-4 space-y-3">
              <div className="flex items-center justify-between">
                <Label htmlFor={`rule-${index}`}>Rule {index + 1}</Label>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => {
                    const newRules: McpRule[] = [...(data.rules || [])];
                    newRules.splice(index, 1);
                    onChange({ ...data, rules: newRules });
                  }}
                >
                  <Trash2 className="h-4 w-4" />
                </Button>
              </div>

              <div className="grid grid-cols-1 md:grid-cols-4 gap-3">
                <div className="space-y-2">
                  <Label htmlFor={`rule-type-${index}`}>Rule Type</Label>
                  <Select
                    value={ruleType}
                    onValueChange={(value: RuleType) => updateRule(index, value, ruleValue)}
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="string">Expression</SelectItem>
                      <SelectItem value="allow">Allow Rule</SelectItem>
                      <SelectItem value="deny">Deny Rule</SelectItem>
                    </SelectContent>
                  </Select>
                </div>

                <div className="md:col-span-3 space-y-2">
                  <Label htmlFor={`rule-expression-${index}`}>CEL Expression</Label>
                  <Textarea
                    id={`rule-expression-${index}`}
                    value={ruleValue}
                    onChange={(e) => updateRule(index, ruleType, e.target.value)}
                    placeholder={
                      ruleType === "string"
                        ? `mcp.tool.name == "echo"`
                        : ruleType === "allow"
                          ? `mcp.tool.name in ["search_code", "get_file_contents"]`
                          : `jwt.sub != "blocked-user"`
                    }
                    rows={3}
                    className="font-mono"
                  />
                </div>
              </div>

              {ruleType !== "string" && (
                <div className="text-xs text-muted-foreground">
                  {ruleType === "allow"
                    ? "This rule explicitly allows access when the condition is true."
                    : "This rule explicitly denies access when the condition is true."}
                </div>
              )}
            </div>
          );
        })}

        <div className="flex gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              const newRules: McpRule[] = [...(data.rules || []), ""];
              onChange({ ...data, rules: newRules });
            }}
          >
            <Plus className="h-4 w-4 mr-2" />
            Add Simple Rule
          </Button>

          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              const newRules: McpRule[] = [...(data.rules || []), { allow: "" }];
              onChange({ ...data, rules: newRules });
            }}
          >
            <Plus className="h-4 w-4 mr-2" />
            Add Allow Rule
          </Button>

          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              const newRules: McpRule[] = [...(data.rules || []), { deny: "" }];
              onChange({ ...data, rules: newRules });
            }}
          >
            <Plus className="h-4 w-4 mr-2" />
            Add Deny Rule
          </Button>
        </div>
      </div>
    </div>
  );
}

export function renderBackendAuthForm({ data, onChange }: FormRendererProps) {
  return (
    <div className="space-y-6">
      <div className="space-y-3">
        <Label>Authentication Type</Label>
        <Select
          value={
            data.passthrough
              ? "passthrough"
              : data.key
                ? "key"
                : data.gcp
                  ? "gcp"
                  : data.aws
                    ? "aws"
                    : ""
          }
          onValueChange={(value) => {
            switch (value) {
              case "passthrough":
                onChange({ passthrough: {} });
                break;
              case "key":
                onChange({ key: "" });
                break;
              case "gcp":
                onChange({ gcp: {} });
                break;
              case "aws":
                onChange({ aws: {} });
                break;
              default:
                onChange({});
            }
          }}
        >
          <SelectTrigger>
            <SelectValue placeholder="Select authentication type" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="passthrough">Passthrough</SelectItem>
            <SelectItem value="key">API Key</SelectItem>
            <SelectItem value="gcp">Google Cloud Platform</SelectItem>
            <SelectItem value="aws">Amazon Web Services</SelectItem>
          </SelectContent>
        </Select>
      </div>
      {data.key !== undefined && (
        <div className="space-y-3">
          <Label>API Key</Label>
          <div className="space-y-2">
            <div className="flex items-center space-x-2">
              <Checkbox
                id="keyFile"
                checked={typeof data.key === "object" && data.key?.file !== undefined}
                onCheckedChange={(checked: boolean) => {
                  if (checked) {
                    onChange({ ...data, key: { file: "" } });
                  } else {
                    onChange({ ...data, key: "" });
                  }
                }}
              />
              <Label htmlFor="keyFile">Load from file</Label>
            </div>
            {typeof data.key === "object" && data.key?.file !== undefined ? (
              <Input
                value={data.key.file || ""}
                onChange={(e) => onChange({ ...data, key: { file: e.target.value } })}
                placeholder="/path/to/api-key.txt"
              />
            ) : (
              <Input
                value={typeof data.key === "string" ? data.key : ""}
                onChange={(e) => onChange({ ...data, key: e.target.value })}
                placeholder="your-api-key"
                type="password"
              />
            )}
          </div>
        </div>
      )}
    </div>
  );
}

export function renderRequestRedirectForm({ data, onChange }: FormRendererProps) {
  return (
    <div className="space-y-6">
      <div className="space-y-3">
        <Label htmlFor="scheme">Scheme</Label>
        <Select
          value={data.scheme || ""}
          onValueChange={(value) => onChange({ ...data, scheme: value || null })}
        >
          <SelectTrigger>
            <SelectValue placeholder="Select scheme" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="http">HTTP</SelectItem>
            <SelectItem value="https">HTTPS</SelectItem>
          </SelectContent>
        </Select>
      </div>

      <div className="space-y-3">
        <Label>Authority Redirect</Label>
        <RadioGroup
          value={
            data.authority?.full !== undefined
              ? "full"
              : data.authority?.host !== undefined
                ? "host"
                : data.authority?.port !== undefined
                  ? "port"
                  : "none"
          }
          onValueChange={(value) => {
            switch (value) {
              case "full":
                onChange({ ...data, authority: { full: "" } });
                break;
              case "host":
                onChange({ ...data, authority: { host: "" } });
                break;
              case "port":
                onChange({ ...data, authority: { port: 80 } });
                break;
              case "none":
              default:
                onChange({ ...data, authority: null });
                break;
            }
          }}
        >
          <div className="space-y-3">
            <div className="flex items-center space-x-2">
              <RadioGroupItem value="none" id="authorityNone" />
              <Label htmlFor="authorityNone">No authority redirect</Label>
            </div>

            <div className="flex items-center space-x-2">
              <RadioGroupItem value="full" id="authorityFull" />
              <Label htmlFor="authorityFull">Full authority (host:port)</Label>
            </div>
            {data.authority?.full !== undefined && (
              <div className="ml-6">
                <Input
                  value={data.authority.full || ""}
                  onChange={(e) => onChange({ ...data, authority: { full: e.target.value } })}
                  placeholder="example.com:8080"
                />
              </div>
            )}

            <div className="flex items-center space-x-2">
              <RadioGroupItem value="host" id="authorityHost" />
              <Label htmlFor="authorityHost">Host only</Label>
            </div>
            {data.authority?.host !== undefined && (
              <div className="ml-6">
                <Input
                  value={data.authority.host || ""}
                  onChange={(e) => onChange({ ...data, authority: { host: e.target.value } })}
                  placeholder="example.com"
                />
              </div>
            )}

            <div className="flex items-center space-x-2">
              <RadioGroupItem value="port" id="authorityPort" />
              <Label htmlFor="authorityPort">Port only</Label>
            </div>
            {data.authority?.port !== undefined && (
              <div className="ml-6">
                <Input
                  type="number"
                  min="1"
                  max="65535"
                  value={data.authority.port || ""}
                  onChange={(e) =>
                    onChange({ ...data, authority: { port: parseInt(e.target.value) || 80 } })
                  }
                  placeholder="8080"
                />
              </div>
            )}
          </div>
        </RadioGroup>
      </div>

      <div className="space-y-3">
        <Label>Path Redirect</Label>
        <RadioGroup
          value={
            data.path?.full !== undefined
              ? "full"
              : data.path?.prefix !== undefined
                ? "prefix"
                : "none"
          }
          onValueChange={(value) => {
            switch (value) {
              case "full":
                onChange({ ...data, path: { full: "" } });
                break;
              case "prefix":
                onChange({ ...data, path: { prefix: "" } });
                break;
              case "none":
              default:
                onChange({ ...data, path: null });
                break;
            }
          }}
        >
          <div className="space-y-3">
            <div className="flex items-center space-x-2">
              <RadioGroupItem value="none" id="pathNone" />
              <Label htmlFor="pathNone">No path redirect</Label>
            </div>

            <div className="flex items-center space-x-2">
              <RadioGroupItem value="full" id="pathFull" />
              <Label htmlFor="pathFull">Full path replacement</Label>
            </div>
            {data.path?.full !== undefined && (
              <div className="ml-6">
                <Input
                  value={data.path.full || ""}
                  onChange={(e) => onChange({ ...data, path: { full: e.target.value } })}
                  placeholder="/new/path"
                />
              </div>
            )}

            <div className="flex items-center space-x-2">
              <RadioGroupItem value="prefix" id="pathPrefix" />
              <Label htmlFor="pathPrefix">Prefix replacement</Label>
            </div>
            {data.path?.prefix !== undefined && (
              <div className="ml-6">
                <Input
                  value={data.path.prefix || ""}
                  onChange={(e) => onChange({ ...data, path: { prefix: e.target.value } })}
                  placeholder="/api"
                />
              </div>
            )}
          </div>
        </RadioGroup>
      </div>

      <div className="space-y-3">
        <Label htmlFor="status">HTTP Status Code</Label>
        <Input
          id="status"
          type="number"
          min="300"
          max="399"
          value={data.status || ""}
          onChange={(e) => onChange({ ...data, status: parseInt(e.target.value) || null })}
          placeholder="302"
        />
      </div>
    </div>
  );
}

export function renderUrlRewriteForm({ data, onChange }: FormRendererProps) {
  return (
    <div className="space-y-6">
      <div className="space-y-3">
        <Label>Authority Rewrite</Label>
        <RadioGroup
          value={
            data.authority?.full !== undefined
              ? "full"
              : data.authority?.host !== undefined
                ? "host"
                : data.authority?.port !== undefined
                  ? "port"
                  : "none"
          }
          onValueChange={(value) => {
            switch (value) {
              case "full":
                onChange({ ...data, authority: { full: "" } });
                break;
              case "host":
                onChange({ ...data, authority: { host: "" } });
                break;
              case "port":
                onChange({ ...data, authority: { port: 80 } });
                break;
              case "none":
              default:
                onChange({ ...data, authority: null });
                break;
            }
          }}
        >
          <div className="space-y-3">
            <div className="flex items-center space-x-2">
              <RadioGroupItem value="none" id="rewriteAuthorityNone" />
              <Label htmlFor="rewriteAuthorityNone">No authority rewrite</Label>
            </div>

            <div className="flex items-center space-x-2">
              <RadioGroupItem value="full" id="rewriteAuthorityFull" />
              <Label htmlFor="rewriteAuthorityFull">Full authority (host:port)</Label>
            </div>
            {data.authority?.full !== undefined && (
              <div className="ml-6">
                <Input
                  value={data.authority.full || ""}
                  onChange={(e) => onChange({ ...data, authority: { full: e.target.value } })}
                  placeholder="example.com:8080"
                />
              </div>
            )}

            <div className="flex items-center space-x-2">
              <RadioGroupItem value="host" id="rewriteAuthorityHost" />
              <Label htmlFor="rewriteAuthorityHost">Host only</Label>
            </div>
            {data.authority?.host !== undefined && (
              <div className="ml-6">
                <Input
                  value={data.authority.host || ""}
                  onChange={(e) => onChange({ ...data, authority: { host: e.target.value } })}
                  placeholder="example.com"
                />
              </div>
            )}

            <div className="flex items-center space-x-2">
              <RadioGroupItem value="port" id="rewriteAuthorityPort" />
              <Label htmlFor="rewriteAuthorityPort">Port only</Label>
            </div>
            {data.authority?.port !== undefined && (
              <div className="ml-6">
                <Input
                  type="number"
                  min="1"
                  max="65535"
                  value={data.authority.port || ""}
                  onChange={(e) =>
                    onChange({ ...data, authority: { port: parseInt(e.target.value) || 80 } })
                  }
                  placeholder="8080"
                />
              </div>
            )}
          </div>
        </RadioGroup>
      </div>

      <div className="space-y-3">
        <Label>Path Rewrite</Label>
        <RadioGroup
          value={
            data.path?.full !== undefined
              ? "full"
              : data.path?.prefix !== undefined
                ? "prefix"
                : "none"
          }
          onValueChange={(value) => {
            switch (value) {
              case "full":
                onChange({ ...data, path: { full: "" } });
                break;
              case "prefix":
                onChange({ ...data, path: { prefix: "" } });
                break;
              case "none":
              default:
                onChange({ ...data, path: null });
                break;
            }
          }}
        >
          <div className="space-y-3">
            <div className="flex items-center space-x-2">
              <RadioGroupItem value="none" id="rewritePathNone" />
              <Label htmlFor="rewritePathNone">No path rewrite</Label>
            </div>

            <div className="flex items-center space-x-2">
              <RadioGroupItem value="full" id="rewritePathFull" />
              <Label htmlFor="rewritePathFull">Full path replacement</Label>
            </div>
            {data.path?.full !== undefined && (
              <div className="ml-6">
                <Input
                  value={data.path.full || ""}
                  onChange={(e) => onChange({ ...data, path: { full: e.target.value } })}
                  placeholder="/new/path"
                />
              </div>
            )}

            <div className="flex items-center space-x-2">
              <RadioGroupItem value="prefix" id="rewritePathPrefix" />
              <Label htmlFor="rewritePathPrefix">Prefix replacement</Label>
            </div>
            {data.path?.prefix !== undefined && (
              <div className="ml-6">
                <Input
                  value={data.path.prefix || ""}
                  onChange={(e) => onChange({ ...data, path: { prefix: e.target.value } })}
                  placeholder="/api"
                />
              </div>
            )}
          </div>
        </RadioGroup>
      </div>
    </div>
  );
}

export function renderAiForm({ data, onChange }: FormRendererProps) {
  return (
    <div className="space-y-6">
      <div className="space-y-3">
        <Label>AI Provider *</Label>
        <Select
          value={
            data.provider?.openAI
              ? "openai"
              : data.provider?.gemini
                ? "gemini"
                : data.provider?.vertex
                  ? "vertex"
                  : data.provider?.anthropic
                    ? "anthropic"
                    : data.provider?.bedrock
                      ? "bedrock"
                      : data.provider?.azureOpenAI
                        ? "azureOpenAI"
                        : ""
          }
          onValueChange={(value) => {
            let provider = null;
            switch (value) {
              case "openai":
                provider = { openAI: { model: null } };
                break;
              case "gemini":
                provider = { gemini: { model: null } };
                break;
              case "vertex":
                provider = { vertex: { projectId: "", model: null, region: null } };
                break;
              case "anthropic":
                provider = { anthropic: { model: null } };
                break;
              case "bedrock":
                provider = { bedrock: { model: "", region: "" } };
                break;
              case "azureOpenAI":
                provider = { azureOpenAI: { model: null, host: "", apiVersion: null } };
                break;
            }
            onChange({ ...data, provider });
          }}
        >
          <SelectTrigger>
            <SelectValue placeholder="Select AI provider" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="openai">OpenAI</SelectItem>
            <SelectItem value="gemini">Google Gemini</SelectItem>
            <SelectItem value="vertex">Google Vertex AI</SelectItem>
            <SelectItem value="anthropic">Anthropic</SelectItem>
            <SelectItem value="bedrock">AWS Bedrock</SelectItem>
            <SelectItem value="azureOpenAI">Azure OpenAI</SelectItem>
          </SelectContent>
        </Select>
      </div>

      {/* Provider-specific configuration */}
      {data.provider?.openAI && (
        <div className="space-y-3">
          <Label htmlFor="openai-model">OpenAI Model</Label>
          <Input
            id="openai-model"
            value={data.provider.openAI.model || ""}
            onChange={(e) =>
              onChange({
                ...data,
                provider: {
                  openAI: {
                    model: e.target.value || null,
                  },
                },
              })
            }
            placeholder="gpt-4o, gpt-3.5-turbo, etc. (optional)"
          />
        </div>
      )}

      {data.provider?.gemini && (
        <div className="space-y-3">
          <Label htmlFor="gemini-model">Gemini Model</Label>
          <Input
            id="gemini-model"
            value={data.provider.gemini.model || ""}
            onChange={(e) =>
              onChange({
                ...data,
                provider: {
                  gemini: {
                    model: e.target.value || null,
                  },
                },
              })
            }
            placeholder="gemini-pro, gemini-1.5-flash, etc. (optional)"
          />
        </div>
      )}

      {data.provider?.vertex && (
        <div className="space-y-3">
          <Label htmlFor="vertex-project-id">Project ID *</Label>
          <Input
            id="vertex-project-id"
            value={data.provider.vertex.projectId || ""}
            onChange={(e) =>
              onChange({
                ...data,
                provider: {
                  vertex: {
                    ...data.provider.vertex,
                    projectId: e.target.value,
                  },
                },
              })
            }
            placeholder="your-gcp-project-id"
          />
          <Label htmlFor="vertex-model">Model</Label>
          <Input
            id="vertex-model"
            value={data.provider.vertex.model || ""}
            onChange={(e) =>
              onChange({
                ...data,
                provider: {
                  vertex: {
                    ...data.provider.vertex,
                    model: e.target.value || null,
                  },
                },
              })
            }
            placeholder="gemini-pro, claude-3-opus, etc. (optional)"
          />
          <Label htmlFor="vertex-region">Region</Label>
          <Input
            id="vertex-region"
            value={data.provider.vertex.region || ""}
            onChange={(e) =>
              onChange({
                ...data,
                provider: {
                  vertex: {
                    ...data.provider.vertex,
                    region: e.target.value || null,
                  },
                },
              })
            }
            placeholder="us-central1, europe-west1, etc. (optional)"
          />
        </div>
      )}

      {data.provider?.anthropic && (
        <div className="space-y-3">
          <Label htmlFor="anthropic-model">Anthropic Model</Label>
          <Input
            id="anthropic-model"
            value={data.provider.anthropic.model || ""}
            onChange={(e) =>
              onChange({
                ...data,
                provider: {
                  anthropic: {
                    model: e.target.value || null,
                  },
                },
              })
            }
            placeholder="claude-3-opus, claude-3-sonnet, etc. (optional)"
          />
        </div>
      )}

      {data.provider?.bedrock && (
        <div className="space-y-3">
          <Label htmlFor="bedrock-model">Model *</Label>
          <Input
            id="bedrock-model"
            value={data.provider.bedrock.model || ""}
            onChange={(e) =>
              onChange({
                ...data,
                provider: {
                  bedrock: {
                    ...data.provider.bedrock,
                    model: e.target.value,
                  },
                },
              })
            }
            placeholder="anthropic.claude-3-opus-20240229-v1:0"
          />
          <Label htmlFor="bedrock-region">Region *</Label>
          <Input
            id="bedrock-region"
            value={data.provider.bedrock.region || ""}
            onChange={(e) =>
              onChange({
                ...data,
                provider: {
                  bedrock: {
                    ...data.provider.bedrock,
                    region: e.target.value,
                  },
                },
              })
            }
            placeholder="us-east-1, us-west-2, etc."
          />
        </div>
      )}

      {data.provider?.azureOpenAI && (
        <div className="space-y-3">
          <Label htmlFor="azureopenai-host">Host *</Label>
          <Input
            id="azureopenai-host"
            value={data.provider.azureOpenAI.host || ""}
            onChange={(e) =>
              onChange({
                ...data,
                provider: {
                  azureOpenAI: {
                    ...data.provider.azureOpenAI,
                    host: e.target.value,
                  },
                },
              })
            }
            placeholder="my-resource-name.openai.azure.com"
          />
          <Label htmlFor="azureopenai-model">Model</Label>
          <Input
            id="azureopenai-model"
            value={data.provider.azureOpenAI.model || ""}
            onChange={(e) =>
              onChange({
                ...data,
                provider: {
                  azureOpenAI: {
                    ...data.provider.azureOpenAI,
                    model: e.target.value || null,
                  },
                },
              })
            }
            placeholder="gpt-4o, gpt-4.1-mini, etc. (optional)"
          />
          <Label htmlFor="azureopenai-api-version">API Version</Label>
          <Input
            id="azureopenai-api-version"
            value={data.provider.azureOpenAI.apiVersion || ""}
            onChange={(e) =>
              onChange({
                ...data,
                provider: {
                  azureOpenAI: {
                    ...data.provider.azureOpenAI,
                    apiVersion: e.target.value,
                  },
                },
              })
            }
            placeholder="v1, preview, 2024-10-21, etc. (optional, defaults to v1)"
          />
        </div>
      )}

      {/* Host Override */}
      <div className="space-y-3">
        <Label htmlFor="host-override">Host Override (Optional)</Label>
        <p className="text-sm text-muted-foreground">
          Override the default host for the AI provider
        </p>
        <Input
          id="host-override"
          value={data.hostOverride}
          onChange={(e) =>
            onChange({
              ...data,
              hostOverride: e.target.value,
            })
          }
          placeholder="api.custom-ai-provider.com:443"
        />
      </div>

      {/* Path Override */}
      <div className="space-y-3">
        <Label htmlFor="path-override">Path Override (Optional)</Label>
        <p className="text-sm text-muted-foreground">
          Override the default path for the AI provider
        </p>
        <Input
          id="path-override"
          value={data.pathOverride}
          onChange={(e) =>
            onChange({
              ...data,
              pathOverride: e.target.value,
            })
          }
          placeholder="/v1/chat/completions"
        />
      </div>
    </div>
  );
}

export function renderA2aForm({ data: _data, onChange: _onChange }: FormRendererProps) {
  return (
    <div className="space-y-6">
      <div className="bg-muted/50 border rounded-lg p-4">
        <div className="flex items-start gap-3">
          <div className="flex-shrink-0 w-6 h-6 bg-primary/10 rounded-full flex items-center justify-center mt-0.5">
            <span className="text-primary text-xs font-semibold">i</span>
          </div>
          <div className="space-y-2">
            <h4 className="font-medium text-sm">Agent-to-Agent Policy</h4>
            <p className="text-sm text-muted-foreground">
              This policy marks traffic as Agent-to-Agent (A2A) to enable A2A processing and
              telemetry. No additional configuration is required - simply enabling this policy will
              activate A2A features for this route.
            </p>
            <p className="text-sm text-muted-foreground">
              Use this policy when you want to enable specialized handling for agent-to-agent
              communications, including enhanced telemetry, logging, and processing optimizations.
            </p>
          </div>
        </div>
      </div>
    </div>
  );
}
