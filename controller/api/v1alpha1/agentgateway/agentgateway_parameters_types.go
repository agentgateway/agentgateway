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
	// created with its selector automatically configured to target the
	// agentgateway proxy `Deployment`. The `metadata` and `spec` fields from
	// this overlay are applied to the generated PDB.
	// +optional
	PodDisruptionBudget *KubernetesResourceOverlay `json:"podDisruptionBudget,omitempty"`

	// Creates a `HorizontalPodAutoscaler`
	// for the agentgateway proxy. If absent, no HPA is created. If present, an
	// HPA is created with its `scaleTargetRef` automatically configured to
	// target the agentgateway proxy `Deployment`. The `metadata` and `spec`
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
