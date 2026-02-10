package agentgatewaysyncer

import (
	"cmp"
	"context"
	"fmt"
	"slices"
	"time"

	"github.com/avast/retry-go/v4"
	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/kclient"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	"k8s.io/client-go/tools/cache"
	"sigs.k8s.io/controller-runtime/pkg/manager"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gwv1a2 "sigs.k8s.io/gateway-api/apis/v1alpha2"
	gwxv1a1 "sigs.k8s.io/gateway-api/apisx/v1alpha1"

	"github.com/kgateway-dev/kgateway/v2/api/v1alpha1/agentgateway"
	agwplugins "github.com/kgateway-dev/kgateway/v2/pkg/agentgateway/plugins"
	"github.com/kgateway-dev/kgateway/v2/pkg/apiclient"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/agentgatewaysyncer/status"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/wellknown"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/stopwatch"
)

var _ manager.LeaderElectionRunnable = &AgentGwStatusSyncer{}

const (
	// Retry configuration constants for status updates
	maxRetryAttempts = 5
	retryDelay       = 100 * time.Millisecond

	// Log message keys
	logKeyError = "error"
)

// AgentGwStatusSyncer runs only on the leader and syncs the status of agent gateway resources.
// It subscribes to the report queues, parses and updates the resource status.
type AgentGwStatusSyncer struct {
	client apiclient.Client

	agentgatewayPolicies StatusSyncer[*agentgateway.AgentgatewayPolicy, *gwv1.PolicyStatus]
	agentgatewayBackends StatusSyncer[*agentgateway.AgentgatewayBackend, *agentgateway.AgentgatewayBackendStatus]

	// Configuration
	controllerName string
	agwClassName   string

	statusCollections *status.StatusCollections

	cacheSyncs []cache.InformerSynced

	listenerSets StatusSyncer[*gwxv1a1.XListenerSet, *gwxv1a1.ListenerSetStatus]
	gateways     StatusSyncer[*gwv1.Gateway, *gwv1.GatewayStatus]
	httpRoutes   StatusSyncer[*gwv1.HTTPRoute, *gwv1.HTTPRouteStatus]
	grpcRoutes   StatusSyncer[*gwv1.GRPCRoute, *gwv1.GRPCRouteStatus]
	tcpRoutes    StatusSyncer[*gwv1a2.TCPRoute, *gwv1a2.TCPRouteStatus]
	tlsRoutes    StatusSyncer[*gwv1a2.TLSRoute, *gwv1a2.TLSRouteStatus]

	extraAgwResourceStatusHandlers map[schema.GroupVersionKind]agwplugins.AgwResourceStatusSyncHandler
}

