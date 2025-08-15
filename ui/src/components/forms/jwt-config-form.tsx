"use client";

import { useState } from "react";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Listener } from "@/lib/types";
import { toast } from "sonner";

interface JWTConfigFormProps {
  listener: Listener | null;
  onSave: (updatedListener: Listener) => void;
  onCancel: () => void;
}

export function JWTConfigForm({ listener, onSave, onCancel }: JWTConfigFormProps) {
  const [config, setConfig] = useState(() => {
    // Initialize with existing JWT config if available
    const existingJwt = listener?.routes?.[0]?.policies?.jwtAuth;

    const getJwksInfo = (jwks: any) => {
      if (typeof jwks === "object" && jwks !== null) {
        if ("file" in jwks) {
          return { localJwksPath: jwks.file, remoteJwksUrl: "", jwksSource: "local" as const };
        } else if ("url" in jwks) {
          return { localJwksPath: "", remoteJwksUrl: jwks.url, jwksSource: "remote" as const };
        }
      }
      // Default to remote URL structure
      return { localJwksPath: "", remoteJwksUrl: "", jwksSource: "remote" as const };
    };

    const jwksInfo = getJwksInfo(existingJwt?.jwks);

    return {
      issuer: existingJwt?.issuer || "",
      audience: existingJwt?.audiences?.join(", ") || "",
      ...jwksInfo,
    };
  });

  const handleSave = () => {
    if (!listener) return;

    // Validate required fields
    if (!config.issuer) {
      toast.error("JWT Issuer is required");
      return;
    }

    if (!config.audience) {
      toast.error("JWT Audience is required");
      return;
    }

    if (config.jwksSource === "local" && !config.localJwksPath) {
      toast.error("Local JWKS file path is required");
      return;
    }

    if (config.jwksSource === "remote" && !config.remoteJwksUrl) {
      toast.error("Remote JWKS URL is required");
      return;
    }

    // Create JWT auth configuration
    const jwtAuth = {
      issuer: config.issuer,
      audiences: config.audience
        .split(",")
        .map((a) => a.trim())
        .filter((a) => a),
      jwks:
        config.jwksSource === "local"
          ? { file: config.localJwksPath }
          : { url: config.remoteJwksUrl },
    };

    // Apply JWT auth to all routes (or create a default route if none exist)
    const updatedListener: Listener = {
      ...listener,
      routes:
        listener.routes && listener.routes.length > 0
          ? listener.routes.map((route) => ({
              ...route,
              policies: {
                ...route.policies,
                jwtAuth,
              },
            }))
          : [
              {
                name: "default",
                hostnames: [listener.hostname || "*"],
                matches: [
                  {
                    path: { pathPrefix: "/" },
                  },
                ],
                policies: {
                  jwtAuth,
                },
                backends: [],
              },
            ],
    };

    onSave(updatedListener);
  };

  return (
    <div className="space-y-4 py-4">
      <div className="space-y-2">
        <Label htmlFor="jwt-issuer">JWT Issuer</Label>
        <Input
          id="jwt-issuer"
          data-cy="jwt-issuer-input"
          value={config.issuer}
          onChange={(e) => setConfig({ ...config, issuer: e.target.value })}
          placeholder="Enter comma-separated issuers"
        />
        <p className="text-xs text-muted-foreground">
          Comma-separated list of allowed JWT issuers.
        </p>
      </div>

      <div className="space-y-2">
        <Label htmlFor="jwt-audience">JWT Audience</Label>
        <Input
          id="jwt-audience"
          data-cy="jwt-audience-input"
          value={config.audience}
          onChange={(e) => setConfig({ ...config, audience: e.target.value })}
          placeholder="Enter comma-separated audiences"
        />
        <p className="text-xs text-muted-foreground">
          Comma-separated list of allowed JWT audiences.
        </p>
      </div>

      <div className="space-y-4">
        <Label>JWKS Source</Label>
        <RadioGroup
          data-cy="jwks-source-radio-group"
          value={config.jwksSource}
          onValueChange={(value) =>
            setConfig({
              ...config,
              jwksSource: value as "local" | "remote",
              ...(value === "local" ? { remoteJwksUrl: "" } : { localJwksPath: "" }),
            })
          }
        >
          <div className="space-y-6">
            <div className="flex items-start space-x-4">
              <RadioGroupItem value="local" id="jwks-local" data-cy="jwks-local-radio" />
              <div className="space-y-2 flex-1">
                <Label htmlFor="jwks-file">Local JWKS File Path</Label>
                <Input
                  id="jwks-file"
                  data-cy="jwks-local-file-input"
                  value={config.localJwksPath}
                  onChange={(e) =>
                    setConfig({
                      ...config,
                      localJwksPath: e.target.value,
                    })
                  }
                  placeholder="/path/to/jwks.json"
                  disabled={config.jwksSource !== "local"}
                />
                <p className="text-xs text-muted-foreground mt-1">Path to a local JWKS file.</p>
              </div>
            </div>

            <div className="flex items-start space-x-4">
              <RadioGroupItem value="remote" id="jwks-remote" data-cy="jwks-remote-radio" />
              <div className="space-y-2 flex-1">
                <Label htmlFor="jwks-url">Remote JWKS URL</Label>
                <Input
                  id="jwks-url"
                  data-cy="jwks-remote-url-input"
                  value={config.remoteJwksUrl}
                  onChange={(e) =>
                    setConfig({
                      ...config,
                      remoteJwksUrl: e.target.value,
                    })
                  }
                  placeholder="https://example.com/.well-known/jwks.json"
                  disabled={config.jwksSource !== "remote"}
                />
                <p className="text-xs text-muted-foreground mt-1">URL to a remote JWKS endpoint.</p>
              </div>
            </div>
          </div>
        </RadioGroup>
      </div>

      <div className="flex justify-end space-x-2 pt-4">
        <Button variant="outline" onClick={onCancel} data-cy="jwt-config-cancel-button">
          Cancel
        </Button>
        <Button onClick={handleSave} data-cy="jwt-config-save-button">
          Save Changes
        </Button>
      </div>
    </div>
  );
}
