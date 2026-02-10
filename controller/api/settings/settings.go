package settings

import (
	"encoding/json"
	"fmt"
	"strings"

	"github.com/kelseyhightower/envconfig"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
)

// ValidationMode determines how invalid routes and policies are handled during translation.
// Higher levels increase safety guarantees, but may have performance implications.
type ValidationMode string

const (
	// ValidationStandard rewrites invalid routes to direct responses
	// (typically HTTP 500), preserving a valid config while isolating failures.
	// This limits the blast radius of misconfigured routes or policies without
	// affecting unrelated tenants.
	ValidationStandard ValidationMode = "STANDARD"
	// ValidationStrict builds on standard by running targeted validation
	// (e.g. RDS, CDS, and security-related policies). Routes that fail these
	// checks are also replaced with direct responses, and helps prevent unsafe
	// config from reaching Envoy.
	// Strict Validation is not supported with Rustformation yet,
	// see docs/guides/transformation.md for details
	ValidationStrict ValidationMode = "STRICT"
)

// Decode implements envconfig.Decoder.
func (v *ValidationMode) Decode(value string) error {
	level := ValidationMode(strings.ToUpper(value))
	switch level {
	case ValidationStandard, ValidationStrict:
		*v = level
		return nil
	default:
		return fmt.Errorf("invalid validation mode: %q", value)
	}
}

// DnsLookupFamily controls the DNS lookup family for all static clusters created via Backend resources.
type DnsLookupFamily string

const (
	// DnsLookupFamilyV4Preferred is the default value for DnsLookupFamily.
	// The DNS resolver will first perform a lookup for addresses in the IPv4 family
	// and fallback to a lookup for addresses in the IPv6 family. The callback target
	// will only get v6 addresses if there were no v4 addresses to return.
	DnsLookupFamilyV4Preferred DnsLookupFamily = "V4_PREFERRED"
	// DnsLookupFamilyV4Only is the value for DnsLookupFamily that only performs
	// DNS lookups for addresses in the IPv4 family.
	DnsLookupFamilyV4Only DnsLookupFamily = "V4_ONLY"
	// DnsLookupFamilyV6Only is the value for DnsLookupFamily that only performs
	// DNS lookups for addresses in the IPv6 family.
	DnsLookupFamilyV6Only DnsLookupFamily = "V6_ONLY"
	// DnsLookupFamilyAll is the value for DnsLookupFamily that performs lookups
	// for both IPv4 and IPv6 families and returns all resolved addresses.
	DnsLookupFamilyAll DnsLookupFamily = "ALL"
	// DnsLookupFamilyAuto is the value for DnsLookupFamily that first performs
	// a lookup for addresses in the IPv6 family and falls back to a lookup for
	// addresses in the IPv4 family. This is semantically equivalent to a
	// non-existent V6_PREFERRED option and is a legacy name that will be
	// deprecated in favor of V6_PREFERRED in a future major version.
	DnsLookupFamilyAuto DnsLookupFamily = "AUTO"
)

// Decode implements envconfig.Decoder.
func (m *DnsLookupFamily) Decode(value string) error {
	mode := DnsLookupFamily(value)
	switch mode {
	case DnsLookupFamilyV4Preferred, DnsLookupFamilyV4Only, DnsLookupFamilyV6Only, DnsLookupFamilyAll, DnsLookupFamilyAuto:
		*m = mode
		return nil
	default:
		return fmt.Errorf("invalid DNS lookup family: %q", value)
	}
}

// GatewayClassParametersRefs maps GatewayClass names to ParametersReference
type GatewayClassParametersRefs map[string]*gwv1.ParametersReference

// Decode implements envconfig.Decoder
func (r *GatewayClassParametersRefs) Decode(value string) error {
	if value == "" {
		*r = nil
		return nil
	}

	// First parse as a simple map to validate name is present
	var simpleParsed map[string]struct {
		Name      string `json:"name"`
		Namespace string `json:"namespace"`
		Group     string `json:"group,omitempty"`
		Kind      string `json:"kind,omitempty"`
	}
	if err := json.Unmarshal([]byte(value), &simpleParsed); err != nil {
		return fmt.Errorf("invalid gateway class parameters refs: %w", err)
	}

	parsed := make(map[string]*gwv1.ParametersReference, len(simpleParsed))
	for className, ref := range simpleParsed {
		if strings.TrimSpace(ref.Name) == "" {
			return fmt.Errorf("gateway class %q parametersRef.name must be set", className)
		}
		if strings.TrimSpace(ref.Namespace) == "" {
			return fmt.Errorf("gateway class %q parametersRef.namespace must be set", className)
		}
		ns := gwv1.Namespace(ref.Namespace)
		paramsRef := &gwv1.ParametersReference{
			Name:      ref.Name,
			Namespace: &ns,
		}
		if ref.Group != "" {
			paramsRef.Group = gwv1.Group(ref.Group)
		}
		if ref.Kind != "" {
			paramsRef.Kind = gwv1.Kind(ref.Kind)
		}

		parsed[className] = paramsRef
	}

	*r = parsed
	return nil
}

