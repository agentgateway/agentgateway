# mTLS + OIDC Certificate Passthrough Example

This example demonstrates how to use **AgentGateway** as an mTLS-terminating proxy that extracts client certificates and forwards them to **Keycloak** for passwordless OIDC authentication.

## Architecture

1.  **Frontend (Browser/Curl)**: Connects to AgentGateway via `HTTPS` on port `3000`. mTLS is required.
2.  **AgentGateway**:
    *   Terminates mTLS.
    *   Extracts the certificate using the CEL expression `source.certificate`.
    *   Forwards the cert to Keycloak in the `SSL_CLIENT_CERT` header via the `/auth` route.
    *   Handles the OIDC flow for the root `/` route.
3.  **Keycloak**:
    *   Listens on port `7080` (internal).
    *   Uses the `nginx` X.509 SPI provider to decode the certificate from the header.
    *   Automatically logs in the user if the certificate Subject matches a user (e.g., `CN=test-client`).
4.  **Backend App**: A simple HTTP echo service that displays your authenticated identity.

## Prerequisites

*   **Rust Patch**: This example requires a fix in the AgentGateway source code (provided during the setup of this example) to prevent OIDC cookie stripping. Ensure you have rebuilt the gateway with `cargo build`.
*   **OpenSSL**: For generating certificates and browser bundles.

## Setup

1.  **Generate Certificates**:
    ```bash
    ./certs/gen.sh
    ```

2.  **Create Browser Certificate Bundle**:
    Browsers require a PKCS#12 bundle (`.p12`) to use client certificates.
    ```bash
    openssl pkcs12 -export -out certs/client.p12 \
                   -inkey certs/client-key.pem \
                   -in certs/client-cert.pem \
                   -name "test-client" \
                   -passout pass:1234
    ```

## Running the Example

1.  **Start Keycloak and the App**:
    ```bash
    docker compose up -d
    ```

2.  **Start AgentGateway**:
    ```bash
    # Ensure you are using the rebuilt binary with the fix
    export OIDC_COOKIE_SECRET="a-very-secret-key-32-chars-long!!"
    cargo run -- -f config.yaml
    ```

3.  **Configure your Browser (Firefox/Chrome)**:
    *   **Import Client Cert**: Go to Browser Settings -> Certificates -> View Certificates -> Your Certificates -> Import. Select `certs/client.p12` (password: `1234`).
    *   **Trust CA**: In the Authorities tab, import `certs/ca-cert.pem` and check "Trust this CA to identify websites".

4.  **Test the Flow**:
    Open **`https://localhost:3000/`** in your browser.
    *   Select the `test-client` certificate when prompted.
    *   The gateway will redirect you through Keycloak.
    *   You should be logged in automatically and see the message: 
        `"Success! You are authenticated via mTLS certificate and OIDC."`

## Debugging with Curl

You can verify the entire flow (including redirects) from the terminal:

```bash
curl -v -k -L --cert certs/client-cert.pem \
              --key certs/client-key.pem \
              https://localhost:3000/
```

Check the `mtls-keycloak` logs to see the certificate being processed:
```bash
docker logs -f mtls-keycloak
```

## Configuration Summary

- **Gateway**: `https://localhost:3000`
- **Keycloak**: `http://localhost:7080` (Internal, proxied at `/auth`)
- **Backend App**: `http://localhost:18080` (Internal, proxied at `/`)
- **Test User**: `CN=test-client` (Matches the certificate Subject)
