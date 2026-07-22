package agentgateway

import (
	corev1 "k8s.io/api/core/v1"
	apiextensionsv1 "k8s.io/apiextensions-apiserver/pkg/apis/apiextensions/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
)

// +kubebuilder:rbac:groups=agentgateway.dev,resources=agentgatewayparameters,verbs=get;list;watch
// +kubebuilder:rbac:groups=agentgateway.dev,resources=agentgatewayparameters/status,verbs=get;update;patch

// Configures dynamic provisioning for the agentgateway data plane.
// Labels and annotations that apply to
// all resources may be specified at a higher level; see
// https://gateway-api.sigs.k8s.io/reference/api-spec/main/spec/#gatewayinfrastructure
//
// +kubebuilder:printcolumn:name="Age",type=date,JSONPath=`.metadata.creationTimestamp`
// +genclient
// +kubebuilder:object:root=true
// +kubebuilder:metadata:labels={app=agentgateway,app.kubernetes.io/name=agentgateway}
// +kubebuilder:resource:categories=agentgateway,shortName=agpar,path=agentgatewayparameters
// +kubebuilder:subresource:status
// +kubebuilder:metadata:labels="gateway.networking.k8s.io/policy=Direct"
type AgentgatewayParameters struct {
	metav1.TypeMeta `json:",inline"`
	// metadata for the object
	// More info: https://git.k8s.io/community/contributors/devel/sig-architecture/api-conventions.md#metadata
	// +optional
	metav1.ObjectMeta `json:"metadata"`

	// Desired data plane provisioning settings.
	// +required
	Spec AgentgatewayParametersSpec `json:"spec"`

	// Current status for these provisioning settings.
	// +optional
	Status AgentgatewayParametersStatus `json:"status"`
}

// Current status for these provisioning settings.
type AgentgatewayParametersStatus struct{}

// +kubebuilder:object:root=true
type AgentgatewayParametersList struct {
	metav1.TypeMeta `json:",inline"`
	metav1.ListMeta `json:"metadata"`
	Items           []AgentgatewayParameters `json:"items"`
}

// +kubebuilder:validation:XValidation:rule="!has(self.deployment) || !has(self.workload) || !has(self.workload.kind) || self.workload.kind == 'Deployment'",message="deployment overlays are only valid when workload.kind is Deployment or unset"
// +kubebuilder:validation:XValidation:rule="!has(self.daemonSet) || (has(self.workload) && has(self.workload.kind) && self.workload.kind == 'DaemonSet')",message="daemonSet overlays are only valid when workload.kind is DaemonSet"
// +kubebuilder:validation:XValidation:rule="!has(self.horizontalPodAutoscaler) || !has(self.workload) || !has(self.workload.kind) || self.workload.kind != 'DaemonSet'",message="horizontalPodAutoscaler is not valid when workload.kind is DaemonSet"
type AgentgatewayParametersSpec struct {
	AgentgatewayParametersConfigs  `json:",inline"`
	AgentgatewayParametersOverlays `json:",inline"`
}

// The default logging format is text.
// +k8s:enum
type AgentgatewayParametersLoggingFormat string

const (
	AgentgatewayParametersLoggingJson AgentgatewayParametersLoggingFormat = "json"
	AgentgatewayParametersLoggingText AgentgatewayParametersLoggingFormat = "text"
)

// AgentgatewayParametersWorkloadKind selects the Kubernetes workload kind used
// for a managed Gateway data plane.
// +k8s:enum
type AgentgatewayParametersWorkloadKind string

const (
	// AgentgatewayParametersWorkloadDeployment uses a Deployment for the managed
	// Gateway data plane.
	AgentgatewayParametersWorkloadDeployment AgentgatewayParametersWorkloadKind = "Deployment"

	// AgentgatewayParametersWorkloadDaemonSet uses a DaemonSet for the managed
	// Gateway data plane.
	AgentgatewayParametersWorkloadDaemonSet AgentgatewayParametersWorkloadKind = "DaemonSet"
)

// AgentgatewayParametersWorkload selects the Kubernetes workload kind used for
// a managed Gateway data plane.
type AgentgatewayParametersWorkload struct {
	// Kind selects the Kubernetes workload kind. When unset, Deployment is used.
	//
	// +optional
	Kind AgentgatewayParametersWorkloadKind `json:"kind,omitempty"`
}

