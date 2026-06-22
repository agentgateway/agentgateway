import type {
  LogEntry,
  AnalyticsSummaryRequest,
  AnalyticsSummaryResponse,
  SearchLogsRequest,
  SearchLogsResponse,
  TailEvent,
} from "../types";
import { apiBase, requestJson } from "./base";

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

export function analyticsSummary(request: AnalyticsSummaryRequest) {
  return requestJson<AnalyticsSummaryResponse>("/api/logs/analytics/summary", {
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
