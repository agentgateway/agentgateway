import { useState } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { AgentgatewayLogo } from "@/components/agentgateway-logo";
import { ArrowLeft, ArrowRight, Globe, Server, MessageSquare } from "lucide-react";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import { LocalConfig, Backend, McpBackend, McpTarget, McpStatefulMode } from "@/lib/types";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Textarea } from "@/components/ui/textarea";

interface BackendStepProps {
  onNext: () => void;
  onPrevious: () => void;
  config: LocalConfig;
  onConfigChange: (config: LocalConfig) => void;
}

export function BackendStep({ onNext, onPrevious, config, onConfigChange }: BackendStepProps) {
  const [backendType, setBackendType] = useState<"mcp" | "host" | "service">("mcp");
  const [mcpStateful, setMcpStateful] = useState(true); // Default to stateful
  const [targetType, setTargetType] = useState<"mcp" | "stdio" | "sse" | "openapi">("mcp");
  const [targetName, setTargetName] = useState("default-target");

  // MCP Connection Target
  const [mcpUrl, setMcpUrl] = useState("https://example.com/mcp");

  // Stdio Target
  const [stdioCmd, setStdioCmd] = useState("npx");
  const [stdioArgs, setStdioArgs] = useState("@modelcontextprotocol/server-everything");

  // SSE Target
  const [sseUrl, setSseUrl] = useState("https://example.com/sse");

  // OpenAPI Target
  const [openApiHost, setOpenApiHost] = useState("localhost");
  const [openApiPort, setOpenApiPort] = useState("8080");
  const [openApiSchema, setOpenApiSchema] = useState("{}");

  // Host Backend
  const [hostAddress, setHostAddress] = useState("localhost:8080");

  const [isUpdating, setIsUpdating] = useState(false);

  const handleNext = async () => {
    if (backendType === "mcp") {
      if (!targetName.trim()) {
        toast.error("Target name is required.");
        return;
      }
    } else if (backendType === "host") {
      if (!hostAddress.trim()) {
        toast.error("Host address is required.");
        return;
      }
    }

    setIsUpdating(true);

    try {
      let backend: Backend;

      if (backendType === "mcp") {
        let target: McpTarget;

        switch (targetType) {
          case "mcp":
            target = {
              name: targetName,
              mcp: {
                host: mcpUrl,
              },
            } as McpTarget;
            break;
          case "stdio":
            target = {
              name: targetName,
              stdio: {
                cmd: stdioCmd,
                args: stdioArgs.split(" ").filter((arg) => arg.trim()),
              },
            };
            break;
          case "sse":
            target = {
              name: targetName,
              sse: {
                host: sseUrl,
              },
            };
            break;
          case "openapi": {
            let schema;
            try {
              schema = JSON.parse(openApiSchema);
            } catch {
              toast.error("Invalid OpenAPI schema JSON");
              return;
            }
            const openApiPortValue = openApiPort.trim() ? parseInt(openApiPort, 10) : undefined;
            target = {
              name: targetName,
              openapi: {
                host: openApiHost,
                port: Number.isFinite(openApiPortValue) ? openApiPortValue : undefined,
                schema,
              },
            };
            break;
          }
          default:
            throw new Error("Invalid target type");
        }

        const mcpBackend: McpBackend = {
          targets: [target],
          statefulMode: mcpStateful ? McpStatefulMode.STATEFUL : McpStatefulMode.STATELESS,
        };

        backend = {
          weight: 1,
          mcp: mcpBackend,
        };
      } else if (backendType === "host") {
        backend = {
          weight: 1,
          host: {
            Address: hostAddress,
          },
        };
      } else {
        throw new Error("Invalid backend type");
      }

      // Update the first route's backends
      const newConfig = { ...config };
      if (newConfig.binds && newConfig.binds.length > 0) {
        const firstBind = newConfig.binds[0];
        if (firstBind.listeners && firstBind.listeners.length > 0) {
          const firstListener = firstBind.listeners[0];
          if (firstListener.routes && firstListener.routes.length > 0) {
            const firstRoute = firstListener.routes[0];
            newConfig.binds[0].listeners[0].routes![0] = {
              ...firstRoute,
              backends: [...(firstRoute.backends || []), backend],
            };
          }
        }
      }

      onConfigChange(newConfig);
      toast.success("Backend configured successfully!");
      onNext();
    } catch (err) {
      console.error("Error configuring backend:", err);
      toast.error(err instanceof Error ? err.message : "Failed to configure backend");
    } finally {
      setIsUpdating(false);
    }
  };

  const renderBackendConfig = () => {
    if (backendType === "mcp") {
      return (
        <div className="space-y-4">
          <div className="space-y-3">
            <Label>Target Type</Label>
            <RadioGroup
              value={targetType}
              onValueChange={(value) => setTargetType(value as "mcp" | "stdio" | "sse" | "openapi")}
              className="grid grid-cols-2 gap-4"
            >
              <div className="flex items-center space-x-2">
                <RadioGroupItem value="mcp" id="mcp-target" />
                <Label htmlFor="mcp-target">MCP Connection</Label>
              </div>
              <div className="flex items-center space-x-2">
                <RadioGroupItem value="stdio" id="stdio-target" />
                <Label htmlFor="stdio-target">Stdio</Label>
              </div>
              <div className="flex items-center space-x-2">
                <RadioGroupItem value="sse" id="sse-target" />
                <Label htmlFor="sse-target">SSE</Label>
              </div>
              <div className="flex items-center space-x-2">
                <RadioGroupItem value="openapi" id="openapi-target" />
                <Label htmlFor="openapi-target">OpenAPI</Label>
              </div>
            </RadioGroup>
          </div>

          <div className="space-y-3">
            <Label htmlFor="targetName">Target Name</Label>
            <Input
              id="targetName"
              value={targetName}
              onChange={(e) => setTargetName(e.target.value)}
              placeholder="e.g., default-target"
            />
          </div>

          <div className="space-y-1">
            <Label>
              <input
                type="checkbox"
                checked={mcpStateful}
                onChange={(e) => setMcpStateful(e.target.checked)}
                className="mr-2"
              />
              Enable Stateful MCP
            </Label>
            <p className="text-xs text-muted-foreground">
              If enabled, the MCP backend will maintain state across requests.
            </p>
          </div>

          {targetType === "mcp" && (
            <div className="space-y-3">
              <div>
                <Label htmlFor="mcpUrl">MCP URL</Label>
                <Input
                  id="mcpUrl"
                  value={mcpUrl}
                  onChange={(e) => setMcpUrl(e.target.value)}
                  placeholder="https://example.com/mcp"
                />
              </div>
            </div>
          )}

          {targetType === "stdio" && (
            <div className="space-y-3">
              <div>
                <Label htmlFor="stdioCmd">Command</Label>
                <Input
                  id="stdioCmd"
                  value={stdioCmd}
                  onChange={(e) => setStdioCmd(e.target.value)}
                  placeholder="npx"
                />
              </div>
              <div>
                <Label htmlFor="stdioArgs">Arguments</Label>
                <Input
                  id="stdioArgs"
                  value={stdioArgs}
                  onChange={(e) => setStdioArgs(e.target.value)}
                  placeholder="@modelcontextprotocol/server-everything"
                />
              </div>
            </div>
          )}

          {targetType === "sse" && (
            <div className="space-y-3">
              <div>
                <Label htmlFor="sseUrl">SSE URL</Label>
                <Input
                  id="sseUrl"
                  value={sseUrl}
                  onChange={(e) => setSseUrl(e.target.value)}
                  placeholder="https://example.com/sse"
                />
              </div>
            </div>
          )}

          {targetType === "openapi" && (
            <div className="space-y-3">
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <Label htmlFor="openApiHost">Host</Label>
                  <Input
                    id="openApiHost"
                    value={openApiHost}
                    onChange={(e) => setOpenApiHost(e.target.value)}
                    placeholder="localhost"
                  />
                </div>
                <div>
                  <Label htmlFor="openApiPort">Port</Label>
                  <Input
                    id="openApiPort"
                    type="number"
                    value={openApiPort}
                    onChange={(e) => setOpenApiPort(e.target.value)}
                    placeholder="8080"
                  />
                </div>
              </div>
              <div>
                <Label htmlFor="openApiSchema">OpenAPI Schema (JSON)</Label>
                <Textarea
                  id="openApiSchema"
                  value={openApiSchema}
                  onChange={(e) => setOpenApiSchema(e.target.value)}
                  placeholder='{"openapi": "3.0.0", "info": {"title": "API", "version": "1.0.0"}}'
                  rows={4}
                />
              </div>
            </div>
          )}
        </div>
      );
    } else if (backendType === "host") {
      return (
        <div className="space-y-4">
          <div className="space-y-3">
            <Label htmlFor="hostAddress">Host Address</Label>
            <Input
              id="hostAddress"
              value={hostAddress}
              onChange={(e) => setHostAddress(e.target.value)}
              placeholder="localhost:8080"
            />
            <p className="text-xs text-muted-foreground">
              The host address to forward requests to (host:port).
            </p>
          </div>
        </div>
      );
    }
  };

  return (
    <Card className="w-full max-w-3xl">
      <CardHeader>
        <div className="flex justify-center mb-6">
          <AgentgatewayLogo className="h-12" />
        </div>
        <CardTitle className="text-center flex items-center justify-center gap-2">
          <Globe className="h-5 w-5 text-orange-500" />
          Configure Backend
        </CardTitle>
        <CardDescription className="text-center">
          Set up where your requests will be forwarded to
        </CardDescription>
      </CardHeader>
      <CardContent>
        <div className="space-y-6">
          <div className="space-y-3">
            <h3 className="font-medium">What is a Backend?</h3>
            <p className="text-sm text-muted-foreground">
              A backend defines where matching requests are sent. You can configure different types
              of backends like MCP (Model Context Protocol), direct host connections, or Kubernetes
              services.
            </p>
          </div>

          <div className="space-y-4">
            <div className="space-y-3">
              <Label>Backend Type</Label>
              <RadioGroup
                value={backendType}
                onValueChange={(value) => setBackendType(value as "mcp" | "host" | "service")}
                className="grid grid-cols-2 gap-4"
              >
                <div className="flex items-center space-x-2">
                  <RadioGroupItem value="mcp" id="mcp-backend" />
                  <Label htmlFor="mcp-backend" className="flex items-center gap-2">
                    <MessageSquare className="h-4 w-4" />
                    MCP Backend
                  </Label>
                </div>
                <div className="flex items-center space-x-2">
                  <RadioGroupItem value="host" id="host-backend" />
                  <Label htmlFor="host-backend" className="flex items-center gap-2">
                    <Server className="h-4 w-4" />
                    Host Backend
                  </Label>
                </div>
              </RadioGroup>
            </div>

            {renderBackendConfig()}
          </div>

          <div className="p-4 bg-muted/30 rounded-lg">
            <h4 className="font-medium text-sm mb-2">Preview</h4>
            <p className="text-sm text-muted-foreground">
              Requests will be forwarded to:{" "}
              <code className="bg-muted px-1 py-0.5 rounded text-xs">
                {backendType === "mcp"
                  ? targetType === "mcp"
                    ? mcpUrl
                    : targetType === "stdio"
                      ? stdioCmd + " " + stdioArgs
                      : targetType === "sse"
                        ? sseUrl
                        : openApiHost + ":" + openApiPort
                  : hostAddress}
              </code>
            </p>
          </div>
        </div>
      </CardContent>
      <CardFooter className="flex justify-between">
        <Button variant="outline" onClick={onPrevious}>
          <ArrowLeft className="mr-2 h-4 w-4" />
          Back
        </Button>
        <Button onClick={handleNext} disabled={isUpdating}>
          {isUpdating ? "Configuring..." : "Next"}
          <ArrowRight className="ml-2 h-4 w-4" />
        </Button>
      </CardFooter>
    </Card>
  );
}
