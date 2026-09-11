# NetBird Agent Network with agentgateway

These examples place [agentgateway](https://agentgateway.dev/) behind a
[NetBird Agent Network](https://netbird.ai/) endpoint. NetBird authenticates
and authorizes callers, replaces identity headers with trusted values, and
forwards requests to a private agentgateway listener for LLM routing.

## Kubernetes

The [Kubernetes example](kubernetes/README.md) uses Gateway API, cert-manager,
and the agentgateway controller. It includes public management routing, a
private AI gateway, NetworkPolicy isolation, and an in-cluster NetBird client
for verification.

## Standalone

The [standalone example](standalone/README.md) uses Docker Compose to run the
NetBird server, dashboard, Agent Network proxy, test client, and two standalone
agentgateway instances on one Docker host. It uses generated static
certificates instead of cert-manager.
