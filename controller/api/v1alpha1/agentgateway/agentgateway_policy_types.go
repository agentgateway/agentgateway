package agentgateway

import (
	"iter"
	"log/slog"
	"math"

	corev1 "k8s.io/api/core/v1"
	apiextensionsv1 "k8s.io/apiextensions-apiserver/pkg/apis/apiextensions/v1"
	"k8s.io/apimachinery/pkg/api/resource"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
)

// +kubebuilder:rbac:groups=agentgateway.dev,resources=agentgatewaypolicies,verbs=get;list;watch
// +kubebuilder:rbac:groups=agentgateway.dev,resources=agentgatewaypolicies/status,verbs=get;update;patch

// +kubebuilder:printcolumn:name="Accepted",type=string,JSONPath=".status.ancestors[*].conditions[?(@.type=='Accepted')].status",description="Agentgateway policy acceptance status"
// +kubebuilder:printcolumn:name="Attached",type=string,JSONPath=".status.ancestors[*].conditions[?(@.type=='Attached')].status",description="Agentgateway policy attachment status"
// +kubebuilder:printcolumn:name="Age",type=date,JSONPath=`.metadata.creationTimestamp`

// +genclient
// +kubebuilder:object:root=true
// +kubebuilder:metadata:labels={app=agentgateway,app.kubernetes.io/name=agentgateway}
// +kubebuilder:resource:categories=agentgateway,shortName=agpol
// +kubebuilder:subresource:status
// +kubebuilder:metadata:labels="gateway.networking.k8s.io/policy=Direct"
type AgentgatewayPolicy struct {
	metav1.TypeMeta `json:",inline"`
	// metadata for the object
	// More info: https://git.k8s.io/community/contributors/devel/sig-architecture/api-conventions.md#metadata
	// +optional
	metav1.ObjectMeta `json:"metadata,omitempty"`

	// Desired policy configuration.
	// +required
	Spec AgentgatewayPolicySpec `json:"spec"`

	// Current policy status.
	// +optional
	Status gwv1.PolicyStatus `json:"status,omitzero"`
	// TODO: embed this into a typed Status field when
	// https://github.com/kubernetes/kubernetes/issues/131533 is resolved
}

// +kubebuilder:object:root=true
type AgentgatewayPolicyList struct {
	metav1.TypeMeta `json:",inline"`
	metav1.ListMeta `json:"metadata,omitempty"`
	Items           []AgentgatewayPolicy `json:"items"`
}

// +kubebuilder:validation:ExactlyOneOf=targetRefs;targetSelectors
// +kubebuilder:validation:XValidation:rule="has(self.traffic) || has(self.frontend) || has(self.backend)",message="At least one of traffic, frontend, or backend must be provided."
// +kubebuilder:validation:XValidation:rule="!has(self.strategy) || !has(self.strategy.inheritance) || (has(self.traffic) && !has(self.frontend) && !has(self.backend))",message="strategy.inheritance may only be set on traffic policies"
// +kubebuilder:validation:XValidation:rule="!has(self.backend) || !has(self.backend.mcp) || ((!has(self.targetRefs) || !self.targetRefs.exists(t, t.kind == 'Service')) && (!has(self.targetSelectors) || !self.targetSelectors.exists(t, t.kind == 'Service')))",message="backend.mcp may not be used with a Service target"
// +kubebuilder:validation:XValidation:rule="!has(self.backend) || !has(self.backend.mcp) || ((!has(self.targetRefs) || !self.targetRefs.exists(t, t.kind == 'AgentgatewayBackend' && has(t.sectionName))) && (!has(self.targetSelectors) || !self.targetSelectors.exists(t, t.kind == 'AgentgatewayBackend' && has(t.sectionName))))",message="backend.mcp may not target an AgentgatewayBackend sectionName"
// +kubebuilder:validation:XValidation:rule="!has(self.backend) || !has(self.backend.ai) || ((!has(self.targetRefs) || !self.targetRefs.exists(t, t.kind == 'Service')) && (!has(self.targetSelectors) || !self.targetSelectors.exists(t, t.kind == 'Service')))",message="backend.ai may not be used with a Service target"
// +kubebuilder:validation:XValidation:rule="!(has(self.traffic) && has(self.traffic.jwtAuthentication) && has(self.backend) && has(self.backend.mcp) && has(self.backend.mcp.authentication))",message="traffic.jwtAuthentication may not be used with backend.mcp.authentication in the same policy"
// +kubebuilder:validation:XValidation:rule="has(self.frontend) && has(self.targetRefs) ? self.targetRefs.all(t, t.kind == 'Gateway' && !has(t.sectionName)) : true",message="the 'frontend' field can only target a Gateway"
// +kubebuilder:validation:XValidation:rule="has(self.frontend) && has(self.targetSelectors) ? self.targetSelectors.all(t, t.kind == 'Gateway' && !has(t.sectionName)) : true",message="the 'frontend' field can only target a Gateway"
// +kubebuilder:validation:XValidation:rule="has(self.traffic) && has(self.targetRefs) ? self.targetRefs.all(t, t.kind in ['Gateway', 'HTTPRoute', 'GRPCRoute', 'ListenerSet', 'InferencePool']) : true",message="the 'traffic' field can only target a Gateway, ListenerSet, GRPCRoute, HTTPRoute, or InferencePool"
// +kubebuilder:validation:XValidation:rule="has(self.traffic) && has(self.targetSelectors) ? self.targetSelectors.all(t, t.kind in ['Gateway', 'HTTPRoute', 'GRPCRoute', 'ListenerSet', 'InferencePool']) : true",message="the 'traffic' field can only target a Gateway, ListenerSet, GRPCRoute, HTTPRoute, or InferencePool"
// +kubebuilder:validation:XValidation:rule="has(self.targetRefs) && has(self.traffic) && has(self.traffic.phase) && self.traffic.phase == 'PreRouting' ? self.targetRefs.all(t, t.kind in ['Gateway', 'ListenerSet']) : true",message="the 'traffic.phase=PreRouting' field can only target a Gateway or ListenerSet"
// +kubebuilder:validation:XValidation:rule="has(self.targetSelectors) && has(self.traffic) && has(self.traffic.phase) && self.traffic.phase == 'PreRouting' ? self.targetSelectors.all(t, t.kind in ['Gateway', 'ListenerSet']) : true",message="the 'traffic.phase=PreRouting' field can only target a Gateway or ListenerSet"
type AgentgatewayPolicySpec struct {
	// Target resources to attach the
	// policy to.
	//
	// +listType=atomic
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +kubebuilder:validation:XValidation:rule="self.all(r, (r.kind == 'Service' && r.group == '') || (r.kind == 'AgentgatewayBackend' && r.group == 'agentgateway.dev') || (r.kind in ['Gateway', 'HTTPRoute', 'GRPCRoute'] && r.group == 'gateway.networking.k8s.io') || (r.kind == 'ListenerSet' && r.group == 'gateway.networking.k8s.io') || (r.kind == 'InferencePool' && r.group == 'inference.networking.k8s.io'))",message="targetRefs may only reference Gateway, HTTPRoute, GRPCRoute, ListenerSet, Service, AgentgatewayBackend, or InferencePool resources"
	// +kubebuilder:validation:XValidation:message="Only one Kind of targetRef can be set on one policy",rule="self.all(l1, !self.exists(l2, l1.kind != l2.kind))"
	// +optional
	TargetRefs []LocalPolicyTargetReferenceWithSectionName `json:"targetRefs,omitempty"`

	// Target selectors used to select resources to attach the policy to.
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +kubebuilder:validation:XValidation:rule="self.all(r, (r.kind == 'Service' && r.group == '') || (r.kind == 'AgentgatewayBackend' && r.group == 'agentgateway.dev') || (r.kind in ['Gateway', 'HTTPRoute', 'GRPCRoute'] && r.group == 'gateway.networking.k8s.io') || (r.kind == 'ListenerSet' && r.group == 'gateway.networking.k8s.io') || (r.kind == 'InferencePool' && r.group == 'inference.networking.k8s.io'))",message="targetRefs may only reference Gateway, HTTPRoute, GRPCRoute, ListenerSet, Service, AgentgatewayBackend, or InferencePool resources"
	// +kubebuilder:validation:XValidation:message="Only one Kind of targetRef can be set on one policy",rule="self.all(l1, !self.exists(l2, l1.kind != l2.kind))"
	// +optional
	TargetSelectors []LocalPolicyTargetSelectorWithSectionName `json:"targetSelectors,omitempty"`

	// Policy merge and conflict resolution strategy.
	//
	// Strategy settings apply to the policy object as a whole. Individual strategy fields may
	// only be valid for specific policy kinds; for example, inheritance is only valid when this
	// policy contains traffic settings.
	// +optional
	Strategy *PolicyStrategy `json:"strategy,omitempty"`

	// Settings for how to handle incoming traffic.
	//
	// A frontend policy can only target a `Gateway`. `Listener` and
	// `ListenerSet` are not valid targets.
	//
	// When multiple policies are selected for a given request, they are merged on a field-level basis, but not a deep
	// merge. For example, policy A sets `tcp` and `tls`, and policy B sets
	// `tls`; the effective policy would be `tcp` from policy A, and `tls` from
	// policy B.
	// +optional
	Frontend *Frontend `json:"frontend,omitempty"`

	// Settings for how to process traffic.
	//
	// A traffic policy can target a `Gateway` (optionally, with a
	// `sectionName` indicating the listener), `ListenerSet`, or `Route`
	// (optionally, with a `sectionName` indicating the route rule).
	//
	// When multiple policies are selected for a given request, they are merged on a field-level basis, but not a deep
	// merge. Precedence is given to more precise policies: `Gateway` <
	// `Listener` < `Route` < `Route Rule`. For example, policy A sets
	// `timeouts` and `retries`, and policy B sets `retries`; the effective
	// policy would be `timeouts` from policy A, and `retries` from policy B.
	// +optional
	Traffic *Traffic `json:"traffic,omitempty"`

	// Settings for how to connect to destination backends.
	//
	// A backend policy can target a `Gateway` (optionally, with a
	// `sectionName` indicating the listener), `ListenerSet`, `Route`
	// (optionally, with a `sectionName` indicating the route rule), or a
	// `Service` or `Backend` (optionally, with a `sectionName` indicating the
	// port for `Service`, or sub-backend for `Backend`).
	//
	// Note that a backend policy applies when connecting to a specific destination backend. Targeting a higher level
	// resource, like `Gateway`, is just a way to easily apply a policy to a
	// group of backends.
	//
	// When multiple policies are selected for a given request, they are merged on a field-level basis, but not a deep
	// merge. Precedence is given to more precise policies: `Gateway` <
	// `Listener` < `Route` < `Route Rule` < `Backend` or `Service`. For
	// example, if a `Gateway` policy sets `tcp` and `tls`, and a `Backend`
	// policy sets `tls`, the effective policy would be `tcp` from the
	// `Gateway`, and `tls` from the `Backend`.
	// +optional
	Backend *BackendFull `json:"backend,omitempty"`
}

type PolicyStrategy struct {
	// Controls whether less-specific traffic policies prevent more-specific traffic policies
	// from contributing to the effective policy.
	//
	// This field is only valid on traffic policies. Frontend and backend policy merging does not use
	// inheritance.
	//
	// When unset or set to `Default`, traffic policy fields are merged by specificity, with more-specific
	// attachment points such as routes and route rules able to override fields from less-specific
	// attachment points such as gateways and listeners.
	// In other words, this policy provides `Default`s that can be overridden. For example, you may provide a `Default`
	// timeout policy for the entire Gateway that is overridden by specific routes.
	//
	// When set to `Override`, this policy blocks traffic policies at more-specific attachment points from
	// being included in the effective policy. This is useful when a gateway-level policy must remain
	// authoritative for all routes below it.
	//
	// +optional
	Inheritance *PolicyInheritance `json:"inheritance,omitempty"`
}

// How a traffic policy affects policy inheritance across attachment
// specificity levels.
// +k8s:enum
type PolicyInheritance string

const (
	// PolicyInheritanceDefault allows the normal traffic policy merge order, where more-specific
	// policies may override fields from less-specific policies.
	PolicyInheritanceDefault PolicyInheritance = "Default"
	// PolicyInheritanceOverride makes the policy authoritative for lower levels, excluding
	// more-specific traffic policies from the effective policy.
	PolicyInheritanceOverride PolicyInheritance = "Override"
)

type BackendSimple struct {
	// Settings for managing TCP connections to the backend.
	// +optional
	TCP *BackendTCP `json:"tcp,omitempty"`
	// Settings for managing TLS connections to the backend.
	//
	// If this field is set, TLS will be initiated to the backend; the system trusted CA certificates will be used to
	// validate the server, and the SNI will automatically be set based on the destination.
	// +optional
	TLS *BackendTLS `json:"tls,omitempty"`
	// Settings for managing HTTP requests to the backend.
	// +optional
	HTTP *BackendHTTP `json:"http,omitempty"`

	// Settings for managing tunnel connections, with behavior like `HTTPS_PROXY`, to the backend.
	// +optional
	Tunnel *BackendTunnel `json:"tunnel,omitempty"`

	// Settings for managing authentication to the backend.
	// +optional
	Auth *BackendAuth `json:"auth,omitempty"`
}