func NewAgwStatusSyncer(
	controllerName string,
	agwClassName string,
	client apiclient.Client,
	statusCollections *status.StatusCollections,
	cacheSyncs []cache.InformerSynced,
	extraHandlers map[schema.GroupVersionKind]agwplugins.AgwResourceStatusSyncHandler,
) *AgentGwStatusSyncer {
	f := kclient.Filter{ObjectFilter: client.ObjectFilter()}
	syncer := &AgentGwStatusSyncer{
		controllerName:                 controllerName,
		agwClassName:                   agwClassName,
		client:                         client,
		statusCollections:              statusCollections,
		cacheSyncs:                     cacheSyncs,
		extraAgwResourceStatusHandlers: extraHandlers,

		agentgatewayPolicies: StatusSyncer[*agentgateway.AgentgatewayPolicy, *gwv1.PolicyStatus]{
			name:           "agentgatewayPolicy",
			controllerName: controllerName,
			client:         kclient.NewFilteredDelayed[*agentgateway.AgentgatewayPolicy](client, wellknown.AgentgatewayPolicyGVR, f),
			build: func(om metav1.ObjectMeta, s *gwv1.PolicyStatus) *agentgateway.AgentgatewayPolicy {
				return &agentgateway.AgentgatewayPolicy{
					ObjectMeta: om,
					Status: gwv1.PolicyStatus{
						Ancestors: s.Ancestors,
					},
				}
			},
		},
		agentgatewayBackends: StatusSyncer[*agentgateway.AgentgatewayBackend, *agentgateway.AgentgatewayBackendStatus]{
			name:           "agentgatewayPolicy",
			controllerName: controllerName,
			client:         kclient.NewFilteredDelayed[*agentgateway.AgentgatewayBackend](client, wellknown.AgentgatewayBackendGVR, f),
			build: func(om metav1.ObjectMeta, s *agentgateway.AgentgatewayBackendStatus) *agentgateway.AgentgatewayBackend {
				return &agentgateway.AgentgatewayBackend{
					ObjectMeta: om,
					Status:     *s,
				}
			},
		},
		httpRoutes: StatusSyncer[*gwv1.HTTPRoute, *gwv1.HTTPRouteStatus]{
			name:           "httpRoute",
			controllerName: controllerName,
			client:         kclient.NewFilteredDelayed[*gwv1.HTTPRoute](client, wellknown.HTTPRouteGVR, f),
			build: func(om metav1.ObjectMeta, s *gwv1.HTTPRouteStatus) *gwv1.HTTPRoute {
				return &gwv1.HTTPRoute{
					ObjectMeta: om,
					Status:     *s,
				}
			},
		},
		grpcRoutes: StatusSyncer[*gwv1.GRPCRoute, *gwv1.GRPCRouteStatus]{
			name:           "grpcRoute",
			controllerName: controllerName,
			client:         kclient.NewFilteredDelayed[*gwv1.GRPCRoute](client, wellknown.GRPCRouteGVR, f),
			build: func(om metav1.ObjectMeta, s *gwv1.GRPCRouteStatus) *gwv1.GRPCRoute {
				return &gwv1.GRPCRoute{
					ObjectMeta: om,
					Status:     *s,
				}
			},
		},
		tlsRoutes: StatusSyncer[*gwv1a2.TLSRoute, *gwv1a2.TLSRouteStatus]{
			name:           "tlsRoute",
			controllerName: controllerName,
			client:         kclient.NewFilteredDelayed[*gwv1a2.TLSRoute](client, wellknown.TLSRouteGVR, f),
			build: func(om metav1.ObjectMeta, s *gwv1a2.TLSRouteStatus) *gwv1a2.TLSRoute {
				return &gwv1a2.TLSRoute{
					ObjectMeta: om,
					Status:     *s,
				}
			},
		},
		tcpRoutes: StatusSyncer[*gwv1a2.TCPRoute, *gwv1a2.TCPRouteStatus]{
			name:           "tcpRoute",
			controllerName: controllerName,
			client:         kclient.NewFilteredDelayed[*gwv1a2.TCPRoute](client, wellknown.TCPRouteGVR, f),
			build: func(om metav1.ObjectMeta, s *gwv1a2.TCPRouteStatus) *gwv1a2.TCPRoute {
				return &gwv1a2.TCPRoute{
					ObjectMeta: om,
					Status:     *s,
				}
			},
		},
		listenerSets: StatusSyncer[*gwxv1a1.XListenerSet, *gwxv1a1.ListenerSetStatus]{
			name:           "listenerSet",
			controllerName: controllerName,
			client:         kclient.NewFilteredDelayed[*gwxv1a1.XListenerSet](client, wellknown.XListenerSetGVR, f),
			build: func(om metav1.ObjectMeta, s *gwxv1a1.ListenerSetStatus) *gwxv1a1.XListenerSet {
				return &gwxv1a1.XListenerSet{
					ObjectMeta: om,
					Status:     *s,
				}
			},
		},
		gateways: StatusSyncer[*gwv1.Gateway, *gwv1.GatewayStatus]{
			name:           "gateway",
			controllerName: controllerName,
			client:         kclient.NewFilteredDelayed[*gwv1.Gateway](client, wellknown.GatewayGVR, f),
			build: func(om metav1.ObjectMeta, s *gwv1.GatewayStatus) *gwv1.Gateway {
				return &gwv1.Gateway{
					ObjectMeta: om,
					Status:     *s,
				}
			},
		},
	}

	return syncer
}

func (s *AgentGwStatusSyncer) Start(ctx context.Context) error {
	logger.Info("starting agentgateway Status Syncer", "controllername", s.controllerName)
	logger.Info("waiting for agentgateway cache to sync")

	// wait for krt collections to sync
	logger.Info("waiting for cache to sync")
	s.client.WaitForCacheSync(
		"agent gateway status syncer",
		ctx.Done(),
		s.cacheSyncs...,
	)
	s.client.WaitForCacheSync(
		"agent gateway status clients",
		ctx.Done(),
		s.listenerSets.client.HasSynced,
		s.gateways.client.HasSynced,
		s.httpRoutes.client.HasSynced,
		s.grpcRoutes.client.HasSynced,
		s.tcpRoutes.client.HasSynced,
		s.tlsRoutes.client.HasSynced,
	)

	logger.Info("caches warm!")

	// Create a controllers.Queue that wraps our async queue for Istio's StatusCollections
	// The policyStatusQueue implements https://github.com/istio/istio/blob/531c61709aaa9bc9187c625e9e460be98f2abf2e/pilot/pkg/status/manager.go#L107
	nq := s.NewStatusWorker(ctx)
	s.statusCollections.SetQueue(nq)

	<-ctx.Done()
	return nil
}

