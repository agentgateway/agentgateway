package agentgateway

import (
	corev1 "k8s.io/api/core/v1"
	apiextensionsv1 "k8s.io/apiextensions-apiserver/pkg/apis/apiextensions/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/shared"
)

// +kubebuilder:rbac:groups=agentgateway.dev,resources=agentgatewaypolicies,verbs=get;list;watch
// +kubebuilder:rbac:groups=agentgateway.dev,resources=agentgatewaypolicies/status,verbs=get;update;patch

// +kubebuilder:printcolumn:name="Accepted",type=string,JSONPath=".status.ancestors[*].conditions[?(@.type=='Accepted')].status",description="Agentgateway policy acceptance status"
// +kubebuilder:printcolumn:name="Attached",type=string,JSONPath=".status.ancestors[*].conditions[?(@.type=='Attached')].status",description="Agentgateway policy attachment status"
// +kubebuilder:printcolumn:name="Age",type=date,JSONPath=`.metadata.creationTimestamp`

// +genclient
// +kubebuilder:object:root=true
// +kubebuilder:metadata:labels={app=kgateway,app.kubernetes.io/name=kgateway}
// +kubebuilder:resource:categories=kgateway,shortName=agpol
// +kubebuilder:subresource:status
// +kubebuilder:metadata:labels="gateway.networking.k8s.io/policy=Direct"
type AgentgatewayPolicy struct {
	metav1.TypeMeta `json:",inline"`
	// metadata for the object
	// More info: https://git.k8s.io/community/contributors/devel/sig-architecture/api-conventions.md#metadata
	// +optional
	metav1.ObjectMeta `json:"metadata,omitempty"`

	// spec defines the desired state of AgentgatewayPolicy.
	// +required
	Spec AgentgatewayPolicySpec `json:"spec"`

	// status defines the current state of AgentgatewayPolicy.
	// +optional
	Status gwv1.PolicyStatus `json:"status,omitempty"`
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
// +kubebuilder:validation:XValidation:rule="!has(self.backend) || !has(self.backend.mcp) || ((!has(self.targetRefs) || !self.targetRefs.exists(t, t.kind == 'Service')) && (!has(self.targetSelectors) || !self.targetSelectors.exists(t, t.kind == 'Service')))",message="backend.mcp may not be used with a Service target"
// +kubebuilder:validation:XValidation:rule="!has(self.backend) || !has(self.backend.ai) || ((!has(self.targetRefs) || !self.targetRefs.exists(t, t.kind == 'Service')) && (!has(self.targetSelectors) || !self.targetSelectors.exists(t, t.kind == 'Service')))",message="backend.ai may not be used with a Service target"
// +kubebuilder:validation:XValidation:rule="has(self.frontend) && has(self.targetRefs) ? self.targetRefs.all(t, t.kind == 'Gateway' && !has(t.sectionName)) : true",message="the 'frontend' field can only target a Gateway"
// +kubebuilder:validation:XValidation:rule="has(self.frontend) && has(self.targetSelectors) ? self.targetSelectors.all(t, t.kind == 'Gateway' && !has(t.sectionName)) : true",message="the 'frontend' field can only target a Gateway"
// +kubebuilder:validation:XValidation:rule="has(self.traffic) && has(self.targetRefs) ? self.targetRefs.all(t, t.kind in ['Gateway', 'HTTPRoute', 'GRPCRoute', 'XListenerSet']) : true",message="the 'traffic' field can only target a Gateway, XListenerSet, GRPCRoute, or HTTPRoute"
// +kubebuilder:validation:XValidation:rule="has(self.traffic) && has(self.targetSelectors) ? self.targetSelectors.all(t, t.kind in ['Gateway', 'HTTPRoute', 'GRPCRoute', 'XListenerSet']) : true",message="the 'traffic' field can only target a Gateway, XListenerSet, GRPCRoute, or HTTPRoute"
// +kubebuilder:validation:XValidation:rule="has(self.targetRefs) && has(self.traffic) && has(self.traffic.phase) && self.traffic.phase == 'PreRouting' ? self.targetRefs.all(t, t.kind in ['Gateway', 'XListenerSet']) : true",message="the 'traffic.phase=PreRouting' field can only target a Gateway or XListenerSet"
// +kubebuilder:validation:XValidation:rule="has(self.targetSelectors) && has(self.traffic) && has(self.traffic.phase) && self.traffic.phase == 'PreRouting' ? self.targetSelectors.all(t, t.kind in ['Gateway', 'XListenerSet']) : true",message="the 'traffic.phase=PreRouting' field can only target a Gateway or XListenerSet"
type AgentgatewayPolicySpec struct {
	// targetRefs specifies the target resources by reference to attach the policy to.
	//
	// +listType=atomic
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +kubebuilder:validation:XValidation:rule="self.all(r, (r.kind == 'Service' && r.group == '') || (r.kind == 'AgentgatewayBackend' && r.group == 'agentgateway.dev') || (r.kind in ['Gateway', 'HTTPRoute', 'GRPCRoute'] && r.group == 'gateway.networking.k8s.io') || (r.kind == 'XListenerSet' && r.group == 'gateway.networking.x-k8s.io'))",message="targetRefs may only reference Gateway, HTTPRoute, GRPCRoute, XListenerSet, Service, or AgentgatewayBackend resources"
	// +kubebuilder:validation:XValidation:message="Only one Kind of targetRef can be set on one policy",rule="self.all(l1, !self.exists(l2, l1.kind != l2.kind))"
	// +optional
	TargetRefs []shared.LocalPolicyTargetReferenceWithSectionName `json:"targetRefs,omitempty"`

	// targetSelectors specifies the target selectors to select resources to attach the policy to.
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +kubebuilder:validation:XValidation:rule="self.all(r, (r.kind == 'Service' && r.group == '') || (r.kind == 'AgentgatewayBackend' && r.group == 'agentgateway.dev') || (r.kind in ['Gateway', 'HTTPRoute', 'GRPCRoute'] && r.group == 'gateway.networking.k8s.io') || (r.kind == 'XListenerSet' && r.group == 'gateway.networking.x-k8s.io'))",message="targetRefs may only reference Gateway, HTTPRoute, GRPCRoute, XListenerSet, Service, or AgentgatewayBackend resources"
	// +kubebuilder:validation:XValidation:message="Only one Kind of targetRef can be set on one policy",rule="self.all(l1, !self.exists(l2, l1.kind != l2.kind))"
	// +optional
	TargetSelectors []shared.LocalPolicyTargetSelectorWithSectionName `json:"targetSelectors,omitempty"`

	// frontend defines settings for how to handle incoming traffic.
	//
	// A frontend policy can only target a Gateway. Listener and ListenerSet are not valid targets.
	//
	// When multiple policies are selected for a given request, they are merged on a field-level basis, but not a deep
	// merge. For example, policy A sets 'tcp' and 'tls', and policy B sets 'tls', the effective policy would be 'tcp' from
	// policy A, and 'tls' from policy B.
	// +optional
	Frontend *Frontend `json:"frontend,omitempty"`

	// traffic defines settings for how process traffic.
	//
	// A traffic policy can target a Gateway (optionally, with a sectionName indicating the listener), ListenerSet, Route
	// (optionally, with a sectionName indicating the route rule).
	//
	// When multiple policies are selected for a given request, they are merged on a field-level basis, but not a deep
	// merge. Precedence is given to more precise policies: Gateway < Listener < Route < Route Rule. For example, policy A
	// sets 'timeouts' and 'retries', and policy B sets 'retries', the effective policy would be 'timeouts' from policy A,
	// and 'retries' from policy B.
	// +optional
	Traffic *Traffic `json:"traffic,omitempty"`

	// backend defines settings for how to connect to destination backends.
	//
	// A backend policy can target a Gateway (optionally, with a sectionName indicating the listener), ListenerSet, Route
	// (optionally, with a sectionName indicating the route rule), or a Service/Backend (optionally, with a sectionName
	// indicating the port (for Service) or sub-backend (for Backend).
	//
	// Note that a backend policy applies when connecting to a specific destination backend. Targeting a higher level
	// resource, like Gateway, is just a way to easily apply a policy to a group of backends.
	//
	// When multiple policies are selected for a given request, they are merged on a field-level basis, but not a deep
	// merge. Precedence is given to more precise policies: Gateway < Listener < Route < Route Rule < Backend/Service. For
	// example, if a Gateway policy sets 'tcp' and 'tls', and a Backend policy sets 'tls', the effective policy would be
	// 'tcp' from the Gateway, and 'tls' from the Backend.
	// +optional
	Backend *BackendFull `json:"backend,omitempty"`
}

type BackendSimple struct {
	// tcp defines settings for managing TCP connections to the backend.
	// +optional
	TCP *BackendTCP `json:"tcp,omitempty"`
	// tls defines settings for managing TLS connections to the backend.
	//
	// If this field is set, TLS will be initiated to the backend; the system trusted CA certificates will be used to
	// validate the server, and the SNI will automatically be set based on the destination.
	// +optional
	TLS *BackendTLS `json:"tls,omitempty"`
	// http defines settings for managing HTTP requests to the backend.
	// +optional
	HTTP *BackendHTTP `json:"http,omitempty"`

	// auth defines settings for managing authentication to the backend
	// +optional
	Auth *BackendAuth `json:"auth,omitempty"`
}

// +kubebuilder:validation:AtLeastOneFieldSet
type BackendWithMCP struct {
	BackendSimple `json:",inline"`

	// mcp specifies settings for MCP workloads. This is only applicable when connecting to a Backend of type 'mcp'.
	// +optional
	MCP *BackendMCP `json:"mcp,omitempty"`
}

// +kubebuilder:validation:AtLeastOneFieldSet
type BackendWithAI struct {
	BackendSimple `json:",inline"`

	// ai specifies settings for AI workloads. This is only applicable when connecting to a Backend of type 'ai'.
	// +optional
	AI *BackendAI `json:"ai,omitempty"`
}

// +kubebuilder:validation:AtLeastOneFieldSet
type BackendFull struct {
	BackendSimple `json:",inline"`

	// ai specifies settings for AI workloads. This is only applicable when connecting to a Backend of type 'ai'.
	// +optional
	AI *BackendAI `json:"ai,omitempty"`

	// mcp specifies settings for MCP workloads. This is only applicable when connecting to a Backend of type 'mcp'.
	// +optional
	MCP *BackendMCP `json:"mcp,omitempty"`
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
	// mtlsCertificateRef enables mutual TLS to the backend, using the specified key (tls.key) and cert (tls.crt) from the
	// refenced Secret.
	//
	// An optional 'ca.cert' field, if present, will be used to verify the server certificate if present. If
	// caCertificateRefs is also specified, the caCertificateRefs field takes priority.
	//
	// If unspecified, no client certificate will be used.
	//
	// +listType=atomic
	// +kubebuilder:validation:MaxItems=1
	// +optional
	MtlsCertificateRef []corev1.LocalObjectReference `json:"mtlsCertificateRef,omitempty"`
	// caCertificateRefs defines the CA certificate ConfigMap to use to verify the server certificate.
	// If unset, the system's trusted certificates are used.
	//
	// +listType=atomic
	// +kubebuilder:validation:MaxItems=1
	// +optional
	CACertificateRefs []corev1.LocalObjectReference `json:"caCertificateRefs,omitempty"`

	// insecureSkipVerify originates TLS but skips verification of the backend's certificate.
	// WARNING: This is an insecure option that should only be used if the risks are understood.
	//
	// There are two modes:
	// * All disables all TLS verification
	// * Hostname verifies the CA certificate is trusted, but ignores any mismatch of hostname/SANs. Note that this method
	//  is still insecure; prefer setting verifySubjectAltNames to customize the valid hostnames if possible.
	//
	// +kubebuilder:validation:Enum=All;Hostname
	// +optional
	InsecureSkipVerify *InsecureTLSMode `json:"insecureSkipVerify,omitempty"`

	// sni specifies the Server Name Indicator (SNI) to be used in the TLS handshake. If unset, the SNI is automatically
	// set based on the destination hostname.
	// +optional
	Sni *SNI `json:"sni,omitempty"`

	// verifySubjectAltNames specifies the Subject Alternative Names (SAN) to verify in the server certificate.
	// If not present, the destination hostname is automatically used.
	//
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +optional
	VerifySubjectAltNames []ShortString `json:"verifySubjectAltNames,omitempty"`

	// alpnProtocols sets the Application Level Protocol Negotiation (ALPN) value to use in the TLS handshake.
	//
	// If not present, defaults to ["h2", "http/1.1"].
	//
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +optional
	AlpnProtocols *[]TinyString `json:"alpnProtocols,omitempty"`
}

// +kubebuilder:validation:AtLeastOneFieldSet
type Frontend struct {
	// tcp defines settings on managing incoming TCP connections.
	// +optional
	TCP *FrontendTCP `json:"tcp,omitempty"`
	// tls defines settings on managing incoming TLS connections.
	// +optional
	TLS *FrontendTLS `json:"tls,omitempty"`
	// http defines settings on managing incoming HTTP requests.
	// +optional
	HTTP *FrontendHTTP `json:"http,omitempty"`

	// AccessLoggingConfig contains access logging configuration
	// +optional
	AccessLog *AccessLog `json:"accessLog,omitempty"`

	// Tracing contains various settings for OpenTelemetry tracer.
	// +optional
	Tracing *Tracing `json:"tracing,omitempty"`
}

// +kubebuilder:validation:AtLeastOneFieldSet
type FrontendHTTP struct {
	// maxBufferSize defines the maximum size HTTP body that will be buffered into memory.
	// Bodies will only be buffered for policies which require buffering.
	// If unset, this defaults to 2mb.
	// +kubebuilder:validation:Minimum=1
	// +optional
	MaxBufferSize *int32 `json:"maxBufferSize,omitempty"`

	// http1MaxHeaders defines the maximum number of headers that are allowed in HTTP/1.1 requests.
	// If unset, this defaults to 100.
	// +kubebuilder:validation:Minimum=1
	// +kubebuilder:validation:Maximum=4096
	// +optional
	HTTP1MaxHeaders *int32 `json:"http1MaxHeaders,omitempty"`
	// http1IdleTimeout defines the timeout before an unused connection is closed.
	// If unset, this defaults to 10 minutes.
	// +kubebuilder:validation:XValidation:rule="matches(self, '^([0-9]{1,5}(h|m|s|ms)){1,4}$')",message="invalid duration value"
	// +kubebuilder:validation:XValidation:rule="duration(self) >= duration('1s')",message="http1IdleTimeout must be at least 1 second"
	// +optional
	HTTP1IdleTimeout *metav1.Duration `json:"http1IdleTimeout,omitempty"`

	// http2WindowSize indicates the initial window size for stream-level flow control for received data.
	// +kubebuilder:validation:Minimum=1
	// +optional
	HTTP2WindowSize *int32 `json:"http2WindowSize,omitempty"`
	// http2ConnectionWindowSize indicates the initial window size for connection-level flow control for received data.
	// +kubebuilder:validation:Minimum=1
	// +optional
	HTTP2ConnectionWindowSize *int32 `json:"http2ConnectionWindowSize,omitempty"`
	// http2FrameSize sets the maximum frame size to use.
	// If unset, this defaults to 16kb
	// +kubebuilder:validation:Minimum=16384
	// +kubebuilder:validation:Maximum=1677215
	// +optional
	HTTP2FrameSize *int32 `json:"http2FrameSize,omitempty"`
	// +kubebuilder:validation:XValidation:rule="matches(self, '^([0-9]{1,5}(h|m|s|ms)){1,4}$')",message="invalid duration value"
	// +kubebuilder:validation:XValidation:rule="duration(self) >= duration('1s')",message="http2KeepaliveInterval must be at least 1 second"
	// +optional
	HTTP2KeepaliveInterval *metav1.Duration `json:"http2KeepaliveInterval,omitempty"`
	// +kubebuilder:validation:XValidation:rule="matches(self, '^([0-9]{1,5}(h|m|s|ms)){1,4}$')",message="invalid duration value"
	// +kubebuilder:validation:XValidation:rule="duration(self) >= duration('1s')",message="http2KeepaliveTimeout must be at least 1 second"
	// +optional
	HTTP2KeepaliveTimeout *metav1.Duration `json:"http2KeepaliveTimeout,omitempty"`
}

// +kubebuilder:validation:AtLeastOneFieldSet
type FrontendTLS struct {
	// handshakeTimeout specifies the deadline for a TLS handshake to complete.
	// If unset, this defaults to 15s.
	// +kubebuilder:validation:XValidation:rule="matches(self, '^([0-9]{1,5}(h|m|s|ms)){1,4}$')",message="invalid duration value"
	// +kubebuilder:validation:XValidation:rule="duration(self) >= duration('100ms')",message="handshakeTimeout must be at least 100ms"
	// +optional
	HandshakeTimeout *metav1.Duration `json:"handshakeTimeout,omitempty"`

	// alpnProtocols sets the Application Level Protocol Negotiation (ALPN) value to use in the TLS handshake.
	//
	// If not present, defaults to ["h2", "http/1.1"].
	//
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +optional
	AlpnProtocols *[]TinyString `json:"alpnProtocols,omitempty"`

	// MinTLSVersion configures the minimum TLS version to support.
	// +optional
	MinTLSVersion *TLSVersion `json:"minProtocolVersion,omitempty"`

	// MaxTLSVersion configures the maximum TLS version to support.
	// +optional
	MaxTLSVersion *TLSVersion `json:"maxProtocolVersion,omitempty"`

	// CipherSuites configures the list of cipher suites for a TLS listener.
	// The value is a comma-separated list of cipher suites, e.g "TLS13_AES_256_GCM_SHA384,TLS13_AES_128_GCM_SHA256".
	// Use in the TLS options field of a TLS listener.
	// +optional
	CipherSuites []CipherSuite `json:"cipherSuites,omitempty"`

	// TODO: mirror the tuneables on BackendTLS
}

// +kubebuilder:validation:Enum="1.2";"1.3"
type TLSVersion string

const (
	// agentgateway currently only supports TLS 1.2 and TLS 1.3
	TLSVersion1_2 TLSVersion = "1.2"
	TLSVersion1_3 TLSVersion = "1.3"
)

// +kubebuilder:validation:Enum=TLS13_AES_256_GCM_SHA384;TLS13_AES_128_GCM_SHA256;TLS13_CHACHA20_POLY1305_SHA256;TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384;TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256;TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256;TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384;TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256;TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256
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

// +kubebuilder:validation:AtLeastOneFieldSet
type FrontendTCP struct {
	// keepalive defines settings for enabling TCP keepalives on the connection.
	// +optional
	KeepAlive *Keepalive `json:"keepalive,omitempty"`
}

// TCP Keepalive settings
type Keepalive struct {
	// retries specifies the maximum number of keep-alive probes to send before dropping the connection.
	// If unset, this defaults to 9.
	// +kubebuilder:validation:Minimum=1
	// +kubebuilder:validation:Maximum=64
	// +optional
	Retries *int32 `json:"retries,omitempty"`

	// time specifies the number of seconds a connection needs to be idle before keep-alive probes start being sent.
	// If unset, this defaults to 180s.
	// +kubebuilder:validation:XValidation:rule="matches(self, '^([0-9]{1,5}(h|m|s|ms)){1,4}$')",message="invalid duration value"
	// +kubebuilder:validation:XValidation:rule="duration(self) >= duration('1s')",message="time must be at least 1 second"
	// +optional
	Time *metav1.Duration `json:"time,omitempty"`

	// interval specifies the number of seconds between keep-alive probes.
	// If unset, this defaults to 180s.
	// +kubebuilder:validation:XValidation:rule="matches(self, '^([0-9]{1,5}(h|m|s|ms)){1,4}$')",message="invalid duration value"
	// +kubebuilder:validation:XValidation:rule="duration(self) >= duration('1s')",message="interval must be at least 1 second"
	// +optional
	Interval *metav1.Duration `json:"interval,omitempty"`
}

// +kubebuilder:validation:Enum=PreRouting;PostRouting
type PolicyPhase string

const (
	PolicyPhasePreRouting  PolicyPhase = "PreRouting"
	PolicyPhasePostRouting PolicyPhase = "PostRouting"
)

// +kubebuilder:validation:IfThenOnlyFields:if="has(self.phase) && self.phase == 'PreRouting'",fields=phase;transformation;extProc;extAuth;jwtAuthentication;basicAuthentication;apiKeyAuthentication,message="phase PreRouting only supports extAuth, transformation, extProc, jwtAuthentication, basicAuthentication, and apiKeyAuthentication"
type Traffic struct {
	// The phase to apply the traffic policy to. If the phase is PreRouting, the targetRef must be a Gateway or a Listener.
	// PreRouting is typically used only when a policy needs to influence the routing decision.
	//
	// Even when using PostRouting mode, the policy can target the Gateway/Listener. This is a helper for applying the policy
	// to all routes under that Gateway/Listener, and follows the merging logic described above.
	//
	// Note: PreRouting and PostRouting rules do not merge together. These are independent execution phases. That is, all
	// PreRouting rules will merge and execute, then all PostRouting rules will merge and execute.
	//
	// If unset, this defaults to PostRouting.
	// +optional
	Phase *PolicyPhase `json:"phase,omitempty"` //nolint:kubeapilinter // false positive for the nophase sub-linter

	// transformation is used to mutate and transform requests and responses
	// before forwarding them to the destination.
	// +optional
	Transformation *Transformation `json:"transformation,omitempty"`

	// extProc specifies the external processing configuration for the policy.
	// +optional
	ExtProc *ExtProc `json:"extProc,omitempty"`

	// extAuth specifies the external authentication configuration for the policy.
	// This controls what external server to send requests to for authentication.
	// +optional
	ExtAuth *ExtAuth `json:"extAuth,omitempty"`

	// rateLimit specifies the rate limiting configuration for the policy.
	// This controls the rate at which requests are allowed to be processed.
	// +optional
	RateLimit *RateLimits `json:"rateLimit,omitempty"`

	// cors specifies the CORS configuration for the policy.
	// +optional
	Cors *CORS `json:"cors,omitempty"`

	// csrf specifies the Cross-Site Request Forgery (CSRF) policy for this traffic policy.
	//
	// The CSRF policy has the following behavior:
	// * Safe methods (GET, HEAD, OPTIONS) are automatically allowed
	// * Requests without Sec-Fetch-Site or Origin headers are assumed to be same-origin or non-browser requests and are allowed.
	// * Otherwise, the Sec-Fetch-Site header is checked, with a fallback to comparing the Origin header to the Host header.
	// +optional
	Csrf *CSRF `json:"csrf,omitempty"`

	// headerModifiers defines the policy to modify request and response headers.
	// +optional
	HeaderModifiers *shared.HeaderModifiers `json:"headerModifiers,omitempty"`

	// hostRewrite specifies how to rewrite the Host header for requests.
	//
	// If the HTTPRoute `urlRewrite` filter already specifies a host rewrite, this setting is ignored.
	// +kubebuilder:validation:Enum=Auto;None
	// +optional
	HostnameRewrite *HostnameRewrite `json:"hostRewrite,omitempty"`

	// timeouts defines the timeouts for requests
	// It is applicable to HTTPRoutes and ignored for other targeted kinds.
	// +optional
	Timeouts *Timeouts `json:"timeouts,omitempty"`

	// retry defines the policy for retrying requests.
	// +optional
	Retry *Retry `json:"retry,omitempty"`

	// authorization specifies the access rules based on roles and permissions.
	// If multiple authorization rules are applied across different policies (at the same, or different, attahcment points),
	// all rules are merged.
	// +optional
	Authorization *shared.Authorization `json:"authorization,omitempty"`

	// jwtAuthentication authenticates users based on JWT tokens.
	// +optional
	JWTAuthentication *JWTAuthentication `json:"jwtAuthentication,omitempty"`

	// basicAuthentication authenticates users based on the "Basic" authentication scheme (RFC 7617), where a username and password
	// are encoded in the request.
	// +optional
	BasicAuthentication *BasicAuthentication `json:"basicAuthentication,omitempty"`

	// apiKeyAuthentication authenticates users based on a configured API Key.
	// +optional
	APIKeyAuthentication *APIKeyAuthentication `json:"apiKeyAuthentication,omitempty"`

	// direct response configures the policy to send a direct response to the client.
	// +optional
	DirectResponse *DirectResponse `json:"directResponse,omitempty"`
}

// DirectResponse defines the policy to send a direct response to the client.
type DirectResponse struct {
	// StatusCode defines the HTTP status code to return for this route.
	//
	// +required
	// +kubebuilder:validation:Minimum=200
	// +kubebuilder:validation:Maximum=599
	StatusCode int32 `json:"status"`
	// Body defines the content to be returned in the HTTP response body.
	// The maximum length of the body is restricted to prevent excessively large responses.
	// If this field is omitted, no body is included in the response.
	//
	// +optional
	// +kubebuilder:validation:MinLength=1
	// +kubebuilder:validation:MaxLength=4096
	Body *string `json:"body,omitempty"`
}

// +kubebuilder:validation:Enum=Strict;Optional;Permissive
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

type JWTAuthentication struct {
	// validation mode for JWT authentication.
	// +kubebuilder:default=Strict
	// +optional
	Mode JWTAuthenticationMode `json:"mode,omitempty"`
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=64
	// +required
	Providers []JWTProvider `json:"providers"`
}

type JWTProvider struct {
	// issuer identifies the IdP that issued the JWT. This corresponds to the 'iss' claim (https://tools.ietf.org/html/rfc7519#section-4.1.1).
	// +required
	Issuer ShortString `json:"issuer"`
	// audiences specifies the list of allowed audiences that are allowed access. This corresponds to the 'aud' claim (https://datatracker.ietf.org/doc/html/rfc7519#section-4.1.3).
	// If unset, any audience is allowed.
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=64
	// +optional
	Audiences []string `json:"audiences,omitempty"`
	// jwks defines the JSON Web Key Set used to validate the signature of the JWT.
	// +required
	JWKS JWKS `json:"jwks"`
}

// +kubebuilder:validation:ExactlyOneOf=remote;inline
type JWKS struct {
	// remote specifies how to reach the JSON Web Key Set from a remote address.
	// +optional
	Remote *RemoteJWKS `json:"remote,omitempty"`
	// inline specifies an inline JSON Web Key Set used validate the signature of the JWT.
	// +kubebuilder:validation:MinLength=2
	// +kubebuilder:validation:MaxLength=65536
	// +optional
	Inline *string `json:"inline,omitempty"`
}

type RemoteJWKS struct {
	// Path to IdP jwks endpoint, relative to the root, commonly ".well-known/jwks.json".
	// +required
	// +kubebuilder:validation:MinLength=1
	// +kubebuilder:validation:MaxLength=2000
	JwksPath string `json:"jwksPath"`
	// +optional
	// +kubebuilder:validation:XValidation:rule="matches(self, '^([0-9]{1,5}(h|m|s|ms)){1,4}$')",message="invalid duration value"
	// +kubebuilder:validation:XValidation:rule="duration(self) >= duration('5m')",message="cacheDuration must be at least 5m."
	// +kubebuilder:default="5m"
	CacheDuration *metav1.Duration `json:"cacheDuration,omitempty"`
	// backendRef references the remote JWKS server to reach.
	// Supported types are Service and (static) Backend. An AgentgatewayPolicy containing backend tls config
	// can then be attached to the service/backend in order to set tls options for a connection to the remote jwks source.
	// +required
	BackendRef gwv1.BackendObjectReference `json:"backendRef"`
}

// +kubebuilder:validation:Enum=Strict;Optional
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
	// validation mode for basic auth authentication.
	// +kubebuilder:default=Strict
	// +optional
	Mode BasicAuthenticationMode `json:"mode,omitempty"`

	// realm specifies the 'realm' to return in the WWW-Authenticate header for failed authentication requests.
	// If unset, "Restricted" will be used.
	// +optional
	Realm *string `json:"realm,omitempty"`

	// users provides an inline list of username/password pairs that will be accepted.
	// Each entry represents one line of the htpasswd format: https://httpd.apache.org/docs/2.4/programs/htpasswd.html.
	//
	// Note: passwords should be the hash of the password, not the raw password. Use the `htpasswd` or similar commands
	// to generate a hash. MD5, bcrypt, crypt, and SHA-1 are supported.
	//
	// Example:
	// users:
	// - "user1:$apr1$ivPt0D4C$DmRhnewfHRSrb3DQC.WHC."
	// - "user2:$2y$05$r3J4d3VepzFkedkd/q1vI.pBYIpSqjfN0qOARV3ScUHysatnS0cL2"
	//
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=256
	// +optional
	Users []string `json:"users,omitempty"`

	// secretRef references a Kubernetes secret storing the .htaccess file. The Secret must have a key named '.htaccess',
	// and should contain the complete .htaccess file.
	//
	// Note: passwords should be the hash of the password, not the raw password. Use the `htpasswd` or similar commands
	// to generate a hash. MD5, bcrypt, crypt, and SHA-1 are supported.
	//
	// Example:
	//
	// apiVersion: v1
	// kind: Secret
	// metadata:
	//   name: basic-auth
	// stringData:
	//   .htaccess: |
	//     alice:$apr1$3zSE0Abt$IuETi4l5yO87MuOrbSE4V.
	//     bob:$apr1$Ukb5LgRD$EPY2lIfY.A54jzLELNIId/
	// +optional
	SecretRef *corev1.LocalObjectReference `json:"secretRef,omitempty"`
}