type Health struct {
	// CEL expression that determines whether a response indicates an unhealthy backend.
	// When the expression evaluates to true, the backend is considered unhealthy and may be evicted.
	//
	// For example, to evict on 5xx responses: `response.code >= 500`.
	//
	// When unset, any 5xx response, or a connection failure, is treated as unhealthy.
	// This default lowers the backend's health score but does not trigger eviction on its own.
	//
	// +optional
	UnhealthyCondition *CELExpression `json:"unhealthyCondition,omitempty"`

	// Settings for evicting unhealthy backends.
	// +optional
	Eviction *BackendEviction `json:"eviction,omitempty"`
}

// Settings for evicting unhealthy backends.
type BackendEviction struct {
	// Base time a backend should be evicted after being marked unhealthy.
	// Subsequent evictions use multiplicative backoff (duration * times_evicted).
	// If all endpoints are evicted, the load balancer falls back to returning evicted endpoints
	// rather than failing entirely.
	// If unset, defaults to `3s`.
	// +kubebuilder:validation:XValidation:rule="matches(self, '^([0-9]{1,5}(h|m|s|ms)){1,4}$')",message="invalid duration value"
	// +kubebuilder:validation:XValidation:rule="duration(self) >= duration('1s')",message="evictionDuration must be at least 1 second"
	// +kubebuilder:default="3s"
	// +optional
	Duration *metav1.Duration `json:"duration,omitempty"`

	// Health score from 0 to 100 assigned to a backend when it returns from eviction.
	// For gradual recovery, set below 100; for full recovery immediately, set 100.
	// If unset, the backend resumes with the health it had when evicted.
	//
	// +kubebuilder:validation:Minimum=0
	// +kubebuilder:validation:Maximum=100
	// +optional
	RestoreHealth *int32 `json:"restoreHealth,omitempty"`

	// Number of consecutive unhealthy responses required before the backend is evicted.
	// For example, a value of 5 means the backend must receive 5 unhealthy responses in a row before being evicted.
	// When both consecutiveFailures and healthThreshold are set, the backend is evicted when either condition is met.
	// When neither is set, a single unhealthy response can trigger eviction.
	//
	// +kubebuilder:validation:Minimum=0
	// +optional
	ConsecutiveFailures *int32 `json:"consecutiveFailures,omitempty"`

	// EWMA health score threshold, expressed as 0 to 100.
	// When set, a backend is only evicted if its computed health drops below this value after an unhealthy response.
	// For example, 50 means the backend is evicted when its EWMA health falls below 50% following failures.
	// Unlike consecutiveFailures (which counts consecutive failures), this uses a sliding-window average
	// so a single success in a stream of failures can delay eviction.
	// When both consecutiveFailures and healthThreshold are set, the backend is evicted when either condition is met.
	// When neither is set, a single unhealthy response triggers eviction.
	//
	// +kubebuilder:validation:Minimum=0
	// +kubebuilder:validation:Maximum=100
	// +optional
	HealthThreshold *int32 `json:"healthThreshold,omitempty"`
}

// +kubebuilder:validation:AtLeastOneFieldSet
type BackendWithAI struct {
	BackendSimple `json:",inline"`

	// Settings for AI workloads. This is only applicable when
	// connecting to a `Backend` of type `ai`.
	// +optional
	AI *BackendAI `json:"ai,omitempty"`

	// Mutates and transforms requests and responses sent to and from the backend.
	// +optional
	Transformation *Transformation `json:"transformation,omitempty"`

	// Settings for passive and active health checking.
	// +optional
	Health *Health `json:"health,omitempty"`
}

// +kubebuilder:validation:AtLeastOneFieldSet
type BackendFull struct {
	BackendSimple `json:",inline"`

	// Settings for AI workloads. This is only applicable when
	// connecting to a `Backend` of type `ai`.
	// +optional
	AI *BackendAI `json:"ai,omitempty"`

	// Settings for MCP workloads. This is only applicable when
	// connecting to a `Backend` of type `mcp`.
	//
	// +optional
	MCP *BackendMCP `json:"mcp,omitempty"`

	// Mutates and transforms requests and responses sent to and from the backend.
	// +optional
	Transformation *Transformation `json:"transformation,omitempty"`

	// Settings for passive and active health checking.
	// +optional
	Health *Health `json:"health,omitempty"`

	// External authentication configuration for requests
	// sent to this backend.
	// +optional
	ExtAuth *ExtAuth `json:"extAuth,omitempty"`
}

// +kubebuilder:validation:MinLength=1
// +kubebuilder:validation:MaxLength=64
type TinyString = string

// +kubebuilder:validation:MinLength=1
// +kubebuilder:validation:MaxLength=256
type ShortString = string

// +kubebuilder:validation:MinLength=1
// +kubebuilder:validation:MaxLength=1024
type LongString = string

// +kubebuilder:validation:MinLength=1
// +kubebuilder:validation:MaxLength=253
// +kubebuilder:validation:Pattern=`^[a-z0-9]([-a-z0-9]*[a-z0-9])?(\.[a-z0-9]([-a-z0-9]*[a-z0-9])?)*$`
type SNI = string

// Byte quantity that must fit in the data plane size limit.
// +kubebuilder:validation:XIntOrString
// +kubebuilder:validation:MaxLength=32
// +kubebuilder:validation:MinLength=1
// +kubebuilder:validation:Pattern=`^[+-]?([0-9]+(\.[0-9]*)?|\.[0-9]+)(([KMGTPE]i)|[numkMGTPE]|[eE](\+?0*([0-9]|1[0-8])|-0*[0-9]))?$`
// +kubebuilder:validation:XValidation:rule="(self >= 1 && self <= 4294967295) || self.size() > 0",message="value must be at least 1 byte and fit within uint32"
type ByteSize struct {
	Value *resource.Quantity `json:"-"`
}

// ClampedValue returns the quantity as a uint32, clamping values to 0..MaxUint32.
func (b *ByteSize) ClampedValue() *uint32 {
	if b == nil || b.Value == nil {
		return nil
	}
	v := b.Value.Value()
	if v < 0 {
		return new(uint32(0))
	}
	if v > math.MaxUint32 {
		return new(uint32(math.MaxUint32))
	}
	return new(uint32(v))
}

func (b ByteSize) MarshalJSON() ([]byte, error) {
	if b.Value == nil {
		return []byte("null"), nil
	}
	return b.Value.MarshalJSON()
}

func (b *ByteSize) UnmarshalJSON(data []byte) error {
	var q resource.Quantity
	if err := q.UnmarshalJSON(data); err != nil {
		// Invalid byte sizes must not block informer decoding. CEL cannot validate
		// this safely until the quantity cost bug is fixed, so treat it as unset.
		slog.Warn("failed to unmarshal quantity, ignoring", "value", string(data), "error", err)
		b.Value = nil
		return nil
	}
	b.Value = &q
	return nil
}

func (b ByteSize) DeepCopy() ByteSize {
	if b.Value == nil {
		return ByteSize{}
	}
	q := b.Value.DeepCopy()
	return ByteSize{Value: &q}
}

// +k8s:enum
type InsecureTLSMode string

const (
	// InsecureTLSModeInsecure disables all TLS verification
	InsecureTLSModeAll InsecureTLSMode = "All"
	// InsecureTLSModeHostname enables verifying the CA certificate, but disables verification of the hostname/SAN.
	// Note this is still, generally, very "insecure" as the name suggests.
	InsecureTLSModeHostname InsecureTLSMode = "Hostname"
)

// +kubebuilder:validation:AtMostOneOf=verifySubjectAltNames;insecureSkipVerify
// +kubebuilder:validation:XValidation:rule="has(self.insecureSkipVerify) && self.insecureSkipVerify == 'All' ? !has(self.caCertificateRefs) : true",message="insecureSkipVerify All and caCertificateRefs may not be set together"
// +kubebuilder:validation:XValidation:rule="has(self.insecureSkipVerify) ? !has(self.verifySubjectAltNames) : true",message="insecureSkipVerify and verifySubjectAltNames may not be set together"
type BackendTLS struct {
	// Enables mutual TLS to the backend, using the
	// specified key (`tls.key`) and cert (`tls.crt`) from the referenced
	// credential source, defaulting to a Kubernetes `Secret`.
	//
	// An optional `ca.cert` field, if present, will be used to verify the
	// server certificate. If `caCertificateRefs` is also specified, the
	// `caCertificateRefs` field takes priority.
	//
	// If unspecified, no client certificate will be used.
	//
	// +listType=atomic
	// +kubebuilder:validation:MaxItems=1
	// +optional
	MtlsCertificateRef []LocalSecretObjectRef `json:"mtlsCertificateRef,omitempty"`
	// CA certificate `ConfigMap` to use to
	// verify the server certificate.
	// If unset, the system's trusted certificates are used.
	//
	// +listType=atomic
	// +kubebuilder:validation:MaxItems=1
	// +optional
	CACertificateRefs []corev1.LocalObjectReference `json:"caCertificateRefs,omitempty"`

	// Originates TLS but skips verification of the backend's certificate.
	// WARNING: This is an insecure option that should only be used if the risks are understood.
	//
	// There are two modes:
	// * `All` disables all TLS verification.
	// * `Hostname` verifies the CA certificate is trusted, but ignores any
	//   mismatch of hostname or SANs. Note that this method is still insecure;
	//   prefer setting `verifySubjectAltNames` to customize the valid hostnames
	//   if possible.
	//
	// +optional
	InsecureSkipVerify *InsecureTLSMode `json:"insecureSkipVerify,omitempty"`

	// Server Name Indicator (`SNI`) to use in the TLS
	// handshake. If unset, the `SNI` is automatically set based on the
	// destination hostname.
	// +optional
	Sni *SNI `json:"sni,omitempty"`

	// Subject Alternative Names (`SAN`)
	// to verify in the server certificate.
	// If not present, the destination hostname is automatically used.
	//
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +optional
	VerifySubjectAltNames []ShortString `json:"verifySubjectAltNames,omitempty"`

	// Application-Layer Protocol Negotiation (`ALPN`)
	// value to use in the TLS handshake.
	//
	// If not present, defaults to `["h2", "http/1.1"]`.
	//
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +optional
	AlpnProtocols *[]TinyString `json:"alpnProtocols,omitempty"`

	// Ordered list of key exchange groups for a TLS connection.
	// For example: `X25519_MLKEM768,X25519`.
	// +optional
	KeyExchangeGroups []KeyExchangeGroup `json:"keyExchangeGroups,omitempty"`
}

// +kubebuilder:validation:AtLeastOneFieldSet
type Frontend struct {
	// Settings for managing incoming TCP connections.
	// +optional
	TCP *FrontendTCP `json:"tcp,omitempty"`
	// CEL authorization on downstream network connections.
	//
	// This runs before protocol handling and is intended for L4 access control,
	// for example using `source.address` with `cidr(...).containsIP(...)`.
	// +optional
	NetworkAuthorization *Authorization `json:"networkAuthorization,omitempty"`
	// Settings for managing incoming TLS connections.
	// +optional
	TLS *FrontendTLS `json:"tls,omitempty"`
	// Settings for managing incoming HTTP requests.
	// +optional
	HTTP *FrontendHTTP `json:"http,omitempty"`

	// Settings for downstream PROXY protocol handling.
	//
	// If configured, incoming connections may require a PROXY header before
	// normal protocol handling. This can also be configured to allow both
	// PROXY and non-PROXY traffic on the same listener.
	// +optional
	ProxyProtocol *FrontendProxyProtocol `json:"proxyProtocol,omitempty"`

	// Settings for downstream HTTP CONNECT handling.
	//
	// If unset, CONNECT requests are rejected with Method Not Allowed.
	// +optional
	Connect *FrontendConnect `json:"connect,omitempty"`

	// Access logging configuration.
	// +optional
	AccessLog *AccessLog `json:"accessLog,omitempty"`

	// OpenTelemetry tracing settings.
	// +optional
	Tracing *Tracing `json:"tracing,omitempty"`

	// Custom Prometheus metric label configuration.
	// CEL expressions are evaluated per-request and added as labels to all
	// Prometheus metrics exposed by agentgateway.
	// +optional
	Metrics *MetricLabels `json:"metrics,omitempty"`
}

// +k8s:enum
type ProxyProtocolVersion string

const (
	ProxyProtocolVersionV1  ProxyProtocolVersion = "V1"
	ProxyProtocolVersionV2  ProxyProtocolVersion = "V2"
	ProxyProtocolVersionAll ProxyProtocolVersion = "All"
)

// +k8s:enum
type ProxyProtocolMode string

const (
	// A valid PROXY header must be present. This is the default option.
	ProxyProtocolModeStrict ProxyProtocolMode = "Strict"
	// Accept either a PROXY header or plain downstream traffic.
	ProxyProtocolModeOptional ProxyProtocolMode = "Optional"
)

// +k8s:enum
type HTTPHeaderCase string

const (
	HTTPHeaderCaseLowercase HTTPHeaderCase = "Lowercase"
	HTTPHeaderCasePreserve  HTTPHeaderCase = "Preserve"
)

