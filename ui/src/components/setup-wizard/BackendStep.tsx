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
import { LocalConfig, Backend, McpBackend, McpTarget } from "@/lib/types";
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
  const [mcpName, setMcpName] = useState("default-mcp");
  const [targetType, setTargetType] = useState<"mcp" | "stdio" | "sse" | "openapi">("mcp");
  const [targetName, setTargetName] = useState("default-target");

  // MCP Connection Target
  const [mcpHost, setMcpHost] = useState("localhost");
  const [mcpPort, setMcpPort] = useState("3000");
  const [mcpPath, setMcpPath] = useState("/mcp");

  // Stdio Target
  const [stdioCmd, setStdioCmd] = useState("npx");
  const [stdioArgs, setStdioArgs] = useState("@modelcontextprotocol/server-everything");

  // SSE Target
  const [sseHost, setSseHost] = useState("localhost");
  const [ssePort, setSsePort] = useState("8080");
  const [ssePath, setSsePath] = useState("/events");

  // OpenAPI Target
  const [openApiHost, setOpenApiHost] = useState("localhost");
  const [openApiPort, setOpenApiPort] = useState("8080");
  const [openApiSchema, setOpenApiSchema] = useState("{}");

  // Host Backend
  const [hostAddress, setHostAddress] = useState("localhost:8080");

  const [isUpdating, setIsUpdating] = useState(false);

  const handleNext = async () => {
    if (backendType === "mcp") {
      if (!mcpName.trim()) {
        toast.error("MCP backend name is required.");
        return;
      }
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
                host: mcpHost,
                port: parseInt(mcpPort),
                path: mcpPath,
              },
            };
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
                host: sseHost,
                port: parseInt(ssePort),
                path: ssePath,
              },
            };
            break;
          case "openapi":
            let schema;
            try {
              schema = JSON.parse(openApiSchema);
            } catch {
              toast.error("Invalid OpenAPI schema JSON");
              return;
            }
            target = {
              name: targetName,
              openapi: {
                host: openApiHost,
                port: parseInt(openApiPort),
                schema,
              },
            };
            break;
          default:
            throw new Error("Invalid target type");
        }

        const mcpBackend: McpBackend = {
          name: mcpName,
          targets: [target],
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
            <Label htmlFor="mcpName">MCP Backend Name</Label>
            <Input
              id="mcpName"
              value={mcpName}
              onChange={(e) => setMcpName(e.target.value)}
              placeholder="e.g., default-mcp"
              data-cy="backend-name-input"
            />
          </div>

          <div className="space-y-3">
            <Label>Target Type</Label>
            <RadioGroup
              value={targetType}
              onValueChange={(value) => setTargetType(value as "mcp" | "stdio" | "sse" | "openapi")}
              className="grid grid-cols-2 gap-4"
              data-cy="backend-target-type-select"
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
              data-cy="backend-target-name-input"
            />
          </div>

          {targetType === "mcp" && (
            <div className="space-y-3">
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <Label htmlFor="mcpHost">Host</Label>
                  <Input
                    id="mcpHost"
                    value={mcpHost}
                    onChange={(e) => setMcpHost(e.target.value)}
                    placeholder="localhost"
                  />
                </div>
                <div>
                  <Label htmlFor="mcpPort">Port</Label>
                  <Input
                    id="mcpPort"
                    type="number"
                    value={mcpPort}
                    onChange={(e) => setMcpPort(e.target.value)}
                    placeholder="3000"
                  />
                </div>
              </div>
              <div>
                <Label htmlFor="mcpPath">Path</Label>
                <Input
                  id="mcpPath"
                  value={mcpPath}
                  onChange={(e) => setMcpPath(e.target.value)}
                  placeholder="/mcp"
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
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <Label htmlFor="sseHost">Host</Label>
                  <Input
                    id="sseHost"
                    value={sseHost}
                    onChange={(e) => setSseHost(e.target.value)}
                    placeholder="localhost"
                  />
                </div>
                <div>
                  <Label htmlFor="ssePort">Port</Label>
                  <Input
                    id="ssePort"
                    type="number"
                    value={ssePort}
                    onChange={(e) => setSsePort(e.target.value)}
                    placeholder="8080"
                  />
                </div>
              </div>
              <div>
                <Label htmlFor="ssePath">Path</Label>
                <Input
                  id="ssePath"
                  value={ssePath}
                  onChange={(e) => setSsePath(e.target.value)}
                  placeholder="/events"
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
    <Card className="w-full max-w-3xl" data-cy="wizard-backend-step">
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
                data-cy="backend-type-select"
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
                  ? `${targetType}://${
                      targetType === "mcp"
                        ? mcpHost + ":" + mcpPort + mcpPath
                        : targetType === "stdio"
                          ? stdioCmd + " " + stdioArgs
                          : targetType === "sse"
                            ? sseHost + ":" + ssePort + ssePath
                            : openApiHost + ":" + openApiPort
                    }`
                  : hostAddress}
              </code>
            </p>
          </div>
        </div>
      </CardContent>
      <CardFooter className="flex justify-between">
        <Button variant="outline" onClick={onPrevious} data-cy="wizard-backend-previous">
          <ArrowLeft className="mr-2 h-4 w-4" />
          Back
        </Button>
        <Button onClick={handleNext} disabled={isUpdating} data-cy="wizard-backend-next">
          {isUpdating ? "Configuring..." : "Next"}
          <ArrowRight className="ml-2 h-4 w-4" />
        </Button>
      </CardFooter>
    </Card>
  );
}