// +kubebuilder:validation:Enum=Strict;Optional
type APIKeyAuthenticationMode string

const (
	// A valid API Key must be present.
	// This is the default option.
	APIKeyAuthenticationModeStrict APIKeyAuthenticationMode = "Strict"
	// If an API Key exists, validate it.
	// Warning: this allows requests without an API Key!
	APIKeyAuthenticationModeOptional APIKeyAuthenticationMode = "Optional"
)

// +kubebuilder:validation:ExactlyOneOf=secretRef;secretSelector
type APIKeyAuthentication struct {
	// Validation mode for api key authentication.
	// +kubebuilder:default=Strict
	// +optional
	Mode APIKeyAuthenticationMode `json:"mode,omitempty"`

	// secretRef references a Kubernetes secret storing a set of API Keys. If there are many keys, 'secretSelector' can be
	// used instead.
	//
	// Each entry in the Secret represents one API Key. The key is an arbitrary identifier. The value can either be:
	// * A string, representing the API Key.
	// * A JSON object, with two fields, `key` and `metadata`. `key` contains the API Key. `metadata` contains arbitrary JSON
	//   metadata associated with the key, which may be used by other policies. For example, you may write an authorization
	//   policy allow `apiKey.group == 'sales'`.
	//
	// Example:
	//
	// apiVersion: v1
	// kind: Secret
	// metadata:
	//   name: api-key
	// stringData:
	//   client1: |
	//     {
	//       "key": "k-123",
	//       "metadata": {
	//         "group": "sales",
	//         "created_at": "2024-10-01T12:00:00Z",
	//       }
	//     }
	//   client2: "k-456"
	// +optional
	SecretRef *corev1.LocalObjectReference `json:"secretRef,omitempty"`

	// secretSelector selects multiple secrets containing API Keys. If the same key is defined in multiple secrets, the
	// behavior is undefined.
	//
	// Each entry in the Secret represents one API Key. The key is an arbitrary identifier. The value can either be:
	// * A string, representing the API Key.
	// * A JSON object, with two fields, `key` and `metadata`. `key` contains the API Key. `metadata` contains arbitrary JSON
	//   metadata associated with the key, which may be used by other policies. For example, you may write an authorization
	//   policy allow `apiKey.group == 'sales'`.
	//
	// Example:
	//
	// apiVersion: v1
	// kind: Secret
	// metadata:
	//   name: api-key
	// stringData:
	//   client1: |
	//     {
	//       "key": "k-123",
	//       "metadata": {
	//         "group": "sales",
	//         "created_at": "2024-10-01T12:00:00Z",
	//       }
	//     }
	//   client2: "k-456"
	// +optional
	SecretSelector *SecretSelector `json:"secretSelector,omitempty"`
}