type FrontendProxyProtocol struct {
	// PROXY protocol version to accept.
	//
	// If unset, this defaults to `V2`.
	// +kubebuilder:default=V2
	// +optional
	Version ProxyProtocolVersion `json:"version,omitempty"`

	// Whether PROXY headers are required or optional.
	//
	// If unset, this defaults to `Strict`.
	// +kubebuilder:default=Strict
	// +optional
	Mode ProxyProtocolMode `json:"mode,omitempty"`
}

// +kubebuilder:validation:Enum=Deny;Route;Tunnel
type FrontendConnectMode string

const (
	// Deny rejects downstream CONNECT requests.
	FrontendConnectModeDeny FrontendConnectMode = "Deny"
	// Route treats CONNECT as an HTTP request and routes it through the HTTP
	// matching chain before establishing a raw tunnel to the selected backend.
	FrontendConnectModeRoute FrontendConnectMode = "Route"
	// Tunnel terminates CONNECT and sends the upgraded stream through the
	// addressed gateway bind as a new downstream connection.
	FrontendConnectModeTunnel FrontendConnectMode = "Tunnel"
)

type FrontendConnect struct {
	// Whether downstream CONNECT requests are accepted.
	// +required
	Mode FrontendConnectMode `json:"mode"`
}

// +kubebuilder:validation:AtLeastOneFieldSet
type FrontendHTTP struct {
	// Maximum HTTP body size that will be buffered
	// into memory.
	// Bodies will only be buffered for policies which require buffering.
	// If unset, this defaults to `2mb`.
	// +optional
	MaxBufferSize *ByteSize `json:"maxBufferSize,omitempty"`

	// Maximum number of headers allowed
	// in `HTTP/1.1` requests.
	// If unset, this defaults to 100.
	// +kubebuilder:validation:Minimum=1
	// +kubebuilder:validation:Maximum=4096
	// +optional
	HTTP1MaxHeaders *int32 `json:"http1MaxHeaders,omitempty"`
	// Timeout before an unused connection is
	// closed.
	// If unset, this defaults to 10 minutes.
	// +kubebuilder:validation:XValidation:rule="matches(self, '^([0-9]{1,5}(h|m|s|ms)){1,4}$')",message="invalid duration value"
	// +kubebuilder:validation:XValidation:rule="duration(self) >= duration('1s')",message="http1IdleTimeout must be at least 1 second"
	// +optional
	HTTP1IdleTimeout *metav1.Duration `json:"http1IdleTimeout,omitempty"`
	// Controls HTTP/1 request header name casing when encoding responses on the same connection.
	// This only applies to `HTTP/1`. If a request is HTTP/2 in either the incoming or outgoing request, this will be ignored.
	// HTTP/2 requests are always lower case.
	//
	// Modifying the headers from other policies may result in the original case being lost.
	//
	// +optional
	HTTP1HeaderCase *HTTPHeaderCase `json:"http1HeaderCase,omitempty"`

	// Initial window size for stream-level flow
	// control for received data.
	// +optional
	HTTP2WindowSize *ByteSize `json:"http2WindowSize,omitempty"`
	// Initial window size for
	// connection-level flow control for received data.
	// +optional
	HTTP2ConnectionWindowSize *ByteSize `json:"http2ConnectionWindowSize,omitempty"`
	// Maximum frame size to use.
	// If unset, this defaults to `16kb`.
	// +kubebuilder:validation:XValidation:rule="!quantity(string(self)).isLessThan(quantity('16384'))",message="http2FrameSize must be at least 16384 bytes"
	// +kubebuilder:validation:XValidation:rule="!quantity(string(self)).isGreaterThan(quantity('1677215'))",message="http2FrameSize must be at most 1677215 bytes"
	// +optional
	HTTP2FrameSize *ByteSize `json:"http2FrameSize,omitempty"`
	// Maximum aggregate size of decoded HTTP/2
	// request headers.
	// If unset, this defaults to `16Ki`.
	// +optional
	HTTP2MaxHeaderSize *ByteSize `json:"http2MaxHeaderSize,omitempty"`
	// +kubebuilder:validation:XValidation:rule="matches(self, '^([0-9]{1,5}(h|m|s|ms)){1,4}$')",message="invalid duration value"
	// +kubebuilder:validation:XValidation:rule="duration(self) >= duration('1s')",message="http2KeepaliveInterval must be at least 1 second"
	// +optional
	HTTP2KeepaliveInterval *metav1.Duration `json:"http2KeepaliveInterval,omitempty"`
	// +kubebuilder:validation:XValidation:rule="matches(self, '^([0-9]{1,5}(h|m|s|ms)){1,4}$')",message="invalid duration value"
	// +kubebuilder:validation:XValidation:rule="duration(self) >= duration('1s')",message="http2KeepaliveTimeout must be at least 1 second"
	// +optional
	HTTP2KeepaliveTimeout *metav1.Duration `json:"http2KeepaliveTimeout,omitempty"`
	// Maximum time a connection is allowed to remain open.
	// After this duration, the connection is gracefully closed after the current in-flight request completes.
	// Useful for ensuring even traffic distribution behind load balancers during scaling events.
	// +kubebuilder:validation:XValidation:rule="matches(self, '^([0-9]{1,5}(h|m|s|ms)){1,4}$')",message="invalid duration value"
	// +kubebuilder:validation:XValidation:rule="duration(self) >= duration('1s')",message="maxConnectionDuration must be at least 1 second"
	// +optional
	MaxConnectionDuration *metav1.Duration `json:"maxConnectionDuration,omitempty"`
}

// +kubebuilder:validation:AtLeastOneFieldSet
type FrontendTLS struct {
	// Deadline for a TLS handshake to
	// complete. If unset, this defaults to `15s`.
	// +kubebuilder:validation:XValidation:rule="matches(self, '^([0-9]{1,5}(h|m|s|ms)){1,4}$')",message="invalid duration value"
	// +kubebuilder:validation:XValidation:rule="duration(self) >= duration('100ms')",message="handshakeTimeout must be at least 100ms"
	// +optional
	HandshakeTimeout *metav1.Duration `json:"handshakeTimeout,omitempty"`

	// Application-Layer Protocol Negotiation (`ALPN`)
	// value to use in the TLS handshake.
	//
	// If not present, defaults to `["h2", "http/1.1"]`.
	//
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +optional
	AlpnProtocols *[]TinyString `json:"alpnProtocols,omitempty"`

	// Minimum TLS version to support.
	// +optional
	MinTLSVersion *TLSVersion `json:"minProtocolVersion,omitempty"`

	// Maximum TLS version to support.
	// +optional
	MaxTLSVersion *TLSVersion `json:"maxProtocolVersion,omitempty"`

	// Cipher suites for a TLS listener.
	// The value is a comma-separated list of cipher suites, for example
	// `TLS13_AES_256_GCM_SHA384,TLS13_AES_128_GCM_SHA256`.
	// Use this in the TLS options field of a TLS listener.
	// +optional
	CipherSuites []CipherSuite `json:"cipherSuites,omitempty"`

	// Ordered list of key exchange groups for a TLS listener.
	// For example: `X25519_MLKEM768,X25519`.
	// +optional
	KeyExchangeGroups []KeyExchangeGroup `json:"keyExchangeGroups,omitempty"`

	// TODO: mirror the tuneables on BackendTLS
}

// +k8s:enum
type TLSVersion string

const (
	// agentgateway currently only supports `TLS 1.2` and `TLS 1.3`.
	TLSVersion1_2 TLSVersion = "1.2"
	TLSVersion1_3 TLSVersion = "1.3"
)

// +k8s:enum
type CipherSuite string

const (
	// TLS 1.3 cipher suites
	CipherSuiteTLS13_AES_256_GCM_SHA384       CipherSuite = "TLS13_AES_256_GCM_SHA384"
	CipherSuiteTLS13_AES_128_GCM_SHA256       CipherSuite = "TLS13_AES_128_GCM_SHA256"
	CipherSuiteTLS13_CHACHA20_POLY1305_SHA256 CipherSuite = "TLS13_CHACHA20_POLY1305_SHA256"

	// TLS 1.2 cipher suites
	CipherSuiteTLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384       CipherSuite = "TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384"
	CipherSuiteTLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256       CipherSuite = "TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256"
	CipherSuiteTLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256 CipherSuite = "TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256"

	CipherSuiteTLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384       CipherSuite = "TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384"
	CipherSuiteTLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256       CipherSuite = "TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256"
	CipherSuiteTLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256 CipherSuite = "TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256"
)

// +k8s:enum
type KeyExchangeGroup string

const (
	KeyExchangeGroupX25519         KeyExchangeGroup = "X25519"
	KeyExchangeGroupP256           KeyExchangeGroup = "P-256"
	KeyExchangeGroupP384           KeyExchangeGroup = "P-384"
	KeyExchangeGroupX25519MLKEM768 KeyExchangeGroup = "X25519_MLKEM768"
)

// +kubebuilder:validation:AtLeastOneFieldSet
type FrontendTCP struct {
	// Settings for enabling TCP keepalives on the connection.
	// +optional
	KeepAlive *Keepalive `json:"keepalive,omitempty"`
}

// TCP keepalive settings.
type Keepalive struct {
	// Maximum number of keepalive probes to send before dropping the connection.
	// If unset, this defaults to 9.
	// +kubebuilder:validation:Minimum=1
	// +kubebuilder:validation:Maximum=64
	// +optional
	Retries *int32 `json:"retries,omitempty"`

	// Time a connection needs to be idle before keepalive probes start being sent.
	// If unset, this defaults to 180s.
	// +kubebuilder:validation:XValidation:rule="matches(self, '^([0-9]{1,5}(h|m|s|ms)){1,4}$')",message="invalid duration value"
	// +kubebuilder:validation:XValidation:rule="duration(self) >= duration('1s')",message="time must be at least 1 second"
	// +optional
	Time *metav1.Duration `json:"time,omitempty"`

	// Time between keepalive probes.
	// If unset, this defaults to 180s.
	// +kubebuilder:validation:XValidation:rule="matches(self, '^([0-9]{1,5}(h|m|s|ms)){1,4}$')",message="invalid duration value"
	// +kubebuilder:validation:XValidation:rule="duration(self) >= duration('1s')",message="interval must be at least 1 second"
	// +optional
	Interval *metav1.Duration `json:"interval,omitempty"`
}

// +k8s:enum
type PolicyPhase string

const (
	PolicyPhasePreRouting  PolicyPhase = "PreRouting"
	PolicyPhasePostRouting PolicyPhase = "PostRouting"
)

// +kubebuilder:validation:IfThenOnlyFields:if="has(self.phase) && self.phase == 'PreRouting'",fields=phase;transformation;extProc;extAuth;jwtAuthentication;basicAuthentication;apiKeyAuthentication;cors,message="phase PreRouting only supports extAuth, transformation, extProc, jwtAuthentication, basicAuthentication, apiKeyAuthentication and cors"
type Traffic struct {
	// The phase to apply the traffic policy to. If the phase is `PreRouting`,
	// the `targetRef` must be a `Gateway` or a `Listener`. `PreRouting` is
	// typically used only when a policy needs to influence the routing
	// decision.
	//
	// Even when using `PostRouting` mode, the policy can target the
	// `Gateway` or `Listener`. This is a helper for applying the policy to all
	// routes under that `Gateway` or `Listener`, and follows the merging logic
	// described above.
	//
	// Note: `PreRouting` and `PostRouting` rules do not merge together. These
	// are independent execution phases. That is, all `PreRouting` rules will
	// merge and execute, then all `PostRouting` rules will merge and execute.
	//
	// If unset, this defaults to `PostRouting`.
	// +optional
	Phase *PolicyPhase `json:"phase,omitempty"` //nolint:kubeapilinter // false positive for the nophase sub-linter

	// Mutates and transforms requests and responses
	// before forwarding them to the destination.
	// +optional
	Transformation *TransformationOrConditional `json:"transformation,omitempty"`

	// External processing configuration for the policy.
	// +optional
	ExtProc *ExtProcOrConditional `json:"extProc,omitempty"`

	// External authentication configuration for the policy.
	// This selects the external server to send requests to for authentication.
	//
	// An extAuth policy can be conditionally set by nesting configuration under the `conditional` field.
	// +optional
	ExtAuth *ExtAuthOrConditional `json:"extAuth,omitempty"`

	// Rate limiting configuration for the policy.
	// This limits the rate at which requests are processed.
	// +optional
	RateLimit *RateLimitsOrConditional `json:"rateLimit,omitempty"`

	// CORS configuration for the policy.
	// +optional
	Cors *CORS `json:"cors,omitempty"`

	// Cross-Site Request Forgery (CSRF) policy for this traffic policy.
	//
	// The CSRF policy has the following behavior:
	// * Safe methods (`GET`, `HEAD`, `OPTIONS`) are automatically allowed.
	// * Requests without `Sec-Fetch-Site` or `Origin` headers are assumed to
	//   be same-origin or non-browser requests and are allowed.
	// * Otherwise, the `Sec-Fetch-Site` header is checked, with a fallback to
	//   comparing the `Origin` header to the `Host` header.
	// +optional
	Csrf *CSRF `json:"csrf,omitempty"`

	// Request and response header modification policy.
	// +optional
	HeaderModifiers *HeaderModifiers `json:"headerModifiers,omitempty"`

	// How to rewrite the `Host` header for requests.
	//
	// If the `HTTPRoute` `urlRewrite` filter already specifies a host rewrite,
	// this setting is ignored.
	// +optional
	HostnameRewrite *HostnameRewrite `json:"hostRewrite,omitempty"`

	// Request timeouts.
	// It is applicable to `HTTPRoute` resources and ignored for other targeted
	// kinds.
	// +optional
	Timeouts *Timeouts `json:"timeouts,omitempty"`

	// Retry policy.
	// +optional
	Retry *Retry `json:"retry,omitempty"`

	// Access rules based on roles and
	// permissions.
	// If multiple authorization rules are applied across different policies, at the same or different attachment points,
	// all rules are merged.
	// +optional
	Authorization *Authorization `json:"authorization,omitempty"`

	// Authenticates users based on JWT tokens.
	// +optional
	JWTAuthentication *JWTAuthentication `json:"jwtAuthentication,omitempty"`

	// Authenticates users based on the `Basic`
	// authentication scheme (RFC 7617), where a username and password are
	// encoded in the request.
	// +optional
	BasicAuthentication *BasicAuthentication `json:"basicAuthentication,omitempty"`

	// Authenticates users based on a configured API
	// key.
	// +optional
	APIKeyAuthentication *APIKeyAuthentication `json:"apiKeyAuthentication,omitempty"`

	// Sends a direct response to the
	// client.
	// +optional
	DirectResponse *DirectResponseOrConditional `json:"directResponse,omitempty"`

	// Buffers request and response bodies. Buffered bodies are accumulated in memory
	// by the proxy until completion before being forwarded. This changes the proxies default behavior, which streams bodies.
	//
	// Warning: large bodies can lead to excessive memory usage in the proxy. Utilize with care, or with strict limits.
	//
	// +optional
	Buffer *Buffer `json:"buffer,omitempty"`
}

