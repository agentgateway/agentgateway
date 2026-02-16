import { useState, useEffect, forwardRef, useImperativeHandle } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import { Target, TargetWithType, A2aTarget, BackendTLS } from "@/lib/types";

interface DirectA2aTarget extends Omit<TargetWithType, "a2a">, A2aTarget {
  type: "a2a";
  tls?: BackendTLS;
}

interface A2ATargetFormProps {
  targetName: string;
  onSubmit: (target: TargetWithType) => Promise<void>;
  isLoading: boolean;
  existingTarget?: TargetWithType;
  hideSubmitButton?: boolean;
}

export const A2ATargetForm = forwardRef<{ submitForm: () => Promise<void> }, A2ATargetFormProps>(
  ({ targetName, onSubmit, isLoading, existingTarget, hideSubmitButton = false }, ref) => {
    const [a2aUrl, setA2aUrl] = useState("");

    useEffect(() => {
      if (existingTarget?.type === "a2a") {
        const a2aData = existingTarget as DirectA2aTarget;
        const { host, port, path, tls } = a2aData;

        if (host && port) {
          const protocol = tls ? "https" : "http";
          const defaultPort = protocol === "https" ? 443 : 80;
          const portString = port === defaultPort ? "" : `:${port}`;
          const formattedPath = path ? (path.startsWith("/") ? path : `/${path}`) : "/";
          const constructedUrl = `${protocol}://${host}${portString}${formattedPath}`;
          setA2aUrl(constructedUrl);
        } else {
          console.error("Existing A2A target is missing host or port", existingTarget);
          setA2aUrl("");
        }
      } else {
        setA2aUrl("");
      }
    }, [existingTarget]);

    const handleSubmit = async () => {
      let isValidationError = false;
      try {
        if (!a2aUrl.trim()) {
          toast.error("URL is required for A2A targets");
          isValidationError = true;
          return;
        }

        let parsedUrl;
        try {
          parsedUrl = new URL(a2aUrl);
        } catch {
          toast.error("Invalid URL format. Please include protocol (e.g., http:// or https://).");
          isValidationError = true;
          return;
        }

        if (!parsedUrl.hostname) {
          toast.error("URL must include a valid hostname.");
          isValidationError = true;
          return;
        }

        let finalPortNumber: number;
        const initialParsedPort = parseInt(parsedUrl.port, 10);

        if (!isNaN(initialParsedPort)) {
          finalPortNumber = initialParsedPort;
        } else if (parsedUrl.protocol === "http:" || parsedUrl.protocol === "https:") {
          finalPortNumber = parsedUrl.protocol === "https:" ? 443 : 80;
        } else {
          toast.error(`Port is required for protocol "${parsedUrl.protocol}" or use http/https.`);
          isValidationError = true;
          return;
        }

        const target: Target = {
          name: targetName,
          a2a: {
            host: parsedUrl.hostname,
            port: finalPortNumber,
            path: parsedUrl.pathname || "/",
          },
        };

        await onSubmit(target as TargetWithType);
      } catch (err) {
        console.error("Error processing A2A target:", err);
        if (!isValidationError) {
          toast.error(err instanceof Error ? err.message : "An unexpected error occurred.");
        }
        if (!isValidationError) {
          throw err;
        }
      }
    };

    useImperativeHandle(ref, () => ({
      submitForm: handleSubmit,
    }));

    return (
      <form
        id="a2a-target-form"
        onSubmit={(e) => {
          e.preventDefault();
          handleSubmit();
        }}
        className="space-y-4"
      >
        <div className="space-y-2">
          <Label htmlFor="a2aUrl">
            Target URL <span className="text-red-500">*</span>
          </Label>
          <Input
            id="a2aUrl"
            type="url"
            value={a2aUrl}
            onChange={(e) => setA2aUrl(e.target.value)}
            placeholder="e.g., http://localhost:8080/my-agent-service"
            required
          />
          <p className="text-sm text-muted-foreground">
            Enter the full URL of the A2A target server (including http/https).
          </p>
        </div>

        {!hideSubmitButton && (
          <Button type="submit" className="w-full" disabled={isLoading || !a2aUrl.trim()}>
            {isLoading
              ? existingTarget
                ? "Updating Target..."
                : "Adding Target..."
              : existingTarget
                ? "Update Target"
                : "Add Target"}
          </Button>
        )}
      </form>
    );
  }
);

A2ATargetForm.displayName = "A2ATargetForm";
