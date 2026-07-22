package plugins

import (
	"fmt"

	"istio.io/istio/pkg/kube/krt"
	"k8s.io/apimachinery/pkg/runtime/schema"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

// TranslateCustomProviderFormats converts Kubernetes custom-provider formats to xDS formats.
func TranslateCustomProviderFormats(formats []agentgateway.ProviderFormatConfig) ([]*api.AIBackend_ProviderFormatConfig, error) {
	translated := make([]*api.AIBackend_ProviderFormatConfig, 0, len(formats))
	for _, format := range formats {
		formatType, err := translateCustomProviderFormat(format.Type)
		if err != nil {
			return nil, err
		}
		var path *string
		if format.Path != "" {
			path = new(string(format.Path))
		}
		translated = append(translated, &api.AIBackend_ProviderFormatConfig{Format: formatType, Path: path})
	}
	return translated, nil
}

func translateCustomProviderFormat(format agentgateway.ProviderFormat) (api.AIBackend_ProviderFormat, error) {
	switch format {
	case agentgateway.ProviderFormatCompletions:
		return api.AIBackend_COMPLETIONS, nil
	case agentgateway.ProviderFormatMessages:
		return api.AIBackend_MESSAGES, nil
	case agentgateway.ProviderFormatResponses:
		return api.AIBackend_RESPONSES, nil
	case agentgateway.ProviderFormatEmbeddings:
		return api.AIBackend_EMBEDDINGS, nil
	case agentgateway.ProviderFormatAnthropicTokenCount:
		return api.AIBackend_ANTHROPIC_TOKEN_COUNT, nil
	case agentgateway.ProviderFormatRealtime:
		return api.AIBackend_REALTIME, nil
	case agentgateway.ProviderFormatRerank:
		return api.AIBackend_RERANK, nil
	default:
		return api.AIBackend_PROVIDER_FORMAT_UNSPECIFIED, fmt.Errorf("unsupported custom provider format %q", format)
	}
}

// RouteBackendResolver resolves a backend reference from a route-like resource.
type RouteBackendResolver func(
	krtctx krt.HandlerContext,
	defaultNamespace string,
	gk schema.GroupKind,
	name gwv1.ObjectName,
	namespace *gwv1.Namespace,
	port *gwv1.PortNumber,
) (*api.BackendReference, error)

// TranslateCustomProviderBackendRef validates and resolves a custom-provider backend reference.
func TranslateCustomProviderBackendRef(
	krtctx krt.HandlerContext,
	routeBackend RouteBackendResolver,
	namespace string,
	ref agentgateway.LocalBackendObjectReference,
) (*api.BackendReference, error) {
	kind := wellknown.ServiceKind
	if ref.Kind != nil {
		kind = *ref.Kind
	}
	group := ""
	if ref.Group != nil {
		group = *ref.Group
	}
	gk := schema.GroupKind{Group: group, Kind: kind}
	switch gk {
	case wellknown.ServiceGVK.GroupKind(), wellknown.InferencePoolGVK.GroupKind():
	default:
		return nil, fmt.Errorf("custom provider backendRef may target only Service or InferencePool")
	}

	var port *gwv1.PortNumber
	if ref.Port != nil {
		port = new(gwv1.PortNumber(*ref.Port))
	}
	return routeBackend(krtctx, namespace, gk, gwv1.ObjectName(ref.Name), nil, port)
}