const (
	// TLSSecretName is the name of the Kubernetes Secret containing the TLS certificate,
	// private key, and CA certificate for xDS communication. This secret must exist in the
	// kgateway installation namespace when TLS is enabled.
	TLSSecretName = "kgateway-xds-cert" //nolint:gosec // G101: This is a well-known xDS TLS secret name, not a credential

	// TLSCertPath is the path to the TLS certificate
	TLSCertPath = "/etc/xds-tls/tls.crt"

	// TLSKeyPath is the path to the TLS key
	TLSKeyPath = "/etc/xds-tls/tls.key"

	// TLSRootCAPath is the path to the TLS root CA
	TLSRootCAPath = "/etc/xds-tls/ca.crt"
)

type Settings struct {
	// Controls the DnsLookupFamily for all static clusters created via Backend resources.
	// If not set, kgateway will default to "V4_PREFERRED". Note that this is different
	// from the Envoy default of "AUTO", which is effectively "V6_PREFERRED".
	// Supported values are: "ALL", "AUTO", "V4_PREFERRED", "V4_ONLY", "V6_ONLY"
	// Details on the behavior of these options are available on the Envoy documentation:
	// https://www.envoyproxy.io/docs/envoy/latest/api-v3/config/cluster/v3/cluster.proto#enum-config-cluster-v3-cluster-dnslookupfamily
	DnsLookupFamily DnsLookupFamily `split_words:"true" default:"V4_PREFERRED"`

	// Controls the listener bind address. Can be either V4 or V6
	ListenerBindIpv6 bool `split_words:"true" default:"true"`

	// IstioNamespace is the namespace where Istio control plane components are installed.
	// Defaults to "istio-system".
	IstioNamespace string `split_words:"true" default:"istio-system"`

	// XdsServiceHost is the host that serves xDS config.
	// It overrides xdsServiceName if set.
	XdsServiceHost string `split_words:"true"`

	// XdsServiceName is the name of the Kubernetes Service that serves xDS config.
	// It is assumed to be in the kgateway install namespace.
	// Ignored if XdsServiceHost is set.
	XdsServiceName string `split_words:"true" default:"agentgateway"`

	// XdsServicePort is the port of the Kubernetes Service that serves xDS config.
	// This corresponds to the value of the `grpc-xds` port in the service.
	XdsServicePort uint32 `split_words:"true" default:"9977"`

	// XdsAuth enables or disables xDS authentication between the data-plane and control-plane.
	// By default, this is enabled.
	XdsAuth bool `split_words:"true" default:"true"`

	// XdsTLS enables or disables TLS encryption for xDS communication between the data-plane and control-plane.
	// By default, this is disabled.
	XdsTLS bool `split_words:"true" default:"false"`

	// AgentgatewayXdsServicePort is the port of the Kubernetes Service that serves xDS config for agentgateway.
	// This corresponds to the value of the `grpc-xds-agw` port in the service.
	AgentgatewayXdsServicePort uint32 `split_words:"true" default:"9978"`

	// EnableInferExt defines whether to enable/disable support for Gateway API inference extension.
	// If enabled, EnableAgentgateway should also be set to true. Enabling inference extension without agentgateway
	// is deprecated in v2.1 and will not be supported in v2.2.
	EnableInferExt bool `split_words:"true"`

	// LogLevel specifies the logging level (e.g., "trace", "debug", "info", "warn", "error").
	// Defaults to "info" if not set.
	LogLevel string `split_words:"true" default:"info"`

	// EnableBuiltinDefaultMetrics enables the default builtin controller-runtime metrics and go runtime metrics.
	// Since these metrics can be numerous, it is disabled by default.
	EnableBuiltinDefaultMetrics bool `split_words:"true" default:"false"`

	// Controls if leader election is disabled. Defaults to false.
	DisableLeaderElection bool `split_words:"true" default:"false"`

	// EnableExperimentalGatewayAPIFeatures enables kgateway to support experimental features and APIs
	EnableExperimentalGatewayAPIFeatures bool `split_words:"true" default:"true"`

	// GatewayClassParametersRefs configures the GatewayParameters references to set on the default GatewayClasses.
	// Format: JSON map where keys are GatewayClass names and values are objects with "name" (required),
	// "namespace" (required), "group" (optional), and "kind" (optional) fields.
	// E.g., {"gateway-class-name":{"name":"params-name","namespace":"params-namespace","group":"gateway.networking.k8s.io","kind":"GatewayParameters"}}
	GatewayClassParametersRefs GatewayClassParametersRefs `split_words:"true" default:"{}"`
}

// BuildSettings returns a zero-valued Settings obj if error is encountered when parsing env
func BuildSettings() (*Settings, error) {
	settings := &Settings{}
	if err := envconfig.Process("AGW", settings); err != nil {
		return settings, err
	}
	return settings, nil
}