// Direct response policy.
//
// +kubebuilder:validation:XValidation:rule="!(has(self.body) && has(self.bodyExpression))",message="body and bodyExpression may not both be set"
type DirectResponse struct {
	// HTTP status code to return.
	//
	// +optional
	// +kubebuilder:validation:Minimum=200
	// +kubebuilder:validation:Maximum=599
	StatusCode *int32 `json:"status,omitempty"`
	// Content to return in the HTTP response body.
	// The maximum length of the body is restricted to prevent excessively large responses.
	// If this field is omitted, no body is included in the response.
	//
	// +optional
	// +kubebuilder:validation:MinLength=1
	// +kubebuilder:validation:MaxLength=4096
	Body *string `json:"body,omitempty"`

	// CEL expression that produces the HTTP response body.
	// Strings and bytes are written directly; other values are serialized as JSON.
	// If this field is omitted, no expression body is included in the response.
	//
	// +optional
	BodyExpression *CELExpression `json:"bodyExpression,omitempty"`

	// Response headers to set on the direct response.
	//
	// +listType=map
	// +listMapKey=name
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +optional
	Headers []DirectResponseHeader `json:"headers,omitempty"`
}

type DirectResponseConditional struct {
	// CEL expression that must evaluate to true for this policy to execute.
	// +optional
	Condition CELExpression `json:"condition,omitempty"`
	// Policy to apply when the condition matches.
	// +required
	// +kubebuilder:validation:XValidation:rule="has(self.status)",message="status is required"
	Policy DirectResponse `json:"policy"`
}

// +kubebuilder:validation:ConditionalPolicy:fields=status
type DirectResponseOrConditional struct {
	// +optional
	DirectResponse `json:",inline"`
	// Conditional policy execution. Set this or the top-level directResponse fields.
	// The first matching policy will be executed.
	// A single policy may be provided without a condition set; if so, it must be the last policy and will be the fallback
	// in case no conditions are met.
	// +optional
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +kubebuilder:validation:XValidation:message="conditional entries without condition must be last",rule="self.filter(e, !has(e.condition)).size() <= 1 && (!self.exists(e, !has(e.condition)) || !has(self[size(self) - 1].condition))"
	Conditional []DirectResponseConditional `json:"conditional,omitempty"`
}

func (d *DirectResponseOrConditional) ConditionalPolicy() (*DirectResponse, iter.Seq[ConditionalPolicyEntry[DirectResponse]]) {
	seq := mapseq(d.Conditional, func(d DirectResponseConditional) ConditionalPolicyEntry[DirectResponse] {
		return ConditionalPolicyEntry[DirectResponse]{
			Condition: d.Condition,
			Policy:    d.Policy,
		}
	})
	if len(d.Conditional) > 0 {
		return nil, seq
	}
	return &d.DirectResponse, seq
}

// +k8s:enum
type JWTAuthenticationMode string

const (
	// A valid token, issued by a configured issuer, must be present.
	// This is the default option.
	JWTAuthenticationModeStrict JWTAuthenticationMode = "Strict"
	// If a token exists, validate it.
	// Warning: this allows requests without a JWT token!
	JWTAuthenticationModeOptional JWTAuthenticationMode = "Optional"
	// Requests are never rejected. This is useful for usage of claims in later steps (authorization, logging, etc).
	// Warning: this allows requests without a JWT token!
	JWTAuthenticationModePermissive JWTAuthenticationMode = "Permissive"
)

type AuthorizationLocationFields struct {
	// +optional
	Header *AuthorizationHeaderLocation `json:"header,omitempty"`
	// +optional
	QueryParameter *AuthorizationQueryParameterLocation `json:"queryParameter,omitempty"`
	// +optional
	Cookie *AuthorizationCookieLocation `json:"cookie,omitempty"`
}

// +kubebuilder:validation:ExactlyOneOf=header;queryParameter;cookie
type AuthorizationLocation struct {
	AuthorizationLocationFields `json:",inline"`
}

// +kubebuilder:validation:ExactlyOneOf=header;queryParameter;cookie;expression
type AuthorizationExtractionLocation struct {
	AuthorizationLocationFields `json:",inline"`

	// CEL expression that extracts the credential from the request.
	// +optional
	Expression *CELExpression `json:"expression,omitempty"`
}

type AuthorizationHeaderLocation struct {
	// +required
	Name gwv1.HTTPHeaderName `json:"name"`
	// +kubebuilder:validation:MinLength=1
	// +kubebuilder:validation:MaxLength=256
	// +optional
	Prefix *string `json:"prefix,omitempty"`
}

type AuthorizationQueryParameterLocation struct {
	// +kubebuilder:validation:MinLength=1
	// +kubebuilder:validation:MaxLength=256
	// +required
	Name string `json:"name"`
}

type AuthorizationCookieLocation struct {
	// +kubebuilder:validation:MinLength=1
	// +kubebuilder:validation:MaxLength=256
	// +required
	Name string `json:"name"`
}

// +kubebuilder:validation:XValidation:rule="!has(self.mcp) || size(self.providers) == 1",message="jwtAuthentication.mcp requires exactly one provider"
// +kubebuilder:validation:XValidation:rule="!has(self.mcp) || !has(self.mode) || self.mode == 'Strict'",message="jwtAuthentication.mcp requires mode Strict"
type JWTAuthentication struct {
	// Validation mode for JWT authentication.
	// +kubebuilder:default=Strict
	// +optional
	Mode JWTAuthenticationMode `json:"mode,omitempty"`

	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=64
	// +required
	Providers []JWTProvider `json:"providers"`

	// Where JWT credentials are read from.
	// If omitted, credentials are read from the `Authorization` header with the `Bearer ` prefix.
	// +optional
	Location *AuthorizationExtractionLocation `json:"location,omitempty"`

	// Enables MCP OAuth metadata endpoint handling
	// and MCP-specific authentication behavior on top of standard JWT validation.
	// When set, the gateway will serve the MCP OAuth metadata discovery endpoints.
	// +optional
	MCP *JWTMCPConfig `json:"mcp,omitempty"`
}

type JWTProvider struct {
	// IdP that issued the JWT. This corresponds to the
	// `iss` claim ([RFC 7519 §4.1.1](https://tools.ietf.org/html/rfc7519#section-4.1.1)).
	// +required
	Issuer ShortString `json:"issuer"`
	// Allowed audiences that are allowed
	// access. This corresponds to the `aud` claim
	// ([RFC 7519 §4.1.3](https://datatracker.ietf.org/doc/html/rfc7519#section-4.1.3)).
	// If unset, any audience is allowed.
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=64
	// +optional
	Audiences []string `json:"audiences,omitempty"`
	// JSON Web Key Set used to validate the signature of the
	// JWT.
	// +required
	JWKS JWKS `json:"jwks"`
}

// MCP-specific extensions for JWT authentication.
type JWTMCPConfig struct {
	// Metadata to use for MCP resources,
	// served at the MCP OAuth metadata endpoints.
	// +optional
	ResourceMetadata map[string]apiextensionsv1.JSON `json:"resourceMetadata,omitempty"`

	// Identity provider to use for MCP authentication flows.
	// +kubebuilder:validation:Enum=Auth0;Keycloak;Okta
	// +optional
	Provider *McpIDP `json:"provider,omitempty"`

	// Client ID to use for short-circuiting Dynamic Client Registration.
	// If set, the gateway will not proxy registration requests to the IDP and instead return this client ID.
	// +optional
	ClientID *string `json:"clientId,omitempty"`
}

// +kubebuilder:validation:ExactlyOneOf=remote;inline
type JWKS struct {
	// How to reach the JSON Web Key Set from a remote
	// address.
	// +optional
	Remote *RemoteJWKS `json:"remote,omitempty"`
	// Inline JSON Web Key Set used to validate the
	// signature of the JWT.
	// +kubebuilder:validation:MinLength=2
	// +kubebuilder:validation:MaxLength=65536
	// +optional
	Inline *string `json:"inline,omitempty"`
}

type RemoteJWKS struct {
	// Path to the IdP `jwks` endpoint, relative to the root, commonly
	// `".well-known/jwks.json"`.
	// +required
	// +kubebuilder:validation:MinLength=1
	// +kubebuilder:validation:MaxLength=2000
	JwksPath string `json:"jwksPath"`
	// +optional
	// +kubebuilder:validation:XValidation:rule="matches(self, '^([0-9]{1,5}(h|m|s|ms)){1,4}$')",message="invalid duration value"
	// +kubebuilder:validation:XValidation:rule="duration(self) >= duration('5m')",message="cacheDuration must be at least 5m."
	// +kubebuilder:default="5m"
	CacheDuration *metav1.Duration `json:"cacheDuration,omitempty"`
	// Remote JWKS server to reach.
	// Supported types are `Service` and static `Backend`. An
	// `AgentgatewayPolicy` containing backend TLS config can then be attached
	// to the `Service` or `Backend` in order to set TLS options for a
	// connection to the remote `jwks` source.
	// +required
	BackendRef gwv1.BackendObjectReference `json:"backendRef"`
}

// +k8s:enum
type BasicAuthenticationMode string

const (
	// A valid username and password must be present.
	// This is the default option.
	BasicAuthenticationModeStrict BasicAuthenticationMode = "Strict"
	// If a username and password exists, validate it.
	// Warning: this allows requests without a username!
	BasicAuthenticationModeOptional BasicAuthenticationMode = "Optional"
)

// +kubebuilder:validation:ExactlyOneOf=users;secretRef
type BasicAuthentication struct {
	// Validation mode for basic authentication.
	// +kubebuilder:default=Strict
	// +optional
	Mode BasicAuthenticationMode `json:"mode,omitempty"`

	// `realm` value to return in the `WWW-Authenticate`
	// header for failed authentication requests. If unset, `Restricted` will
	// be used.
	// +optional
	Realm *string `json:"realm,omitempty"`

	// Inline list of username and password pairs that will
	// be accepted. Each entry represents one line of the `htpasswd` format:
	// https://httpd.apache.org/docs/2.4/programs/htpasswd.html.
	//
	// Note: passwords should be the hash of the password, not the raw password. Use the `htpasswd` or similar commands
	// to generate a hash. MD5, bcrypt, crypt, and SHA-1 are supported.
	//
	// Example:
	//
	//	users:
	//	- "user1:$apr1$ivPt0D4C$DmRhnewfHRSrb3DQC.WHC."
	//	- "user2:$2y$05$r3J4d3VepzFkedkd/q1vI.pBYIpSqjfN0qOARV3ScUHysatnS0cL2"
	//
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=256
	// +optional
	Users []string `json:"users,omitempty"`

	// Credential source, defaulting to a Kubernetes
	// `Secret`, storing the `.htaccess` file. When using the default Secret
	// resolver, the `Secret` must have a key named `.htaccess`, and should
	// contain the complete `.htaccess` file.
	//
	// Note: passwords should be the hash of the password, not the raw password. Use the `htpasswd` or similar commands
	// to generate a hash. MD5, bcrypt, crypt, and SHA-1 are supported.
	//
	// Example:
	//
	//	apiVersion: v1
	//	kind: Secret
	//	metadata:
	//	  name: basic-auth
	//	stringData:
	//	  .htaccess: |
	//	    alice:$apr1$3zSE0Abt$IuETi4l5yO87MuOrbSE4V.
	//	    bob:$apr1$Ukb5LgRD$EPY2lIfY.A54jzLELNIId/
	// +optional
	SecretRef *LocalSecretObjectRef `json:"secretRef,omitempty"`

	// Where Basic credentials are read from.
	// If omitted, credentials are read from the `Authorization` header with the `Basic ` prefix.
	// +optional
	Location *AuthorizationExtractionLocation `json:"location,omitempty"`
}

