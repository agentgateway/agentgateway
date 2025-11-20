# Test Certificate Files

This directory contains pre-generated certificate files used by AgentGateway integration tests for HBONE mTLS testing.

## Files

- **ca-key.pem**: CA private key used to sign test certificates
- **root-cert.pem**: CA certificate used as the trust root
- **key.pem**: Static test private key used for consistent test certificate generation

## Regenerating Certificates

To regenerate these test certificates, run:

```bash
cd crates/agentgateway/tests/common/testdata
./gen_certs.sh
```

This script uses **OpenSSL** to generate:
1. A CA private key (RSA 2048-bit) and self-signed certificate (valid for ~274 years)
2. A static test private key (EC P-256, reused across tests for consistency)

All keys are converted to PKCS#8 format, which is required by the `rcgen` library used in the test infrastructure. The format matches the original test certificates to ensure compatibility.

## Requirements

- OpenSSL (standard on most Unix-like systems)

## Note

These certificates are **test-only** and should never be used in production. The private keys are intentionally committed to the repository for test consistency.