type AgentgatewayParametersLogging struct {
	// Logging level in standard `RUST_LOG` syntax, for example `info` (the
	// default), or a comma-separated per-module setting such as
	// `rmcp=warn,hickory_server::server::server_future=off,typespec_client_core::http::policies::logging=warn`.
	// +optional
	Level string `json:"level,omitempty"`
	// Logging output format.
	// +optional
	Format AgentgatewayParametersLoggingFormat `json:"format,omitempty"`
}

type AgentgatewayParametersConfigs struct {
	// `workload` selects the Kubernetes workload kind for the managed Gateway
	// data plane. If unset, Deployment is used.
	//
	// +optional
	Workload *AgentgatewayParametersWorkload `json:"workload,omitempty"`

	// Logging configuration. By default, all logs are set to
	// `info` level.
	// +optional
	Logging *AgentgatewayParametersLogging `json:"logging,omitempty"`

	// Raw agentgateway configuration to merge into the generated config file.
	// This is merged with
	// configuration derived from typed fields like `logging.format`, and those
	// typed fields will take precedence.
	//
	// Example:
	//
	//	rawConfig:
	//	  binds:
	//	  - port: 3000
	//	    listeners:
	//	    - routes:
	//	      - policies:
	//	          cors:
	//	            allowOrigins:
	//	            - "*"
	//	            allowHeaders:
	//	            - mcp-protocol-version
	//	            - content-type
	//	            - cache-control
	//	        backends:
	//	        - mcp:
	//	            targets:
	//	            - name: everything
	//	              stdio:
	//	                cmd: npx
	//	                args: ["@modelcontextprotocol/server-everything"]
	//
	// +optional
	// +kubebuilder:validation:Type=object
	// +kubebuilder:pruning:PreserveUnknownFields
	RawConfig *apiextensionsv1.JSON `json:"rawConfig,omitempty"`

	// The agentgateway container image. See
	// https://kubernetes.io/docs/concepts/containers/images
	// for details.
	//
	// Default values, which may be overridden individually:
	//
	//	registry: cr.agentgateway.dev
	//	repository: agentgateway
	//	tag: <agentgateway version>
	//	pullPolicy: <omitted, relying on Kubernetes defaults which depend on the tag>
	//
	// +optional
	Image *Image `json:"image,omitempty"`

	// Container environment variables. These override any existing
	// values. If you want to delete an environment variable entirely, use
	// `$patch: delete` with an overlay instead. Note that
	// [variable
	// expansion](https://kubernetes.io/docs/tasks/inject-data-application/define-interdependent-environment-variables/)
	// does apply, but is highly discouraged -- to set dependent environment
	// variables, you can use `$(VAR_NAME)`, but it's highly discouraged.
	// `$$(VAR_NAME)` avoids expansion and results in a literal
	// `$(VAR_NAME)`.
	//
	// If `SESSION_KEY` is specified, it takes precedence over the
	// controller-managed per-`Gateway` session key `Secret`.
	//
	// +optional
	Env []corev1.EnvVar `json:"env,omitempty"`

	// Compute resources required by this container. See
	// https://kubernetes.io/docs/concepts/configuration/manage-resources-containers/
	// for details.
	//
	// +optional
	Resources *corev1.ResourceRequirements `json:"resources,omitempty"`

	// Shutdown delay configuration. How graceful planned or unplanned data
	// plane changes happen is in tension with how quickly rollouts of the data
	// plane complete. How long a data plane pod must wait for shutdown to be
	// perfectly graceful depends on how you have configured your `Gateway`
	// resources.
	//
	// +optional
	Shutdown *ShutdownSpec `json:"shutdown,omitempty"`

	// Istio integration settings. If enabled, agentgateway can natively connect to Istio-enabled pods with mTLS.
	//
	// +optional
	Istio *IstioSpec `json:"istio,omitempty"`

	// SPIFFE integration settings. When set, the gateway sources its TLS identity (X.509-SVID)
	// and trust bundle from the local SPIFFE Workload API, and the controller
	// mounts the Workload API socket into the pod. Listeners and backends opt in to SPIFFE individually
	// (via the `agentgateway.dev/tls-certificate-source: SPIFFE` listener option and the
	// AgentgatewayPolicy `backend.tls.certificateSource: SPIFFE` field respectively).
	//
	// +optional
	Spiffe *SpiffeSpec `json:"spiffe,omitempty"`

	// Model cost catalog sources. Only effective when set on a Gateway-level
	// AgentgatewayParameters (via Gateway.spec.infrastructure.parametersRef);
	// ignored on GatewayClass-level parameters because ConfigMap references
	// are resolved from the Gateway's deployment namespace.
	//
	// +optional
	ModelCatalog *ModelCatalogSpec `json:"modelCatalog,omitempty"`
}

