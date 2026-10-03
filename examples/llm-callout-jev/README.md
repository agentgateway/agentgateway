# Jev callout routing

The `auto` virtual model asks [router.ts](router.ts) which model to use. The router
scores each request's complexity with [Jev](https://docs.typesafe.ai/introduction) and
picks `gpt-5.6-luna`, `gpt-5.6-terra`, or `gpt-5.6-sol`, falling back to
`gpt-5.6-luna` on errors.

## Run

```sh
export OPENAI_API_KEY="your-openai-key"
export TYPESAFE_API_KEY="your-typesafe-key"
agentgateway -f config.yaml
```

In another terminal:

```sh
./router.ts
```

## Requests

```sh
curl http://localhost:4000/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{"model":"auto","messages":[{"role":"user","content":"What is the capital of France?"}]}'
```

The response `model` shows the selected model. Add an `x-session-id` header to keep
a session on its first selection for 10 minutes.
