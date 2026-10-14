#!/usr/bin/env -S bun run --install=force
import "zod"; // AI SDK peer dependency, also auto-installed by Bun.
import { experimental_evaluate as evaluate } from "ai";
import { createTypeSafeAi } from "@ai-sdk/typesafe-ai";
import { ROOT_CONTEXT, SpanKind, trace } from "@opentelemetry/api";
import { W3CTraceContextPropagator } from "@opentelemetry/core";
import { resourceFromAttributes } from "@opentelemetry/resources";
import { BasicTracerProvider, SimpleSpanProcessor } from "@opentelemetry/sdk-trace-base";
import { OTLPTraceExporter } from "@opentelemetry/exporter-trace-otlp-grpc";

// Temporary tracing for the demo.
const provider = new BasicTracerProvider({
  resource: resourceFromAttributes({ "service.name": "jev-router" }),
  spanProcessors: [new SimpleSpanProcessor(new OTLPTraceExporter({
    url: "http://localhost:4317",
  }))],
});
const tracer = provider.getTracer("jev-router");
const propagator = new W3CTraceContextPropagator();

// The callout body is set by `body` in config.yaml, and the response is read by `transformation`.
type Message = { role: string; content: unknown };
type RouteRequest = { messages: Message[] };
type RouteResponse = { model: string; reasoning_effort: string };

const typesafe = createTypeSafeAi({
  baseURL: "http://127.0.0.1:4000/v1",
  // Agentgateway supplies the real upstream API key.
  apiKey: "unused",
});

const criteria = ["Trivial", "Simple", "Moderate", "Hard"];
// Indexed by complexity score.
const routes: RouteResponse[] = [
  { model: "gpt-5.6-luna", reasoning_effort: "low" },
  { model: "gpt-5.6-luna", reasoning_effort: "medium" },
  { model: "gpt-5.6-terra", reasoning_effort: "medium" },
  { model: "gpt-5.6-sol", reasoning_effort: "high" },
];

Bun.serve({
  hostname: "127.0.0.1",
  port: 8000,
  async fetch(request) {
    if (request.method !== "POST" || new URL(request.url).pathname !== "/route") {
      return new Response("Not found", { status: 404 });
    }

    const payload: RouteRequest = await request.json();
    // Route on the newest message; earlier turns are context the chosen model will see anyway.
    const messages = payload.messages.slice(-1).map(({ role, content }) => ({
      role,
      content: typeof content === "string" ? content : JSON.stringify(content),
    }));

    const headers: Record<string, string> = {};
    for (const name of ["traceparent", "tracestate", "baggage"]) {
      const value = request.headers.get(name);
      if (value !== null) headers[name] = value;
    }

    const parent = propagator.extract(ROOT_CONTEXT, headers, {
      keys: (carrier) => Object.keys(carrier),
      get: (carrier, key) => carrier[key],
    });
    const span = tracer.startSpan("Jev router", { kind: SpanKind.SERVER }, parent);
    propagator.inject(trace.setSpan(parent, span), headers, {
      set: (carrier, key, value) => { carrier[key] = value; },
    });

    try {
      const { answers } = await evaluate({
        model: typesafe.evaluationModel("jev-latest"),
        headers,
        state: { messages },
        questions: {
          complexity: {
            type: "score",
            instructions: "Rate how much reasoning is needed to answer the latest message well. Greetings and lookups of well-known facts are trivial; multi-step math, code design, and nuanced analysis are hard.",
            criteria,
          },
        },
        maxRetries: 0,
        abortSignal: AbortSignal.timeout(8000),
      });

      // Jev returns a weighted score, such as 2.39, so round it to the nearest criterion.
      const score = answers.complexity.score;
      const level = Math.min(Math.max(Math.round(score), 0), routes.length - 1);
      const route = routes[level];
      console.log(`complexity ${score} (${criteria[level]}) ->`, route);
      span.setAttribute("router.complexity", score);
      span.setAttribute("router.model", route.model);

      // Errors (including Jev failures) make agentgateway use the configured fallback model.
      return Response.json(route);
    } finally {
      span.end();
    }
  },
});

console.log("Jev router listening on http://127.0.0.1:8000");