// ModelCatalogSpec configures model cost catalog sources for the agentgateway proxy.
type ModelCatalogSpec struct {
	// +optional
	Sources []ModelCatalogSource `json:"sources,omitempty"`
}

// ModelCatalogSource is a single source of model cost catalog data.
type ModelCatalogSource struct {
	// +optional
	ConfigMap *ModelCatalogConfigMapRef `json:"configMap,omitempty"`
}

// ModelCatalogConfigMapRef identifies a ConfigMap holding model cost catalog JSON.
// The ConfigMap must be in the same namespace as the Gateway that references it.
type ModelCatalogConfigMapRef struct {
	// +required
	// +kubebuilder:validation:MinLength=1
	Name string `json:"name"`

	// Data key whose value is the catalog JSON. Defaults to "catalog.json".
	//
	// +optional
	Key string `json:"key,omitempty"`
}

type IstioSpec struct {
	// Explicitly turns Istio integration on or off for this gateway.
	//
	// +optional
	Enabled *bool `json:"enabled,omitempty"`
	// Address of the Istio CA. If unset, defaults to `https://istiod.istio-system.svc:15012`.
	//
	// +optional
	CaAddress string `json:"caAddress,omitempty"`
	// Istio trust domain. If not set, defaults to `cluster.local`, or the default
	// trust domain for the control plane's istio revision.
	//
	// +optional
	TrustDomain string `json:"trustDomain,omitempty"`
	// Additional SPIFFE trust domains accepted on inbound HBONE connections.
	// The local trust domain is always implicitly included.
	//
	// +optional
	AdditionalTrustDomains []string `json:"additionalTrustDomains,omitempty"`
	// ID of the cluster this gateway runs in. If unset, defaults to `Kubernetes`.
	//
	// +optional
	ClusterId string `json:"clusterId,omitempty"`
	// Istio network this gateway runs in. If unset, defaults to the empty network.
	//
	// +optional
	Network string `json:"network,omitempty"`
}

// SpiffeSpec configures gateway-wide SPIFFE Workload API integration: where the Workload API
// socket comes from (mounted into the pod by the controller) and how long to wait for
// the initial connection.
type SpiffeSpec struct {
	// Explicitly turns SPIFFE integration on or off for this gateway. When unset, the presence
	// of the spiffe block opts in. Set to false on a Gateway-level AgentgatewayParameters to opt
	// a gateway out of SPIFFE enabled at the GatewayClass level.
	//
	// +optional
	Enabled *bool `json:"enabled,omitempty"`

	// Volume source for the SPIFFE Workload API socket. When omitted (i.e. `spiffe: {}`),
	// the socket is sourced from the SPIFFE CSI driver with default settings.
	//
	// +optional
	Source *SpiffeWorkloadAPISource `json:"source,omitempty"`

	// How long to wait (in seconds) for the initial connection to the SPIFFE Workload API
	// before failing startup. Rendered into the gateway config as `spiffeConnectTimeout`.
	// Defaults to 10.
	//
	// +optional
	// +kubebuilder:validation:Minimum=1
	ConnectTimeout *int64 `json:"connectTimeout,omitempty"`
}

// SpiffeWorkloadAPISource describes how the SPIFFE Workload API socket is mounted into the
// gateway pod. At most one of `csi` or `hostPath` may be set; when neither is set, the SPIFFE
// CSI driver is used. `mountPath` and `socketName` describe the container-side location of the
// socket and apply regardless of the source kind.
//
// +kubebuilder:validation:AtMostOneOf=csi;hostPath
type SpiffeWorkloadAPISource struct {
	// Source the Workload API socket from the SPIFFE CSI driver (the default).
	//
	// +optional
	CSI *SpiffeCSISource `json:"csi,omitempty"`

	// Source the Workload API socket from a host directory.
	//
	// +optional
	HostPath *SpiffeHostPathSource `json:"hostPath,omitempty"`

	// Mount path inside the container for the Workload API socket directory.
	// Must be an absolute path. Defaults to `/spiffe-workload-api`.
	//
	// +optional
	// +kubebuilder:validation:Pattern=`^/`
	MountPath string `json:"mountPath,omitempty"`

	// Socket filename within the mount directory. Defaults to `spire-agent.sock`.
	//
	// +optional
	SocketName string `json:"socketName,omitempty"`
}

