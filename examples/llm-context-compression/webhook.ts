#!/usr/bin/env bun
// Adapts Headroom's messages-based compression API to agentgateway's raw webhook protocol:
// the webhook receives the complete JSON request, and returns 200 with a replacement body,
// or 204 to leave it unchanged.

const engineUrl = process.env.ENGINE_URL ?? "http://127.0.0.1:8787/v1/compress";
// Small requests are not worth the round trip.
const minSizeBytes = Number(process.env.MIN_SIZE_BYTES ?? 16384);

type CompressResponse = { messages: unknown[]; tokens_before: number; tokens_after: number };

const unchanged = () => new Response(null, { status: 204 });

Bun.serve({
  hostname: process.env.HOST ?? "127.0.0.1",
  port: 8788,
  async fetch(request) {
    if (request.method !== "POST" || new URL(request.url).pathname !== "/compress") {
      return new Response("Not found", { status: 404 });
    }

    const raw = await request.text();
    if (Buffer.byteLength(raw) < minSizeBytes) return unchanged();
    const body = JSON.parse(raw);
    // Only chat-style requests have messages to compress.
    if (!Array.isArray(body.messages)) return unchanged();

    const res = await fetch(engineUrl, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ messages: body.messages, model: body.model }),
      signal: AbortSignal.timeout(5000),
    });
    if (!res.ok) {
      // A non-2xx response makes agentgateway apply the configured failureMode.
      return new Response(`compression engine returned ${res.status}`, { status: 502 });
    }
    const result: CompressResponse = await res.json();
    console.log(`compressed ${result.tokens_before} -> ${result.tokens_after} tokens`);

    // Replace only the messages; every other field of the request is preserved.
    return Response.json({ ...body, messages: result.messages });
  },
});

console.log("Compression webhook listening on :8788");
