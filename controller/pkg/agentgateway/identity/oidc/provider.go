package oidc

import (
	"crypto/md5" //nolint:gosec
	"crypto/tls"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net"
	"net/url"
	"reflect"
	"strings"
	"time"

	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
)

const (
	configMapKey            = "oidc-provider-store"
	storeComponentLabel     = "app.kubernetes.io/component"
	DefaultProviderStoreTTL = 5 * time.Minute
	DefaultStorePrefix      = "oidc-provider-store"
)

var ProviderConfigMapNamespacedName = func(resourceKey string) *types.NamespacedName {
	return nil
}

type sourceKey struct {
	Issuer     string         `json:"issuer"`
	BackendRef *backendRefKey `json:"backendRef,omitempty"`
}

type backendRefKey struct {
	Group     string `json:"group,omitempty"`
	Kind      string `json:"kind,omitempty"`
	Namespace string `json:"namespace,omitempty"`
	Name      string `json:"name"`
	Port      int32  `json:"port,omitempty"`
}

type ProviderSource struct {
	// OwnerKey keeps the KRT output per policy owner while ResourceKey identifies
	// the shared fetched provider state. The ideal end state is to keep this
	// owner/shared split, but derive the shared identity from an explicit
	// transport model rather than comparing tls.Config directly.
	OwnerKey     string
	ResourceKey  string
	Issuer       string
	RequestURL   string
	HostOverride string
	Ttl          time.Duration
	Deleted      bool
	TlsConfig    *tls.Config `json:"-"`
}

func (s ProviderSource) ResourceName() string {
	if s.OwnerKey != "" {
		return s.OwnerKey + ":" + s.ResourceKey
	}
	return s.ResourceKey
}

func (s ProviderSource) Equals(other ProviderSource) bool {
	return s.OwnerKey == other.OwnerKey &&
		s.ResourceKey == other.ResourceKey &&
		s.Issuer == other.Issuer &&
		s.RequestURL == other.RequestURL &&
		s.HostOverride == other.HostOverride &&
		s.Ttl == other.Ttl &&
		s.Deleted == other.Deleted &&
		reflect.DeepEqual(s.TlsConfig, other.TlsConfig)
}

func (s ProviderSource) Equivalent(other ProviderSource) bool {
	// Equivalent compares the shared fetch identity only. OwnerKey stays out of
	// this comparison so the store can dedupe multiple policy owners onto one
	// fetched provider source.
	return s.ResourceKey == other.ResourceKey &&
		s.Issuer == other.Issuer &&
		s.RequestURL == other.RequestURL &&
		s.HostOverride == other.HostOverride &&
		s.Ttl == other.Ttl &&
		reflect.DeepEqual(s.TlsConfig, other.TlsConfig)
}

type StoredProvider struct {
	ResourceKey                       string   `json:"resourceKey"`
	Issuer                            string   `json:"issuer"`
	AuthorizationEndpoint             string   `json:"authorizationEndpoint"`
	TokenEndpoint                     string   `json:"tokenEndpoint"`
	JwksURI                           string   `json:"jwksUri"`
	EndSessionEndpoint                string   `json:"endSessionEndpoint,omitempty"`
	TokenEndpointAuthMethodsSupported []string `json:"tokenEndpointAuthMethodsSupported,omitempty"`
}

type DiscoveryDocument struct {
	Issuer                            string   `json:"issuer"`
	AuthorizationEndpoint             string   `json:"authorization_endpoint"`
	TokenEndpoint                     string   `json:"token_endpoint"`
	JwksURI                           string   `json:"jwks_uri"`
	EndSessionEndpoint                string   `json:"end_session_endpoint"`
	TokenEndpointAuthMethodsSupported []string `json:"token_endpoint_auth_methods_supported"`
}

func CanonicalSourceKey(
	issuer string,
	policyNamespace string,
	backendRef *gwv1.BackendObjectReference,
) (string, error) {
	key := sourceKey{Issuer: issuer}
	if backendRef != nil {
		namespace := policyNamespace
		if backendRef.Namespace != nil {
			namespace = string(*backendRef.Namespace)
		}
		kind := "Service"
		if backendRef.Kind != nil {
			kind = string(*backendRef.Kind)
		}
		group := ""
		if backendRef.Group != nil {
			group = string(*backendRef.Group)
		}
		port := int32(0)
		if backendRef.Port != nil {
			port = int32(*backendRef.Port)
		}
		key.BackendRef = &backendRefKey{
			Group:     group,
			Kind:      kind,
			Namespace: namespace,
			Name:      string(backendRef.Name),
			Port:      port,
		}
	}
	b, err := json.Marshal(key)
	if err != nil {
		return "", fmt.Errorf("failed marshaling oidc provider source key: %w", err)
	}
	return string(b), nil
}