func (s *AgentGwStatusSyncer) SyncStatus(ctx context.Context, resource status.Resource, statusObj any) {
	switch resource.GroupVersionKind {
	case wellknown.GatewayGVK:
		s.gateways.ApplyStatus(ctx, resource, statusObj)
	case wellknown.XListenerSetGVK:
		s.listenerSets.ApplyStatus(ctx, resource, statusObj)
	case wellknown.GRPCRouteGVK:
		s.grpcRoutes.ApplyStatus(ctx, resource, statusObj)
	case wellknown.TLSRouteGVK:
		s.tlsRoutes.ApplyStatus(ctx, resource, statusObj)
	case wellknown.TCPRouteGVK:
		s.tcpRoutes.ApplyStatus(ctx, resource, statusObj)
	case wellknown.HTTPRouteGVK:
		s.httpRoutes.ApplyStatus(ctx, resource, statusObj)
	case wellknown.AgentgatewayPolicyGVK:
		s.agentgatewayPolicies.ApplyStatus(ctx, resource, statusObj)
	case wellknown.AgentgatewayBackendGVK:
		s.agentgatewayBackends.ApplyStatus(ctx, resource, statusObj)
	default:
		// Attempt to handle resource policy kinds via registered handlers.
		if s.extraAgwResourceStatusHandlers != nil {
			key := resource.GroupVersionKind
			if handler, ok := s.extraAgwResourceStatusHandlers[key]; ok {
				if err := handler(ctx, s.client, types.NamespacedName{Name: resource.Name, Namespace: resource.Namespace}, statusObj); err != nil {
					logger.Error("external policy status handler failed", "gvk", resource.GroupVersionKind.String(), logKeyError, err)
				}
				return
			}
		}
		logger.Error("sync status: unknown resource type", "gvk", resource.GroupVersionKind.String())
	}
}

func (s *AgentGwStatusSyncer) NewStatusWorker(ctx context.Context) *status.WorkerPool {
	return status.NewWorkerPool(ctx, s.SyncStatus, 100)
}

type StatusSyncer[O controllers.ComparableObject, S any] struct {
	// Name for logging
	name string

	// controllerName is the controller whose status entries this syncer owns.
	// We preserve entries owned by other controllers and only publish entries owned by this controller. This
	// avoids clobbering status from other controllers or subsystems.
	controllerName string

	client kclient.Client[O]

	build func(om metav1.ObjectMeta, s S) O
}