// SpiffeCSISource sources the SPIFFE Workload API socket from a CSI driver (the SPIFFE CSI driver).
type SpiffeCSISource struct {
	// CSI driver name. Defaults to `csi.spiffe.io`.
	//
	// +optional
	Driver string `json:"driver,omitempty"`
}

// SpiffeHostPathSource sources the SPIFFE Workload API socket from a directory on the host node.
//
// Note: this mounts an arbitrary host directory (read-only) into the gateway pod, so anyone
// who can set it can read that directory's contents. Prefer the CSI source, and consider
// restricting hostPath to GatewayClass-level AgentgatewayParameters managed by cluster admins.
type SpiffeHostPathSource struct {
	// Host directory containing the SPIFFE Workload API socket, e.g. `/run/spire/agent-sockets`.
	//
	// +required
	// +kubebuilder:validation:MinLength=1
	Path string `json:"path"`
}

// +kubebuilder:validation:XValidation:rule="self.min <= self.max",message="The 'min' value must be less than or equal to the 'max' value."
type ShutdownSpec struct {
	// Minimum time (in seconds) to wait before allowing Agentgateway to
	// terminate. Refer to the `CONNECTION_MIN_TERMINATION_DEADLINE`
	// environment variable for details.
	//
	// +required
	// +kubebuilder:validation:Minimum=0
	// +kubebuilder:validation:Maximum=31536000
	Min int64 `json:"min"`

	// Maximum time (in seconds) to wait before allowing Agentgateway to
	// terminate. Refer to the `TERMINATION_GRACE_PERIOD_SECONDS`
	// environment variable for details.
	//
	// +required
	// +kubebuilder:validation:Minimum=0
	// +kubebuilder:validation:Maximum=31536000
	Max int64 `json:"max"`
}

type AgentgatewayParametersOverlays struct {
	// Overrides for the generated
	// `Deployment` resource.
	// +optional
	Deployment *KubernetesResourceOverlay `json:"deployment,omitempty"`

	// Overrides for the generated
	// `DaemonSet` resource.
	// +optional
	DaemonSet *KubernetesResourceOverlay `json:"daemonSet,omitempty"`

	// Overrides for the generated `Service`
	// resource.
	// +optional
	Service *KubernetesResourceOverlay `json:"service,omitempty"`

	// Overrides for the generated
	// `ServiceAccount` resource.
	// +optional
	ServiceAccount *KubernetesResourceOverlay `json:"serviceAccount,omitempty"`

	// Creates a `PodDisruptionBudget` for the
	// agentgateway proxy. If absent, no PDB is created. If present, a PDB is
	// created with its selector automatically configured to target the selected
	// generated workload. The `metadata` and `spec` fields from this overlay are
	// applied to the generated PDB.
	// +optional
	PodDisruptionBudget *KubernetesResourceOverlay `json:"podDisruptionBudget,omitempty"`

	// Creates a `HorizontalPodAutoscaler`
	// for Deployment-backed agentgateway proxies. If absent, no HPA is created.
	// If present, an HPA is created with its `scaleTargetRef` automatically
	// configured to target the generated `Deployment`. The `metadata` and `spec`
	// fields from this overlay are applied to the generated HPA.
	// +optional
	HorizontalPodAutoscaler *KubernetesResourceOverlay `json:"horizontalPodAutoscaler,omitempty"`
}

// Container image settings. See https://kubernetes.io/docs/concepts/containers/images
// for details.
type Image struct {
	// Image registry.
	//
	// +optional
	Registry *string `json:"registry,omitempty"`

	// Image repository.
	//
	// +optional
	Repository *string `json:"repository,omitempty"`

	// Image tag.
	//
	// +optional
	Tag *string `json:"tag,omitempty"`

	// Image digest, such as `sha256:12345...`.
	//
	// +optional
	Digest *string `json:"digest,omitempty"`

	// Image pull policy for the container. See
	// https://kubernetes.io/docs/concepts/containers/images/#image-pull-policy
	// for details.
	//
	// +optional
	PullPolicy *corev1.PullPolicy `json:"pullPolicy,omitempty"`
}
