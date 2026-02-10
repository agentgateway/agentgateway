# Agentgateway API Upgrade Guide

## Overview

Starting in version v2.2.0-beta.4, agentgateway has been fully separated into its own API group with breaking changes to resource naming and structure. This guide helps you understand the new architecture and how to upgrade.

## Who Needs to Take Action?

### Envoy Users (GatewayClass: `kgateway`)

**No changes required.** You can upgrade to v2.2.0-beta.4 using the same kgateway chart. All Envoy resources remain unchanged:

- Continue using `gateway.kgateway.dev/v1alpha1` API group
- GatewayClass `kgateway` with controller `kgateway.dev/kgateway`
- HTTPListenerPolicy, TrafficPolicy, BackendConfigPolicy, etc. all work as before

### Agentgateway Users (GatewayClass: `agentgateway`)

**Breaking changes require migration to new charts.** Agentgateway has moved to:
- New dedicated Helm charts (separate from kgateway)
- New API group (`agentgateway.dev/v1alpha1`)

We recommend a **blue-green deployment strategy** (see below) to minimize risk.

### Mixed Deployment Users (Both Envoy and Agentgateway)

If you're currently running both Envoy and agentgateway from the same kgateway installation:
- **Envoy gateways**: Keep your existing kgateway chart installation
- **Agentgateway gateways**: Migrate to the new dedicated agentgateway charts following the blue-green strategy below
- Both can run side-by-side in the same cluster without conflicts

## What Changed for Agentgateway

### Dedicated Helm Charts

Agentgateway now has its own dedicated Helm charts, separate from the kgateway chart:

| Component | Chart Location                                            |
|-----------|-----------------------------------------------------------|
| **Agentgateway CRDs** | `oci://cr.agentgateway.dev/charts/agentgateway-crds`      |
| **Agentgateway Controller** | `oci://cr.agentgateway.dev/charts/agentgateway`           |
| **Kgateway CRDs (Envoy)** | `oci://cr.kgateway.dev/kgateway-dev/charts/kgateway-crds` |
| **Kgateway (Envoy)** | `oci://cr.kgateway.dev/kgateway-dev/charts/kgateway`      |


This separation allows independent versioning and deployment of each data plane.

### New API Group and Controller

Agentgateway resources now use their own dedicated API group and controller:

| Aspect | Old | New |
|--------|-----|-----|
| **API Group** | `gateway.kgateway.dev/v1alpha1` | `agentgateway.dev/v1alpha1` |
| **Controller Name** | `kgateway.dev/agentgateway` | `agentgateway.dev/agentgateway` |

### Agentgateway Resources (Now in agentgateway.dev/v1alpha1)

- `AgentgatewayPolicy` - Policy configuration for AI/ML routing
- `AgentgatewayBackend` - Backend configuration for AI/ML services  
- `AgentgatewayParameters` - Gateway deployment parameters

## Recommended Upgrade Strategy for Agentgateway Users

Due to the breaking changes, we recommend a **blue-green deployment** approach:

### Why Blue-Green?

In order to not conflict with your existing agentgateway installation, we recommend first manually creating a new 
GatewayClass with a different name to migrate your existing resources to:

```shell
kubectl apply -f - <<EOF
apiVersion: gateway.networking.k8s.io/v1
kind: GatewayClass
metadata:
  name: agentgateway-v2
spec:
  controllerName: agentgateway.dev/agentgateway # new controller name 
  description: Specialized class for agentgateway for blue-green upgrade.
EOF
```

The new agentgateway charts will still create a GatewayClass named `agentgateway` but with a different controller name 
(`agentgateway.dev/agentgateway`). Using the manually created `agentgateway-v2` GatewayClass will allow you to upgrade
without conflicts with your existing installation. This allows you to:

1. Install the new agentgateway charts side-by-side with your current deployment
2. Test the new setup thoroughly without affecting production traffic
3. Gradually switch traffic when ready
4. Keep the old version as a fallback

### Step-by-Step Upgrade Process

#### 1. Install New Agentgateway Charts in New Namespace

Agentgateway now has dedicated charts separate from kgateway. Install them in a new namespace:

```bash
# Install agentgateway CRDs
helm install agentgateway-crds oci://cr.agentgateway.dev/charts/agentgateway-crds \
  --version v2.2.0-beta.4 \
  --namespace agentgateway-system \
  --create-namespace

# Install agentgateway controller
helm install agentgateway oci://cr.agentgateway.dev/charts/agentgateway \
  --version v2.2.0-beta.4 \
  --namespace agentgateway-system
```
#### 2. Manually Create New GatewayClass

