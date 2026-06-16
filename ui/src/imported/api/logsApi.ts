import type {
  LogEntry,
  SearchLogsRequest,
  SearchLogsResponse,
  TailEvent,
  TokenUsageGroup,
  TokenUsageRequest,
} from "../types";

const apiBase = import.meta.env.VITE_API_URL ?? "";

async function requestJson<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`${apiBase}${path}`, {
    credentials: "include",
    headers: { "Content-Type": "application/json", ...(init?.headers ?? {}) },
    ...init,
  });
  if (!response.ok) {
    let message = `${response.status} ${response.statusText}`;
    try {
      const text = await response.text();
      if (text) {
        try {
          const body = JSON.parse(text);
          message = typeof body === "string" ? body : JSON.stringify(body);
        } catch {
          message = text;
        }
      }
    } catch {
      // Keep the status text fallback when the body cannot be read.
    }
    throw new Error(message || "request failed");
  }
  return response.json() as Promise<T>;
}

export function searchLogs(request: SearchLogsRequest) {
  return requestJson<SearchLogsResponse>("/api/logs/search", {
    method: "POST",
    body: JSON.stringify(request),
  });
}

export function getLog(id: string) {
  return requestJson<{ log: LogEntry | null }>("/api/logs/get", {
    method: "POST",
    body: JSON.stringify({ id, includePayload: true }),
  });
}

export function tokenUsage(request: TokenUsageRequest = { groupBy: [{ field: "requestModel" }] }) {
  return requestJson<{ groups: TokenUsageGroup[] }>("/api/logs/analytics/token-usage", {
    method: "POST",
    body: JSON.stringify(request),
  });
}

export async function* streamLogs(
  request: SearchLogsRequest,
  signal: AbortSignal,
): AsyncGenerator<TailEvent> {
  const response = await fetch(`${apiBase}/api/logs/tail`, {
    method: "POST",
    credentials: "include",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ ...request, includeAttributes: true }),
    signal,
  });
  if (!response.ok || !response.body) {
    throw new Error(await response.text());
  }

  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";

  while (true) {
    const { value, done } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });
    const events = buffer.split("\n\n");
    buffer = events.pop() ?? "";
    for (const raw of events) {
      const eventName = raw.match(/^event:\s*(.+)$/m)?.[1];
      const data = raw.match(/^data:\s*(.+)$/m)?.[1];
      if (eventName === "log" && data) {
        yield JSON.parse(data) as TailEvent;
      }
      if (eventName === "error" && data) {
        throw new Error(JSON.parse(data).message ?? "log stream failed");
      }
    }
  }
}
