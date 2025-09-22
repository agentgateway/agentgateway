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
import { ArrowLeft, CheckCircle, Network, Route, Globe, Shield } from "lucide-react";
import { LocalConfig } from "@/lib/types";
import { Badge } from "@/components/ui/badge";
import { updateConfig } from "@/lib/api";

interface ReviewStepProps {
  onNext: () => void;
  onPrevious: () => void;
  config: LocalConfig;
  onConfigChange: (config: LocalConfig) => void;
}

export function ReviewStep({ onNext, onPrevious, config }: ReviewStepProps) {
  const [isCompleting, setIsCompleting] = useState(false);

  const handleComplete = async () => {
    setIsCompleting(true);
    try {
      await updateConfig(config);
      toast.success("Gateway configuration completed successfully!");
      onNext();
    } catch (err) {
      console.error("Error completing setup:", err);
      toast.error(err instanceof Error ? err.message : "Failed to complete setup");
    } finally {
      setIsCompleting(false);
    }
  };

  const getConfigSummary = () => {
    const summary = {
      listeners: 0,
      routes: 0,
      backends: 0,
      policies: 0,
      ports: new Set<number>(),
    };

    if (config.binds) {
      config.binds.forEach((bind) => {
        summary.ports.add(bind.port);
        if (bind.listeners) {
          summary.listeners += bind.listeners.length;
          bind.listeners.forEach((listener) => {
            if (listener.routes) {
              summary.routes += listener.routes.length;
              listener.routes.forEach((route) => {
                if (route.backends) {
                  summary.backends += route.backends.length;
                }
                if (route.policies) {
                  summary.policies += Object.keys(route.policies).length;
                }
              });
            }
          });
        }
      });
    }

    return summary;
  };

  const summary = getConfigSummary();

  return (
    <Card className="w-full max-w-3xl">
      <CardHeader>
        <div className="flex justify-center mb-6">
          <AgentgatewayLogo className="h-12" />
        </div>
        <CardTitle className="text-center flex items-center justify-center gap-2">
          <CheckCircle className="h-5 w-5 text-green-500" />
          Review Configuration
        </CardTitle>
        <CardDescription className="text-center">
          Review your gateway configuration before completing the setup
        </CardDescription>
      </CardHeader>
      <CardContent>
        <div className="space-y-6">
          <div className="space-y-3">
            <h3 className="font-medium">Configuration Summary</h3>
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              <div className="flex items-center space-x-3 p-3 rounded-lg bg-muted/30 border">
                <Network className="h-5 w-5 text-primary" />
                <div>
                  <p className="font-medium text-sm">Listeners</p>
                  <p className="text-xs text-muted-foreground">
                    {summary.listeners} listener{summary.listeners !== 1 ? "s" : ""} on{" "}
                    {summary.ports.size} port{summary.ports.size !== 1 ? "s" : ""}
                  </p>
                </div>
              </div>
              <div className="flex items-center space-x-3 p-3 rounded-lg bg-muted/30 border">
                <Route className="h-5 w-5 text-green-500" />
                <div>
                  <p className="font-medium text-sm">Routes</p>
                  <p className="text-xs text-muted-foreground">
                    {summary.routes} route{summary.routes !== 1 ? "s" : ""} configured
                  </p>
                </div>
              </div>
              <div className="flex items-center space-x-3 p-3 rounded-lg bg-muted/30 border">
                <Globe className="h-5 w-5 text-orange-500" />
                <div>
                  <p className="font-medium text-sm">Backends</p>
                  <p className="text-xs text-muted-foreground">
                    {summary.backends} backend{summary.backends !== 1 ? "s" : ""} configured
                  </p>
                </div>
              </div>
              <div className="flex items-center space-x-3 p-3 rounded-lg bg-muted/30 border">
                <Shield className="h-5 w-5 text-red-500" />
                <div>
                  <p className="font-medium text-sm">Policies</p>
                  <p className="text-xs text-muted-foreground">
                    {summary.policies} polic{summary.policies !== 1 ? "ies" : "y"} configured
                  </p>
                </div>
              </div>
            </div>
          </div>

          {config.binds && config.binds.length > 0 && (
            <div className="space-y-3">
              <h3 className="font-medium">Detailed Configuration</h3>
              <div className="space-y-4">
                {config.binds.map((bind, bindIndex) => (
                  <div key={bindIndex} className="border rounded-lg p-4">
                    <div className="flex items-center justify-between mb-3">
                      <h4 className="font-medium text-sm">Port {bind.port}</h4>
                      <Badge>
                        {bind.listeners?.length || 0} listener
                        {bind.listeners?.length !== 1 ? "s" : ""}
                      </Badge>
                    </div>

                    {bind.listeners?.map((listener, listenerIndex) => (
                      <div
                        key={listenerIndex}
                        className="ml-4 border-l-2 border-muted pl-4 space-y-2"
                      >
                        <div className="flex items-center gap-2">
                          <Badge variant="outline">{listener.protocol}</Badge>
                          <span className="text-sm">{listener.name}</span>
                          <span className="text-xs text-muted-foreground">
                            {listener.hostname || "localhost"}
                          </span>
                        </div>

                        {listener.routes?.map((route, routeIndex) => (
                          <div key={routeIndex} className="ml-4 space-y-2">
                            <div className="flex items-center gap-2">
                              <Badge variant="outline" className="text-xs">
                                Route
                              </Badge>
                              <span className="text-sm">{route.name}</span>
                            </div>

                            {route.backends && route.backends.length > 0 && (
                              <div className="ml-4 flex flex-wrap gap-1">
                                {route.backends.map((backend, backendIndex) => (
                                  <Badge key={backendIndex} variant="secondary" className="text-xs">
                                    {backend.mcp
                                      ? "MCP"
                                      : backend.host
                                        ? "Host"
                                        : backend.service
                                          ? "Service"
                                          : "Backend"}
                                  </Badge>
                                ))}
                              </div>
                            )}

                            {route.policies && Object.keys(route.policies).length > 0 && (
                              <div className="ml-4 flex flex-wrap gap-1">
                                {Object.keys(route.policies).map((policyType) => (
                                  <Badge key={policyType} variant="outline" className="text-xs">
                                    {policyType}
                                  </Badge>
                                ))}
                              </div>
                            )}
                          </div>
                        ))}
                      </div>
                    ))}
                  </div>
                ))}
              </div>
            </div>
          )}

          <div className="p-4 bg-green-500/10 rounded-lg border border-green-500/20">
            <h4 className="font-medium text-sm mb-2 text-green-600 dark:text-green-400">
              ✅ Ready to Deploy
            </h4>
            <p className="text-sm text-green-700 dark:text-green-300">
              Your gateway configuration is complete and ready to be deployed. Click &quot;Complete
              Setup&quot; to save the configuration and start using your gateway.
            </p>
          </div>

          <div className="p-4 bg-primary/10 rounded-lg border border-primary/20">
            <h4 className="font-medium text-sm mb-2 text-primary">💡 Next Steps</h4>
            <ul className="text-sm text-primary space-y-1">
              <li>• Test your configuration with sample requests</li>
              <li>• Add more routes and backends as needed</li>
              <li>• Configure additional policies for security and performance</li>
              <li>• Monitor your gateway using the dashboard</li>
            </ul>
          </div>
        </div>
      </CardContent>
      <CardFooter className="flex justify-between">
        <Button variant="outline" onClick={onPrevious}>
          <ArrowLeft className="mr-2 h-4 w-4" />
          Back
        </Button>
        <Button
          onClick={handleComplete}
          disabled={isCompleting}
          className="bg-green-600 hover:bg-green-700 dark:bg-green-600 dark:hover:bg-green-700"
        >
          {isCompleting ? "Completing..." : "Complete Setup"}
          <CheckCircle className="ml-2 h-4 w-4" />
        </Button>
      </CardFooter>
    </Card>
  );
}
