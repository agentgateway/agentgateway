package agentgatewaybackend

import (
	"fmt"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/labels"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
)

// TranslatePoolBackend translates a pool backend into an xDS Backend resource.
// It discovers pods matching the label selector and builds a PoolBackend with
// their addresses as endpoints.
func TranslatePoolBackend(
	ctx plugins.PolicyCtx,
	backend *agentgateway.AgentgatewayBackend,
	pool *agentgateway.PoolBackend,
	inlinePolicies []*api.BackendPolicySpec,
) (*api.Backend, error) {
	podSelector, err := metav1.LabelSelectorAsSelector(&pool.Selector)
	if err != nil {
		return nil, fmt.Errorf("invalid pool selector: %w", err)
	}

	// Find all pods in the backend's namespace matching the selector.
	allPods := krt.Fetch(ctx.Krt, ctx.Collections.Pods,
		krt.FilterGeneric(func(obj any) bool {
			pod := obj.(*corev1.Pod)
			return pod.Namespace == backend.Namespace &&
				podSelector.Matches(labels.Set(pod.Labels))
		}),
	)

	var endpoints []*api.PoolEndpoint
	for _, pod := range allPods {
		if pod.Status.PodIP == "" {
			continue
		}
		if pod.Status.Phase != corev1.PodRunning {
			continue
		}
		endpoints = append(endpoints, &api.PoolEndpoint{
			Address: pod.Status.PodIP,
			Name:    pod.Name,
		})
	}

	stateful := pool.SessionRouting == agentgateway.Stateful

	return &api.Backend{
		Key:  backend.Namespace + "/" + backend.Name,
		Name: plugins.ResourceName(backend),
		Kind: &api.Backend_Pool{
			Pool: &api.PoolBackend{
				Endpoints:  endpoints,
				Port:       uint32(pool.Port),
				Stateful:   stateful,
				SessionKey: ptr.OrEmpty(pool.SessionKey),
			},
		},
		InlinePolicies: inlinePolicies,
	}, nil
}
