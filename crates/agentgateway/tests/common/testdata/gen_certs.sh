#!/bin/bash
# Generate test certificate files for AgentGateway integration tests
#
# This script generates:
#   - ca-key.pem: CA private key (RSA 2048-bit, PKCS#8 format)
#   - root-cert.pem: CA certificate (RSA 2048-bit, self-signed)
#   - key.pem: Static test private key (EC P-256, PKCS#8 format)
#
# These files are used by the test infrastructure to generate certificates
# for HBONE mTLS testing without requiring a real CA.
#
# Format matches the original test certificates to ensure compatibility with
# rcgen::KeyPair::from_pem() used in tests.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "Generating test certificates using OpenSSL..."

# Generate CA private key (RSA 2048-bit) - keep in RSA format for cert generation
openssl genrsa -out ca-key-rsa.pem 2048

# Generate CA certificate (self-signed, valid for a very long time to match original)
# Note: Original had validity until 2299, we'll use a long period
# Original Subject was just "O=cluster.local" without CN
openssl req -new -x509 -key ca-key-rsa.pem -out root-cert.pem \
  -days 99999 \
  -subj "/O=cluster.local" \
  -addext "basicConstraints=critical,CA:TRUE" \
  -addext "keyUsage=critical,keyCertSign,cRLSign"

# Convert CA key to PKCS#8 format (required by rcgen) after cert generation
openssl pkcs8 -topk8 -nocrypt -in ca-key-rsa.pem -out ca-key.pem
rm ca-key-rsa.pem

# Generate test private key (EC P-256, prime256v1 curve)
openssl ecparam -genkey -name prime256v1 -out key-ec.pem

# Convert test key to PKCS#8 format (required by rcgen)
openssl pkcs8 -topk8 -nocrypt -in key-ec.pem -out key.pem
rm key-ec.pem

if [ -f "ca-key.pem" ] && [ -f "root-cert.pem" ] && [ -f "key.pem" ]; then
  echo "✓ Generated test certificates:"
  echo "  - ca-key.pem (CA private key)"
  echo "  - root-cert.pem (CA certificate)"
  echo "  - key.pem (test private key)"
  echo ""
  echo "These files are used by the test infrastructure for HBONE mTLS testing."
else
  echo "Error: Failed to generate certificate files"
  exit 1
fi