// +k8s:enum
type APIKeyAuthenticationMode string

const (
	// A valid API Key must be present.
	// This is the default option.
	APIKeyAuthenticationModeStrict APIKeyAuthenticationMode = "Strict"
	// If an API Key exists, validate it.
	// Warning: this allows requests without an API Key!
	APIKeyAuthenticationModeOptional APIKeyAuthenticationMode = "Optional"
	// Requests are never rejected for missing or invalid API keys.
	// Warning: this allows requests without a valid API key!
	APIKeyAuthenticationModePermissive APIKeyAuthenticationMode = "Permissive"
)

// +kubebuilder:validation:ExactlyOneOf=secretRef;secretSelector
type APIKeyAuthentication struct {
	// Validation mode for API key authentication.
	// +kubebuilder:default=Strict
	// +optional
	Mode APIKeyAuthenticationMode `json:"mode,omitempty"`

	// Credential source, defaulting to a Kubernetes
	// `Secret`, storing a set of API keys. If there are many Secret-backed
	// keys, `secretSelector` can be used instead.
	//
	// Each entry in the credential data represents one API key. The key is an
	// arbitrary identifier. The value can either be:
	// * A string representing the API key.
	// * A JSON object with two fields, `key` and `metadata`. `key` contains
	//   the API key. `metadata` contains arbitrary JSON metadata associated
	//   with the key, which may be used by other policies. For example, you
	//   may write an authorization policy allowing `apiKey.group == 'sales'`.
	//
	// Example:
	//
	//	apiVersion: v1
	//	kind: Secret
	//	metadata:
	//	  name: api-key
	//	stringData:
	//	  client1: |
	//	    {
	//	      "key": "k-123",
	//	      "metadata": {
	//	        "group": "sales",
	//	        "created_at": "2024-10-01T12:00:00Z"
	//	      }
	//	    }
	//	  client2: "k-456"
	// +optional
	SecretRef *LocalSecretObjectRef `json:"secretRef,omitempty"`

	// Selects multiple Kubernetes `Secret` resources
	// containing API keys. It is Secret-only; use `secretRef` for other
	// credential kinds. If the same key is defined in multiple secrets, the
	// behavior is undefined.
	//
	// Each entry in the `Secret` data represents one API key. The key is an
	// arbitrary identifier. The value can either be:
	// * A string representing the API key.
	// * A JSON object with two fields, `key` and `metadata`. `key` contains
	//   the API key. `metadata` contains arbitrary JSON metadata associated
	//   with the key, which may be used by other policies. For example, you
	//   may write an authorization policy allowing `apiKey.group == 'sales'`.
	//
	// Example:
	//
	//	apiVersion: v1
	//	kind: Secret
	//	metadata:
	//	  name: api-key
	//	stringData:
	//	  client1: |
	//	    {
	//	      "key": "k-123",
	//	      "metadata": {
	//	        "group": "sales",
	//	        "created_at": "2024-10-01T12:00:00Z"
	//	      }
	//	    }
	//	  client2: "k-456"
	// +optional
	SecretSelector *SecretSelector `json:"secretSelector,omitempty"`

	// Where API keys are read from.
	// If omitted, credentials are read from the `Authorization` header with the `Bearer ` prefix.
	// +optional
	Location *AuthorizationExtractionLocation `json:"location,omitempty"`
}

type BufferBody struct {
	// Maximum number of bytes to buffer from the request or response body.
	// +optional
	// If unset, defaults to the global proxy setting, which defaults to 2Mi.
	MaxBytes *ByteSize `json:"maxBytes,omitempty"`
}

// +kubebuilder:validation:AtLeastOneFieldSet
type Buffer struct {
	// Request body buffering settings.
	// +optional
	Request *BufferBody `json:"request,omitempty"`
	// Response body buffering settings.
	// +optional
	Response *BufferBody `json:"response,omitempty"`
}

type SecretSelector struct {
	// Labels that must be present on each selected Secret.
	// +required
	MatchLabels map[string]string `json:"matchLabels"`
}

// +k8s:enum
type HostnameRewriteMode string

const (
	HostnameRewriteModeAuto HostnameRewriteMode = "Auto"
	HostnameRewriteModeNone HostnameRewriteMode = "None"
)

// +kubebuilder:validation:ExactlyOneOf=key;secretRef;passthrough;aws;azure;gcp
// +kubebuilder:validation:XValidation:rule="has(self.location) ? has(self.key) || has(self.secretRef) || has(self.passthrough) : true",message="location may only be set for key or passthrough auth"
type BackendAuth struct {
	// Inline key to use as the value of the
	// `Authorization` header. This option is the least secure; usage of a
	// `Secret` is preferred.
	// +kubebuilder:validation:MaxLength=2048
	// +optional
	InlineKey *string `json:"key,omitempty"`

	// Credential source, defaulting to a Kubernetes
	// `Secret`, storing the key to use as the authorization value. When using
	// the default Secret resolver, this must be stored in the `Authorization`
	// key.
	// +optional
	SecretRef *LocalSecretObjectRef `json:"secretRef,omitempty"`

	// Passes through an existing token that has been sent by the
	// client and validated. Other policies, like JWT and API key
	// authentication, will strip the original client credentials. Passthrough backend authentication
	// causes the original token to be added back into the request. If there are no client authentication policies on the
	// request, the original token would be unchanged, so this would have no effect.
	// +optional
	Passthrough *BackendAuthPassthrough `json:"passthrough,omitempty"`

	// Explicit AWS authentication method for the backend.
	// When omitted, default AWS SDK credential discovery is used.
	//
	// +optional
	AWS *AwsAuth `json:"aws,omitempty"`

	// Azure authentication method for the backend.
	//
	// +optional
	Azure *AzureAuth `json:"azure,omitempty"`

	// Google authentication method for the backend.
	// When omitted, default Google credential discovery is used.
	//
	// +optional
	GCP *GcpAuth `json:"gcp,omitempty"`

	// Where backend credentials are inserted.
	// If omitted, credentials are written to the `Authorization` header with the `Bearer ` prefix.
	// This applies to `key`, `secretRef`, and `passthrough`.
	// +optional
	Location *AuthorizationLocation `json:"location,omitempty"`
}

// +k8s:enum
type GcpAuthType string

const (
	GcpAuthTypeAccessToken GcpAuthType = "AccessToken"
	GcpAuthTypeIdToken     GcpAuthType = "IdToken"
)

// Google Cloud authentication settings.
// +kubebuilder:validation:XValidation:rule="has(self.audience) ? self.type == 'IdToken' : true",message="audience is only valid with IdToken"
type GcpAuth struct {
	// The type of token to generate. To authenticate to GCP services,
	// generally an `AccessToken` is used. To authenticate to Cloud Run, an
	// `IdToken` is used.
	//
	// +optional
	Type *GcpAuthType `json:"type,omitempty"`
	// Credential source, defaulting to a Kubernetes
	// `Secret`, containing ADC-compatible Google credential JSON. When using
	// the default Secret resolver, this must be stored in the `credentials.json`
	// key. When omitted, ambient credentials are used.
	//
	// +optional
	SecretRef *LocalSecretObjectRef `json:"secretRef,omitempty"`
	// Explicit `aud` value for the ID token. Only
	// valid with `IdToken` type. If not set, the `aud` is automatically
	// derived from the backend hostname.
	//
	// +optional
	Audience *ShortString `json:"audience,omitempty"`
}

// AWS authentication settings for the backend.
//
// +kubebuilder:validation:XValidation:rule="!(has(self.secretRef) && has(self.assumeRole))",message="secretRef and assumeRole are mutually exclusive"
type AwsAuth struct {
	// Credential source, defaulting to a Kubernetes
	// `Secret`, containing the AWS credentials. When using the default Secret
	// resolver, the `Secret` must have keys `accessKey`, `secretKey`, and
	// optionally `sessionToken`.
	// +optional
	SecretRef *LocalSecretObjectRef `json:"secretRef,omitempty"`

	// AWS STS AssumeRole settings to use before signing backend requests.
	// Ambient AWS credentials are used as the source credentials for STS.
	//
	// +optional
	AssumeRole *AwsAssumeRole `json:"assumeRole,omitempty"`

	// AWS SigV4 signing service name, for example
	// `bedrock`, `bedrock-agentcore`, or `execute-api`). If unset, typed AWS
	// backends may provide this automatically.
	//
	// +optional
	ServiceName *ShortString `json:"serviceName,omitempty"`
}

// AWS STS AssumeRole settings for backend authentication.
type AwsAssumeRole struct {
	// AWS IAM role ARN to assume.
	//
	// +kubebuilder:validation:MinLength=1
	// +kubebuilder:validation:Pattern="^arn:aws[a-z-]*:iam::[0-9]{12}:role/.+$"
	// +required
	RoleArn string `json:"roleArn"`
}

type AzureAuth struct {
	// Credential source, defaulting to a Kubernetes
	// `Secret`, containing the Azure credentials. When using the default Secret
	// resolver, the `Secret` must have keys `clientID`, `tenantID`, and
	// `clientSecret`.
	//
	// +optional
	SecretRef *LocalSecretObjectRef `json:"secretRef,omitempty"`

	// Managed identity authentication settings.
	//
	// +optional
	ManagedIdentity *AzureManagedIdentity `json:"managedIdentity,omitempty"`
}

type AzureManagedIdentity struct {
	// +required
	ClientID string `json:"clientId"`
	// +required
	ObjectID string `json:"objectId"`
	// +required
	ResourceID string `json:"resourceId"`
}

type BackendAuthPassthrough struct {
}

// +kubebuilder:validation:AtLeastOneFieldSet
type BackendAI struct {
	// Enriches requests sent to the LLM provider by appending and prepending system prompts. This can be configured only for
	// LLM providers that use the `CHAT` or `CHAT_STREAMING` API route type.
	// +optional
	PromptEnrichment *AIPromptEnrichment `json:"prompt,omitempty"`

	// Guardrails for LLM requests and responses.
	// +optional
	PromptGuard *AIPromptGuard `json:"promptGuard,omitempty"`

	// Defaults to merge with user input fields. If the field is already set, the field in the request is used.
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=64
	// +optional
	Defaults []FieldDefault `json:"defaults,omitempty"`
	// Overrides to merge with user input fields. If the field is already set, the field is overwritten.
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=64
	// +optional
	Overrides []FieldDefault `json:"overrides,omitempty"`
	// CEL transformations to compute and set fields in the request body.
	// The expression result overwrites any existing value for that field.
	// This has a higher priority than `overrides` if both are set for the same
	// key.
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=64
	// +optional
	Transformations []FieldTransformation `json:"transformations,omitempty"`

	// Maps friendly model names to actual provider model names.
	// Example: `{"fast": "gpt-3.5-turbo", "smart": "gpt-4-turbo"}`.
	// Note: This field is only applicable when using the agentgateway data plane.
	// +kubebuilder:validation:MaxProperties=64
	// +optional
	ModelAliases map[string]string `json:"modelAliases,omitempty"`

	// Automatic prompt caching for supported
	// providers, currently AWS Bedrock.
	// Reduces API costs by caching static content like system prompts and tool definitions.
	// Only applicable for Bedrock Claude 3+ and Nova models.
	// +optional
	PromptCaching *PromptCachingConfig `json:"promptCaching,omitempty"`

	// Rules for identifying the type of traffic to handle.
	// The keys are URL path suffixes matched using ends-with comparison, for
	// example `"/v1/chat/completions"`.
	// The special `*` wildcard matches any path.
	// If not specified, all traffic defaults to `completions` type.
	// +optional
	Routes map[string]RouteType `json:"routes,omitempty"`
}

// How the AI gateway should process incoming requests
// based on the URL path and the API format expected.
// +k8s:enum
type RouteType string

const (
	// RouteTypeCompletions processes OpenAI `/v1/chat/completions` format requests.
	RouteTypeCompletions RouteType = "Completions"

	// RouteTypeMessages processes Anthropic `/v1/messages` format requests.
	RouteTypeMessages RouteType = "Messages"

	// RouteTypeModels handles the `/v1/models` endpoint.
	RouteTypeModels RouteType = "Models"

	// RouteTypePassthrough sends requests upstream as-is without LLM processing.
	RouteTypePassthrough RouteType = "Passthrough"

	// RouteTypeDetect sends requests as-is but attempts to extract
	// request/response metadata for telemetry and rate limiting.
	RouteTypeDetect RouteType = "Detect"

	// RouteTypeResponses processes OpenAI `/v1/responses` format requests.
	RouteTypeResponses RouteType = "Responses"

	// RouteTypeAnthropicTokenCount processes Anthropic
	// `/v1/messages/count_tokens` format requests.
	RouteTypeAnthropicTokenCount RouteType = "AnthropicTokenCount" //nolint:gosec // G101: False positive - this is a route type name, not credentials

	// RouteTypeEmbeddings processes OpenAI `/v1/embeddings` format requests.
	RouteTypeEmbeddings RouteType = "Embeddings"

	// RouteTypeRealtime processes OpenAI `/v1/realtime` requests.
	RouteTypeRealtime RouteType = "Realtime"

	// RouteTypeRerank processes Cohere `/v2/rerank` format requests.
	RouteTypeRerank RouteType = "Rerank"
)

