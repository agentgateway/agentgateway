
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

interface ResponseDisplayProps {
  connectionType: "mcp" | "a2a" | null;
  mcpResponse: any;
  a2aResponse: any;
}

export function ResponseDisplay({
  connectionType,
  mcpResponse,
  a2aResponse,
}: ResponseDisplayProps) {
  const responseData = connectionType === "a2a" ? a2aResponse : mcpResponse;

  if (!responseData) {
    return null; // Don't render anything if there's no response
  }

  return (
    <Card className="mt-4">
      <CardHeader>
        <CardTitle>Response</CardTitle>
      </CardHeader>
      <CardContent>
        <pre className="bg-muted p-4 rounded-lg overflow-auto max-h-[500px]">
          {JSON.stringify(responseData, null, 2)}
        </pre>
      </CardContent>
    </Card>
  );
}
