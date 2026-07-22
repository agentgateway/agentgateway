#!/usr/bin/env sh
# Emit a large synthetic reference document to stdout — no file is written.
# Pipe it straight into a request with process substitution:
#
#   jq -n --rawfile ctx <(examples/llm-context-compression/gen-context.sh) ...
#
# The output is deliberately repetitive so it (a) comfortably exceeds the compression
# `minSizeBytes` threshold and (b) compresses well, making token savings easy to observe.
#
# Optional arg: number of paragraphs to emit (default 400, ~120 KB).
n="${1:-400}"
i=1
while [ "$i" -le "$n" ]; do
  printf 'Section %d. agentgateway is a proxy for LLM and MCP traffic. Context compression sends the request messages to an external service before they reach the provider and applies the shorter messages it returns, reducing the tokens billed. This paragraph is filler reference material whose only job is to push the request above the compression size threshold so the /v1/compress callout fires. Because the text repeats, a compressor can collapse it dramatically, which makes the savings obvious in the response usage.\n\n' "$i"
  i=$((i + 1))
done
