# Switchyard callout routing

The `auto` virtual model asks [NVIDIA NeMo Switchyard](https://github.com/NVIDIA-NeMo/Switchyard)
which model to use through its `/v1/decision` endpoint. Switchyard's Auto routing
reads the agent's recent tool results without calling a model: errors and churn route
to `gpt-5.6-sol`, routine work to `gpt-5.6-luna`. Errors fall back to `gpt-5.6-luna`.

Agentgateway sends Switchyard the request's `request.agent.session`, so routing state follows the
agent session. The session is detected from the headers coding agents send, such as
Claude Code's `x-claude-code-session-id`, and can be overridden with
`config.standardAttributes.session`.

## Run

```sh
export OPENAI_API_KEY="your-openai-key"
agentgateway -f config.yaml
```

In another terminal:

```sh
cargo install --locked switchyard-server
switchyard-server --config routes.toml --host 127.0.0.1 --port 4123
```

## Requests

Point a coding agent at `http://localhost:4000/v1` with model `auto`. Requests
without tool history, like this one, go to `gpt-5.6-luna`:

```sh
curl http://localhost:4000/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{"model":"auto","messages":[{"role":"user","content":"What is the capital of France?"}]}'
```

The same compiler error twice in a row routes to `gpt-5.6-sol`:

```sh
curl http://localhost:4000/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d @- <<'EOF'
{
  "model": "auto",
  "reasoning_effort": "none",
  "tools": [{"type": "function", "function": {"name": "Bash", "parameters": {"type": "object", "properties": {"command": {"type": "string"}}}}}],
  "messages": [
    {"role": "user", "content": "Fix the build."},
    {"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "Bash", "arguments": "{\"command\": \"cargo test\"}"}}]},
    {"role": "tool", "tool_call_id": "call_1", "content": "error[E0308]: mismatched types at src/lib.rs:12"},
    {"role": "assistant", "content": null, "tool_calls": [{"id": "call_2", "type": "function", "function": {"name": "Bash", "arguments": "{\"command\": \"cargo test\"}"}}]},
    {"role": "tool", "tool_call_id": "call_2", "content": "error[E0308]: mismatched types at src/lib.rs:12"}
  ]
}
EOF
```

The response `model` shows the selected model.
