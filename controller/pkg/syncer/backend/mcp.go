package agentgatewaybackend

import (
	"fmt"
	"strings"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/slices"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/labels"
	"k8s.io/apimachinery/pkg/util/validation"

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
) ([]mcpServiceTarget, error) {
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

	matchingServices := slices.SortBy(krt.Fetch(ctx.Krt, ctx.Collections.Services, opts...), func(s *corev1.Service) string {
		return s.Namespace + "/" + s.Name
	})
	var mcpTargets []mcpServiceTarget
	for _, service := range matchingServices {
		for _, port := range service.Spec.Ports {
			appProtocol := ptr.OrEmpty(port.AppProtocol)
			if appProtocol != mcpProtocol && appProtocol != mcpProtocolSSE &&
				appProtocol != mcpProtocolLegacy && appProtocol != mcpProtocolSSELegacy {
				continue
			}
			targetName := service.Annotations[apiannotations.MCPServiceTargetName]
			if targetName == "" {
				targetName = service.Name + fmt.Sprintf("-%d", port.Port)
				if port.Name != "" {
					targetName = service.Name + "-" + port.Name
				}
			} else if errs := validation.IsDNS1123Subdomain(targetName); len(errs) > 0 {
				return nil, fmt.Errorf(
					"invalid Service %s/%s annotation %s value %q: must be a valid Gateway API SectionName: %s",
					service.Namespace,
					service.Name,
					apiannotations.MCPServiceTargetName,
					targetName,
					strings.Join(errs, "; "),
				)
			}

			path := service.Annotations[apiannotations.MCPServiceHTTPPath]
			// use legacy annotation for backwards compatibility
			if path == "" && service.Annotations[apiannotations.LegacyMCPServiceHTTPPath] != "" {
				path = service.Annotations[apiannotations.LegacyMCPServiceHTTPPath]
			}

			svcHostname := kubeutils.ServiceFQDN(service.ObjectMeta)
			mcpTargets = append(mcpTargets, mcpServiceTarget{service: service, target: &api.MCPTarget{
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
			}})
		}
	}
	return mcpTargets, nil
}

type mcpServiceTarget struct {
	service *corev1.Service
	target  *api.MCPTarget
}

// dedupeMCPTargets drops targets whose name is already used by a preferred target.
// Static targets always win; among Services, prefer the Backend's namespace, then
// Services without the target name annotation, then the oldest.
func dedupeMCPTargets(
	namespace string,
	targets []*api.MCPTarget,
	services map[*api.MCPTarget]*corev1.Service,
) ([]*api.MCPTarget, []error) {
	winners := map[string]*api.MCPTarget{}
	for _, t := range targets {
		cur, ok := winners[t.Name]
		if !ok || preferMCPService(namespace, services[t], services[cur]) {
			winners[t.Name] = t
		}
	}
	var errs []error
	res := make([]*api.MCPTarget, 0, len(winners))
	for _, t := range targets {
		w := winners[t.Name]
		if w == t {
			res = append(res, t)
			continue
		}
		svc := services[t]
		winner := "static target"
		if ws := services[w]; ws != nil {
			winner = "Service " + ws.Namespace + "/" + ws.Name
		}
		errs = append(errs, fmt.Errorf(
			"MCP target %q from Service %s/%s conflicts with %s; ignored", t.Name, svc.Namespace, svc.Name, winner))
	}
	return res, errs
}

// preferMCPService reports whether a should replace b. A nil Service is a static target.
func preferMCPService(namespace string, a, b *corev1.Service) bool {
	if b == nil {
		return false
	}
	if a == nil {
		return true
	}
	if aLocal, bLocal := a.Namespace == namespace, b.Namespace == namespace; aLocal != bLocal {
		return aLocal
	}
	aRenamed := a.Annotations[apiannotations.MCPServiceTargetName] != ""
	bRenamed := b.Annotations[apiannotations.MCPServiceTargetName] != ""
	if aRenamed != bRenamed {
		return bRenamed
	}
	if !a.CreationTimestamp.Equal(&b.CreationTimestamp) {
		return a.CreationTimestamp.Before(&b.CreationTimestamp)
	}
	return a.Namespace+"/"+a.Name < b.Namespace+"/"+b.Name
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