func (s StatusSyncer[O, S]) ApplyStatus(ctx context.Context, obj status.Resource, statusObj any) {
	status := statusObj.(S)
	stopwatch := stopwatch.NewTranslatorStopWatch(s.name + "Status")
	stopwatch.Start()
	defer stopwatch.Stop(ctx)

	logger := logger.With("kind", s.name, "resource", obj.NamespacedName.String())
	// TODO: move this to retry by putting it back on the queue, with some limit on the retry attempts allowed
	err := retry.Do(func() error {
		// Fetch the current object so we can preserve status written by other controllers/subsystems.
		// NOTE: This is especially important for Gateway.status.addresses (written by the gateway reconciler),
		// and for Route status Parents (multi-controller field).
		current := s.client.Get(obj.Name, obj.Namespace)
		if controllers.IsNil(current) {
			// Harmless race: status write after resource was deleted.
			logger.Debug("resource not found, skipping status update")
			return nil
		}

		mergedAny := any(status)
		switch desired := mergedAny.(type) {
		case *gwv1.PolicyStatus:
			// PolicyStatus is multi-writer across controllers, so preserve entries not owned by our controller.
			// NOTE: We can only merge if the current object is the expected type.
			curPol, ok := any(current).(*agentgateway.AgentgatewayPolicy)
			if ok {
				merged := *desired
				merged.Ancestors = mergePolicyAncestorStatuses(s.controllerName, curPol.Status.Ancestors, desired.Ancestors)
				mergedAny = &merged
			}
		case *gwv1.GatewayStatus:
			// Preserve addresses unless the desired status explicitly sets them.
			// Addresses are computed from the generated Service by the gateway reconciler and are not
			// part of the agentgateway translation report.
			curGw, ok := any(current).(*gwv1.Gateway)
			if ok {
				merged := *desired
				merged.Addresses = mergeGatewayAddresses(curGw.Status.Addresses, desired.Addresses)
				mergedAny = &merged
			}
		case *gwv1.HTTPRouteStatus:
			cur, ok := any(current).(*gwv1.HTTPRoute)
			if ok {
				merged := *desired
				merged.Parents = mergeRouteParentStatuses(s.controllerName, cur.Status.Parents, desired.Parents)
				mergedAny = &merged
			}
		case *gwv1.GRPCRouteStatus:
			cur, ok := any(current).(*gwv1.GRPCRoute)
			if ok {
				merged := *desired
				merged.Parents = mergeRouteParentStatuses(s.controllerName, cur.Status.Parents, desired.Parents)
				mergedAny = &merged
			}
		case *gwv1a2.TCPRouteStatus:
			cur, ok := any(current).(*gwv1a2.TCPRoute)
			if ok {
				merged := *desired
				merged.Parents = mergeRouteParentStatuses(s.controllerName, cur.Status.Parents, desired.Parents)
				mergedAny = &merged
			}
		case *gwv1a2.TLSRouteStatus:
			cur, ok := any(current).(*gwv1a2.TLSRoute)
			if ok {
				merged := *desired
				merged.Parents = mergeRouteParentStatuses(s.controllerName, cur.Status.Parents, desired.Parents)
				mergedAny = &merged
			}
		}

		merged, ok := mergedAny.(S)
		if !ok {
			// This should never happen; indicates a mismatch between the syncer's type parameter S
			// and the object being published.
			logger.Error("unexpected status type; skipping status update", "status_type", fmt.Sprintf("%T", mergedAny))
			return nil
		}

		// Prefer the latest resourceVersion to avoid avoidable conflicts.
		// Conflicts are still handled (and expected), but using the latest RV reduces churn.
		rv := obj.ResourceVersion
		if crv := current.GetResourceVersion(); crv != "" {
			rv = crv
		}

		// Pass only the status and minimal part of ObjectMetadata to find the resource and validate it.
		// Passing Spec is ignored by the API server but has costs.
		// Passing ResourceVersion is important to ensure we are not writing stale data. The collection is responsible for
		// re-enqueuing a resource if it ends up being rejected due to stale ResourceVersion.
		_, err := s.client.UpdateStatus(s.build(metav1.ObjectMeta{
			Name:            obj.Name,
			Namespace:       obj.Namespace,
			ResourceVersion: rv,
		}, merged))
		if err != nil {
			if apierrors.IsConflict(err) {
				// This is normal. It is expected the collection will re-enqueue the write
				logger.Debug("updating stale status, skipping", logKeyError, err)
				return nil
			}
			if apierrors.IsNotFound(err) {
				// ignore status write after resource was deleted.
				logger.Debug("resource not found, skipping status update", logKeyError, err)
				return nil
			}
			logger.Error("error updating status", logKeyError, err)
			return err
		}
		logger.Debug("updated status")
		return nil
	}, retry.Attempts(maxRetryAttempts), retry.Delay(retryDelay))

	if err != nil {
		logger.Error("failed to sync status after retries", logKeyError, err, "policy", obj.NamespacedName.String())
	} else {
		logger.Debug("updated policy status")
	}
}

func mergePolicyAncestorStatuses(ourControllerName string, existing []gwv1.PolicyAncestorStatus, desired []gwv1.PolicyAncestorStatus) []gwv1.PolicyAncestorStatus {
	out := make([]gwv1.PolicyAncestorStatus, 0, len(existing)+len(desired))

	// Preserve any entries not owned by our controller.
	for _, a := range existing {
		if string(a.ControllerName) != ourControllerName {
			out = append(out, a)
		}
	}

	// Only add entries owned by our controller from the desired status.
	// This ensures we can clear stale entries by publishing an empty desired list.
	ours := make([]gwv1.PolicyAncestorStatus, 0, len(desired))
	for _, a := range desired {
		if string(a.ControllerName) == ourControllerName {
			ours = append(ours, a)
		}
	}

	// Ensure stable ordering of our entries so status doesn't flap due to map/set iteration upstream.
	slices.SortFunc(ours, func(a, b gwv1.PolicyAncestorStatus) int {
		if c := cmp.Compare(string(a.ControllerName), string(b.ControllerName)); c != 0 {
			return c
		}
		return compareParentReference(a.AncestorRef, b.AncestorRef)
	})

	out = append(out, ours...)
	return out
}

