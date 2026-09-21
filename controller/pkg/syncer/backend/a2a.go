package agentgatewaybackend

import (
	"fmt"
	"strings"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/labels"
	"k8s.io/apimachinery/pkg/util/validation"

	"github.com/agentgateway/agentgateway/api"
	apiannotations "github.com/agentgateway/agentgateway/controller/api/annotations"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils"
)

const (
	// a2aAppProtocol marks a Service port as serving the A2A protocol.
	a2aAppProtocol = "agentgateway.dev/a2a"

	// a2aAppProtocolLegacy is the legacy annotation for A2A, kept for backwards compatibility.
	a2aAppProtocolLegacy = "kgateway.dev/a2a"
)

// TranslateA2ASelectorTargets discovers A2A services matching the given label
// selector and returns a list of A2ATarget entries. Each Service port with an
// A2A appProtocol produces one target.
func TranslateA2ASelectorTargets(
	ctx plugins.PolicyCtx,
	namespace string,
	selector *agentgateway.A2ASelector,
) ([]*api.A2ATarget, error) {
	// KRT only allows 1 filter per type, so compose filters into one.
	generic := func(_ any) bool { return true }
	var nsFilter krt.FetchOption
	addFilter := func(nf func(svc any) bool) {
		og := generic
		generic = func(svc any) bool {
			return nf(svc) && og(svc)
		}
	}

	// Apply service label filter.
	if selector.Services != nil {
		svcSelector, err := metav1.LabelSelectorAsSelector(selector.Services)
		if err != nil {
			return nil, fmt.Errorf("invalid service selector: %w", err)
		}
		if !svcSelector.Empty() {
			addFilter(func(obj any) bool {
				svc := obj.(*corev1.Service)
				return svcSelector.Matches(labels.Set(svc.Labels))
			})
		}
	}

	// Apply namespace label filter.
	if selector.Namespaces != nil {
		nsSelector, err := metav1.LabelSelectorAsSelector(selector.Namespaces)
		if err != nil {
			return nil, fmt.Errorf("invalid namespace selector: %w", err)
		}
		if !nsSelector.Empty() {
			allNamespaces := krt.Fetch(ctx.Krt, ctx.Collections.Namespaces)
			matchingNs := make(map[string]bool, len(allNamespaces))
			for _, ns := range allNamespaces {
				if nsSelector.Matches(labels.Set(ns.Labels)) {
					matchingNs[ns.Name] = true
				}
			}
			addFilter(func(obj any) bool {
				svc := obj.(*corev1.Service)
				return matchingNs[svc.Namespace]
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
	var targets []*api.A2ATarget

	for _, svc := range matchingServices {
		for _, port := range svc.Spec.Ports {
			appProtocol := ptr.OrEmpty(port.AppProtocol)
			if appProtocol != a2aAppProtocol && appProtocol != a2aAppProtocolLegacy {
				continue
			}

			targetName := svc.Annotations[apiannotations.A2AServiceTargetName]
			if targetName == "" {
				if port.Name != "" {
					targetName = svc.Name + "-" + port.Name
				} else {
					targetName = fmt.Sprintf("%s-%d", svc.Name, port.Port)
				}
			} else if errs := validation.IsDNS1123Subdomain(targetName); len(errs) > 0 {
				return nil, fmt.Errorf(
					"invalid Service %s/%s annotation %s value %q: must be a valid DNS-1123 subdomain: %s",
					svc.Namespace, svc.Name,
					apiannotations.A2AServiceTargetName, targetName,
					strings.Join(errs, "; "),
				)
			}

			path := svc.Annotations[apiannotations.A2AServiceHTTPPath]

			svcHostname := kubeutils.ServiceFQDN(svc.ObjectMeta)
			targets = append(targets, &api.A2ATarget{
				Name: targetName,
				Backend: &api.BackendReference{
					Kind: &api.BackendReference_Service_{
						Service: &api.BackendReference_Service{
							Hostname:  svcHostname,
							Namespace: svc.Namespace,
						},
					},
					Port: uint32(port.Port), //nolint:gosec // G115: Kubernetes service ports are always positive
				},
				Path:      path,
				Namespace: svc.Namespace,
			})
		}
	}

	return targets, nil
}

// ResolveA2ABackendRefHost looks up a Service by namespace-local name and
// returns its FQDN for use as a static backendRef target.
func ResolveA2ABackendRefHost(
	ctx plugins.PolicyCtx,
	namespace string,
	ref *corev1.LocalObjectReference,
) (string, error) {
	if ref == nil || ref.Name == "" {
		return "", fmt.Errorf("a2a backendRef name is required")
	}

	key := namespace + "/" + ref.Name
	svc := ptr.Flatten(krt.FetchOne(ctx.Krt, ctx.Collections.Services, krt.FilterKey(key)))
	if svc == nil {
		return "", fmt.Errorf("a2a backendRef service %s not found", key)
	}

	return kubeutils.ServiceFQDN(svc.ObjectMeta), nil
}

// translateA2ABackends translates an A2ABackend with targets into proto Backend(s).
// Static host targets produce sub-backends (like MCP static host targets).
// Selector targets are discovered via TranslateA2ASelectorTargets.
func translateA2ABackends(
	ctx plugins.PolicyCtx,
	be *agentgateway.AgentgatewayBackend,
	a2a *agentgateway.A2ABackend,
	inlinePolicies []*api.BackendPolicySpec,
) ([]*api.Backend, error) {
	var a2aTargets []*api.A2ATarget
	var backends []*api.Backend

	for _, target := range a2a.Targets {
		switch {
		case target.Static != nil:
			s := target.Static
			port := a2a.Port
			if s.Port != nil {
				port = *s.Port
			}

			switch {
			case s.BackendRef != nil:
				// In-cluster Service reference
				svcHostname, err := ResolveA2ABackendRefHost(ctx, be.Namespace, s.BackendRef)
				if err != nil {
					return nil, err
				}
				a2aTargets = append(a2aTargets, &api.A2ATarget{
					Name: string(target.Name),
					Backend: &api.BackendReference{
						Kind: &api.BackendReference_Service_{
							Service: &api.BackendReference_Service{
								Hostname:  svcHostname,
								Namespace: be.Namespace,
							},
						},
						Port: uint32(port), //nolint:gosec // G115: validated by the CRD schema
					},
					Path:      ptr.OrEmpty(s.Path),
					Namespace: be.Namespace,
				})

			case s.Host != nil:
				// External host → create a sub-backend
				subKey := utils.InternalMCPStaticBackendName(be.Namespace, be.Name, string(target.Name))
				subBackend := &api.Backend{
					Key:  subKey,
					Name: plugins.ResourceName(be),
					Kind: &api.Backend_Static{
						Static: &api.StaticBackend{
							Host: *s.Host,
							Port: port,
						},
					},
				}
				backends = append(backends, subBackend)

				a2aTargets = append(a2aTargets, &api.A2ATarget{
					Name: string(target.Name),
					Backend: &api.BackendReference{
						Kind: &api.BackendReference_Backend{
							Backend: subKey,
						},
					},
					Path:      ptr.OrEmpty(s.Path),
					Namespace: be.Namespace,
				})
			}

		case target.Selector != nil:
			discovered, err := TranslateA2ASelectorTargets(ctx, be.Namespace, target.Selector)
			if err != nil {
				return nil, fmt.Errorf("target %s: %w", target.Name, err)
			}
			// Deduplicate names within the full target set.
			// Use the service name (or annotation override) directly since URL path
			// segments cannot contain '/'.
			seen := make(map[string]int, len(a2aTargets))
			for _, t := range a2aTargets {
				seen[t.Name]++
			}
			for _, d := range discovered {
				if seen[d.Name] > 0 {
					d.Name = fmt.Sprintf("%s-%d", d.Name, seen[d.Name])
				}
				seen[d.Name]++
			}
			a2aTargets = append(a2aTargets, discovered...)
		}
	}

	a2aBackend := &api.Backend{
		Key:  be.Namespace + "/" + be.Name,
		Name: plugins.ResourceName(be),
		Kind: &api.Backend_A2A{
			A2A: &api.A2ABackend{
				Targets: a2aTargets,
			},
		},
		InlinePolicies: inlinePolicies,
	}
	backends = append(backends, a2aBackend)
	return backends, nil
}