// +kubebuilder:validation:AtLeastOneFieldSet
type BackendMCP struct {
	// MCP backend authorization. Unlike authorization at the HTTP level, which rejects
	// unauthorized requests with a `403` error, this policy works at the
	// `MCPBackend` level.
	//
	// List operations, such as `list_tools`, will have each item evaluated.
	// Items that do not meet the rule will be filtered.
	//
	// Get or call operations, such as `call_tool`, will evaluate the specific
	// item and reject requests that do not meet the rule.
	// +optional
	Authorization *Authorization `json:"authorization,omitempty"`
	// MCP backend-specific authentication rules.
	//
	// This field is deprecated; prefer to use traffic policy `jwtAuthentication.mcp`, which ensures authentication runs before
	// other policies such as transformation and rate limiting.
	//
	// +optional
	Authentication *MCPAuthentication `json:"authentication,omitempty"`

	// `guardrails` routes selected JSON-RPC methods through a remote policy server.
	// +optional
	Guardrails *MCPGuardrails `json:"guardrails,omitempty"`
}

// MCPMethodPhase controls when an MCP method is run through the guardrails pipeline.
// +k8s:enum
type MCPMethodPhase string

const (
	MCPMethodPhaseOff      MCPMethodPhase = "Off"
	MCPMethodPhaseRequest  MCPMethodPhase = "Request"
	MCPMethodPhaseResponse MCPMethodPhase = "Response"
	MCPMethodPhaseFull     MCPMethodPhase = "Full"
)

// MCPGuardrails is the MCP-layer analog of Envoy ext_authz: an ordered chain of
// policy processors invoked per JSON-RPC method.
type MCPGuardrails struct {
	// `processors` is the ordered list of policy processors applied to matched
	// methods. Processors run in the order listed; the first to reject a request
	// short-circuits the chain.
	// +required
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	Processors []MCPGuardrailsProcessor `json:"processors"`
}

// MCPGuardrailsProcessor selects a single policy processor. Exactly one variant must be set.
// +kubebuilder:validation:ExactlyOneOf=remote
type MCPGuardrailsProcessor struct {
	// `remote` configures a gRPC policy server.
	// +optional
	Remote *MCPGuardrailsRemote `json:"remote,omitempty"`

	// `methods` is the allowlist of JSON-RPC methods (e.g. `tools/call`,
	// `tools/list`) routed through this processor, keyed by method name with the
	// phase it runs in. Keys may be exact, a prefix wildcard (`tools/*`), a suffix
	// wildcard (`*/list`), or `*` for all methods; the most specific match wins.
	// Methods matching no key, including unknown ones, bypass this processor.
	// +required
	// +kubebuilder:validation:MinProperties=1
	// +kubebuilder:validation:MaxProperties=64
	// +kubebuilder:validation:XValidation:rule="self.all(k, !k.contains('*') || (k.indexOf('*') == k.lastIndexOf('*') && (k.indexOf('*') == 0 || k.indexOf('*') == size(k) - 1)))",message="method wildcards must be '*', a prefix like 'tools/*', or a suffix like '*/list'"
	Methods map[string]MCPMethodPhase `json:"methods"`
}

type MCPGuardrailsRemote struct {
	// `backendRef` references the remote guardrails policy server.
	// Supported types: `Service` and `Backend`.
	// +required
	BackendRef gwv1.BackendObjectReference `json:"backendRef"`

	// `failureMode` controls behavior when the policy server is unreachable
	// or returns an error. `FailOpen` allows the request; `FailClosed`
	// (default) denies it.
	// +optional
	FailureMode FailureMode `json:"failureMode,omitempty"`

	// `metadata` is static or CEL-evaluated context surfaced to the policy
	// server as fields of the `metadata_context` google.protobuf.Struct,
	// keyed by config key. Values are CEL expressions.
	// +optional
	// +kubebuilder:validation:MaxProperties=64
	Metadata map[string]CELExpression `json:"metadata,omitempty"`

	// `allowedRequestHeaders` lists the incoming request headers forwarded to
	// the policy server in `McpRequest.headers`. If empty, all headers and
	// pseudo-headers (`:authority`, `:method`, ...) are forwarded. Matching is
	// case-insensitive.
	// +optional
	// +listType=set
	// +kubebuilder:validation:MaxItems=64
	AllowedRequestHeaders []HeaderName `json:"allowedRequestHeaders,omitempty"`

	// `disallowedRequestHeaders` lists header names never forwarded to the
	// policy server, even if listed in `allowedRequestHeaders`. Matching is
	// case-insensitive.
	// +optional
	// +listType=set
	// +kubebuilder:validation:MaxItems=64
	DisallowedRequestHeaders []HeaderName `json:"disallowedRequestHeaders,omitempty"`
}

type MCPAuthentication struct {
	// Metadata to use for MCP resources.
	// +optional
	ResourceMetadata map[string]apiextensionsv1.JSON `json:"resourceMetadata"`

	// Identity provider to use for authentication.
	// +kubebuilder:validation:Enum=Auth0;Keycloak;Okta
	// +optional
	McpIDP *McpIDP `json:"provider,omitempty"`

	// IdP that issued the JWT. This corresponds to the
	// `iss` claim ([RFC 7519 §4.1.1](https://tools.ietf.org/html/rfc7519#section-4.1.1)).
	// +optional
	Issuer ShortString `json:"issuer,omitempty"`

	// Allowed audiences that are allowed
	// access. This corresponds to the `aud` claim
	// ([RFC 7519 §4.1.3](https://datatracker.ietf.org/doc/html/rfc7519#section-4.1.3)).
	// If unset, any audience is allowed.
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=64
	// +optional
	Audiences []string `json:"audiences,omitempty"`

	// Remote JSON Web Key used to validate the signature of
	// the JWT.
	// +required
	JWKS RemoteJWKS `json:"jwks"`

	// Validation mode for JWT authentication.
	// +kubebuilder:default=Strict
	// +optional
	Mode JWTAuthenticationMode `json:"mode,omitempty"`

	// Client ID to use for short-circuiting Dynamic Client Registration.
	// If set, the gateway will not proxy registration requests to the IDP and instead return this client ID.
	// +optional
	ClientID *string `json:"clientId,omitempty"`
}

// +k8s:enum
type McpIDP string

const (
	Auth0    McpIDP = "Auth0"
	Keycloak McpIDP = "Keycloak"
	Okta     McpIDP = "Okta"
)

type BackendTunnel struct {
	// Proxy server to reach.
	// Supported types: `Service` and `Backend`.
	// +required
	BackendRef gwv1.BackendObjectReference `json:"backendRef"`
}

type BackendHTTP struct {
	// HTTP protocol version to use when connecting to
	// the backend.
	// If not specified, the version is automatically determined:
	// * `Service` types can specify it with `appProtocol` on the `Service`
	//   port.
	// * If traffic is identified as gRPC, `HTTP2` is used.
	// * If the incoming traffic was plaintext HTTP, the original protocol will
	//   be used.
	// * If the incoming traffic was HTTPS, `HTTP1` will be used. This is
	//   because most clients will transparently upgrade HTTPS traffic to
	//   `HTTP2`, even if the backend doesn't support it.
	// +optional
	Version *HTTPVersion `json:"version,omitempty"`

	// Deadline for receiving a response from the backend.
	// +kubebuilder:validation:XValidation:rule="matches(self, '^([0-9]{1,5}(h|m|s|ms)){1,4}$')",message="invalid duration value"
	// +kubebuilder:validation:XValidation:rule="duration(self) >= duration('1ms')",message="requestTimeout must be at least 1ms"
	// +optional
	RequestTimeout *metav1.Duration `json:"requestTimeout,omitempty"`
}

// +k8s:enum
type HTTPVersion string

const (
	HTTPVersion1 HTTPVersion = "HTTP1"
	HTTPVersion2 HTTPVersion = "HTTP2"
)

type BackendTCP struct {
	// Settings for enabling TCP keepalives on the
	// connection.
	// +optional
	Keepalive *Keepalive `json:"keepalive,omitempty"`
	// Deadline for establishing a connection to
	// the destination.
	// +kubebuilder:validation:XValidation:rule="matches(self, '^([0-9]{1,5}(h|m|s|ms)){1,4}$')",message="invalid duration value"
	// +kubebuilder:validation:XValidation:rule="duration(self) >= duration('100ms')",message="connectTimeout must be at least 100ms"
	// +optional
	ConnectTimeout *metav1.Duration `json:"connectTimeout,omitempty"`
}

// +kubebuilder:validation:AtLeastOneFieldSet
type Transformation struct {
	// Request transformation settings.
	// +optional
	Request *Transform `json:"request,omitempty"`

	// Response transformation settings.
	// +optional
	Response *Transform `json:"response,omitempty"`
}

type TransformationConditional struct {
	// CEL expression that must evaluate to true for this policy to execute.
	// +optional
	Condition CELExpression `json:"condition,omitempty"`
	// Policy to apply when the condition matches.
	// +required
	Policy Transformation `json:"policy"`
}

// +kubebuilder:validation:ConditionalPolicy
// +kubebuilder:validation:AtLeastOneFieldSet
type TransformationOrConditional struct {
	// Request transformation settings.
	// +optional
	Request *Transform `json:"request,omitempty"`

	// Response transformation settings.
	// +optional
	Response *Transform `json:"response,omitempty"`

	// Conditional policy execution. Set this or the top-level transformation fields.
	// The first matching policy will be executed.
	// A single policy may be provided without a condition set; if so, it must be the last policy and will be the fallback
	// in case no conditions are met.
	// +optional
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +kubebuilder:validation:XValidation:message="conditional entries without condition must be last",rule="self.filter(e, !has(e.condition)).size() <= 1 && (!self.exists(e, !has(e.condition)) || !has(self[size(self) - 1].condition))"
	Conditional []TransformationConditional `json:"conditional,omitempty"`
}

func (t *TransformationOrConditional) ConditionalPolicy() (*Transformation, iter.Seq[ConditionalPolicyEntry[Transformation]]) {
	seq := mapseq(t.Conditional, func(t TransformationConditional) ConditionalPolicyEntry[Transformation] {
		return ConditionalPolicyEntry[Transformation]{
			Condition: t.Condition,
			Policy:    t.Policy,
		}
	})
	if len(t.Conditional) > 0 {
		return nil, seq
	}
	return &Transformation{Request: t.Request, Response: t.Response}, seq
}

// +kubebuilder:validation:AtLeastOneFieldSet
type Transform struct {
	// Headers to set and the values to use.
	//
	// +listType=map
	// +listMapKey=name
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +optional
	Set []HeaderTransformation `json:"set,omitempty"`

	// Headers to add to the request and what each value
	// should be set to. If there is already a header with these values then
	// append the value as an extra entry.
	//
	// +listType=map
	// +listMapKey=name
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +optional
	Add []HeaderTransformation `json:"add,omitempty"`

	// Header names to remove from the request or
	// response.
	//
	// +listType=set
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +optional
	Remove []HeaderName `json:"remove,omitempty"`

	// HTTP body transformation.
	// +optional
	Body *CELExpression `json:"body,omitempty"`

	// Stores CEL-evaluated values under the `metadata` CEL variable
	// for subsequent policy evaluations. `metadata` is evaluated before header
	// or body transformations.
	//
	// +kubebuilder:validation:MinProperties=1
	// +kubebuilder:validation:MaxProperties=16
	// +optional
	Metadata map[string]CELExpression `json:"metadata,omitempty"`
}

// HTTP header name.
//
// +kubebuilder:validation:MinLength=1
// +kubebuilder:validation:MaxLength=256
// +kubebuilder:validation:Pattern=`^:?[A-Za-z0-9!#$%&'*+\-.^_\x60|~]+$`
// +kubebuilder:validation:XValidation:rule="!self.startsWith(':') || self in [':authority', ':method', ':path', ':scheme', ':status']",message="pseudo-headers must be one of :authority, :method, :path, :scheme, or :status"
// +k8s:deepcopy-gen=false
type HeaderName string

// HTTP header name that does not allow pseudo-headers.
//
// +kubebuilder:validation:MinLength=1
// +kubebuilder:validation:MaxLength=256
// +kubebuilder:validation:Pattern=`^[A-Za-z0-9!#$%&'*+\-.^_\x60|~]+$`
// +k8s:deepcopy-gen=false
type HTTPHeaderName string

type DirectResponseHeader struct {
	// The name of the header to set.
	// +required
	Name HTTPHeaderName `json:"name"`
	// CEL expression that generates the output value for
	// the header.
	// +required
	Value CELExpression `json:"value"`
}

type HeaderTransformation struct {
	// The name of the header to add.
	// +required
	Name HeaderName `json:"name"`
	// CEL expression that generates the output value for
	// the header.
	// +required
	Value CELExpression `json:"value"`
}

// How HTTP bodies are delivered to the external processor.
// +kubebuilder:validation:Enum=None;Buffered;BufferedPartial;FullDuplexStreamed
type BodySendMode string

