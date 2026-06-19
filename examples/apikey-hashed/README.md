## Hashed API Key Example

This example shows how to authenticate clients with **hashed** API keys, so the
raw key is never stored in the gateway configuration. It is recommended to
complete the [basic](../basic) example first.

### How it works

`apiKey` entries accept a self-describing `key` value:

- `sha256:<lowercase-hex>` — a SHA-256 digest (recommended). The gateway hashes
  the presented key and compares digests, so the raw key is unrecoverable at rest.
- A bcrypt digest in modular crypt format (`$2a$`, `$2b$`, `$2y$`).
- Any other value is treated as a plaintext key (the default, backward compatible).

Plaintext and hashed entries can be mixed in the same policy.

> Prefer SHA-256. API keys are high-entropy random tokens, so a slow hash like
> bcrypt only adds per-request latency without meaningful brute-force resistance.

### Producing a digest

Hash a raw key with SHA-256 and prefix it with `sha256:`:

```bash
printf '%s' 'my-secret-key' | sha256sum | awk '{print "sha256:"$1}'
# sha256:1311f8fc80a7ea28d78dd7723f09c44c1754cd35160ca8e7133ae3d7f636a19a
```

This matches the output of the `sha256.encode` CEL function, so the same digest
works whether you store it natively or hash via a `location.expression`.

### Running the example

```bash
cargo run -- -f examples/apikey-hashed/config.yaml
```

### Testing

The raw key `my-secret-key` authenticates against the stored digest:

```bash
curl -H "Authorization: Bearer my-secret-key" http://localhost:3000/
```

The plaintext key also works:

```bash
curl -H "Authorization: Bearer k-plaintext-456" http://localhost:3000/
```

An unknown key is rejected with `401 Unauthorized`:

```bash
curl -H "Authorization: Bearer nosuchkey" http://localhost:3000/
```