type SecretSelector struct {
	// Label selector to select the target resource.
	// +required
	MatchLabels map[string]string `json:"matchLabels"`
}

type HostnameRewriteMode string

const (
	HostnameRewriteModeAuto HostnameRewriteMode = "Auto"
	HostnameRewriteModeNone HostnameRewriteMode = "None"
)

// +kubebuilder:validation:ExactlyOneOf=key;secretRef;passthrough;aws;azure;gcp
type BackendAuth struct {
	// key provides an inline key to use as the value of the Authorization header.
	// This option is the least secure; usage of a Secret is preferred.
	// +kubebuilder:validation:MaxLength=2048
	// +optional
	InlineKey *string `json:"key,omitempty"`

	// secretRef references a Kubernetes secret storing the key to use the authorization value. This must be stored in the
	// 'Authorization' key.
	// +optional
	SecretRef *corev1.LocalObjectReference `json:"secretRef,omitempty"`

	// passthrough passes through an existing token that has been sent by the client and validated. Other policies, like
	// JWT and API Key authentication, will strip the original client credentials. Passthrough backend authentication
	// causes the original token to be added back into the request. If there are no client authentication policies on the
	// request, the original token would be unchanged, so this would have no effect.
	// +optional
	Passthrough *BackendAuthPassthrough `json:"passthrough,omitempty"`
	// TODO: azure, gcp

	// Auth specifies an explicit AWS authentication method for the backend.
	// When omitted, we will try to use the default AWS SDK authentication methods.
	//
	// +optional
	AWS *AwsAuth `json:"aws,omitempty"`

	// Azure specifies an Azure authentication method for the backend.
	//
	// +optional
	Azure *AzureAuth `json:"azure,omitempty"`

	// Auth specifies to use a Google  authentication method for the backend.
	// When omitted, we will try to use the default AWS SDK authentication methods.
	//
	// +optional
	GCP *GcpAuth `json:"gcp,omitempty"`
}

