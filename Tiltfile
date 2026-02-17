# Tiltfile for AgentGateway development
# This deploys both control plane (Go) and data plane (Rust) to Kind with live updates
load('ext://restart_process', 'docker_build_with_restart')

# Configuration
version = 'v1.0.1-dev'
git_revision = str(local('git rev-parse HEAD')).strip()
cluster_name = 'kind'
install_namespace = k8s_namespace()
image_registry = 'localhost:5000'

default_registry(image_registry)

# Ensure Kind cluster exists
allow_k8s_contexts('kind-' + cluster_name)

# Helper to run Make targets
def run_make(target, cwd='.'):
    return local('make -C ' + cwd + ' ' + target)

def run_controller_make(target):
    return run_make(target, cwd='./controller')
# =============================================================================
# Setup: Ensure cluster is ready
# =============================================================================

# Check if kind cluster exists, create if not
if str(local('kind get clusters 2>/dev/null | grep -c "^' + cluster_name + '$" || true')).strip() == '0':
    print('Creating kind cluster...')
    local('ctlptl create cluster kind --name kind-' + cluster_name + ' --registry=ctlptl-registry')
    fail("started kind cluster. run tilt again")

print('Installing Gateway API CRDs...')
run_controller_make('gw-api-crds')
run_controller_make('gie-crds')

# Install CRDs
print('Installing AgentGateway CRDs...')
k8s_yaml(helm(
    'controller/install/helm/agentgateway-crds',
    name='agentgateway-crds',
    namespace=install_namespace,
    set=[
        'version=' + version,
    ]
))

# =============================================================================
# Control Plane (Go-based controller)
# =============================================================================

local_resource(
  'go-compile-controller',
  'make -C ./controller VERSION=' + version + ' GCFLAGS=all="-N -l" agentgateway-controller && mv ./controller/_output/pkg/agentgateway/agentgateway-$(go env GOOS)-$(go env GOARCH) ./hack/tilt/agentgateway-controller',
  deps=['./controller/'],
  ignore=['./controller/_output/'],
)

# Build control plane Docker image
docker_build_with_restart(
    image_registry + '/agentgateway-controller' ,
    context='./hack/tilt/',
    entrypoint='/usr/local/bin/agentgateway-controller',
    dockerfile_contents="""
FROM ubuntu:24.04
COPY agentgateway-controller /usr/local/bin/agentgateway-controller
ENTRYPOINT /usr/local/bin/agentgateway-controller
    """,
    # Live update: sync Go binaries
    live_update=[
        # Sync Go code changes
        sync('./hack/tilt/agentgateway-controller', '/usr/local/bin/agentgateway-controller'), 
    ],
    only=[
        './agentgateway-controller',
    ],
)


# =============================================================================
# Deploy via Helm
# =============================================================================

# Deploy AgentGateway via Helm
k8s_yaml(helm(
    'controller/install/helm/agentgateway',
    name='agentgateway',
    namespace=install_namespace,
    set=[
        'image.registry=' + image_registry,
        'image.tag=' + version,
        'image.pullPolicy=IfNotPresent',
        'controller.image.repository=agentgateway-controller',
        'controller.image.tag=' + version,
        'controller.replicaCount=1',
        'controller.logLevel=debug',
        'proxy.image.repository=agentgateway',
        'proxy.image.tag=' + version,
    ],
    values=['./controller/hack/helm/dev.yaml'] if os.path.exists('controller/hack/helm/dev.yaml') else [],
 ))

k8s_resource('agentgateway',
             resource_deps=['go-compile-controller'])

# =============================================================================
# Data Plane (Rust-based proxy)
# =============================================================================

local_resource(
  'rust-compile-dataplane',
  'cargo build && ln -f ./target/debug/agentgateway ./hack/tilt/agentgateway',
  deps=['./crates',
        './Cargo.toml',
        './Cargo.lock',
        './.cargo'])
# 
# Build data plane Docker image
docker_build(
    'agentgateway',
    context='./hack/tilt/',
    dockerfile_contents="""
FROM ubuntu:24.04
COPY start.sh /scripts/start.sh
COPY restart.sh /scripts/restart.sh
COPY agentgateway /usr/local/bin/
ENTRYPOINT ["/scripts/start.sh", "/usr/local/bin/agentgateway"]
    """,
    live_update=[
        sync('./hack/tilt/agentgateway', '/usr/local/bin/agentgateway'),
        run('/scripts/restart.sh'),
    ],
    only=[
        './agentgateway',
        './start.sh',
        './restart.sh',
    ],
)

k8s_kind('AgentgatewayParameters', image_object={'json_path': '{.spec.image}', 'repo_field': 'repository', 'tag_field': 'tag'})
k8s_kind('Gateway', pod_readiness='wait')

k8s_yaml(blob("""
apiVersion: agentgateway.dev/v1alpha1
kind: AgentgatewayParameters
metadata:
  name: image-params
spec:
  image:
    registry: ""
    repository: agentgateway
    tag: """ + version + """
  deployment:
    spec:
      template:
        spec:
          containers:
          # Delete container-level securityContext so that Tilt can apply live updates
          - name: agentgateway
            securityContext:
             $patch: delete
---
kind: Gateway
apiVersion: gateway.networking.k8s.io/v1
metadata:
  name: tilt-gw
spec:
  gatewayClassName: agentgateway
  infrastructure:
    parametersRef:
      group: agentgateway.dev
      kind: AgentgatewayParameters
      name: image-params
  listeners:
    - name: http
      protocol: HTTP
      port: 8080
"""))
k8s_resource(workload='image-params', extra_pod_selectors={"gateway.networking.k8s.io/gateway-name":"tilt-gw"}, 
# objects=[ "image-params:agentgatewayparameters"],
 resource_deps=['rust-compile-dataplane'])