func mergeRouteParentStatuses(ourControllerName string, existing []gwv1.RouteParentStatus, desired []gwv1.RouteParentStatus) []gwv1.RouteParentStatus {
	out := make([]gwv1.RouteParentStatus, 0, len(existing)+len(desired))

	// Preserve any entries not owned by our controller.
	for _, a := range existing {
		if string(a.ControllerName) != ourControllerName {
			out = append(out, a)
		}
	}

	// Only add entries owned by our controller from the desired status.
	// This ensures we can clear stale entries by publishing an empty desired list.
	ours := make([]gwv1.RouteParentStatus, 0, len(desired))
	for _, a := range desired {
		if string(a.ControllerName) == ourControllerName {
			ours = append(ours, a)
		}
	}

	// Ensure stable ordering of our entries so status doesn't flap due to map/set iteration upstream.
	slices.SortFunc(ours, func(a, b gwv1.RouteParentStatus) int {
		if c := cmp.Compare(string(a.ControllerName), string(b.ControllerName)); c != 0 {
			return c
		}
		return compareParentReference(a.ParentRef, b.ParentRef)
	})

	out = append(out, ours...)
	return out
}

func mergeGatewayAddresses(existing []gwv1.GatewayStatusAddress, desired []gwv1.GatewayStatusAddress) []gwv1.GatewayStatusAddress {
	var out []gwv1.GatewayStatusAddress
	if len(desired) > 0 {
		out = append([]gwv1.GatewayStatusAddress(nil), desired...)
	} else {
		out = append([]gwv1.GatewayStatusAddress(nil), existing...)
	}

	// Ensure stable ordering so status doesn't flap due to upstream iteration order.
	slices.SortFunc(out, func(a, b gwv1.GatewayStatusAddress) int {
		if c := cmp.Compare(addressTypeOrDefault(a.Type), addressTypeOrDefault(b.Type)); c != 0 {
			return c
		}
		return cmp.Compare(a.Value, b.Value)
	})

	return out
}

func compareParentReference(a, b gwv1.ParentReference) int {
	// ParentReference includes pointer fields with defaults. Canonicalize those defaults so nil vs explicitly-set
	// default values don't introduce ordering churn.
	if c := cmp.Compare(parentRefGroupOrDefault(a.Group), parentRefGroupOrDefault(b.Group)); c != 0 {
		return c
	}
	if c := cmp.Compare(parentRefKindOrDefault(a.Kind), parentRefKindOrDefault(b.Kind)); c != 0 {
		return c
	}
	if c := cmp.Compare(derefStringPtr(a.Namespace), derefStringPtr(b.Namespace)); c != 0 {
		return c
	}
	if c := cmp.Compare(string(a.Name), string(b.Name)); c != 0 {
		return c
	}
	if c := cmp.Compare(derefStringPtr(a.SectionName), derefStringPtr(b.SectionName)); c != 0 {
		return c
	}
	return comparePortNumberPtr(a.Port, b.Port)
}

func parentRefGroupOrDefault(g *gwv1.Group) string {
	if g == nil {
		// ParentReference.Group default.
		return "gateway.networking.k8s.io"
	}
	return string(*g)
}

func parentRefKindOrDefault(k *gwv1.Kind) string {
	if k == nil {
		// ParentReference.Kind default.
		return "Gateway"
	}
	return string(*k)
}

func derefStringPtr[S ~string](p *S) string {
	if p == nil {
		return ""
	}
	return string(*p)
}

func comparePortNumberPtr(a, b *gwv1.PortNumber) int {
	switch {
	case a == nil && b == nil:
		return 0
	case a == nil:
		return -1
	case b == nil:
		return 1
	default:
		return cmp.Compare(int(*a), int(*b))
	}
}

func addressTypeOrDefault(t *gwv1.AddressType) string {
	if t == nil {
		// GatewayStatusAddress.Type default.
		return "IPAddress"
	}
	return string(*t)
}

// NeedLeaderElection returns true to ensure that the AgentGwStatusSyncer runs only on the leader
func (s *AgentGwStatusSyncer) NeedLeaderElection() bool {
	return true
}
