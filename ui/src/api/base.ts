export const apiBase = import.meta.env.VITE_AGENTGATEWAY_API ?? "";

export async function requestJson<T>(
  path: string,
  init?: RequestInit,
): Promise<T> {
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

export function parseJsonOrText(text: string) {
  try {
    return JSON.parse(text) as unknown;
  } catch {
    return text;
  }
}