```shell
kubectl apply -f - <<EOF
apiVersion: gateway.networking.k8s.io/v1
kind: GatewayClass
metadata:
  name: agentgateway-v2
spec:
  controllerName: agentgateway.dev/agentgateway # new controller name 
  description: Specialized class for agentgateway for blue-green upgrade.
EOF
```

This creates the new `agentgateway-v2` GatewayClass with controller `agentgateway.dev/agentgateway`.

#### 3. Create New Gateway Using agentgateway-v2 GatewayClass

```yaml
apiVersion: gateway.networking.k8s.io/v1
kind: Gateway
metadata:
  name: ai-gateway-v2
  namespace: default
spec:
  gatewayClassName: agentgateway-v2  # New GatewayClass
  listeners:
    - name: http
      protocol: HTTP
      port: 8080
```

The new GatewayClass configuration:

```yaml
apiVersion: gateway.networking.k8s.io/v1
kind: GatewayClass
metadata:
  name: agentgateway-v2
spec:
  controllerName: agentgateway.dev/agentgateway  # New controller
  parametersRef:
    group: agentgateway.dev  # New API group
    kind: AgentgatewayParameters
    name: agw-params
```

#### 4. Migrate Your Configuration

Update your agentgateway resources to use the new API group:

**AgentgatewayPolicy:**
```yaml
apiVersion: agentgateway.dev/v1alpha1  # Changed from TrafficPolicy in gateway.kgateway.dev/v1alpha1
kind: AgentgatewayPolicy
metadata:
  name: my-ai-policy
spec:
  targetRefs:
    - group: gateway.networking.k8s.io
      kind: HTTPRoute
      name: my-route
  # Your policy configuration
```

**AgentgatewayBackend:**
```yaml
apiVersion: agentgateway.dev/v1alpha1  # Changed from Backend in gateway.kgateway.dev/v1alpha1
kind: AgentgatewayBackend
metadata:
  name: my-ai-backend
spec:
  # Your backend configuration
```

#### 5. Test the New Setup

- Verify the new Gateway is ready
- Test your HTTPRoutes with the new Gateway
- Validate policies are applied correctly
- Check backend connectivity

#### 6. Clean Up Old Installation

After confirming everything works with the new agentgateway installation:

```bash
# Remove old kgateway installation (if it was only used for agentgateway)
helm uninstall kgateway --namespace kgateway-system
helm uninstall kgateway-crds --namespace kgateway-system
```

**Note:** If your old kgateway installation also served Envoy gateways, you'll want to keep it running and only remove the agentgateway-specific resources.

## Resource Reference

### For Agentgateway Users (agentgateway.dev/v1alpha1)

Use these resources with GatewayClass `agentgateway` with the new `agentgateway.dev/agentgateway` controller:

```yaml
# Policy attachment for AI/ML routing
apiVersion: agentgateway.dev/v1alpha1
kind: AgentgatewayPolicy

# Backend configuration for AI services  
apiVersion: agentgateway.dev/v1alpha1
kind: AgentgatewayBackend

# Gateway deployment parameters
apiVersion: agentgateway.dev/v1alpha1
kind: AgentgatewayParameters
```

### For Envoy Users (gateway.kgateway.dev/v1alpha1)

Use these resources with GatewayClass `kgateway` (unchanged):

```yaml
# HTTP listener configuration
apiVersion: gateway.kgateway.dev/v1alpha1
kind: HTTPListenerPolicy

# Listener configuration (for TCP/TLS listeners)
apiVersion: gateway.kgateway.dev/v1alpha1
kind: ListenerPolicy

# Traffic management policies
apiVersion: gateway.kgateway.dev/v1alpha1
kind: TrafficPolicy

# Backend service configuration
apiVersion: gateway.kgateway.dev/v1alpha1
kind: BackendConfigPolicy

# Backend reference for external services
apiVersion: gateway.kgateway.dev/v1alpha1
kind: Backend

# Direct response configuration
apiVersion: gateway.kgateway.dev/v1alpha1
kind: DirectResponse

# Gateway extensions for advanced features
apiVersion: gateway.kgateway.dev/v1alpha1
kind: GatewayExtensions

# Gateway deployment parameters
apiVersion: gateway.kgateway.dev/v1alpha1
kind: GatewayParameters
```