// +kubebuilder:validation:Enum=AccessToken;IdToken
type GcpAuthType string

const (
	GcpAuthTypeAccessToken GcpAuthType = "AccessToken"
	GcpAuthTypeIdToken     GcpAuthType = "IdToken"
)

// gcp specifies how to authenticate on Google Cloud Platform
// +kubebuilder:validation:XValidation:rule="has(self.audience) ? self.type == 'IdToken' : true",message="audience is only valid with IdToken"
type GcpAuth struct {
	// The type of token to generate. To authenticate to GCP services, generally an AccessToken is used. To authenticate
	// to CloudRun, an IdToken is used.
	//
	// +optional
	Type *GcpAuthType `json:"type,omitempty"`
	// audience allows explicitly configuring the `aud` of the ID Token. Ony valid with `IdToken` type.
	// If not set, the aud is automatically derived from the backend hostname.
	//
	// +optional
	Audience *ShortString `json:"audience,omitempty"`
}

// AwsAuth specifies the authentication method to use for the backend.
type AwsAuth struct {
	// SecretRef references a Kubernetes Secret containing the AWS credentials.
	// The Secret must have keys "accessKey", "secretKey", and optionally "sessionToken".
	// +required
	SecretRef corev1.LocalObjectReference `json:"secretRef"`
}

