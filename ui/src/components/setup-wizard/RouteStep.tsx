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
import { ArrowLeft, ArrowRight, Route as RouteIcon, Plus, X } from "lucide-react";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import { LocalConfig, Route } from "@/lib/types";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Badge } from "@/components/ui/badge";

interface RouteStepProps {
  onNext: () => void;
  onPrevious: () => void;
  config: LocalConfig;
  onConfigChange: (config: LocalConfig) => void;
}

export function RouteStep({ onNext, onPrevious, config, onConfigChange }: RouteStepProps) {
  const [routeName, setRouteName] = useState("default");
  const [pathType, setPathType] = useState<"exact" | "prefix" | "regex">("prefix");
  const [pathValue, setPathValue] = useState("/");
  const [hostnames, setHostnames] = useState<string[]>(["*"]);
  const [newHostname, setNewHostname] = useState("");
  const [httpMethods, setHttpMethods] = useState<string[]>(["GET", "POST"]);
  const [newMethod, setNewMethod] = useState("");
  const [isUpdating, setIsUpdating] = useState(false);

  const addHostname = () => {
    if (newHostname.trim() && !hostnames.includes(newHostname.trim())) {
      setHostnames([...hostnames, newHostname.trim()]);
      setNewHostname("");
    }
  };

  const removeHostname = (hostname: string) => {
    setHostnames(hostnames.filter((h) => h !== hostname));
  };

  const addMethod = () => {
    if (newMethod.trim() && !httpMethods.includes(newMethod.trim().toUpperCase())) {
      setHttpMethods([...httpMethods, newMethod.trim().toUpperCase()]);
      setNewMethod("");
    }
  };

  const removeMethod = (method: string) => {
    setHttpMethods(httpMethods.filter((m) => m !== method));
  };

  const handleNext = async () => {
    if (!routeName.trim()) {
      toast.error("Route name is required.");
      return;
    }
    if (!pathValue.trim()) {
      toast.error("Path value is required.");
      return;
    }
    if (hostnames.length === 0) {
      toast.error("At least one hostname is required.");
      return;
    }

    setIsUpdating(true);

    try {
      // Create the route configuration
      const pathMatch =
        pathType === "exact"
          ? { exact: pathValue }
          : pathType === "prefix"
            ? { pathPrefix: pathValue }
            : { regex: [pathValue, 0] as [string, number] };

      const newRoute: Route = {
        name: routeName,
        hostnames,
        matches: [
          {
            path: pathMatch,
            method: httpMethods.length > 0 ? { method: httpMethods.join("|") } : undefined,
          },
        ],
        backends: [], // Will be configured in the next step
      };

      // Update the first listener with the route
      const newConfig = { ...config };
      if (newConfig.binds && newConfig.binds.length > 0) {
        const firstBind = newConfig.binds[0];
        if (firstBind.listeners && firstBind.listeners.length > 0) {
          const firstListener = firstBind.listeners[0];
          newConfig.binds[0].listeners[0] = {
            ...firstListener,
            routes: [...(firstListener.routes || []), newRoute],
          };
        }
      }

      onConfigChange(newConfig);
      toast.success("Route configured successfully!");
      onNext();
    } catch (err) {
      console.error("Error configuring route:", err);
      toast.error(err instanceof Error ? err.message : "Failed to configure route");
    } finally {
      setIsUpdating(false);
    }
  };

  const commonMethods = ["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"];

  return (
    <Card className="w-full max-w-3xl" data-cy="wizard-route-step">
      <CardHeader>
        <div className="flex justify-center mb-6">
          <AgentgatewayLogo className="h-12" />
        </div>
        <CardTitle className="text-center flex items-center justify-center gap-2">
          <RouteIcon className="h-5 w-5 text-green-500" />
          Configure Route
        </CardTitle>
        <CardDescription className="text-center">
          Define how incoming requests should be matched and routed
        </CardDescription>
      </CardHeader>
      <CardContent>
        <div className="space-y-6">
          <div className="space-y-3">
            <h3 className="font-medium">What is a Route?</h3>
            <p className="text-sm text-muted-foreground">
              A route defines rules for matching incoming requests based on paths, hostnames, and
              HTTP methods. When a request matches a route, it will be forwarded to the configured
              backends.
            </p>
          </div>

          <div className="space-y-4">
            <div className="space-y-3">
              <Label htmlFor="routeName">Route Name</Label>
              <Input
                id="routeName"
                value={routeName}
                onChange={(e) => setRouteName(e.target.value)}
                placeholder="e.g., default"
                data-cy="route-name-input"
              />
              <p className="text-xs text-muted-foreground">A unique name for this route.</p>
            </div>

            <div className="space-y-3">
              <Label>Path Matching</Label>
              <RadioGroup
                value={pathType}
                onValueChange={(value) => setPathType(value as "exact" | "prefix" | "regex")}
                className="grid grid-cols-3 gap-4"
                data-cy="route-match-type-select"
              >
                <div className="flex items-center space-x-2">
                  <RadioGroupItem value="exact" id="exact-match" />
                  <Label htmlFor="exact-match">Exact</Label>
                </div>
                <div className="flex items-center space-x-2">
                  <RadioGroupItem value="prefix" id="prefix-match" />
                  <Label htmlFor="prefix-match">Prefix</Label>
                </div>
                <div className="flex items-center space-x-2">
                  <RadioGroupItem value="regex" id="regex-match" />
                  <Label htmlFor="regex-match">Regex</Label>
                </div>
              </RadioGroup>

              <Input
                value={pathValue}
                onChange={(e) => setPathValue(e.target.value)}
                placeholder={
                  pathType === "exact"
                    ? "/api/v1/users"
                    : pathType === "prefix"
                      ? "/api/"
                      : "/api/.+"
                }
                data-cy="route-path-input"
              />
              <p className="text-xs text-muted-foreground">
                {pathType === "exact" && "Match the exact path"}
                {pathType === "prefix" && "Match paths starting with this prefix"}
                {pathType === "regex" && "Match paths using regular expression"}
              </p>
            </div>

            <div className="space-y-3">
              <Label>Hostnames</Label>
              <div className="flex flex-wrap gap-2 mb-2">
                {hostnames.map((hostname) => (
                  <Badge key={hostname} variant="secondary" className="flex items-center gap-1">
                    {hostname}
                    <X
                      className="h-3 w-3 cursor-pointer"
                      onClick={() => removeHostname(hostname)}
                    />
                  </Badge>
                ))}
              </div>
              <div className="flex gap-2">
                <Input
                  value={newHostname}
                  onChange={(e) => setNewHostname(e.target.value)}
                  placeholder="e.g., api.example.com or *"
                  onKeyPress={(e) => e.key === "Enter" && addHostname()}
                  data-cy="route-hostname-input"
                />
                <Button
                  type="button"
                  variant="outline"
                  onClick={addHostname}
                  data-cy="route-add-hostname-button"
                >
                  <Plus className="h-4 w-4" />
                </Button>
              </div>
              <p className="text-xs text-muted-foreground">
                Hostnames to match. Use * for all hostnames.
              </p>
            </div>

            <div className="space-y-3">
              <Label>HTTP Methods (Optional)</Label>
              <div className="flex flex-wrap gap-2 mb-2">
                {httpMethods.map((method) => (
                  <Badge key={method} variant="secondary" className="flex items-center gap-1">
                    {method}
                    <X className="h-3 w-3 cursor-pointer" onClick={() => removeMethod(method)} />
                  </Badge>
                ))}
              </div>
              <div className="flex gap-2 mb-2">
                <Input
                  value={newMethod}
                  onChange={(e) => setNewMethod(e.target.value)}
                  placeholder="Enter HTTP method"
                  onKeyPress={(e) => e.key === "Enter" && addMethod()}
                  data-cy="route-method-input"
                />
                <Button
                  type="button"
                  variant="outline"
                  onClick={addMethod}
                  data-cy="route-add-method-button"
                >
                  <Plus className="h-4 w-4" />
                </Button>
              </div>
              <div className="flex flex-wrap gap-2">
                {commonMethods.map((method) => (
                  <Button
                    key={method}
                    type="button"
                    variant={httpMethods.includes(method) ? "default" : "outline"}
                    size="sm"
                    onClick={() => {
                      if (httpMethods.includes(method)) {
                        removeMethod(method);
                      } else {
                        setHttpMethods([...httpMethods, method]);
                      }
                    }}
                  >
                    {method}
                  </Button>
                ))}
              </div>
              <p className="text-xs text-muted-foreground">
                HTTP methods to match. Leave empty to match all methods.
              </p>
            </div>
          </div>

          <div className="p-4 bg-muted/30 rounded-lg">
            <h4 className="font-medium text-sm mb-2">Preview</h4>
            <p className="text-sm text-muted-foreground">
              This route will match requests to:{" "}
              <code className="bg-muted px-1 py-0.5 rounded text-xs">
                {httpMethods.length > 0 ? httpMethods.join("|") : "ANY"} {hostnames.join(", ")}
                {pathType === "exact"
                  ? pathValue
                  : pathType === "prefix"
                    ? `${pathValue}*`
                    : `regex(${pathValue})`}
              </code>
            </p>
          </div>
        </div>
      </CardContent>
      <CardFooter className="flex justify-between">
        <Button variant="outline" onClick={onPrevious} data-cy="wizard-route-previous">
          <ArrowLeft className="mr-2 h-4 w-4" />
          Back
        </Button>
        <Button onClick={handleNext} disabled={isUpdating} data-cy="wizard-route-next">
          {isUpdating ? "Configuring..." : "Next"}
          <ArrowRight className="ml-2 h-4 w-4" />
        </Button>
      </CardFooter>
    </Card>
  );
}
