## IP Access Control Examples

This directory contains examples for the `ipAccessControl` policy, which controls access to agentgateway based on client IP addresses using CIDR allow and deny lists.

The policy runs at the **gateway phase** (before authentication and routing), so disallowed IPs are rejected immediately with minimal overhead.

### Running an example

```bash
cargo run -- -f examples/ip-access-control/<example>.yaml
```

### Examples

#### [Allowlist only](allowlist-only.yaml)

Only permit requests from specific CIDR ranges. All other IPs receive a 403.

```yaml
policies:
  ipAccessControl:
    allow:
    - "10.0.0.0/8"
    - "172.16.0.0/12"
    - "192.168.0.0/16"
```

#### [Denylist only](denylist-only.yaml)

Block requests from specific CIDR ranges (embargoed countries, known bad actors). All other IPs are allowed through.

```yaml
policies:
  ipAccessControl:
    deny:
    - "198.51.100.0/24"
    - "203.0.113.0/24"
```

#### [Allow with deny override](allow-with-deny-override.yaml)

Allow a broad range but carve out a specific subnet to deny. Deny rules are always evaluated before allow rules, so an IP matching both lists is rejected.

```yaml
policies:
  ipAccessControl:
    allow:
    - "10.0.0.0/8"
    deny:
    - "10.0.1.0/24"
```

#### [XFF trusted hops](xff-trusted-hops.yaml)

When running behind load balancers or proxies, extract the real client IP from the `X-Forwarded-For` header. Set `xffNumTrustedHops` to the number of trusted proxies between the client and agentgateway.

```yaml
policies:
  ipAccessControl:
    allow:
    - "203.0.113.0/24"
    xffNumTrustedHops: 2
```

#### [Private IP bypass](private-ip-bypass.yaml)

Allow only specific public IP ranges but let internal traffic (RFC 1918, loopback, link-local) through unconditionally. Useful for environments where pod-to-pod or health-check traffic should never be blocked.

```yaml
policies:
  ipAccessControl:
    allow:
    - "203.0.113.0/24"
    skipPrivateIps: true
```

#### [Full chain enforcement](full-chain-enforcement.yaml)

Check every IP in the X-Forwarded-For chain, not just the resolved client IP. If any hop matches a deny rule or fails the allowlist, the request is rejected. Combined with `skipPrivateIps` to avoid blocking internal proxy hops and `maxXffLength` to guard against header abuse.

```yaml
policies:
  ipAccessControl:
    allow:
    - "203.0.113.0/24"
    deny:
    - "203.0.113.128/25"
    enforceFullChain: true
    skipPrivateIps: true
    maxXffLength: 10
```

#### [Multi-source merge](multi-source-merge.yaml)

When `ipAccessControl` policies are attached at multiple levels (listener and route), they are **merged** rather than overridden:

- **Allow lists** are unioned: an IP matching any source's allow list passes.
- **Deny lists** are unioned: a deny from any source blocks.
- **Behavioral flags** use the most-specific (listener-level) value.

This enables an operator to define infrastructure-wide deny rules at the listener level while letting individual routes add their own allowlists.

```yaml
listeners:
- policies:
    ipAccessControl:
      deny:
      - "198.51.100.0/24"
      skipPrivateIps: true
  routes:
  - policies:
      ipAccessControl:
        allow:
        - "10.0.0.0/8"
```

#### [IPv6](ipv6.yaml)

IPv4 and IPv6 CIDR ranges can be mixed freely in allow and deny lists.

```yaml
policies:
  ipAccessControl:
    allow:
    - "10.0.0.0/8"
    - "2001:db8::/32"
    deny:
    - "2001:db8:bad::/48"
```

### Configuration reference

| Field | Type | Default | Description |
|---|---|---|---|
| `allow` | list of CIDR strings | `[]` | When non-empty, only IPs matching at least one entry are permitted |
| `deny` | list of CIDR strings | `[]` | IPs matching any entry are rejected. Evaluated before allow |
| `xffNumTrustedHops` | integer | unset | Extract client IP from X-Forwarded-For, counting from the right |
| `skipPrivateIps` | boolean | `false` | Bypass allow/deny checks for RFC 1918, loopback, and link-local IPs |
| `enforceFullChain` | boolean | `false` | Check every IP in the XFF chain, not just the resolved client |
| `maxXffLength` | integer | `30` | Maximum number of IPs in the XFF chain before rejecting |

### Evaluation order

1. Validate XFF chain length (reject if exceeds `maxXffLength`)
2. If `enforceFullChain`: check every IP in XFF + connection IP
3. Otherwise: resolve single client IP (from XFF or connection)
4. For each checked IP:
   - If `skipPrivateIps` and IP is private -> skip
   - If IP matches any deny entry -> reject (403)
   - If allow list is non-empty and IP doesn't match -> reject (403)
   - Otherwise -> allow