const (
	// BodySendModeNone does not send the body to the external processor.
	BodySendModeNone BodySendMode = "None"
	// BodySendModeBuffered buffers the full body before sending it to the
	// external processor. It returns an error if the body exceeds 8KB.
	BodySendModeBuffered BodySendMode = "Buffered"
	// BodySendModeBufferedPartial buffers up to 8KB. If the body exceeds that
	// limit, it sends the buffered prefix instead of returning an error.
	BodySendModeBufferedPartial BodySendMode = "BufferedPartial"
	// BodySendModeFullDuplexStreamed streams the body to the external processor.
	BodySendModeFullDuplexStreamed BodySendMode = "FullDuplexStreamed"
)

// Whether HTTP headers are delivered to the external processor.
// +kubebuilder:validation:Enum=Send;Skip
type HeaderSendMode string

const (
	// HeaderSendModeSend sends headers to the external processor.
	HeaderSendModeSend HeaderSendMode = "Send"
	// HeaderSendModeSkip does not send headers to the external processor.
	HeaderSendModeSkip HeaderSendMode = "Skip"
)

// Whether HTTP trailers are delivered to the external processor.
// +kubebuilder:validation:Enum=Skip;Send
type TrailerSendMode string

const (
	// TrailerSendModeSkip does not send trailers to the external processor.
	TrailerSendModeSkip TrailerSendMode = "Skip"
	// TrailerSendModeSend sends trailers to the external processor.
	TrailerSendModeSend TrailerSendMode = "Send"
)

// External processor request and response phase settings.
type ProcessingOptions struct {
	// How request bodies are sent to the external processor.
	// `Buffered` buffers the full body and returns an error if it exceeds 8KB.
	// `BufferedPartial` buffers up to 8KB and sends the buffered prefix if the
	// body exceeds that limit. Defaults to `FullDuplexStreamed`.
	// +optional
	// +kubebuilder:default=FullDuplexStreamed
	RequestBodyMode *BodySendMode `json:"requestBodyMode,omitempty"`

	// How response bodies are sent to the external processor.
	// `Buffered` buffers the full body and returns an error if it exceeds 8KB.
	// `BufferedPartial` buffers up to 8KB and sends the buffered prefix if the
	// body exceeds that limit. Defaults to `FullDuplexStreamed`.
	// +optional
	// +kubebuilder:default=FullDuplexStreamed
	ResponseBodyMode *BodySendMode `json:"responseBodyMode,omitempty"`

	// Whether request headers are sent to the external processor.
	// Defaults to `Send`.
	// +optional
	// +kubebuilder:default=Send
	RequestHeaderMode *HeaderSendMode `json:"requestHeaderMode,omitempty"`

	// Whether response headers are sent to the external processor.
	// Defaults to `Send`.
	// +optional
	// +kubebuilder:default=Send
	ResponseHeaderMode *HeaderSendMode `json:"responseHeaderMode,omitempty"`

	// Whether request trailers are sent to the external processor.
	// Defaults to `Send`.
	// +optional
	// +kubebuilder:default=Send
	RequestTrailerMode *TrailerSendMode `json:"requestTrailerMode,omitempty"`

	// Whether response trailers are sent to the external processor.
	// Defaults to `Send`.
	// +optional
	// +kubebuilder:default=Send
	ResponseTrailerMode *TrailerSendMode `json:"responseTrailerMode,omitempty"`

	// Allows ext_proc `mode_override` values from matching header responses to update
	// subsequent request/response processing phases for this exchange. Defaults to `false`.
	// +optional
	// +kubebuilder:default=false
	AllowModeOverride bool `json:"allowModeOverride,omitempty"`
}

type ExtProc struct {
	// External Processor server to reach.
	// Supported types: `Service` and `Backend`.
	// +optional
	BackendRef *gwv1.BackendObjectReference `json:"backendRef,omitempty"`
	// How request and response phases are sent to ext_proc.
	// +optional
	ProcessingOptions *ProcessingOptions `json:"processingOptions,omitempty"`
}

type ExtProcConditional struct {
	// CEL expression that must evaluate to true for this policy to execute.
	// +optional
	Condition CELExpression `json:"condition,omitempty"`
	// Policy to apply when the condition matches.
	// +required
	// +kubebuilder:validation:XValidation:rule="has(self.backendRef)",message="backendRef is required"
	Policy ExtProc `json:"policy"`
}

// +kubebuilder:validation:ConditionalPolicy:fields=backendRef
type ExtProcOrConditional struct {
	// +optional
	ExtProc `json:",inline"`
	// Conditional policy execution. Set this or the top-level extProc fields.
	// The first matching policy will be executed.
	// A single policy may be provided without a condition set; if so, it must be the last policy and will be the fallback
	// in case no conditions are met.
	// +optional
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +kubebuilder:validation:XValidation:message="conditional entries without condition must be last",rule="self.filter(e, !has(e.condition)).size() <= 1 && (!self.exists(e, !has(e.condition)) || !has(self[size(self) - 1].condition))"
	Conditional []ExtProcConditional `json:"conditional,omitempty"`
}

func (e *ExtProcOrConditional) ConditionalPolicy() (*ExtProc, iter.Seq[ConditionalPolicyEntry[ExtProc]]) {
	seq := mapseq(e.Conditional, func(e ExtProcConditional) ConditionalPolicyEntry[ExtProc] {
		return ConditionalPolicyEntry[ExtProc]{
			Condition: e.Condition,
			Policy:    e.Policy,
		}
	})
	if len(e.Conditional) > 0 {
		return nil, seq
	}
	return &e.ExtProc, seq
}

// +k8s:deepcopy-gen=false
// nolint: kubeapilinter
type ConditionalPolicyEntry[T any] struct {
	Condition CELExpression
	Policy    T
}

// +k8s:deepcopy-gen=false
// nolint: kubeapilinter
type ConditionalPolicy[T any] interface {
	ConditionalPolicy() (*T, iter.Seq[ConditionalPolicyEntry[T]])
}

type ExtAuthConditional struct {
	// CEL expression that must evaluate to true for this policy to execute.
	// +optional
	Condition CELExpression `json:"condition,omitempty"`
	// Policy to apply when the condition matches.
	// +required
	// +kubebuilder:validation:XValidation:rule="has(self.backendRef)",message="backendRef is required"
	// +kubebuilder:validation:XValidation:rule="[has(self.grpc),has(self.http)].filter(x,x==true).size() == 1",message="exactly one of the fields in [grpc http] must be set"
	Policy ExtAuth `json:"policy"`
}

// +kubebuilder:validation:ConditionalPolicy:fields=backendRef
// +kubebuilder:validation:XValidation:rule="has(self.conditional) || [has(self.grpc),has(self.http)].filter(x,x==true).size() == 1",message="exactly one of the fields in [grpc http] must be set"
type ExtAuthOrConditional struct {
	// +optional
	ExtAuth `json:",inline"`
	// Conditional policy execution. Set this or the top-level extAuth fields.
	// The first matching policy will be executed.
	// A single policy may be provided without a condition set; if so, it must be the last policy and will be the fallback
	// in case no conditions are met.
	// +optional
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +kubebuilder:validation:XValidation:message="conditional entries without condition must be last",rule="self.filter(e, !has(e.condition)).size() <= 1 && (!self.exists(e, !has(e.condition)) || !has(self[size(self) - 1].condition))"
	Conditional []ExtAuthConditional `json:"conditional,omitempty"`
}

func (e *ExtAuthOrConditional) ConditionalPolicy() (*ExtAuth, iter.Seq[ConditionalPolicyEntry[ExtAuth]]) {
	seq := mapseq(e.Conditional, func(e ExtAuthConditional) ConditionalPolicyEntry[ExtAuth] {
		return ConditionalPolicyEntry[ExtAuth]{
			Condition: e.Condition,
			Policy:    e.Policy,
		}
	})
	if len(e.Conditional) > 0 {
		return nil, seq
	}
	return &e.ExtAuth, seq
}

// mapseq runs f() over all elements in s and returns the result
func mapseq[E any, O any](s []E, f func(E) O) iter.Seq[O] {
	return func(yield func(O) bool) {
		for _, e := range s {
			yield(f(e))
		}
	}
}

// +kubebuilder:validation:XValidation:rule="!has(self.cache) || has(self.grpc)",message="cache requires grpc"
type ExtAuth struct {
	// External Authorization server to reach.
	//
	// Supported types: `Service` and `Backend`.
	// +optional
	BackendRef *gwv1.BackendObjectReference `json:"backendRef,omitempty"`

	// Behavior when the external authorization service is
	// unavailable or returns an error. "FailOpen" allows the request to continue.
	// "FailClosed" (default) denies the request.
	// +optional
	FailureMode FailureMode `json:"failureMode,omitempty"`

	// Uses the gRPC External Authorization
	// [protocol](https://www.envoyproxy.io/docs/envoy/latest/api-v3/service/auth/v3/external_auth.proto) should be used.
	// +optional
	GRPC *AgentExtAuthGRPC `json:"grpc,omitempty"`

	// Uses HTTP to connect to
	// the authorization server. The authorization server must return a `200`
	// status code, otherwise the request is considered an authorization
	// failure.
	// +optional
	HTTP *AgentExtAuthHTTP `json:"http,omitempty"`

	// Whether to include the HTTP body in the authorization request.
	// If enabled, the request body will be buffered.
	// +optional
	ForwardBody *ExtAuthBody `json:"forwardBody,omitempty"`

	// Caches gRPC authorization results.
	//
	// WARNING: the safety of this feature depends on the cache key accurately
	// capturing every request property that the authorization service uses to
	// make a decision. For example, if the service returns different results
	// based on both path and authorization header, both must be included in
	// `key`; otherwise, one request may incorrectly reuse another request's
	// authorization result.
	//
	// If any key expression fails to evaluate or produces an unsupported value,
	// the request is still sent to the authorization service, but its result is
	// not read from or written to the cache.
	//
	// +optional
	Cache *ExtAuthCache `json:"cache,omitempty"`
}

type ExtAuthCache struct {
	// Ordered list of CEL expressions evaluated against the request
	// to construct the cache key.
	//
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +required
	Key []CELExpression `json:"key"`

	// Duration string, such as `5m`, or a CEL expression that
	// returns the duration that cached authorization results may be reused, or a
	// timestamp when the cached authorization result expires. The expression is
	// evaluated after the authorization response has been applied to the request.
	//
	// +required
	TTL CELExpression `json:"ttl"`

	// Maximum number of authorization results to keep in
	// the cache. If unset, this defaults to 10000.
	//
	// +kubebuilder:validation:Minimum=1
	// +optional
	MaxEntries *uint32 `json:"maxEntries,omitempty"`
}

type AgentExtAuthHTTP struct {
	// Path to send to the authorization server. If
	// unset, this defaults to the original request path.
	// This is a CEL expression, which allows customizing the path based on the
	// incoming request. For example, to add a prefix, use
	// `"/prefix/" + request.path`.
	// +optional
	Path *CELExpression `json:"path,omitempty"`

	// Optional expression that determines a path to
	// redirect to on authorization failure. This is useful to redirect to a
	// sign-in page.
	// +optional
	Redirect *CELExpression `json:"redirect,omitempty"`

	// Additional headers from the client request that
	// will be sent to the authorization server.
	//
	// If unset, the following headers are sent by default: `Authorization`.
	//
	// +optional
	// +kubebuilder:validation:MaxItems=64
	AllowedRequestHeaders []ShortString `json:"allowedRequestHeaders,omitempty"`

	// Additional headers to add to the
	// request to the authorization server. While `allowedRequestHeaders` just
	// passes the original headers through, `addRequestHeaders` allows defining
	// custom headers based on CEL expressions.
	//
	// +optional
	// +kubebuilder:validation:MaxProperties=64
	AddRequestHeaders map[string]CELExpression `json:"addRequestHeaders,omitempty"`

	// Headers from the authorization response that
	// will be copied into the request to the backend.
	//
	// +optional
	// +kubebuilder:validation:MaxItems=64
	AllowedResponseHeaders []ShortString `json:"allowedResponseHeaders,omitempty"`

	// Metadata fields to construct
	// from the authorization response. These will be included under the
	// `extauthz` variable in future CEL expressions. Setting this is useful
	// for things like logging usernames, without needing to include them as
	// headers to the backend, as `allowedResponseHeaders` would.
	//
	// +optional
	// +kubebuilder:validation:MaxProperties=64
	ResponseMetadata map[string]CELExpression `json:"responseMetadata,omitempty"`
}

type AgentExtAuthGRPC struct {
	// Additional arbitrary key-value pairs to
	// send to the authorization server in the `context_extensions` field.
	//
	// +optional
	// +kubebuilder:validation:MaxProperties=64
	ContextExtensions map[string]string `json:"contextExtensions,omitempty"`
	// Metadata to send to the authorization
	// server. This maps to the `metadata_context.filter_metadata` field of the
	// request, and allows dynamic CEL expressions. If unset, by default the
	// `envoy.filters.http.jwt_authn` key is set if the JWT policy is used as
	// well, for compatibility.
	//
	// +optional
	// +kubebuilder:validation:MaxProperties=64
	RequestMetadata map[string]CELExpression `json:"requestMetadata,omitempty"`
}

type ExtAuthBody struct {
	// Largest body, in bytes, that will be buffered
	// and sent to the authorization server. If the body size is larger than
	// `maxSize`, then the request will be rejected with a response.
	//
	// +required
	MaxSize ByteSize `json:"maxSize"`
}

