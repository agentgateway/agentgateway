# Copilot in Kubernetes

Route requests to GitHub Copilot through a Gateway and HTTPRoute. Use CRDs, controller and data-plane builds with Kubernetes Copilot support, and make sure the `agentgateway` GatewayClass and `agentgateway-system` namespace exist before applying the example.

## Backend Credential

Create a Secret named `copilot-token` in `agentgateway-system` using your usual secret-management process. Its `Authorization` data key accepts a bare credential or one prefixed with `Bearer `.

Use the same non-expiring credential as standalone Copilot. Agentgateway does not obtain, exchange or refresh it. Don't assume an arbitrary GitHub personal access token will work.

An explicit Secret takes precedence over local Copilot or GitHub CLI credentials. A missing Secret, missing key or empty value reports a configuration error and keeps an explicit empty backend key, so requests cannot fall back to credentials on the gateway host. Upstream authentication and entitlement errors reach the client.

## Route Requests

Apply [kubernetes.yaml](kubernetes.yaml) after creating the Secret. It creates a Gateway named `copilot`, an `AgentgatewayBackend` and an HTTPRoute for these paths:

| Client Path | API Format |
| --- | --- |
| `/v1/chat/completions` | Chat Completions |
| `/v1/messages` | Messages |
| `/v1/responses` | Responses |

Send a model your account can use in the request body, and choose an API format that model supports. Set `stream: true` for streaming.

`copilot: {}` uses the request's model. Set `spec.ai.provider.copilot.model` to override it. Copilot also works in named provider groups and with `provider: Copilot` in `AgentgatewayModel`, using the existing model matching, selection and endpoint settings.

The default upstream is `api.githubcopilot.com:443`, with TLS verification and the existing provider's request-path selection. Host, port and path overrides still apply. Backend transformations and request-header modifiers run after Copilot's protocol defaults and can override them.

## Replace or Revoke a Credential

To replace a credential, update the Secret's `Authorization` value. The controller watches for changes and updates the backend after reconciliation. No gateway restart is needed. Restarting does not renew credentials.

Revoke the credential through its issuer. Deleting or invalidating the Secret prevents host-credential fallback after reconciliation, but does not revoke the credential at GitHub or cancel requests already in flight.

## Policies and Telemetry

Copilot requests use the existing LLM policies, metrics and tracing. Configure frontend authentication and authorization separately if you need caller identity or per-user attribution. The provider doesn't identify callers.
