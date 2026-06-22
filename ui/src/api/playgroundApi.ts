import { parseJsonOrText } from "./base";

export async function sendChatCompletion(args: {
  baseUrl: string;
  model: string;
  apiKey?: string;
  messages: Array<Record<string, unknown>>;
  tools?: unknown[];
  stream?: boolean;
}) {
  const url = `${args.baseUrl.replace(/\/$/, "")}/v1/chat/completions`;
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
  };
  if (args.apiKey) headers.Authorization = `Bearer ${args.apiKey}`;
  const response = await fetch(url, {
    method: "POST",
    headers,
    body: JSON.stringify({
      model: args.model,
      messages: args.messages,
      ...(args.tools?.length ? { tools: args.tools, tool_choice: "auto" } : {}),
      stream: Boolean(args.stream),
    }),
  });
  const text = await response.text();
  if (!response.ok) {
    throw new Error(text || `${response.status} ${response.statusText}`);
  }
  return parseJsonOrText(text);
}

export async function sendMcpJsonRpc(args: {
  baseUrl: string;
  sessionId?: string;
  bearerToken?: string;
  body: unknown;
}) {
  const headers: Record<string, string> = {
    Accept: "application/json, text/event-stream",
    "Content-Type": "application/json",
  };
  if (args.sessionId) headers["mcp-session-id"] = args.sessionId;
  if (args.bearerToken?.trim())
    headers.Authorization = `Bearer ${args.bearerToken.trim()}`;
  let response: Response;
  try {
    response = await fetch(args.baseUrl, {
      method: "POST",
      headers,
      body: JSON.stringify(args.body),
    });
  } catch (err) {
    throw new Error(playgroundFetchErrorMessage(args.baseUrl, err));
  }
  const text = await response.text();
  if (!response.ok) {
    throw new Error(
      text ||
        `MCP gateway returned ${response.status} ${response.statusText} from ${args.baseUrl}`,
    );
  }
  return {
    sessionId: response.headers.get("mcp-session-id"),
    body: parseMcpResponse(text, response.headers.get("content-type") ?? ""),
    status: response.status,
  };
}

function playgroundFetchErrorMessage(url: string, err: unknown) {
  const browserMessage = err instanceof Error ? err.message : String(err);
  return `Could not reach ${url}: ${browserMessage}`;
}

function parseMcpResponse(text: string, contentType: string) {
  if (!text.trim()) return null;
  if (contentType.includes("text/event-stream")) {
    const events = text
      .split("\n\n")
      .map((event) =>
        event
          .split("\n")
          .filter((line) => line.startsWith("data:"))
          .map((line) => line.slice(5).trim())
          .join("\n"),
      )
      .filter(Boolean)
      .map(parseJsonOrText);
    return events.length === 1 ? events[0] : events;
  }
  return parseJsonOrText(text);
}
