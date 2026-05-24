#!/bin/bash
set -e
cd $(dirname $0)

# Generate CA
openssl req -x509 -newkey rsa:2048 -keyout ca-key.pem -out ca-cert.pem -days 365 -nodes -subj "/CN=Test CA"

# Generate Server cert
openssl req -newkey rsa:2048 -keyout key.pem -out server.csr -nodes -subj "/CN=localhost"
openssl x509 -req -in server.csr -CA ca-cert.pem -CAkey ca-key.pem -CAcreateserial -out cert.pem -days 365

# Generate Client cert
openssl req -newkey rsa:2048 -keyout client-key.pem -out client.csr -nodes -subj "/CN=test-client"
openssl x509 -req -in client.csr -CA ca-cert.pem -CAkey ca-key.pem -CAcreateserial -out client-cert.pem -days 365
openssl pkcs12 -export -out client.p12 -inkey client-key.pem -in client-cert.pem -name "test-client" -passout pass:1234

rm *.csr *.srl
