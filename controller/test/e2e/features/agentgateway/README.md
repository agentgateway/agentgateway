# agentgateway e2e tests

## Setup

The agentgateway control plane is automatically enabled when installing kgateway.


## Testing with an unreleased agentgateway commit

Add this line to the kgateway go.mod to point to the unreleased agentgateway commit:
```shell
replace github.com/agentgateway/agentgateway => github.com/<my-fork>/agentgateway <branch-or-commit>
```

Then run:
```shell
go mod tidy
make generate-all -B
```

Build and install kgateway:
```shell
make run -B
```

In the agentgateway repo (on your branch), build the docker image locally with:
```shell
make docker
```

Then load it into the kind cluster where you are running the e2e tests:
```shell
AGW_TAG=$(docker images cr.agentgateway.dev/agentgateway --format "{{.Tag}}" | head -n 1)
kind load --name kind docker-image cr.agentgateway.dev/agentgateway:$AGW_TAG
```

You can either configure the agentgateway GatewayClass or a specific Gateway to use the local image.

GatewayClass example:
```shell
kubectl apply -f - <<EOF
kind: AgentgatewayParameters
apiVersion: agentgateway.dev/v1alpha1
metadata:
  name: agwp
  namespace: default
spec:
  logging:
    level: debug
  image:
    tag: $AGW_TAG
---
apiVersion: gateway.networking.k8s.io/v1
kind: GatewayClass
metadata:
  name: agentgateway
spec:
  controllerName: agentgateway.dev/agentgateway
  parametersRef:
    group: agentgateway.dev
    kind: AgentgatewayParameters
    name: agwp
    namespace: default
---
apiVersion: gateway.networking.k8s.io/v1
kind: Gateway
metadata:
  name: gw
spec:
  gatewayClassName: agentgateway
  listeners:
  - protocol: HTTP
    port: 8080
    name: http
    allowedRoutes:
      namespaces:
        from: All
EOF
```

Gateway example:
```shell
kubectl apply -f - <<EOF
kind: AgentgatewayParameters
apiVersion: agentgateway.dev/v1alpha1
metadata:
  name: agwp
  namespace: default
spec:
  logging:
    level: debug
  image:
    tag: $AGW_TAG
---
apiVersion: gateway.networking.k8s.io/v1
kind: Gateway
metadata:
  name: gw
spec:
  gatewayClassName: agentgateway
  infrastructure:
    parametersRef:
      group: agentgateway.dev
      kind: AgentgatewayParameters
      name: agwp
  listeners:
  - protocol: HTTP
    port: 8080
    name: http
    allowedRoutes:
      namespaces:
        from: All
EOF
```


This is useful for testing, but the final e2e agentgateway tests should use a released agentgateway image and not a local build.