type AzureAuth struct {
	// SecretRef references a Kubernetes Secret containing the Azure credentials.
	// The Secret must have keys "clientId", "tenantId", and "clientSecret".
	//
	// +optional
	SecretRef corev1.LocalObjectReference `json:"secretRef,omitempty"`

	// Details for managed identity authentication
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
	// Enrich requests sent to the LLM provider by appending and prepending system prompts. This can be configured only for
	// LLM providers that use the `CHAT` or `CHAT_STREAMING` API route type.
	// +optional
	PromptEnrichment *AIPromptEnrichment `json:"prompt,omitempty"`

	// promptGuard enables adding guardrails to LLM requests and responses.
	// +optional
	PromptGuard *AIPromptGuard `json:"promptGuard,omitempty"`

	// Provide defaults to merge with user input fields. If the field is already set, the field in the request is used.
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=64
	// +optional
	Defaults []FieldDefault `json:"defaults,omitempty"`
	// Provide overrides to merge with user input fields. If the field is already set, the field will be overwritten.
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=64
	// +optional
	Overrides []FieldDefault `json:"overrides,omitempty"`
	// Provide CEL transformations to compute and set fields in the request body.
	// The expression result overwrites any existing value for that field.
	// This has a higher priority than 'overrides' if both are set for the same key.
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=64
	// +optional
	Transformations []FieldTransformation `json:"transformations,omitempty"`

	// ModelAliases maps friendly model names to actual provider model names.
	// Example: {"fast": "gpt-3.5-turbo", "smart": "gpt-4-turbo"}
	// Note: This field is only applicable when using the agentgateway data plane.
	// +kubebuilder:validation:MaxProperties=64
	// +optional
	ModelAliases map[string]string `json:"modelAliases,omitempty"`

	// promptCaching enables automatic prompt caching for supported providers (AWS Bedrock).
	// Reduces API costs by caching static content like system prompts and tool definitions.
	// Only applicable for Bedrock Claude 3+ and Nova models.
	// +optional
	PromptCaching *PromptCachingConfig `json:"promptCaching,omitempty"`

	// routes defines how to identify the type of traffic to handle.
	// The keys are URL path suffixes matched using ends-with comparison (e.g., "/v1/chat/completions").
	// The special "*" wildcard matches any path.
	// If not specified, all traffic defaults to "completions" type.
	// +optional
	Routes map[string]RouteType `json:"routes,omitempty"`
}

