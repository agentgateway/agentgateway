# Tiltfile for AgentGateway development
# This deploys both control plane (Go) and data plane (Rust) to Kind with live updates.
load('ext://restart_process', 'docker_build_with_restart')
load('ext://helm_resource', 'helm_resource')

def env_command(action):
    return './tools/dev-env.sh --profile local ' + action

def load_dev_env_config():
    return decode_json(str(local(env_command('config'))).strip())

dev_env = load_dev_env_config()
version = dev_env['image_tag']
default_install_namespace = dev_env['install_namespace']
tilt_namespace = k8s_namespace()
install_namespace = tilt_namespace if tilt_namespace else default_install_namespace
image_registry = dev_env['registry_host']
cluster_context = dev_env['cluster_context']
controller_image_repository = dev_env['controller_image_repository']
proxy_image_repository = dev_env['proxy_image_repository']

env_bootstrap_deps = [
    './tools/dev-env.sh',
    './tools/dev_env_config.py',
    './dev/cluster/local.ctlptl.yaml',
    './dev/profiles/base.yaml',
    './dev/profiles/local.yaml',
]

allow_k8s_contexts(cluster_context)

local_resource(
    'env-cluster',
    env_command('cluster'),
    deps=env_bootstrap_deps,
    allow_parallel=False,
)

local_resource(
    'env-addons',
    env_command('crds'),
    deps=env_bootstrap_deps + [
        './controller/test/setup/metallb.yaml',
        './controller/Makefile',
    ],
    resource_deps=['env-cluster'],
    allow_parallel=False,
)

local_resource(
    'env-ready',
    env_command('ready'),
    deps=env_bootstrap_deps + [
        './controller/test/setup/metallb.yaml',
        './controller/Makefile',
    ],
    resource_deps=['env-addons'],
    allow_parallel=False,
)

# =============================================================================
# Cluster add-ons
# =============================================================================

helm_resource(
    'agentgateway-crds',
    'controller/install/helm/agentgateway-crds',
    namespace=install_namespace,
    flags=['--set=version=' + version],
)
k8s_resource('agentgateway-crds', resource_deps=['env-ready'])

# =============================================================================
# Control plane (Go-based controller)
# =============================================================================

local_resource(
    'go-compile-controller',
    'make -C ./controller VERSION=' + version + ' GCFLAGS=all="-N -l" agentgateway-controller && mv ./controller/_output/pkg/agentgateway/agentgateway-linux-$(go env GOARCH) ./tools/tilt/agentgateway-controller',
    deps=['./controller/'],
    ignore=['./controller/_output/'],
)

docker_build_with_restart(
    image_registry + '/' + controller_image_repository,
    context='./tools/tilt/',
    entrypoint='/usr/local/bin/agentgateway-controller',
    dockerfile_contents="""
FROM ubuntu:24.04
COPY agentgateway-controller /usr/local/bin/agentgateway-controller
ENTRYPOINT /usr/local/bin/agentgateway-controller
    """,
    live_update=[
        sync('./tools/tilt/agentgateway-controller', '/usr/local/bin/agentgateway-controller'),
    ],
    only=[
        './agentgateway-controller',
    ],
)

# =============================================================================
# Deploy via Helm
# =============================================================================

k8s_yaml(helm(
    'controller/install/helm/agentgateway',
    name='agentgateway',
    namespace=install_namespace,
    set=[
        'image.registry=' + image_registry,
        'image.tag=' + version,
        'image.pullPolicy=IfNotPresent',
        'controller.image.repository=' + controller_image_repository,
        'controller.image.tag=' + version,
        'controller.replicaCount=1',
        'controller.logLevel=debug',
        'proxy.image.repository=' + proxy_image_repository,
        'proxy.image.tag=' + version,
    ],
    values=[config.main_dir + '/controller/hack/helm/dev.yaml'] if os.path.exists(config.main_dir + '/controller/hack/helm/dev.yaml') else [],
))

k8s_resource(
    'agentgateway',
    resource_deps=['env-ready', 'agentgateway-crds', 'go-compile-controller'],
)

# =============================================================================
# Data plane (Rust-based proxy)
# =============================================================================

local_resource(
    'rust-compile-dataplane',
    'cargo build && if [ -f "./tools/tilt/agentgateway" ]; then rm "./tools/tilt/agentgateway"; fi && mv ./target/debug/agentgateway ./tools/tilt/agentgateway',
    deps=['./crates', './Cargo.toml', './Cargo.lock', './.cargo'],
)

docker_build(
    proxy_image_repository,
    context='./tools/tilt/',
    dockerfile_contents="""
FROM ubuntu:24.04
COPY start.sh /scripts/start.sh
COPY restart.sh /scripts/restart.sh
COPY agentgateway /usr/local/bin/
ENTRYPOINT ["/scripts/start.sh", "/usr/local/bin/agentgateway"]
    """,
    live_update=[
        sync('./tools/tilt/agentgateway', '/usr/local/bin/agentgateway'),
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
  name: dataplane-dev-gwparams
spec:
  image:
    registry: ""
    repository: """ + proxy_image_repository + """
    tag: """ + version + """
  deployment:
    spec:
      template:
        spec:
          containers:
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
      name: dataplane-dev-gwparams
  listeners:
    - name: http
      protocol: HTTP
      port: 8080
"""))

k8s_resource(
    workload='dataplane-dev-gwparams',
    extra_pod_selectors={"gateway.networking.k8s.io/gateway-name": "tilt-gw"},
    resource_deps=['env-ready', 'agentgateway', 'rust-compile-dataplane'],
)
