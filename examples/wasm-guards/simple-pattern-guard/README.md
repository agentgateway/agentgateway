# Simple Pattern Guard - WASM Example

A simple WebAssembly security guard that blocks MCP tools matching configurable patterns.

## What It Does

This guard inspects tool names and descriptions, blocking any that contain suspicious patterns like:
- `delete`
- `rm -rf`
- `drop table`
- `eval`
- `exec`

Patterns can be customized via configuration.

## Building

### Prerequisites

```bash
# Install Rust with WASM target
rustup target add wasm32-wasi

# Install WASM tools
cargo install wasm-tools
```

### Build Steps

```bash
cd examples/wasm-guards/simple-pattern-guard

# Build the WASM module
cargo build --target wasm32-wasi --release

# Convert to Component Model (required for WIT)
wasm-tools component new \
  target/wasm32-wasi/release/simple_pattern_guard.wasm \
  -o simple_pattern_guard.component.wasm

# Optimize (optional)
wasm-opt -Oz -o simple_pattern_guard.wasm simple_pattern_guard.component.wasm
```

The final module is `simple_pattern_guard.wasm` (~50KB).

## Usage

### Configuration

```yaml
binds:
  - port: 8080
    listeners:
      - routes:
          - backends:
              - mcp:
                  security_guards:
                    - id: wasm-pattern-guard
                      type: wasm
                      enabled: true
                      priority: 100
                      failure_mode: fail_closed
                      timeout_ms: 100
                      runs_on: [response]

                      # WASM-specific config
                      module_path: ./examples/wasm-guards/simple-pattern-guard/simple_pattern_guard.wasm
                      config:
                        blocked_patterns:
                          - "delete"
                          - "remove"
                          - "destroy"
                          - "eval"
                          - "system"

                  targets:
                    - name: github
                      stdio:
                        cmd: npx
                        args: ["-y", "@modelcontextprotocol/server-github"]
```

### Testing

```bash
# Start the gateway
cargo run -- --config examples/wasm-guards/config.yaml

# The guard will now inspect all tools/list responses
# Tools with blocked patterns will be denied
```

## How It Works

```
┌─────────────────────────────────────────────────┐
│  AgentGateway (Host)                            │
│                                                 │
│  1. tools/list response received                │
│  2. Load simple_pattern_guard.wasm              │
│  3. Instantiate WASM module                     │
│  4. Call evaluate_tools_list(tools, context)    │
│     │                                            │
│     ▼                                            │
│  ┌─────────────────────────────────────┐        │
│  │  WASM Module (Guest)                │        │
│  │                                      │        │
│  │  for each tool:                     │        │
│  │    for each pattern:                │        │
│  │      if tool.name.contains(pattern):│        │
│  │        return Deny                  │        │
│  │  return Allow                       │        │
│  └─────────────────────────────────────┘        │
│     │                                            │
│     ▼                                            │
│  5. If Deny: block request                      │
│  6. If Allow: continue                          │
└─────────────────────────────────────────────────┘
```

## Extending

### Custom Logic

Edit `src/lib.rs` to add custom detection logic:

```rust
fn evaluate_tools_list(tools: Vec<Tool>, context: GuardContext) -> Result<Decision, String> {
    // Add your custom logic here

    // Example: Block tools during business hours
    let current_hour = get_current_hour();
    if current_hour >= 9 && current_hour <= 17 {
        // Strict mode during work hours
    }

    // Example: Different rules per identity
    if let Some(identity) = context.identity {
        if identity == "admin@example.com" {
            // Allow admins
            return Ok(Decision::Allow);
        }
    }

    // ... pattern checking ...
}
```

### Other Languages

The same WIT interface can be used with other languages:

**TinyGo**:
```go
//go:generate wit-bindgen tiny-go ../wit --out-dir gen
package main

import "github.com/bytecodealliance/wit-bindgen-go/gen"

type Guard struct{}

func (g Guard) EvaluateToolsList(tools []gen.Tool, ctx gen.GuardContext) gen.Result[gen.Decision, string] {
    // Implement in Go
}
```

**AssemblyScript**:
```typescript
import { Decision, Tool, GuardContext } from "./generated/guard";

export function evaluateToolsList(tools: Tool[], context: GuardContext): Decision {
  // Implement in TypeScript
}
```

## Performance

- **Module size**: ~50KB (optimized)
- **Instantiation**: ~2-3ms (first time)
- **Execution**: ~2-5ms for 50 tools
- **Total overhead**: ~5-10ms

## Security

The WASM module runs in a **sandbox** with:
- No file system access
- No network access
- Limited memory (configurable)
- Timeout protection (100ms default)

It can only:
- Read the tools and context provided
- Call host functions (log, get-time, get-config)
- Return a decision

## Debugging

### View WASM Logs

Host logs from WASM modules appear in gateway logs:

```
[INFO] WASM[wasm-pattern-guard]: Evaluating 12 tools from server 'github'
[WARN] WASM[wasm-pattern-guard]: Blocked tool 'delete_repository' matching pattern 'delete'
```

### Inspect WASM Module

```bash
# View exports
wasm-tools print simple_pattern_guard.wasm | grep export

# Validate
wasm-tools validate simple_pattern_guard.wasm

# Component info
wasm-tools component wit simple_pattern_guard.wasm
```

## FAQ

**Q: Can I update the WASM module without restarting?**
A: Not yet, but hot-reload is planned.

**Q: What if the WASM module crashes?**
A: The guard fails according to `failure_mode` (fail_closed = block request).

**Q: Can I call external APIs from WASM?**
A: Not in this basic version. Use WASI-HTTP for network access (future).

**Q: How do I debug WASM code?**
A: Use host logs, or run WASM in wasmtime CLI for testing.

## Next Steps

- See `docs/mcp-security-guards-architecture.md` for full architecture
- Check `native/` directory for native guard examples
- Try creating a guard in Go or AssemblyScript
