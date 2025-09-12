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
import { ArrowLeft, ArrowRight, Shield, Clock, Globe, Key } from "lucide-react";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import { LocalConfig, JwtAuth, CorsPolicy, TimeoutPolicy } from "@/lib/types";
import { Checkbox } from "@/components/ui/checkbox";
import { Badge } from "@/components/ui/badge";

interface PolicyStepProps {
  onNext: () => void;
  onPrevious: () => void;
  config: LocalConfig;
  onConfigChange: (config: LocalConfig) => void;
}

export function PolicyStep({ onNext, onPrevious, config, onConfigChange }: PolicyStepProps) {
  const [enableJwt, setEnableJwt] = useState(false);
  const [enableCors, setEnableCors] = useState(false);
  const [enableTimeout, setEnableTimeout] = useState(false);

  // JWT Config
  const [jwtIssuer, setJwtIssuer] = useState("");
  const [jwtAudiences, setJwtAudiences] = useState("");
  const [jwtJwks, setJwtJwks] = useState("");

  // CORS Config
  const [corsOrigins, setCorsOrigins] = useState("*");
  const [corsMethods, setCorsMethods] = useState("GET,POST,PUT,DELETE,OPTIONS");
  const [corsHeaders, setCorsHeaders] = useState("Content-Type,Authorization");
  const [corsCredentials, setCorsCredentials] = useState(false);

  // Timeout Config
  const [requestTimeout, setRequestTimeout] = useState("30s");
  const [backendTimeout, setBackendTimeout] = useState("15s");

  const [isUpdating, setIsUpdating] = useState(false);

  const handleNext = async () => {
    if (enableJwt && (!jwtIssuer.trim() || !jwtAudiences.trim() || !jwtJwks.trim())) {
      toast.error("JWT configuration is incomplete. Please fill all JWT fields or disable JWT.");
      return;
    }

    setIsUpdating(true);

    try {
      const newConfig = { ...config };

      // Update the first route's policies
      if (newConfig.binds && newConfig.binds.length > 0) {
        const firstBind = newConfig.binds[0];
        if (firstBind.listeners && firstBind.listeners.length > 0) {
          const firstListener = firstBind.listeners[0];
          if (firstListener.routes && firstListener.routes.length > 0) {
            const firstRoute = firstListener.routes[0];

            const policies: any = {};

            // Add JWT Auth
            if (enableJwt) {
              // Detect if JWKS is a URL or file path
              const isJwksUrl =
                jwtJwks.trim().startsWith("http://") || jwtJwks.trim().startsWith("https://");

              const jwtAuth: JwtAuth = {
                issuer: jwtIssuer,
                audiences: jwtAudiences.split(",").map((a) => a.trim()),
                jwks: isJwksUrl ? { url: jwtJwks } : { file: jwtJwks },
              };
              policies.jwtAuth = jwtAuth;
            }

            // Add CORS
            if (enableCors) {
              const corsPolicy: CorsPolicy = {
                allowOrigins: corsOrigins.split(",").map((o) => o.trim()),
                allowMethods: corsMethods.split(",").map((m) => m.trim()),
                allowHeaders: corsHeaders.split(",").map((h) => h.trim()),
                allowCredentials: corsCredentials,
              };
              policies.cors = corsPolicy;
            }

            // Add Timeout
            if (enableTimeout) {
              const timeoutPolicy: TimeoutPolicy = {
                requestTimeout: requestTimeout,
                backendRequestTimeout: backendTimeout,
              };
              policies.timeout = timeoutPolicy;
            }

            if (Object.keys(policies).length > 0) {
              newConfig.binds[0].listeners[0].routes![0] = {
                ...firstRoute,
                policies,
              };
            }
          }
        }
      }

      onConfigChange(newConfig);
      toast.success("Policies configured successfully!");
      onNext();
    } catch (err) {
      console.error("Error configuring policies:", err);
      toast.error(err instanceof Error ? err.message : "Failed to configure policies");
    } finally {
      setIsUpdating(false);
    }
  };

  return (
    <Card className="w-full max-w-3xl">
      <CardHeader>
        <div className="flex justify-center mb-6">
          <AgentgatewayLogo className="h-12" />
        </div>
        <CardTitle className="text-center flex items-center justify-center gap-2">
          <Shield className="h-5 w-5 text-red-500" />
          Configure Policies (Optional)
        </CardTitle>
        <CardDescription className="text-center">
          Add security, traffic management, and routing policies
        </CardDescription>
      </CardHeader>
      <CardContent>
        <div className="space-y-6">
          <div className="space-y-3">
            <h3 className="font-medium">What are Policies?</h3>
            <p className="text-sm text-muted-foreground">
              Policies control how requests are processed. You can add authentication, CORS headers,
              timeouts, and other rules. All policies are optional and can be configured later.
            </p>
          </div>

          <div className="space-y-6">
            {/* JWT Authentication */}
            <div className="border rounded-lg p-4">
              <div className="flex items-center space-x-2 mb-4">
                <Checkbox
                  id="enable-jwt"
                  checked={enableJwt}
                  onCheckedChange={(checked: boolean) => setEnableJwt(checked)}
                />
                <Label htmlFor="enable-jwt" className="flex items-center gap-2 font-medium">
                  <Key className="h-4 w-4" />
                  JWT Authentication
                </Label>
              </div>

              {enableJwt && (
                <div className="space-y-4 ml-6">
                  <div className="space-y-3">
                    <Label htmlFor="jwt-issuer">JWT Issuer *</Label>
                    <Input
                      id="jwt-issuer"
                      value={jwtIssuer}
                      onChange={(e) => setJwtIssuer(e.target.value)}
                      placeholder="https://your-auth-provider.com/"
                    />
                  </div>
                  <div className="space-y-3">
                    <Label htmlFor="jwt-audiences">Audiences (comma-separated) *</Label>
                    <Input
                      id="jwt-audiences"
                      value={jwtAudiences}
                      onChange={(e) => setJwtAudiences(e.target.value)}
                      placeholder="your-api,another-audience"
                    />
                  </div>
                  <div className="space-y-3">
                    <Label htmlFor="jwt-jwks">JWKS URL or File Path *</Label>
                    <Input
                      id="jwt-jwks"
                      value={jwtJwks}
                      onChange={(e) => setJwtJwks(e.target.value)}
                      placeholder="https://your-auth-provider.com/.well-known/jwks.json"
                    />
                  </div>
                </div>
              )}
            </div>

            {/* CORS */}
            <div className="border rounded-lg p-4">
              <div className="flex items-center space-x-2 mb-4">
                <Checkbox
                  id="enable-cors"
                  checked={enableCors}
                  onCheckedChange={(checked: boolean) => setEnableCors(checked)}
                />
                <Label htmlFor="enable-cors" className="flex items-center gap-2 font-medium">
                  <Globe className="h-4 w-4" />
                  CORS Headers
                </Label>
              </div>

              {enableCors && (
                <div className="space-y-4 ml-6">
                  <div className="space-y-3">
                    <Label htmlFor="cors-origins">Allowed Origins</Label>
                    <Input
                      id="cors-origins"
                      value={corsOrigins}
                      onChange={(e) => setCorsOrigins(e.target.value)}
                      placeholder="*"
                    />
                  </div>
                  <div className="space-y-3">
                    <Label htmlFor="cors-methods">Allowed Methods</Label>
                    <Input
                      id="cors-methods"
                      value={corsMethods}
                      onChange={(e) => setCorsMethods(e.target.value)}
                      placeholder="GET,POST,PUT,DELETE,OPTIONS"
                    />
                  </div>
                  <div className="space-y-3">
                    <Label htmlFor="cors-headers">Allowed Headers</Label>
                    <Input
                      id="cors-headers"
                      value={corsHeaders}
                      onChange={(e) => setCorsHeaders(e.target.value)}
                      placeholder="Content-Type,Authorization"
                    />
                  </div>
                  <div className="flex items-center space-x-2">
                    <Checkbox
                      id="cors-credentials"
                      checked={corsCredentials}
                      onCheckedChange={(checked: boolean) => setCorsCredentials(checked)}
                    />
                    <Label htmlFor="cors-credentials">Allow Credentials</Label>
                  </div>
                </div>
              )}
            </div>

            {/* Timeout */}
            <div className="border rounded-lg p-4">
              <div className="flex items-center space-x-2 mb-4">
                <Checkbox
                  id="enable-timeout"
                  checked={enableTimeout}
                  onCheckedChange={(checked: boolean) => setEnableTimeout(checked)}
                />
                <Label htmlFor="enable-timeout" className="flex items-center gap-2 font-medium">
                  <Clock className="h-4 w-4" />
                  Request Timeout
                </Label>
              </div>

              {enableTimeout && (
                <div className="space-y-4 ml-6">
                  <div className="space-y-3">
                    <Label htmlFor="request-timeout">Request Timeout</Label>
                    <Input
                      id="request-timeout"
                      value={requestTimeout}
                      onChange={(e) => setRequestTimeout(e.target.value)}
                      placeholder="30s"
                    />
                    <p className="text-xs text-muted-foreground">
                      Maximum time to wait for a request (e.g., 30s, 1m)
                    </p>
                  </div>
                  <div className="space-y-3">
                    <Label htmlFor="backend-timeout">Backend Timeout</Label>
                    <Input
                      id="backend-timeout"
                      value={backendTimeout}
                      onChange={(e) => setBackendTimeout(e.target.value)}
                      placeholder="15s"
                    />
                    <p className="text-xs text-muted-foreground">
                      Maximum time to wait for backend response
                    </p>
                  </div>
                </div>
              )}
            </div>
          </div>

          <div className="p-4 bg-muted/30 rounded-lg">
            <h4 className="font-medium text-sm mb-2">Active Policies</h4>
            <div className="flex flex-wrap gap-2">
              {enableJwt && <Badge className="text-xs">JWT Authentication</Badge>}
              {enableCors && <Badge className="text-xs">CORS Headers</Badge>}
              {enableTimeout && <Badge className="text-xs">Request Timeout</Badge>}
              {!enableJwt && !enableCors && !enableTimeout && (
                <span className="text-muted-foreground text-sm">No policies configured</span>
              )}
            </div>
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