// RouteType specifies how the AI gateway should process incoming requests
// based on the URL path and the API format expected.
// +kubebuilder:validation:Enum=Completions;Messages;Models;Passthrough;Responses;AnthropicTokenCount;Embeddings
type RouteType string

const (
	// RouteTypeCompletions processes OpenAI /v1/chat/completions format requests
	RouteTypeCompletions RouteType = "Completions"

	// RouteTypeMessages processes Anthropic /v1/messages format requests
	RouteTypeMessages RouteType = "Messages"

	// RouteTypeModels handles /v1/models endpoint (returns available models)
	RouteTypeModels RouteType = "Models"

	// RouteTypePassthrough sends requests to upstream as-is without LLM processing
	RouteTypePassthrough RouteType = "Passthrough"

	// RouteTypeResponses processes OpenAI /v1/responses format requests
	RouteTypeResponses RouteType = "Responses"

	// RouteTypeAnthropicTokenCount processes Anthropic /v1/messages/count_tokens format requests
	RouteTypeAnthropicTokenCount RouteType = "AnthropicTokenCount" //nolint:gosec // G101: False positive - this is a route type name, not credentials

	// RouteTypeEmbeddings processes OpenAI /v1/embeddings format requests
	RouteTypeEmbeddings RouteType = "Embeddings"

	// RouteTypeRealtime processes OpenAI /v1/realtime requests
	RouteTypeRealtime RouteType = "Realtime"
)

// +kubebuilder:validation:AtLeastOneFieldSet
type BackendMCP struct {
	// authorization defines MCPBackend level authorization. Unlike authorization at the HTTP level, which will reject
	// unauthorized requests with a 403 error, this policy works at the MCPBackend level.
	//
	// List operations, such as list_tools, will have each item evaluated. Items that do not meet the rule will be filtered.
	//
	// Get or call operations, such as call_tool, will evaluate the specific item and reject requests that do not meet the rule.
	// +optional
	Authorization *shared.Authorization `json:"authorization,omitempty"`
	// authentication defines MCPBackend specific authentication rules.
	// +optional
	Authentication *MCPAuthentication `json:"authentication,omitempty"`
}

type MCPAuthentication struct {
	// ResourceMetadata defines the metadata to use for MCP resources.
	// +optional
	ResourceMetadata map[string]apiextensionsv1.JSON `json:"resourceMetadata"`

	// McpIDP specifies the identity provider to use for authentication
	// +kubebuilder:validation:Enum=Auth0;Keycloak
	// +optional
	McpIDP *McpIDP `json:"provider,omitempty"`

	// Issuer identifies the IdP that issued the JWT. This corresponds to the 'iss' claim (https://tools.ietf.org/html/rfc7519#section-4.1.1).
	// +optional
	Issuer ShortString `json:"issuer,omitempty"`

	// audiences specify the list of allowed audiences that are allowed access. This corresponds to the 'aud' claim (https://datatracker.ietf.org/doc/html/rfc7519#section-4.1.3).
	// If unset, any audience is allowed.
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=64
	// +optional
	Audiences []string `json:"audiences,omitempty"`

	// jwks defines the remote JSON Web Key used to validate the signature of the JWT.
	// +required
	JWKS RemoteJWKS `json:"jwks"`

	// validation mode for JWT authentication.
	// +kubebuilder:default=Strict
	// +optional
	Mode JWTAuthenticationMode `json:"mode,omitempty"`
}

type McpIDP string

const (
	Auth0    McpIDP = "Auth0"
	Keycloak McpIDP = "Keycloak"
)

type BackendHTTP struct {
	// version specifies the HTTP protocol version to use when connecting to the backend.
	// If not specified, the version is automatically determined:
	// * Service types can specify it with 'appProtocol' on the Service port.
	// * If traffic is identified as gRPC, HTTP2 is used.
	// * If the incoming traffic was plaintext HTTP, the original protocol will be used.
	// * If the incoming traffic was HTTPS, HTTP1 will be used. This is because most clients will
	//   transparently upgrade HTTPS traffic to HTTP2, even if the backend doesn't support it
	// +kubebuilder:validation:Enum=HTTP1;HTTP2
	// +optional
	Version *HTTPVersion `json:"version,omitempty"`

	// requestTimeout specifies the deadline for receiving a response from the backend.
	// +kubebuilder:validation:XValidation:rule="matches(self, '^([0-9]{1,5}(h|m|s|ms)){1,4}$')",message="invalid duration value"
	// +kubebuilder:validation:XValidation:rule="duration(self) >= duration('1ms')",message="requestTimeout must be at least 1ms"
	// +optional
	RequestTimeout *metav1.Duration `json:"requestTimeout,omitempty"`
}

type HTTPVersion string

const (
	HTTPVersion1 HTTPVersion = "HTTP1"
	HTTPVersion2 HTTPVersion = "HTTP2"
)

type BackendTCP struct {
	// keepAlive defines settings for enabling TCP keepalives on the connection.
	// +optional
	Keepalive *Keepalive `json:"keepalive,omitempty"`
	// connectTimeout defines the deadline for establishing a connection to the destination.
	// +kubebuilder:validation:XValidation:rule="matches(self, '^([0-9]{1,5}(h|m|s|ms)){1,4}$')",message="invalid duration value"
	// +kubebuilder:validation:XValidation:rule="duration(self) >= duration('100ms')",message="connectTimeout must be at least 100ms"
	// +optional
	ConnectTimeout *metav1.Duration `json:"connectTimeout,omitempty"`
}

