#!/bin/bash

# Generate CA private key and certificate
step-cli certificate create "agentgateway.dev" \
  ca-cert.pem ca-key.pem \
  --profile root-ca \
  --no-password --insecure

# Generate localhost private key and certificate signed by CA
step-cli certificate create localhost \
  cert.pem key.pem \
  --profile leaf \
  --ca ca-cert.pem \
  --ca-key ca-key.pem \
  --san localhost \
  --no-password --insecure

echo "Done! Generated files:"
echo "  CA: ca-key.pem, ca-cert.pem"
echo "  Localhost: key.pem, cert.pem"