func StoredProviderFromDiscovery(resourceKey string, metadata DiscoveryDocument) StoredProvider {
	return StoredProvider{
		ResourceKey:                       resourceKey,
		Issuer:                            metadata.Issuer,
		AuthorizationEndpoint:             metadata.AuthorizationEndpoint,
		TokenEndpoint:                     metadata.TokenEndpoint,
		JwksURI:                           metadata.JwksURI,
		EndSessionEndpoint:                metadata.EndSessionEndpoint,
		TokenEndpointAuthMethodsSupported: metadata.TokenEndpointAuthMethodsSupported,
	}
}

func StoreLabelSelector(storePrefix string) string {
	return storeComponentLabel + "=" + storePrefix
}

func StoreConfigMapLabel(storePrefix string) map[string]string {
	return map[string]string{storeComponentLabel: storePrefix}
}

func ProviderConfigMapName(storePrefix, resourceKey string) string {
	hash := md5.Sum([]byte(resourceKey)) //nolint:gosec
	return fmt.Sprintf("%s-%s", storePrefix, hex.EncodeToString(hash[:]))
}

func BuildProviderConfigMapNamespacedNameFunc(storePrefix, deploymentNamespace string) {
	ProviderConfigMapNamespacedName = func(resourceKey string) *types.NamespacedName {
		return &types.NamespacedName{
			Namespace: deploymentNamespace,
			Name:      ProviderConfigMapName(storePrefix, resourceKey),
		}
	}
}

func SetProviderInConfigMap(cm *corev1.ConfigMap, provider StoredProvider) error {
	b, err := json.Marshal(provider)
	if err != nil {
		return err
	}
	cm.Data[configMapKey] = string(b)
	return nil
}

func ProviderFromConfigMap(cm *corev1.ConfigMap) (StoredProvider, error) {
	var provider StoredProvider
	if err := json.Unmarshal([]byte(cm.Data[configMapKey]), &provider); err != nil {
		return StoredProvider{}, err
	}
	return provider, nil
}

func BuildDiscoveryURL(issuer string) (string, error) {
	parsed, err := url.Parse(issuer)
	if err != nil {
		return "", fmt.Errorf("invalid issuer url %q: %w", issuer, err)
	}
	if parsed.Scheme == "" || parsed.Host == "" {
		return "", fmt.Errorf("issuer must be an absolute url: %q", issuer)
	}
	if parsed.RawQuery != "" || parsed.Fragment != "" {
		return "", fmt.Errorf("issuer must not contain query or fragment: %q", issuer)
	}
	if parsed.User != nil {
		return "", fmt.Errorf("issuer must not include userinfo: %q", issuer)
	}
	if parsed.Scheme != "https" {
		if parsed.Scheme != "http" || !IsLoopbackHost(parsed.Hostname()) {
			return "", fmt.Errorf("issuer must use https (or http on loopback hosts): %q", issuer)
		}
	}
	parsed.Path = strings.TrimSuffix(parsed.Path, "/") + "/.well-known/openid-configuration"
	parsed.RawQuery = ""
	parsed.Fragment = ""
	return parsed.String(), nil
}

func ValidateDiscoveryIssuer(configuredIssuer string, discoveredIssuer string) error {
	discoveredIssuer = strings.TrimSpace(discoveredIssuer)
	if discoveredIssuer == "" {
		return fmt.Errorf("issuer is missing in discovery metadata")
	}
	if strings.TrimRight(discoveredIssuer, "/") != strings.TrimRight(configuredIssuer, "/") {
		return fmt.Errorf(
			"issuer mismatch: configured %q, discovery returned %q",
			configuredIssuer,
			discoveredIssuer,
		)
	}
	return nil
}

func ValidateDiscoveryMetadataEndpoints(metadata *DiscoveryDocument) error {
	validate := func(endpoint string, field string) error {
		if endpoint == "" {
			return nil
		}
		parsed, err := url.Parse(endpoint)
		if err != nil {
			return fmt.Errorf("invalid %s URL in discovery metadata: %w", field, err)
		}
		if parsed.Fragment != "" || parsed.User != nil {
			return fmt.Errorf("%s must not contain fragment or userinfo", field)
		}
		if parsed.Host == "" {
			return fmt.Errorf("%s is missing host", field)
		}
		if parsed.Scheme != "https" {
			if parsed.Scheme != "http" || !IsLoopbackHost(parsed.Hostname()) {
				return fmt.Errorf("%s must use https (or http on loopback hosts)", field)
			}
		}
		return nil
	}

	if err := validate(metadata.AuthorizationEndpoint, "authorization_endpoint"); err != nil {
		return err
	}
	if err := validate(metadata.TokenEndpoint, "token_endpoint"); err != nil {
		return err
	}
	if err := validate(metadata.JwksURI, "jwks_uri"); err != nil {
		return err
	}
	if err := validate(metadata.EndSessionEndpoint, "end_session_endpoint"); err != nil {
		return err
	}
	return nil
}

func IsLoopbackHost(host string) bool {
	if strings.EqualFold(host, "localhost") {
		return true
	}
	ip := net.ParseIP(host)
	return ip != nil && ip.IsLoopback()
}