// +kubebuilder:validation:AtLeastOneFieldSet
type RateLimits struct {
	// Local rate limiting policy.
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +optional
	Local []LocalRateLimit `json:"local,omitempty"`

	// Global rate limiting policy using an external service.
	// +optional
	Global *GlobalRateLimit `json:"global,omitempty"`
}

type RateLimitsConditional struct {
	// CEL expression that must evaluate to true for this policy to execute.
	// +optional
	Condition CELExpression `json:"condition,omitempty"`
	// Policy to apply when the condition matches.
	// +required
	Policy RateLimits `json:"policy"`
}

// +kubebuilder:validation:ConditionalPolicy
// +kubebuilder:validation:AtLeastOneFieldSet
type RateLimitsOrConditional struct {
	// Local rate limiting policy.
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +optional
	Local []LocalRateLimit `json:"local,omitempty"`

	// Global rate limiting policy using an external service.
	// +optional
	Global *GlobalRateLimit `json:"global,omitempty"`

	// Conditional policy execution. Set this or the top-level rateLimit fields.
	// The first matching policy will be executed.
	// A single policy may be provided without a condition set; if so, it must be the last policy and will be the fallback
	// in case no conditions are met.
	// +optional
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +kubebuilder:validation:XValidation:message="conditional entries without condition must be last",rule="self.filter(e, !has(e.condition)).size() <= 1 && (!self.exists(e, !has(e.condition)) || !has(self[size(self) - 1].condition))"
	Conditional []RateLimitsConditional `json:"conditional,omitempty"`
}

func (r *RateLimitsOrConditional) ConditionalPolicy() (*RateLimits, iter.Seq[ConditionalPolicyEntry[RateLimits]]) {
	seq := mapseq(r.Conditional, func(r RateLimitsConditional) ConditionalPolicyEntry[RateLimits] {
		return ConditionalPolicyEntry[RateLimits]{
			Condition: r.Condition,
			Policy:    r.Policy,
		}
	})
	if len(r.Conditional) > 0 {
		return nil, seq
	}
	return &RateLimits{Local: r.Local, Global: r.Global}, seq
}

type GlobalRateLimit struct {
	// Rate limit server to reach.
	// Supported types: `Service` and `Backend`.
	// +required
	BackendRef gwv1.BackendObjectReference `json:"backendRef"`

	// Behavior when the remote rate limit service is
	// unavailable or returns an error. `FailOpen` allows the request to continue.
	// `FailClosed` (default) denies the request.
	// +optional
	FailureMode FailureMode `json:"failureMode,omitempty"`

	// Domain under which this limit should apply.
	// This is an arbitrary string that enables a rate limit server to distinguish between different applications.
	// +required
	Domain ShortString `json:"domain"`

	// Dimensions for rate limiting. These values are
	// passed to the rate limit service which applies configured limits based
	// on them. Each descriptor represents a single rate limit rule with one or
	// more entries.
	//
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +required
	Descriptors []RateLimitDescriptor `json:"descriptors"`
}

// +k8s:enum
type RateLimitUnit string

const (
	RateLimitUnitTokens   RateLimitUnit = "Tokens"
	RateLimitUnitRequests RateLimitUnit = "Requests"
)

type RateLimitDescriptor struct {
	// Individual components that make up this descriptor.
	//
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +required
	Entries []RateLimitDescriptorEntry `json:"entries"`
	// Cost unit. If unspecified,
	// `Requests` is used.
	// +optional
	Unit *RateLimitUnit `json:"unit,omitempty"`
	// Common Expression Language (`CEL`) expression that determines
	// the cost of the request for this descriptor. If unset, `Requests` costs
	// default to 1, and `Tokens` costs default to the total token count.
	//
	// `Tokens` cost are evaluated after the request has completed. For non-streaming requests, `request`, `llm`, and
	// `response` fields are all available; for streaming requests, `response` is not available (however, all LLM
	// attributes are in `llm`). For `Requests`, cost is computed during the request phase.
	//
	// See https://agentgateway.dev/docs/standalone/latest/reference/cel/ for more info.
	// +optional
	Cost *CELExpression `json:"cost,omitempty"`
}

// Entry in a rate limit descriptor.
type RateLimitDescriptorEntry struct {
	// Name of the descriptor.
	// +required
	Name TinyString `json:"name"`
	// Common Expression Language (`CEL`) expression that
	// defines the value for the descriptor.
	//
	// For example, to rate limit based on the Client IP: `source.address`.
	//
	// See https://agentgateway.dev/docs/standalone/latest/reference/cel/ for more info.
	// +required
	Expression CELExpression `json:"expression"`
}

// +k8s:enum
type LocalRateLimitUnit string

const (
	LocalRateLimitUnitSeconds LocalRateLimitUnit = "Seconds"
	LocalRateLimitUnitMinutes LocalRateLimitUnit = "Minutes"
	LocalRateLimitUnitHours   LocalRateLimitUnit = "Hours"
)

// Local rate limiting policy. Local rate limits are handled on a per-proxy basis, without coordination
// between instances of the proxy.
// +kubebuilder:validation:ExactlyOneOf=requests;tokens
type LocalRateLimit struct {
	// Number of HTTP requests per unit of time that
	// are allowed. Requests exceeding this limit will fail with a `429`
	// error.
	// +kubebuilder:validation:Minimum=1
	// +optional
	Requests *int32 `json:"requests,omitempty"`

	// Number of LLM tokens per unit of time that are
	// allowed. Requests exceeding this limit will fail with a `429` error.
	//
	// Both input and output tokens are counted. However, token counts are not known until the request completes. As a
	// result, token-based rate limits will apply to future requests only.
	//
	// +kubebuilder:validation:Minimum=1
	// +optional
	Tokens *int32 `json:"tokens,omitempty"`

	// Unit of time for the limit.
	//
	// +required
	Unit LocalRateLimitUnit `json:"unit"`

	// Allowance of requests above the request-per-unit
	// that should be allowed within a short period of time.
	// +optional
	Burst *int32 `json:"burst,omitempty"`
}

type CORS struct {
	// +kubebuilder:pruning:PreserveUnknownFields
	*gwv1.HTTPCORSFilter `json:",inline"`
}

type CSRF struct {
	// Additional source origins that will be
	// allowed in addition to the destination origin. The `Origin` consists of
	// a scheme and a host, with an optional port, and takes the form
	// `<scheme>://<host>(:<port>)`.
	//
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +optional
	AdditionalOrigins []ShortString `json:"additionalOrigins,omitempty"`
}

type HostnameRewrite struct {
	// Hostname rewrite mode.
	//
	// The following may be specified:
	// * `Auto`: automatically set the `Host` header based on the destination.
	// * `None`: do not rewrite the `Host` header. The original `Host` header
	//   will be passed through.
	//
	// This setting defaults to `Auto` when connecting to hostname-based
	// `Backend` types, and `None` otherwise, for `Service` or IP-based
	// backends.
	// +required
	Mode HostnameRewriteMode `json:"mode"`
}

type Timeouts struct {
	// Timeout for an individual request from the gateway to a backend. This covers the time from when
	// the request first starts being sent from the gateway to when the full response has been received from the backend.
	//
	// +kubebuilder:validation:XValidation:rule="matches(self, '^([0-9]{1,5}(h|m|s|ms)){1,4}$')",message="invalid duration value"
	// +kubebuilder:validation:XValidation:rule="duration(self) >= duration('100ms')",message="request must be at least 1ms"
	// +optional
	Request *metav1.Duration `json:"request,omitempty"`
}

// Retry policy.
type Retry struct {
	*gwv1.HTTPRouteRetry `json:",inline"`

	// `precondition` is a CEL expression evaluated against the request before any
	// attempt is made. When it evaluates to `false`, retries are disabled and only
	// the initial attempt is made, for example `request.method == "GET"`.
	// Retrying requires buffering the request body in memory for replay, so this lets
	// us skip that cost when the request is known to be non-retriable (for example
	// streaming uploads or long-lived connections like websockets).
	// +optional
	Precondition *CELExpression `json:"precondition,omitempty"`

	// `condition` is a CEL expression evaluated against each response to decide
	// whether to retry. A response is retried when its status code is in `codes` or
	// this expression evaluates to `true`.
	// +optional
	Condition *CELExpression `json:"condition,omitempty"`
}

// Per-request access log settings.
type AccessLog struct {
	// CEL expression used to filter logs. A log
	// will only be emitted if the expression evaluates to `true`.
	// +optional
	Filter *CELExpression `json:"filter,omitempty"`
	// Customizations to the key-value pairs that are
	// logged.
	// +optional
	Attributes *LogTracingAttributes `json:"attributes,omitempty"`

	// OTLP access log export to an
	// OpenTelemetry-compatible backend.
	// +optional
	Otlp *OtlpAccessLog `json:"otlp,omitempty"`
}

// Ships access logs to an
// OpenTelemetry-compatible backend via OTLP.
// +kubebuilder:validation:XValidation:rule="!has(self.path) || !has(self.protocol) || self.protocol == 'HTTP'",message="path is only valid with protocol HTTP"
// +kubebuilder:validation:XValidation:rule="!has(self.path) || self.path.startsWith('/')",message="path must start with /"
type OtlpAccessLog struct {
	// OTLP server to send access logs to.
	// Supported types: `Service` and `AgentgatewayBackend`.
	// +required
	BackendRef gwv1.BackendObjectReference `json:"backendRef"`

	// OTLP protocol variant to use.
	// +kubebuilder:default=GRPC
	// +optional
	Protocol OTLPProtocol `json:"protocol,omitempty"`

	// OTLP/HTTP path to use. This is only applicable
	// when `protocol` is `HTTP`. If unset, this defaults to `/v1/logs`.
	// +optional
	Path *LongString `json:"path,omitempty"`
}

// +kubebuilder:validation:AtLeastOneFieldSet
type LogTracingAttributes struct {
	// Default fields to remove. For example,
	// `http.method`.
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=32
	// +optional
	Remove []TinyString `json:"remove,omitempty"`
	// Additional key-value pairs to add to each entry.
	// The value is a CEL expression. If the CEL expression fails to evaluate,
	// the pair will be excluded.
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:maxItems=64
	// +optional
	Add []AttributeAdd `json:"add,omitempty"`
}

type AttributeAdd struct {
	// +required
	Name ShortString `json:"name"`
	// +required
	Expression CELExpression `json:"expression"`
}

// Custom labels to add to Prometheus metrics.
type MetricLabels struct {
	// Customizations to the labels that are
	// added to Prometheus metrics.
	// +required
	Attributes MetricAttributes `json:"attributes"`
}

// +kubebuilder:validation:AtLeastOneFieldSet
type MetricAttributes struct {
	// Additional key-value pairs to add as custom labels
	// to all Prometheus metrics. The value is a CEL expression evaluated
	// per-request. If the CEL expression fails to evaluate, the label value
	// is set to "unknown".
	//
	// WARNING: High-cardinality labels (e.g., per-user IDs) can significantly
	// increase Prometheus storage and memory usage. Prefer low-cardinality
	// dimensions like team or environment.
	//
	// +listType=map
	// +listMapKey=name
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +optional
	Add []AttributeAdd `json:"add,omitempty"`
}

// +k8s:enum
type OTLPProtocol string

const (
	OTLPProtocolHttp OTLPProtocol = "HTTP"
	OTLPProtocolGrpc OTLPProtocol = "GRPC"
)

// +kubebuilder:validation:XValidation:rule="!has(self.path) || !has(self.protocol) || self.protocol == 'HTTP'",message="path is only valid with protocol HTTP"
// +kubebuilder:validation:XValidation:rule="!has(self.path) || self.path.startsWith('/')",message="path must start with /"
type Tracing struct {
	// OTLP server to reach.
	// Supported types: `Service` and `AgentgatewayBackend`.
	// +required
	BackendRef gwv1.BackendObjectReference `json:"backendRef"`
	// OTLP protocol variant to use.
	// +kubebuilder:default=GRPC
	// +optional
	Protocol OTLPProtocol `json:"protocol,omitempty"`

	// OTLP path to use. This is only applicable when
	// `protocol` is `HTTP`. If unset, this defaults to `/v1/traces`.
	// +optional
	Path *LongString `json:"path,omitempty"`

	// Customizations to the key-value pairs that are
	// included in the trace.
	// +optional
	Attributes *LogTracingAttributes `json:"attributes,omitempty"`

	// Entity producing telemetry and resources
	// resources to be included in the trace.
	// +optional
	Resources []ResourceAdd `json:"resources,omitempty"`

	// Expression that determines the amount of random
	// sampling. Random sampling will initiate a new trace span if the incoming
	// request does not have a trace initiated already. This should evaluate to
	// a float between `0.0` and `1.0`, or a boolean (`true` or `false`). If
	// unspecified, random sampling is disabled.
	// +optional
	RandomSampling *CELExpression `json:"randomSampling,omitempty"`
	// Expression that determines the amount of client
	// sampling. Client sampling determines whether to initiate a new trace
	// span if the incoming request does have a trace already. This should
	// evaluate to a float between `0.0` and `1.0`, or a boolean (`true` or
	// `false`). If unspecified, client sampling is `100%` enabled.
	// +optional
	ClientSampling *CELExpression `json:"clientSampling,omitempty"`
}

type ResourceAdd struct {
	// +required
	Name ShortString `json:"name"`
	// +required
	Expression CELExpression `json:"expression"`
}
