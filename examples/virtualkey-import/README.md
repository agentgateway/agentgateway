# agctl virtualkey import

This example imports virtual API keys from CSV and emits the Kubernetes Secret
shape consumed by `AgentgatewayPolicy.spec.traffic.apiKeyAuthentication`.
Large imports are split into multiple labeled Secrets automatically.

The CSV contains secret material. The generated manifest also contains secret
material, especially for rows where `key` is empty and `agctl` generates a key.
Store the output securely.

## Generate a Secret manifest

```bash
agctl virtualkey import \
  --file examples/virtualkey-import/virtual-keys.csv \
  --secret-name agw-virtual-keys \
  --namespace default \
  --output yaml > /tmp/agw-virtual-keys.yaml
```

The `charlie` row has an empty `key`, so `agctl` generates a key in the
`sk-charlie-<random>` format and writes it into the Secret manifest.

## Generate keys directly

Use `agctl virtualkey generate` when you need raw virtual API keys before
building or updating an import CSV.

```bash
agctl virtualkey generate --label alice
```

The generated key uses the `sk-alice-<random>` format. The label is sanitized
before it is embedded in the key.

Generate a batch when you want to fill multiple CSV rows:

```bash
agctl virtualkey generate \
  --count 5 \
  --label batch-import \
  --output text > /tmp/generated-virtual-keys.txt
```

The output file contains secret key material. Restrict access to it before
sharing or storing it.

```bash
chmod 0600 /tmp/generated-virtual-keys.txt
```

When generating keys for an existing virtual-key Secret, `agctl` can query the
Secret first and retry if a generated key collides with an existing value.

```bash
agctl virtualkey generate \
  --label alice \
  --collision-check-secret default/agw-virtual-keys
```

Use `--collision-check-selector` instead when keys are split across multiple
Secrets selected by label.

Large imports are automatically split into multiple labeled Secrets before any
one Secret approaches Kubernetes object size limits. The first Secret uses the
requested name, and additional Secrets use deterministic suffixes such as
`agw-virtual-keys-0002`.
If the import is large, the output is a Kubernetes `List` containing multiple
Secrets named from the requested `--secret-name`, such as `agw-virtual-keys`,
`agw-virtual-keys-0002`, and `agw-virtual-keys-0003`.

## Apply the Secret

```bash
kubectl apply -f /tmp/agw-virtual-keys.yaml
```

Or stream the generated manifest directly:

```bash
agctl virtualkey import \
  --file examples/virtualkey-import/virtual-keys.csv \
  --secret-name agw-virtual-keys \
  --namespace default | kubectl apply -f -
```

## Select the imported keys from policy

```yaml
apiVersion: agentgateway.dev/v1alpha1
kind: AgentgatewayPolicy
metadata:
  name: api-key-auth
  namespace: default
spec:
  traffic:
    apiKeyAuthentication:
      secretSelector:
        matchLabels:
          agentgateway.dev/virtual-keys: "true"
```

Each Secret entry is emitted as JSON with `key` and `metadata`. The `id` column
is stored as `metadata.id`, which is the stable virtual-key identity.
ID values containing reserved special characters are rejected by default; pass
`--escape-special-characters` to store escaped values instead.
For large imports, use `secretSelector.matchLabels` as shown above so every
split Secret is selected.
