# WASM Security Guards

This directory contains example WebAssembly security guards for AgentGateway.

## Overview

WASM guards are runtime-loadable security modules that run in a sandbox. They provide extensibility without requiring recompilation of the gateway binary.

## Available Examples

### 1. Simple Pattern Guard (`simple-pattern-guard/`)

Basic example that blocks tools matching configurable patterns.

**Use case**: Block dangerous operations like delete, eval, etc.

**Language**: Rust
**Size**: ~50KB
**Latency**: ~5-10ms

[See README](simple-pattern-guard/README.md)

## Quick Start

```bash
# Build an example guard
cd simple-pattern-guard
cargo build --target wasm32-wasi --release

# Convert to Component Model
wasm-tools component new \
  target/wasm32-wasi/release/simple_pattern_guard.wasm \
  -o simple_pattern_guard.wasm

# Use in config
# (see example config files)
```

## Writing Your Own Guard

### 1. Use the Template

```bash
cp -r simple-pattern-guard my-guard
cd my-guard
```

### 2. Edit `src/lib.rs`

Implement your custom logic in `evaluate_tools_list`:

```rust
fn evaluate_tools_list(
    tools: Vec<Tool>,
    context: GuardContext,
) -> Result<Decision, String> {
    // Your logic here

    for tool in tools {
        if is_suspicious(&tool) {
            return Ok(Decision::Deny(DenyReason {
                code: "suspicious_tool".to_string(),
                message: format!("Tool {} is suspicious", tool.name),
                details: None,
            }));
        }
    }

    Ok(Decision::Allow)
}
```

### 3. Build and Test

```bash
cargo build --target wasm32-wasi --release
wasm-tools component new target/wasm32-wasi/release/my_guard.wasm -o my_guard.wasm

# Test with agentgateway
cargo run -p agentgateway -- --config my-config.yaml
```

## Interface Specification

All guards must implement the WIT interface defined in `simple-pattern-guard/wit/guard.wit`.

### Guest Exports (Your Code)

```rust
/// Evaluate a tools/list response
evaluate-tools-list: func(
    tools: list<tool>,
    context: guard-context
) -> result<decision, string>
```

### Host Imports (Available to You)

```rust
/// Log a message
log: func(level: u8, message: string)

/// Get current time (Unix ms)
get-time: func() -> u64

/// Get configuration value
get-config: func(key: string) -> string
```

## Configuration

```yaml
security_guards:
  - id: my-wasm-guard
    type: wasm
    enabled: true
    priority: 100
    failure_mode: fail_closed  # or fail_open
    timeout_ms: 100
    runs_on: [response]  # or [request, tool_invoke, etc.]

    # WASM-specific
    module_path: /path/to/my_guard.wasm

    # Custom config (accessible via get-config)
    config:
      max_tool_count: 50
      blocked_servers:
        - untrusted-server
      custom_patterns:
        - "dangerous"
```

## Language Support

### Rust ✅ (Recommended)

```bash
cargo install wit-bindgen-cli
wit-bindgen rust wit/ --out-dir src/generated
```

### TinyGo ✅

```bash
wit-bindgen tiny-go wit/ --out-dir gen
```

### AssemblyScript ⚠️ (Experimental)

```bash
npm install -g assemblyscript
# Use jco for WIT bindings
```

### C/C++ ⚠️ (Advanced)

```bash
wit-bindgen c wit/ --out-dir generated
```

## Performance Considerations

| Aspect | Typical Value |
|--------|---------------|
| Module size | 50-500 KB |
| Load time | 2-5 ms (cached) |
| Instantiation | 1-3 ms |
| Execution | 0.5-5 ms |
| **Total overhead** | **5-10 ms** |

**Tips**:
- Keep modules small (<500KB)
- Minimize allocations in hot path
- Use `wasm-opt -Oz` for size optimization
- Cache expensive computations

## Security & Sandboxing

WASM guards run with **strong isolation**:

✅ **Allowed**:
- Read tools and context data
- Call host functions (log, get-time, get-config)
- Allocate memory (within limits)
- Pure computation

❌ **Forbidden**:
- File system access
- Network access
- System calls
- Arbitrary code execution
- Access to other guards

### Resource Limits

Configurable limits (future):
```yaml
module_path: ./my_guard.wasm
limits:
  max_memory_bytes: 10485760  # 10MB
  max_execution_time_ms: 100
  max_fuel: 1000000  # Instruction limit
```

## Debugging

### View Logs

```bash
# WASM logs appear with [WASM] prefix
RUST_LOG=info cargo run -- --config config.yaml

# Example output:
[INFO] WASM[my-guard]: Evaluating 12 tools
[WARN] WASM[my-guard]: Suspicious pattern detected
```

### Test Standalone

```bash
# Run WASM module directly with wasmtime
wasmtime run --invoke evaluate_tools_list my_guard.wasm -- '{"tools": [...], "context": {...}}'
```

### Inspect Module

```bash
# Show exports
wasm-tools print my_guard.wasm | grep export

# Validate WIT compliance
wasm-tools component wit my_guard.wasm

# Show size breakdown
wasm-tools strip my_guard.wasm | wc -c
```

## Advanced Topics

### Stateful Guards

Guards can maintain state across invocations (within same module instance):

```rust
static mut REQUEST_COUNT: u32 = 0;

fn evaluate_tools_list(...) -> Result<Decision, String> {
    unsafe {
        REQUEST_COUNT += 1;
        if REQUEST_COUNT > 100 {
            return Ok(Decision::Deny(...));
        }
    }
    Ok(Decision::Allow)
}
```

**Note**: State is per-module-instance, not global.

### Async Operations (Future)

Future support for WASI-HTTP:

```rust
// Not yet supported - coming soon
async fn evaluate_tools_list(...) -> Result<Decision, String> {
    let threat_score = http_get("https://threat-intel.example.com/check").await?;
    if threat_score > 0.8 {
        return Ok(Decision::Deny(...));
    }
    Ok(Decision::Allow)
}
```

### Composition

Multiple guards run in priority order:

```yaml
security_guards:
  - id: pattern-guard
    type: wasm
    priority: 100  # Runs first
    module_path: ./pattern_guard.wasm

  - id: ml-guard
    type: wasm
    priority: 200  # Runs second
    module_path: ./ml_guard.wasm
```

## Troubleshooting

### "Module failed to load"

- Check WASM module is Component Model format (use `wasm-tools component new`)
- Verify WIT interface matches expected version
- Check file permissions on .wasm file

### "Execution timeout"

- Increase `timeout_ms` in config
- Optimize guard code (reduce allocations)
- Consider using native guard for hot path

### "Invalid decision returned"

- Ensure guard returns valid `Decision` variant
- Check JSON serialization in `details` field
- Verify error handling doesn't panic

## Examples from Community

(Coming soon - submit your guards!)

## Further Reading

- [WebAssembly Component Model](https://component-model.bytecodealliance.org/)
- [WIT Format Specification](https://component-model.bytecodealliance.org/design/wit.html)
- [wasmtime Documentation](https://docs.wasmtime.dev/)
- [MCP Security Architecture](../../docs/mcp-security-guards-architecture.md)