// +kubebuilder:validation:AtLeastOneFieldSet
type Transformation struct {
	// request is used to modify the request path.
	// +optional
	Request *Transform `json:"request,omitempty"`

	// response is used to modify the response path.
	// +optional
	Response *Transform `json:"response,omitempty"`
}

// +kubebuilder:validation:AtLeastOneFieldSet
type Transform struct {
	// set is a list of headers and the value they should be set to.
	//
	// +listType=map
	// +listMapKey=name
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +optional
	Set []HeaderTransformation `json:"set,omitempty"`

	// add is a list of headers to add to the request and what that value should be set to. If there is already a header
	// with these values then append the value as an extra entry.
	//
	// +listType=map
	// +listMapKey=name
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +optional
	Add []HeaderTransformation `json:"add,omitempty"`

	// Remove is a list of header names to remove from the request/response.
	//
	// +listType=set
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +optional
	Remove []HeaderName `json:"remove,omitempty"`

	// body controls manipulation of the HTTP body.
	// +optional
	Body *shared.CELExpression `json:"body,omitempty"`
}

// An HTTP Header Name.
//
// +kubebuilder:validation:MinLength=1
// +kubebuilder:validation:MaxLength=256
// +kubebuilder:validation:Pattern=`^:?[A-Za-z0-9!#$%&'*+\-.^_\x60|~]+$`
// +kubebuilder:validation:XValidation:rule="!self.startsWith(':') || self in [':authority', ':method', ':path', ':scheme', ':status']",message="pseudo-headers must be one of :authority, :method, :path, :scheme, or :status"
// +k8s:deepcopy-gen=false
type HeaderName string

type HeaderTransformation struct {
	// the name of the header to add.
	// +required
	Name HeaderName `json:"name"`
	// value is the CEL expression to apply to generate the output value for the header.
	// +required
	Value shared.CELExpression `json:"value"`
}

type ExtProc struct {
	// backendRef references the External Processor server to reach.
	// Supported types: Service and Backend.
	// +required
	BackendRef gwv1.BackendObjectReference `json:"backendRef"`
}

// +kubebuilder:validation:ExactlyOneOf=grpc;http
type ExtAuth struct {
	// backendRef references the External Authorization server to reach.
	//
	// Supported types: Service and Backend.
	// +required
	BackendRef gwv1.BackendObjectReference `json:"backendRef"`

	// grpc specifies that the gRPC External Authorization
	// [protocol](https://www.envoyproxy.io/docs/envoy/latest/api-v3/service/auth/v3/external_auth.proto) should be used.
	// +optional
	GRPC *AgentExtAuthGRPC `json:"grpc,omitempty"`

	// http specifies that the HTTP protocol should be used for connecting to the authorization server.
	// The authorization server must return a `200` status code, otherwise the request is considered an authorization failure.
	// +optional
	HTTP *AgentExtAuthHTTP `json:"http,omitempty"`

	// forwardBody configures whether to include the HTTP body in the request. If enabled, the request body will be
	// buffered.
	// +optional
	ForwardBody *ExtAuthBody `json:"forwardBody,omitempty"`
}

type AgentExtAuthHTTP struct {
	// path specifies the path to send to the authorization server. If unset, this defaults to the original request path.
	// This is a CEL expression, which allows customizing the path based on the incoming request.
	// For example, to add a prefix: `path: '"/prefix/" + request.path'`.
	// +optional
	Path *shared.CELExpression `json:"path,omitempty"`

	// redirect defines an optional expression to determine a path to redirect to on authorization failure.
	// This is useful to redirect to a sign-in page.
	// +optional
	Redirect *shared.CELExpression `json:"redirect,omitempty"`

	// allowedRequestHeaders specifies what additional headers from the client request
	// will be sent to the authorization server.
	//
	// If unset, the following headers are sent by default: `Authorization`.
	//
	// +optional
	// +kubebuilder:validation:MaxItems=64
	AllowedRequestHeaders []ShortString `json:"allowedRequestHeaders,omitempty"`

	// addRequestHeaders specifies what additional headers to add to the request to the authorization server.
	// While allowedRequestHeaders just passes the original headers through, addRequestHeaders allows defining custom headers
	// based on CEL Expressions.
	//
	// +optional
	// +kubebuilder:validation:MaxProperties=64
	AddRequestHeaders map[string]shared.CELExpression `json:"addRequestHeaders,omitempty"`

	// allowedResponseHeaders specifies what headers from the authorization response
	// will be copied into the request to the backend.
	//
	// +optional
	// +kubebuilder:validation:MaxItems=64
	AllowedResponseHeaders []ShortString `json:"allowedResponseHeaders,omitempty"`

	// responseMetadata specifies what metadata fields should be constructed *from* the authorization response. These will be
	// included under the `extauthz` variable in future CEL expressions. Setting this is useful to do things like logging
	// usernames, without needing to include them as headers to the backend (as `allowedResponseHeaders` would).
	//
	// +optional
	// +kubebuilder:validation:MaxProperties=64
	ResponseMetadata map[string]shared.CELExpression `json:"responseMetadata,omitempty"`
}

type AgentExtAuthGRPC struct {
	// contextExtensions specifies additional arbitrary key-value pairs to send to the authorization server in the `context_extensions` field.
	//
	// +optional
	// +kubebuilder:validation:MaxProperties=64
	ContextExtensions map[string]string `json:"contextExtensions,omitempty"`
	// requestMetadata specifies metadata to be sent *to* the authorization server.
	// This maps to the `metadata_context.filter_metadata` field of the request, and allows dynamic CEL expressions.
	// If unset, by default the `envoy.filters.http.jwt_authn` key is set if the JWT policy is used as well, for compatibility.
	//
	// +optional
	// +kubebuilder:validation:MaxProperties=64
	RequestMetadata map[string]shared.CELExpression `json:"requestMetadata,omitempty"`
}

type ExtAuthBody struct {
	// maxSize specifies how large in bytes the largest body that will be buffered and sent to the authorization server. If
	// the body size is larger than maxSize, then the request will be rejected with a response.
	//
	// +kubebuilder:validation:Minimum=1
	// +required
	MaxSize int32 `json:"maxSize"`
}

// +kubebuilder:validation:AtLeastOneFieldSet
type RateLimits struct {
	// Local defines a local rate limiting policy.
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +optional
	Local []LocalRateLimit `json:"local,omitempty"`

	// Global defines a global rate limiting policy using an external service.
	// +optional
	Global *GlobalRateLimit `json:"global,omitempty"`
}

type GlobalRateLimit struct {
	// backendRef references the Rate Limit server to reach.
	// Supported types: Service and Backend.
	// +required
	BackendRef gwv1.BackendObjectReference `json:"backendRef"`

	// domain specifies the domain under which this limit should apply.
	// This is an arbitrary string that enables a rate limit server to distinguish between different applications.
	// +required
	Domain ShortString `json:"domain"`

	// Descriptors define the dimensions for rate limiting. These values are passed to the rate limit service which applies
	// configured limits based on them. Each descriptor represents a single rate limit rule with one or more entries.
	//
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +required
	Descriptors []RateLimitDescriptor `json:"descriptors"`
}

