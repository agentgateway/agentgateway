import { useState, useEffect, forwardRef, useImperativeHandle } from "react";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import { Checkbox } from "@/components/ui/checkbox";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { ChevronUp, ChevronDown } from "lucide-react";
import { Header, TargetWithType } from "@/lib/types";

interface SSETargetFormProps {
  targetName: string;
  onSubmit: (target: TargetWithType) => Promise<void>;
  isLoading: boolean;
  existingTarget?: TargetWithType;
  hideSubmitButton?: boolean;
}

export const SSETargetForm = forwardRef<{ submitForm: () => Promise<void> }, SSETargetFormProps>(
  ({ targetName, onSubmit, isLoading, existingTarget, hideSubmitButton = false }, ref) => {
    const [sseUrl, setSseUrl] = useState("");
    const [showAdvancedSettings, setShowAdvancedSettings] = useState(false);
    const [headers, setHeaders] = useState<Header[]>([]);
    const [headerKey, setHeaderKey] = useState("");
    const [headerValue, setHeaderValue] = useState("");
    const [passthroughAuth, setPassthroughAuth] = useState(false);
    const [insecureSkipVerify, setInsecureSkipVerify] = useState(false);
    const [selectedListeners, setSelectedListeners] = useState<string[]>([]);

    // Initialize form with existing target data if provided
    useEffect(() => {
      if (existingTarget) {
        if (existingTarget.sse) {
          const sse = existingTarget.sse;
          setSseUrl(String(sse.host));

          // Headers, auth, and TLS are no longer part of SSE target in the new schema
          // These configurations are now handled at the listener/route level
        }
        if (existingTarget.listeners) {
          setSelectedListeners(existingTarget.listeners);
        }
      }
    }, [existingTarget]);

    const addHeader = () => {
      if (headerKey && headerValue) {
        setHeaders([...headers, { key: headerKey, value: { string_value: headerValue } }]);
        setHeaderKey("");
        setHeaderValue("");
      }
    };

    const removeHeader = (index: number) => {
      setHeaders(headers.filter((_, i) => i !== index));
    };

    const handleSubmit = async () => {
      try {
        const target: TargetWithType = {
          name: targetName,
          type: "mcp",
          listeners: selectedListeners,
          sse: {
            host: sseUrl,
          },
        };

        await onSubmit(target as TargetWithType);
      } catch (err) {
        console.error("Error creating SSE target:", err);
        throw err;
      }
    };

    useImperativeHandle(ref, () => ({
      submitForm: handleSubmit,
    }));

    return (
      <form
        id="mcp-target-form"
        onSubmit={(e) => {
          e.preventDefault();
          handleSubmit();
        }}
        className="space-y-4 pt-4"
      >
        <div className="space-y-2">
          <Label htmlFor="sseUrl">Server URL</Label>
          <Input
            id="sseUrl"
            type="url"
            value={sseUrl}
            onChange={(e) => setSseUrl(e.target.value)}
            placeholder="https://example.com/sse"
            required
          />
          <p className="text-sm text-muted-foreground">
            Enter the full URL including protocol, hostname, port, and path
          </p>
        </div>

        <Collapsible open={showAdvancedSettings} onOpenChange={setShowAdvancedSettings}>
          <CollapsibleTrigger asChild>
            <Button variant="ghost" className="flex items-center p-0 h-auto">
              {showAdvancedSettings ? (
                <ChevronUp className="h-4 w-4 mr-1" />
              ) : (
                <ChevronDown className="h-4 w-4 mr-1" />
              )}
              Advanced Settings
            </Button>
          </CollapsibleTrigger>
          <CollapsibleContent className="space-y-4 pt-2">
            <div className="space-y-4">
              <div className="space-y-2">
                <Label>Headers</Label>
                <div className="space-y-2">
                  {headers.map((header, index) => (
                    <div key={index} className="flex items-center gap-2">
                      <div className="flex-1">
                        <Input value={header.key} disabled placeholder="Header key" />
                      </div>
                      <div className="flex-1">
                        <Input
                          value={header.value.string_value}
                          disabled
                          placeholder="Header value"
                        />
                      </div>
                      <Button
                        type="button"
                        variant="outline"
                        size="icon"
                        onClick={() => removeHeader(index)}
                      >
                        <span className="sr-only">Remove header</span>
                        <svg
                          xmlns="http://www.w3.org/2000/svg"
                          width="24"
                          height="24"
                          viewBox="0 0 24 24"
                          fill="none"
                          stroke="currentColor"
                          strokeWidth="2"
                          strokeLinecap="round"
                          strokeLinejoin="round"
                          className="h-4 w-4"
                        >
                          <path d="M18 6 6 18" />
                          <path d="m6 6 12 12" />
                        </svg>
                      </Button>
                    </div>
                  ))}
                  <div className="flex items-center gap-2">
                    <div className="flex-1">
                      <Input
                        value={headerKey}
                        onChange={(e) => setHeaderKey(e.target.value)}
                        placeholder="Header key"
                      />
                    </div>
                    <div className="flex-1">
                      <Input
                        value={headerValue}
                        onChange={(e) => setHeaderValue(e.target.value)}
                        placeholder="Header value"
                      />
                    </div>
                    <Button
                      type="button"
                      variant="outline"
                      onClick={addHeader}
                      disabled={!headerKey || !headerValue}
                    >
                      Add
                    </Button>
                  </div>
                </div>
              </div>

              <div className="space-y-2">
                <Label>Authentication</Label>
                <div className="flex items-center space-x-2">
                  <Checkbox
                    id="passthrough-auth"
                    checked={passthroughAuth}
                    onCheckedChange={(checked: boolean | "indeterminate") =>
                      setPassthroughAuth(checked as boolean)
                    }
                  />
                  <Label htmlFor="passthrough-auth" className="text-sm font-normal">
                    Pass through authentication
                  </Label>
                </div>
              </div>

              <div className="space-y-2">
                <Label>TLS Configuration</Label>
                <div className="flex items-center space-x-2">
                  <Checkbox
                    id="insecure-skip-verify"
                    checked={insecureSkipVerify}
                    onCheckedChange={(checked: boolean | "indeterminate") =>
                      setInsecureSkipVerify(checked as boolean)
                    }
                  />
                  <Label htmlFor="insecure-skip-verify" className="text-sm font-normal">
                    Insecure skip verify
                  </Label>
                </div>
              </div>
            </div>
          </CollapsibleContent>
        </Collapsible>

        {!hideSubmitButton && (
          <Button
            type="submit"
            className="w-full"
            disabled={isLoading || !sseUrl || selectedListeners.length === 0}
          >
            {isLoading
              ? existingTarget
                ? "Updating Target..."
                : "Creating Target..."
              : existingTarget
                ? "Update Target"
                : "Create Target"}
          </Button>
        )}
      </form>
    );
  }
);

SSETargetForm.displayName = "SSETargetForm";
