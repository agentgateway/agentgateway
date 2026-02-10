package plugins

import (
	"fmt"

	"github.com/agentgateway/agentgateway/go/api"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"

	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/wellknown"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/kubeutils"
)

const (
	a2aProtocol = "kgateway.dev/a2a"
)

// NewA2APlugin creates a new A2A policy plugin
func NewA2APlugin(agw *AgwCollections) AgwPlugin {
	policyCol := krt.NewManyCollection(agw.Services, func(krtctx krt.HandlerContext, svc *corev1.Service) []AgwPolicy {
		return translatePoliciesForService(svc, kubeutils.GetClusterDomainName())
	})
	return AgwPlugin{
		ContributesPolicies: map[schema.GroupKind]PolicyPlugin{
			wellknown.ServiceGVK.GroupKind(): {
				Policies: policyCol,
			},
		},
		ExtraHasSynced: func() bool {
			return policyCol.HasSynced()
		},
	}
}

// translatePoliciesForService generates A2A policies for a single service
func translatePoliciesForService(svc *corev1.Service, clusterDomain string) []AgwPolicy {
	var a2aPolicies []AgwPolicy

	for _, port := range svc.Spec.Ports {
		if port.AppProtocol != nil && *port.AppProtocol == a2aProtocol {
			logger.Debug("found A2A service", "service", svc.Name, "namespace", svc.Namespace, "port", port.Port)
			hostname := fmt.Sprintf("%s.%s.svc.%s", svc.Name, svc.Namespace, clusterDomain)
			policy := &api.Policy{
				Key: fmt.Sprintf("a2a/%s/%s/%d", svc.Namespace, svc.Name, port.Port),
				// TODO: this is awkward since its doesn't include a Kind..
				Name: TypedResourceName(wellknown.ServiceKind, svc),
				Target: &api.PolicyTarget{Kind: &api.PolicyTarget_Service{Service: &api.PolicyTarget_ServiceTarget{
					Namespace: svc.Namespace,
					Hostname:  hostname,
					Port:      ptr.Of(uint32(port.Port)), // nolint:gosec // G115: kubebuilder validation ensures safe for uint32
				}}},
				Kind: &api.Policy_Backend{
					Backend: &api.BackendPolicySpec{
						Kind: &api.BackendPolicySpec_A2A_{
							A2A: &api.BackendPolicySpec_A2A{},
						},
					},
				},
			}

			a2aPolicies = append(a2aPolicies, AgwPolicy{Policy: policy})
		}
	}

	return a2aPolicies
}
