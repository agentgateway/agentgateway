# Email data tokenization (Beta)

This is an early feature available for testing and feedback

Enable **Email addresses** tokenization under **LLM Guardrails > Data tokenization**, or configure it in YAML

The UI control corresponds to the YAML setting `llm.policies.guardrails.tokenization.email`. Set it to `true` to enable email tokenization

```yaml
llm:
  policies:
    guardrails:
      tokenization:
        email: true
```

This configuration applies only to incoming Chat Completions requests, normally sent to `POST /v1/chat/completions`, including streaming requests. Clients request streaming with the JSON request body field `"stream": true`; no provider streaming setting is required in YAML. Other APIs, such as `POST /v1/responses`, `POST /v1/messages` and `POST /v1/embeddings`, continue to work normally without email tokenization. Email addresses in those requests can reach the provider unchanged. The provider can use a different wire format internally, provided the gateway translates it to and from Chat Completions

The gateway replaces standard email addresses in system messages, conversation text, tool results and tool arguments. It reuses the built-in ASCII email recognizer. Internationalized email addresses are outside this first version. It does not inspect images, audio, attachments, headers, metadata or tool definitions, and it does not decode obfuscated addresses or JSON escapes inside tool argument strings. This is a scoped text transformation, not a complete data loss prevention system

Each request gets a random token namespace and a private in-memory mapping. Tokens use angle brackets, for example `<AGW_EMAIL_4f89279340ab4e28a9b9ce0f57aca0b2_0>`. Repeated, exactly matching addresses receive the same token within that request. Mappings are not shared between requests or replicas and are not stored in a persistent vault. A request with more than 4096 distinct addresses is rejected

Request enrichment runs before replacement, as do request guards configured in YAML under `llm.policies.guardrails.request`. Addresses already masked by a request guard cannot be restored. Response guards configured in YAML under `llm.policies.guardrails.response` inspect the tokenized reply before restoration

Streaming response guard evaluation is controlled by the YAML setting `llm.policies.guardrails.streaming`, which accepts `Enabled` or `Disabled` and defaults to `Disabled`. The guardrails UI currently preserves this setting but has no dedicated control for it. This YAML setting enables guard evaluation on streamed replies; it does not request streaming from the LLM. Email token restoration works for streamed replies independently of this setting, and enabling data tokenization does not enable or change streaming response guard behavior

Buffered replies restore tokens through the response text visitor. Streaming replies are parsed after provider translation and response guards. Each choice and text or tool argument channel has its own pending prefix. Ordinary text is emitted immediately, while a possible token prefix waits for subsequent deltas. Usage events and finish reasons are retained. Usage and log probabilities still describe the provider text, whose length can differ from the restored text. At a normal choice finish, incomplete tokens are emitted literally. A truncated stream with a pending prefix fails instead of silently losing text

Only exact tokens from the current request are restored. Unknown, altered or previous-request tokens remain unchanged. Addresses newly invented by the model are not tokenized on the response path

For tokenized Chat Completions requests, prompt and completion message logging on the successful LLM path sees the tokenized text. Other APIs retain their existing logging behavior. Request guards, configured request mirrors, diagnostic tracing and other HTTP policies can still see the original request. This feature does not sanitize all logs or protect the mapping against process memory inspection

The local configuration and admin API support this policy. The xDS protocol has not been extended with a data tokenization field

Run the focused behavior tests with

```sh
cargo test --locked --profile ci -p agentgateway --lib tokenization
```
