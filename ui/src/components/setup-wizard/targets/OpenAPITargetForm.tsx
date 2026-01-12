import { useState, useEffect, forwardRef, useImperativeHandle } from "react";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { ChevronUp, ChevronDown } from "lucide-react";
import { Header, LocalDataSource, TargetWithType } from "@/lib/types";

interface OpenAPITargetFormProps {
  targetName: string;
  onSubmit: (target: TargetWithType) => Promise<void>;
  isLoading: boolean;
  existingTarget?: TargetWithType;
  hideSubmitButton?: boolean;
}

export const OpenAPITargetForm = forwardRef<
  { submitForm: () => Promise<void> },
  OpenAPITargetFormProps
>(({ targetName, onSubmit, isLoading, existingTarget, hideSubmitButton = false }, ref) => {
  const [host, setHost] = useState("");
  const [port, setPort] = useState("");
  const [showOpenAPIAdvancedSettings, setShowOpenAPIAdvancedSettings] = useState(false);
  const [headers, setHeaders] = useState<Header[]>([]);
  const [headerKey, setHeaderKey] = useState("");
  const [headerValue, setHeaderValue] = useState("");
  const [schemaType, setSchemaType] = useState<"file" | "inline">("file");
  const [schemaFilePath, setSchemaFilePath] = useState("");
  const [schemaInline, setSchemaInline] = useState("");

  // Initialize form with existing target data if provided
  useEffect(() => {
    if (existingTarget?.openapi) {
      const openapi = existingTarget.openapi;
      setHost(openapi.host);
      setPort(openapi.port?.toString() || "");
      setHeaders([]);
    }
  }, [existingTarget]);

  const addHeader = () => {
    if (headerKey && headerValue) {
      setHeaders([
        ...headers,
        {
          key: headerKey,
          value: { string_value: headerValue },
        },
      ]);
      setHeaderKey("");
      setHeaderValue("");
    }
  };

  const removeHeader = (index: number) => {
    setHeaders(headers.filter((_, i) => i !== index));
  };

  const handleSubmit = async () => {
    try {
      const schema: LocalDataSource =
        schemaType === "file"
          ? { file_path: schemaFilePath }
          : { inline: new TextEncoder().encode(schemaInline) };
      const portValue = port.trim() ? parseInt(port, 10) : undefined;

      const target: TargetWithType = {
        name: targetName,
        type: "openapi",
        openapi: {
          host,
          port: Number.isFinite(portValue) ? portValue : undefined,
          schema,
          // Headers are no longer supported in the new schema
        },
      };

      await onSubmit(target as TargetWithType);
    } catch (err) {
      console.error("Error creating OpenAPI target:", err);
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
        <Label htmlFor="host">Host</Label>
        <Input
          id="host"
          value={host}
          onChange={(e) => setHost(e.target.value)}
          placeholder="e.g., localhost"
        />
      </div>

      <div className="space-y-2">
        <Label htmlFor="port">Port</Label>
        <Input
          id="port"
          value={port}
          onChange={(e) => setPort(e.target.value)}
          placeholder="e.g., 3000"
          type="number"
        />
      </div>

      <div className="space-y-2">
        <Label>Schema</Label>
        <div className="space-y-4">
          <div className="flex items-center space-x-4">
            <label className="flex items-center space-x-2">
              <input
                type="radio"
                checked={schemaType === "file"}
                onChange={() => setSchemaType("file")}
              />
              <span>File</span>
            </label>
            <label className="flex items-center space-x-2">
              <input
                type="radio"
                checked={schemaType === "inline"}
                onChange={() => setSchemaType("inline")}
              />
              <span>Inline</span>
            </label>
          </div>

          {schemaType === "file" ? (
            <Input
              value={schemaFilePath}
              onChange={(e) => setSchemaFilePath(e.target.value)}
              placeholder="Path to OpenAPI schema file"
            />
          ) : (
            <textarea
              value={schemaInline}
              onChange={(e) => setSchemaInline(e.target.value)}
              placeholder="OpenAPI schema JSON/YAML"
              className="w-full h-32 p-2 border rounded"
            />
          )}
        </div>
      </div>

      <Collapsible open={showOpenAPIAdvancedSettings} onOpenChange={setShowOpenAPIAdvancedSettings}>
        <CollapsibleTrigger asChild>
          <Button variant="ghost" className="flex items-center p-0 h-auto">
            {showOpenAPIAdvancedSettings ? (
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
                      <Input value={header.key} disabled placeholder="Header name" />
                    </div>
                    <div className="flex-1">
                      <Input
                        value={header.value.string_value || ""}
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
                      placeholder="Header name"
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
          </div>
        </CollapsibleContent>
      </Collapsible>

      {!hideSubmitButton && (
        <Button
          type="submit"
          className="w-full"
          disabled={
            isLoading || !host || !port || (schemaType === "file" ? !schemaFilePath : !schemaInline)
          }
        >
          {isLoading
            ? existingTarget
              ? "Updating Target..."
              : "Adding Target..."
            : existingTarget
              ? "Update OpenAPI Target"
              : "Add OpenAPI Target"}
        </Button>
      )}
    </form>
  );
});

OpenAPITargetForm.displayName = "OpenAPITargetForm";
