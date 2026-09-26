#!/usr/bin/env sh
# Emit a large, repetitive JSON log (as a string) to use as tool output. Headroom compresses tool
# output like this, but leaves user messages untouched.
#
# Optional arg: number of log records to emit (default 600, ~85 KB).
jq -n --argjson n "${1:-600}" '[range(0; $n) | {
  id: .,
  status: (if . % 50 == 0 then "error" else "ok" end),
  service: "checkout",
  region: "us-east-1",
  latency_ms: (100 + (. % 7)),
  message: "request processed successfully"
}] | tostring'