type RateLimitUnit string

const (
	RateLimitUnitTokens   RateLimitUnit = "Tokens"
	RateLimitUnitRequests RateLimitUnit = "Requests"
)

type RateLimitDescriptor struct {
	// entries are the individual components that make up this descriptor.
	//
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +required
	Entries []RateLimitDescriptorEntry `json:"entries"`
	// unit defines what to use as the cost function. If unspecified, Requests is used.
	// +kubebuilder:validation:Enum=Requests;Tokens
	// +optional
	Unit *RateLimitUnit `json:"unit,omitempty"`
}

// A descriptor entry defines a single entry in a rate limit descriptor.
type RateLimitDescriptorEntry struct {
	// name specifies the name of the descriptor.
	// +required
	Name TinyString `json:"name"`
	// expression is a Common Expression Language (CEL) expression that defines the value for the descriptor.
	//
	// For example, to rate limit based on the Client IP: `source.address`.
	//
	// See https://agentgateway.dev/docs/standalone/latest/reference/cel/ for more info.
	// +required
	Expression shared.CELExpression `json:"expression"`
}

type LocalRateLimitUnit string

const (
	LocalRateLimitUnitSeconds LocalRateLimitUnit = "Seconds"
	LocalRateLimitUnitMinutes LocalRateLimitUnit = "Minutes"
	LocalRateLimitUnitHours   LocalRateLimitUnit = "Hours"
)

// Policy for local rate limiting. Local rate limits are handled locally on a per-proxy basis, without co-ordination
// between instances of the proxy.
// +kubebuilder:validation:ExactlyOneOf=requests;tokens
type LocalRateLimit struct {
	// requests specifies the number of HTTP requests per unit of time that are allowed. Requests exceeding this limit will fail with
	// a 429 error.
	// +kubebuilder:validation:Minimum=1
	// +optional
	Requests *int32 `json:"requests,omitempty"`

	// tokens specifies the number of LLM tokens per unit of time that are allowed. Requests exceeding this limit will fail
	// with a 429 error.
	//
	// Both input and output tokens are counted. However, token counts are not known until the request completes. As a
	// result, token-based rate limits will apply to future requests only.
	//
	// +kubebuilder:validation:Minimum=1
	// +optional
	Tokens *int32 `json:"tokens,omitempty"`

	// unit specifies the unit of time that requests are limited based on.
	//
	// +kubebuilder:validation:Enum=Seconds;Minutes;Hours
	// +required
	Unit LocalRateLimitUnit `json:"unit"`

	// burst specifies an allowance of requests above the request-per-unit that should be allowed within a short period of time.
	// +optional
	Burst *int32 `json:"burst,omitempty"`
}

type CORS struct {
	// +kubebuilder:pruning:PreserveUnknownFields
	*gwv1.HTTPCORSFilter `json:",inline"`
}

type CSRF struct {
	// additionalOrigin specifies additional source origins that will be allowed in addition to the destination origin. The
	// `Origin` consists of a scheme and a host, with an optional port, and takes the form `<scheme>://<host>(:<port>)`.
	//
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +optional
	AdditionalOrigins []ShortString `json:"additionalOrigins,omitempty"`
}

type HostnameRewrite struct {
	// mode sets the hostname rewrite mode.
	//
	// The following may be specified:
	// * Auto: automatically set the Host header based on the destination.
	// * None: do not rewrite the Host header. The original Host header will be passed through.
	//
	// This setting defaults to Auto when connecting to hostname-based Backend types, and None otherwise (for Service or
	// IP-based Backends).
	// +required
	Mode HostnameRewriteMode `json:"mode"`
}

type Timeouts struct {
	// request specifies a timeout for an individual request from the gateway to a backend. This covers the time from when
	// the request first starts being sent from the gateway to when the full response has been received from the backend.
	//
	// +kubebuilder:validation:XValidation:rule="matches(self, '^([0-9]{1,5}(h|m|s|ms)){1,4}$')",message="invalid duration value"
	// +kubebuilder:validation:XValidation:rule="duration(self) >= duration('100ms')",message="request must be at least 1ms"
	// +optional
	Request *metav1.Duration `json:"request,omitempty"`
}

// Retry defines the retry policy
type Retry struct {
	*gwv1.HTTPRouteRetry `json:",inline"`
}

// accessLogs specifies how per-request access logs are emitted.
type AccessLog struct {
	// filter specifies a CEL expression that is used to filter logs. A log will only be emitted if the expression evaluates
	// to 'true'.
	// +optional
	Filter *shared.CELExpression `json:"filter,omitempty"`
	// attributes specifies customizations to the key-value pairs that are logged
	// +optional
	Attributes *LogTracingAttributes `json:"attributes,omitempty"`
}

// +kubebuilder:validation:AtLeastOneFieldSet
type LogTracingAttributes struct {
	// remove lists the default fields that should be removed. For example, "http.method".
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=32
	// +optional
	Remove []TinyString `json:"remove,omitempty"`
	// add specifies additional key-value pairs to be added to each entry.
	// The value is a CEL expression. If the CEL expression fails to evaluate, the pair will be excluded.
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:maxItems=64
	// +optional
	Add []AttributeAdd `json:"add,omitempty"`
}

type AttributeAdd struct {
	// +required
	Name ShortString `json:"name"`
	// +required
	Expression shared.CELExpression `json:"expression"`
}

type TracingProtocol string

const (
	TracingProtocolHttp TracingProtocol = "HTTP"
	TracingProtocolGrpc TracingProtocol = "GRPC"
)

type Tracing struct {
	// backendRef references the OTLP server to reach.
	// Supported types: Service and AgentgatewayBackend.
	// +required
	BackendRef gwv1.BackendObjectReference `json:"backendRef"`
	// protocol specifies the OTLP protocol variant to use.
	// +kubebuilder:default=HTTP
	// +kubebuilder:validation:Enum=HTTP;GRPC
	// +optional
	Protocol TracingProtocol `json:"protocol,omitempty"`

	// attributes specify customizations to the key-value pairs that are included in the trace.
	// +optional
	Attributes *LogTracingAttributes `json:"attributes,omitempty"`

	// resources describe the entity producing telemetry and specify the resources to be included in the trace.
	// +optional
	Resources []ResourceAdd `json:"resources,omitempty"`

	// randomSampling is an expression to determine the amount of random sampling. Random sampling will initiate a new
	// trace span if the incoming request does not have a trace initiated already. This should evaluate to a float between
	// 0.0-1.0, or a boolean (true/false) If unspecified, random sampling is disabled.
	// +optional
	RandomSampling *shared.CELExpression `json:"randomSampling,omitempty"`
	// clientSampling is an expression to determine the amount of client sampling. Client sampling determines whether to
	// initiate a new trace span if the incoming request does have a trace already. This should evaluate to a float between
	// 0.0-1.0, or a boolean (true/false) If unspecified, client sampling is 100% enabled.
	// +optional
	ClientSampling *shared.CELExpression `json:"clientSampling,omitempty"`
}

type ResourceAdd struct {
	// +required
	Name ShortString `json:"name"`
	// +required
	Expression shared.CELExpression `json:"expression"`
}
