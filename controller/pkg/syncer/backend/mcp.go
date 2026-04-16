package agentgatewaybackend

import (
	"fmt"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/labels"

	"github.com/agentgateway/agentgateway/api"
	apiannotations "github.com/agentgateway/agentgateway/controller/api/annotations"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils"
)

const (
	// mcpProtocol specifies that streamable HTTP protocol is to be used for the MCP target
	mcpProtocol = "agentgateway.dev/mcp"

	// mcpProtocolSSE specifies that Server-Sent Events (SSE) protocol is to be used for the MCP target
	mcpProtocolSSE = "agentgateway.dev/mcp-sse"

	// mcpProtocolLegacy is the legacy protocol name for streamable HTTP, kept for backwards compatibility
	mcpProtocolLegacy = "kgateway.dev/mcp"

	// mcpProtocolSSELegacy is the legacy protocol name for SSE, kept for backwards compatibility
	mcpProtocolSSELegacy = "kgateway.dev/mcp-sse"
)

func TranslateMCPSelectorTargets(
	ctx plugins.PolicyCtx,
	namespace string,
	selector *agentgateway.McpSelector,
) ([]*api.MCPTarget, error) {
	// Krt only allows 1 filter per type, so we build a composite filter here
	generic := func(svc any) bool {
		return true
	}
	var nsFilter krt.FetchOption
	addFilter := func(nf func(svc any) bool) {
		og := generic
		generic = func(svc any) bool {
			return nf(svc) && og(svc)
		}
	}

	// Apply service filter
	if selector.Service != nil {
		serviceSelector, err := metav1.LabelSelectorAsSelector(selector.Service)
		if err != nil {
			return nil, fmt.Errorf("invalid service selector: %w", err)
		}
		if !serviceSelector.Empty() {
			addFilter(func(obj any) bool {
				service := obj.(*corev1.Service)
				return serviceSelector.Matches(labels.Set(service.Labels))
			})
		}
	}

	// Apply namespace selector
	if selector.Namespace != nil {
		namespaceSelector, err := metav1.LabelSelectorAsSelector(selector.Namespace)
		if err != nil {
			return nil, fmt.Errorf("invalid namespace selector: %w", err)
		}
		if !namespaceSelector.Empty() {
			allNamespaces := krt.Fetch(ctx.Krt, ctx.Collections.Namespaces)
			matchingNamespaces := make(map[string]bool)
			for _, ns := range allNamespaces {
				if namespaceSelector.Matches(labels.Set(ns.Labels)) {
					matchingNamespaces[ns.Name] = true
				}
			}
			addFilter(func(obj any) bool {
				service := obj.(*corev1.Service)
				return matchingNamespaces[service.Namespace]
			})
		}
	} else {
		nsFilter = krt.FilterIndex(ctx.Collections.ServicesByNamespace, namespace)
	}

	opts := []krt.FetchOption{krt.FilterGeneric(generic)}
	if nsFilter != nil {
		opts = append(opts, nsFilter)
	}

	matchingServices := krt.Fetch(ctx.Krt, ctx.Collections.Services, opts...)
	var mcpTargets []*api.MCPTarget
	for _, service := range matchingServices {
		for _, port := range service.Spec.Ports {
			appProtocol := ptr.OrEmpty(port.AppProtocol)
			if appProtocol != mcpProtocol && appProtocol != mcpProtocolSSE &&
				appProtocol != mcpProtocolLegacy && appProtocol != mcpProtocolSSELegacy {
				continue
			}
			targetName := service.Name + fmt.Sprintf("-%d", port.Port)
			if port.Name != "" {
				targetName = service.Name + "-" + port.Name
			}

			path := service.Annotations[apiannotations.MCPServiceHTTPPath]
			// use legacy annotation for backwards compatibility
			if path == "" && service.Annotations[apiannotations.LegacyMCPServiceHTTPPath] != "" {
				path = service.Annotations[apiannotations.LegacyMCPServiceHTTPPath]
			}

			svcHostname := kubeutils.ServiceFQDN(service.ObjectMeta)
			mcpTargets = append(mcpTargets, &api.MCPTarget{
				Name: targetName,
				Backend: &api.BackendReference{
					Kind: &api.BackendReference_Service_{
						Service: &api.BackendReference_Service{
							Hostname:  svcHostname,
							Namespace: service.Namespace,
						},
					},
					Port: uint32(port.Port), //nolint:gosec // G115: Kubernetes service ports are always positive
				},
				Protocol: toMCPProtocol(appProtocol),
				Path:     path,
			})
		}
	}
	return mcpTargets, nil
}

func ResolveMCPBackendRefHost(
	ctx plugins.PolicyCtx,
	namespace string,
	ref *corev1.LocalObjectReference,
) (string, error) {
	if ref == nil || ref.Name == "" {
		return "", fmt.Errorf("mcp backendRef name is required")
	}

	key := namespace + "/" + ref.Name
	service := ptr.Flatten(krt.FetchOne(ctx.Krt, ctx.Collections.Services, krt.FilterKey(key)))
	if service == nil {
		return "", fmt.Errorf("mcp backendRef service %s not found", key)
	}

	return kubeutils.ServiceFQDN(service.ObjectMeta), nil
}
