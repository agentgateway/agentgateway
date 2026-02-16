import { Alert, AlertDescription } from "@/components/ui/alert";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { AlertTriangle } from "lucide-react";

interface ConfigErrorProps {
  error: Error & { isConfigurationError?: boolean; status?: number };
}

export function ConfigError({ error }: ConfigErrorProps) {
  const isConfigurationError = error.isConfigurationError && error.status === 500;

  if (!isConfigurationError) {
    // Return generic error display for non-500 errors
    return (
      <Alert variant="destructive">
        <AlertTriangle className="h-4 w-4" />
        <AlertDescription>{error.message}</AlertDescription>
      </Alert>
    );
  }

  return (
    <div className="container mx-auto py-8 px-4 max-w-3xl">
      <Card className="border-destructive">
        <CardHeader>
          <div className="flex items-center gap-2">
            <AlertTriangle className="h-5 w-5 text-destructive" />
            <CardTitle className="text-destructive">Configuration Error</CardTitle>
          </div>
          <CardDescription>
            The agentgateway server is running but has a configuration issue.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-6">
          <Alert variant="destructive">
            <AlertTriangle className="h-4 w-4" />
            <AlertDescription>
              <strong>Error Details:</strong> {error.message}
            </AlertDescription>
          </Alert>

          <div className="space-y-4">
            <h3 className="font-semibold">To resolve this issue:</h3>
            <ol className="list-decimal list-inside space-y-2 text-sm">
              <li>
                <strong>Check the agentgateway server logs</strong> for detailed error information
              </li>
              <li>
                <strong>Verify the configuration file</strong> exists and is properly formatted:
                <ul className="list-disc list-inside ml-4 mt-1 space-y-1">
                  <li>Ensure the configuration file is valid JSON/YAML</li>
                  <li>Check for syntax errors or missing required fields</li>
                  <li>Verify file permissions allow the server to read the configuration</li>
                </ul>
              </li>
              <li>
                <strong>Restart the agentgateway server</strong> after fixing the configuration
              </li>
              <li>
                <strong>Check the server status</strong> to ensure it&apos;s running on{" "}
                <code className="bg-muted px-1 py-0.5 rounded">http://localhost:15000</code>
              </li>
            </ol>
          </div>

          <div className="border-t pt-4">
            <h4 className="font-medium mb-2">Common configuration issues:</h4>
            <ul className="text-sm space-y-1 text-muted-foreground">
              <li>Missing or invalid JSON/YAML syntax</li>
              <li>Missing required configuration sections</li>
              <li>Invalid port numbers or addresses</li>
              <li>Incorrect file paths or permissions</li>
              <li>Missing dependencies or certificates</li>
            </ul>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
