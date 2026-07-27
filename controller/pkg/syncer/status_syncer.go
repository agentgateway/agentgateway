package syncer

import (
	"cmp"
	"context"
	"fmt"
	"time"

	"github.com/avast/retry-go/v4"
	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/kclient"
	"istio.io/istio/pkg/slices"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/client-go/tools/cache"
	"sigs.k8s.io/controller-runtime/pkg/manager"
	inf "sigs.k8s.io/gateway-api-inference-extension/api/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gwxv1a1 "sigs.k8s.io/gateway-api/apisx/v1alpha1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
	"github.com/agentgateway/agentgateway/controller/pkg/syncer/status"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
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

	agentgatewayPolicies StatusSyncer[*agentgateway.AgentgatewayPolicy, gwv1.PolicyStatus]
	agentgatewayBackends StatusSyncer[*agentgateway.AgentgatewayBackend, agentgateway.AgentgatewayBackendStatus]

	// Configuration
	controllerName string
	agwClassName   string

	statusCollections *status.StatusCollections

	cacheSyncs []cache.InformerSynced

	listenerSets       StatusSyncer[*gwv1.ListenerSet, *gwv1.ListenerSetStatus]
	gateways           StatusSyncer[*gwv1.Gateway, *gwv1.GatewayStatus]
	httpRoutes         StatusSyncer[*gwv1.HTTPRoute, *gwv1.HTTPRouteStatus]
	grpcRoutes         StatusSyncer[*gwv1.GRPCRoute, *gwv1.GRPCRouteStatus]
	tcpRoutes          StatusSyncer[*gwv1.TCPRoute, *gwv1.TCPRouteStatus]
	tlsRoutes          StatusSyncer[*gwv1.TLSRoute, *gwv1.TLSRouteStatus]
	backendTLSPolicies StatusSyncer[*gwv1.BackendTLSPolicy, gwv1.PolicyStatus]
	xBackends          StatusSyncer[*gwxv1a1.XBackend, *gwxv1a1.BackendStatus]
	inferencePools     StatusSyncer[*inf.InferencePool, inf.InferencePoolStatus]

	extraAgwResourceStatusHandlers map[schema.GroupVersionKind]ResourceStatusSyncer
}

func NewAgwStatusSyncer(
	controllerName string,
	agwClassName string,
	client apiclient.Client,
	statusCollections *status.StatusCollections,
	cacheSyncs []cache.InformerSynced,
	extraHandlers map[schema.GroupVersionKind]ResourceStatusSyncer,
	enableInference bool,
) *AgentGwStatusSyncer {
	f := kclient.Filter{ObjectFilter: client.ObjectFilter()}
	syncer := &AgentGwStatusSyncer{
		controllerName:                 controllerName,
		agwClassName:                   agwClassName,
		client:                         client,
		statusCollections:              statusCollections,
		cacheSyncs:                     cacheSyncs,
		extraAgwResourceStatusHandlers: extraHandlers,

		agentgatewayPolicies: StatusSyncer[*agentgateway.AgentgatewayPolicy, gwv1.PolicyStatus]{
			Name:           "agentgatewayPolicy",
			ControllerName: controllerName,
			Client:         kclient.NewFilteredDelayed[*agentgateway.AgentgatewayPolicy](client, wellknown.AgentgatewayPolicyGVR, f),
			Build: func(om metav1.ObjectMeta, s gwv1.PolicyStatus) *agentgateway.AgentgatewayPolicy {
				return &agentgateway.AgentgatewayPolicy{
					ObjectMeta: om,
					Status: gwv1.PolicyStatus{
						Ancestors: s.Ancestors,
					},
				}
			},
		},
		agentgatewayBackends: StatusSyncer[*agentgateway.AgentgatewayBackend, agentgateway.AgentgatewayBackendStatus]{
			Name:           "agentgatewayBackend",
			ControllerName: controllerName,
			Client:         kclient.NewFilteredDelayed[*agentgateway.AgentgatewayBackend](client, wellknown.AgentgatewayBackendGVR, f),
			Build: func(om metav1.ObjectMeta, s agentgateway.AgentgatewayBackendStatus) *agentgateway.AgentgatewayBackend {
				return &agentgateway.AgentgatewayBackend{
					ObjectMeta: om,
					Status:     s,
				}
			},
		},
		httpRoutes: StatusSyncer[*gwv1.HTTPRoute, *gwv1.HTTPRouteStatus]{
			Name:           "httpRoute",
			ControllerName: controllerName,
			Client:         kclient.NewFilteredDelayed[*gwv1.HTTPRoute](client, wellknown.HTTPRouteGVR, f),
			Build: func(om metav1.ObjectMeta, s *gwv1.HTTPRouteStatus) *gwv1.HTTPRoute {
				return &gwv1.HTTPRoute{
					ObjectMeta: om,
					Status:     *s,
				}
			},
		},
		grpcRoutes: StatusSyncer[*gwv1.GRPCRoute, *gwv1.GRPCRouteStatus]{
			Name:           "grpcRoute",
			ControllerName: controllerName,
			Client:         kclient.NewFilteredDelayed[*gwv1.GRPCRoute](client, wellknown.GRPCRouteGVR, f),
			Build: func(om metav1.ObjectMeta, s *gwv1.GRPCRouteStatus) *gwv1.GRPCRoute {
				return &gwv1.GRPCRoute{
					ObjectMeta: om,
					Status:     *s,
				}
			},
		},
		tlsRoutes: StatusSyncer[*gwv1.TLSRoute, *gwv1.TLSRouteStatus]{
			Name:           "tlsRoute",
			ControllerName: controllerName,
			Client:         kclient.NewFilteredDelayed[*gwv1.TLSRoute](client, wellknown.TLSRouteGVR, f),
			Build: func(om metav1.ObjectMeta, s *gwv1.TLSRouteStatus) *gwv1.TLSRoute {
				return &gwv1.TLSRoute{
					ObjectMeta: om,
					Status:     *s,
				}
			},
		},
		tcpRoutes: StatusSyncer[*gwv1.TCPRoute, *gwv1.TCPRouteStatus]{
			Name:           "tcpRoute",
			ControllerName: controllerName,
			Client:         kclient.NewFilteredDelayed[*gwv1.TCPRoute](client, wellknown.TCPRouteGVR, f),
			Build: func(om metav1.ObjectMeta, s *gwv1.TCPRouteStatus) *gwv1.TCPRoute {
				return &gwv1.TCPRoute{
					ObjectMeta: om,
					Status:     *s,
				}
			},
		},
		listenerSets: StatusSyncer[*gwv1.ListenerSet, *gwv1.ListenerSetStatus]{
			Name:           "listenerSet",
			ControllerName: controllerName,
			Client:         kclient.NewFilteredDelayed[*gwv1.ListenerSet](client, wellknown.ListenerSetGVR, f),
			Build: func(om metav1.ObjectMeta, s *gwv1.ListenerSetStatus) *gwv1.ListenerSet {
				return &gwv1.ListenerSet{
					ObjectMeta: om,
					Status:     *s,
				}
			},
		},
		gateways: StatusSyncer[*gwv1.Gateway, *gwv1.GatewayStatus]{
			Name:           "gateway",
			ControllerName: controllerName,
			Client:         kclient.NewFilteredDelayed[*gwv1.Gateway](client, wellknown.GatewayGVR, f),
			Build: func(om metav1.ObjectMeta, s *gwv1.GatewayStatus) *gwv1.Gateway {
				return &gwv1.Gateway{
					ObjectMeta: om,
					Status:     *s,
				}
			},
		},
		backendTLSPolicies: StatusSyncer[*gwv1.BackendTLSPolicy, gwv1.PolicyStatus]{
			Name:   "backendTLSPolicy",
			Client: kclient.NewFilteredDelayed[*gwv1.BackendTLSPolicy](client, wellknown.BackendTLSPolicyGVR, f),
			Build: func(om metav1.ObjectMeta, s gwv1.PolicyStatus) *gwv1.BackendTLSPolicy {
				return &gwv1.BackendTLSPolicy{
					ObjectMeta: om,
					Status:     s,
				}
			},
		},
		xBackends: StatusSyncer[*gwxv1a1.XBackend, *gwxv1a1.BackendStatus]{
			Name:           "xBackend",
			ControllerName: controllerName,
			Client:         kclient.NewFilteredDelayed[*gwxv1a1.XBackend](client, wellknown.XBackendGVR, f),
			Build: func(om metav1.ObjectMeta, s *gwxv1a1.BackendStatus) *gwxv1a1.XBackend {
				return &gwxv1a1.XBackend{ObjectMeta: om, Status: *s}
			},
		},
	}
	if enableInference {
		syncer.inferencePools = StatusSyncer[*inf.InferencePool, inf.InferencePoolStatus]{
			Name:   "inferencePools",
			Client: kclient.NewFilteredDelayed[*inf.InferencePool](client, wellknown.InferencePoolGVR, f),
			Build: func(om metav1.ObjectMeta, s inf.InferencePoolStatus) *inf.InferencePool {
				return &inf.InferencePool{
					ObjectMeta: om,
					Status:     s,
				}
			},
		}
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
		s.listenerSets.Client.HasSynced,
		s.gateways.Client.HasSynced,
		s.httpRoutes.Client.HasSynced,
		s.grpcRoutes.Client.HasSynced,
		s.tcpRoutes.Client.HasSynced,
		s.tlsRoutes.Client.HasSynced,
		s.backendTLSPolicies.Client.HasSynced,
		s.xBackends.Client.HasSynced,
		s.agentgatewayBackends.Client.HasSynced,
		s.agentgatewayPolicies.Client.HasSynced,
	)
	if s.inferencePools.Client != nil {
		s.client.WaitForCacheSync(
			"agent gateway status clients",
			ctx.Done(),
			s.inferencePools.Client.HasSynced,
		)
	}

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
	case wellknown.ListenerSetGVK:
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
	case wellknown.BackendTLSPolicyGVK:
		s.backendTLSPolicies.ApplyStatus(ctx, resource, statusObj)
	case wellknown.XBackendGVK:
		s.xBackends.ApplyStatus(ctx, resource, statusObj)
	case wellknown.InferencePoolGVK:
		if s.inferencePools.Client != nil {
			s.inferencePools.ApplyStatus(ctx, resource, statusObj)
		}
	default:
		// Attempt to handle resource policy kinds via registered handlers.
		if s.extraAgwResourceStatusHandlers != nil {
			key := resource.GroupVersionKind
			if syncer, ok := s.extraAgwResourceStatusHandlers[key]; ok {
				syncer.ApplyStatus(ctx, resource, statusObj)
				return
			}
		}
		logger.Error("sync status: unknown resource type", "gvk", resource.GroupVersionKind.String())
	}
}

func (s *AgentGwStatusSyncer) NewStatusWorker(ctx context.Context) *status.WorkerPool {
	return status.NewWorkerPool(ctx, s.SyncStatus, 100)
}

type ResourceStatusSyncer interface {
	ApplyStatus(ctx context.Context, obj status.Resource, statusObj any)
}

type StatusSyncer[O controllers.ComparableObject, S any] struct {
	// Name for logging
	Name string

	// ControllerName is the controller whose status entries this syncer owns.
	// We preserve entries owned by other controllers and only publish entries owned by this controller. This
	// avoids clobbering status from other controllers or subsystems.
	ControllerName string

	Client kclient.Client[O]

	Build func(om metav1.ObjectMeta, s S) O
}

func (s StatusSyncer[O, S]) ApplyStatus(ctx context.Context, obj status.Resource, statusObj any) {
	var status S
	if ta, ok := statusObj.(*any); ok {
		if ta != nil && *ta != nil {
			status = (*ta).(S)
		}
	} else {
		status = statusObj.(S)
	}

	logger := logger.With("kind", s.Name, "resource", obj.NamespacedName.String())
	// TODO: move this to retry by putting it back on the queue, with some limit on the retry attempts allowed
	err := retry.Do(func() error {
		// Fetch the current object so we can preserve status written by other controllers/subsystems.
		// NOTE: This is especially important for Gateway.status.addresses (written by the gateway reconciler),
		// and for Route status Parents (multi-controller field).
		current := s.Client.Get(obj.Name, obj.Namespace)
		if controllers.IsNil(current) {
			// Harmless race: status write after resource was deleted.
			logger.Debug("resource not found, skipping status update")
			return nil
		}

		mergedAny := any(status)
		switch desired := mergedAny.(type) {
		case gwv1.PolicyStatus:
			// PolicyStatus is multi-writer across controllers, so preserve entries not owned by our controller.
			// NOTE: We can only merge if the current object is the expected type.
			curPol, ok := any(current).(*agentgateway.AgentgatewayPolicy)
			if ok {
				merged := desired
				merged.Ancestors = mergePolicyAncestorStatuses(s.ControllerName, curPol.Status.Ancestors, desired.Ancestors)
				mergedAny = merged
			}
		case *gwv1.GatewayStatus:
			curGw, ok := any(current).(*gwv1.Gateway)
			if ok {
				merged := mergeGatewayStatus(curGw.Status, *desired)
				mergedAny = &merged
			}
		case *gwv1.HTTPRouteStatus:
			cur, ok := any(current).(*gwv1.HTTPRoute)
			if ok {
				merged := *desired
				merged.Parents = mergeRouteParentStatuses(s.ControllerName, cur.Status.Parents, desired.Parents)
				mergedAny = &merged
			}
		case *gwv1.GRPCRouteStatus:
			cur, ok := any(current).(*gwv1.GRPCRoute)
			if ok {
				merged := *desired
				merged.Parents = mergeRouteParentStatuses(s.ControllerName, cur.Status.Parents, desired.Parents)
				mergedAny = &merged
			}
		case *gwv1.TCPRouteStatus:
			cur, ok := any(current).(*gwv1.TCPRoute)
			if ok {
				merged := *desired
				merged.Parents = mergeRouteParentStatuses(s.ControllerName, cur.Status.Parents, desired.Parents)
				mergedAny = &merged
			}
		case *gwv1.TLSRouteStatus:
			cur, ok := any(current).(*gwv1.TLSRoute)
			if ok {
				merged := *desired
				merged.Parents = mergeRouteParentStatuses(s.ControllerName, cur.Status.Parents, desired.Parents)
				mergedAny = &merged
			}
		case *gwxv1a1.BackendStatus:
			cur, ok := any(current).(*gwxv1a1.XBackend)
			if ok {
				merged := *desired
				merged.Ancestors = mergeXBackendAncestorStatuses(s.ControllerName, cur.Status.Ancestors, desired.Ancestors)
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
		_, err := s.Client.UpdateStatus(s.Build(metav1.ObjectMeta{
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

func mergeXBackendAncestorStatuses(ourControllerName string, existing, desired []gwxv1a1.BackendAncestorStatus) []gwxv1a1.BackendAncestorStatus {
	out := make([]gwxv1a1.BackendAncestorStatus, 0, len(existing)+len(desired))
	for _, ancestor := range existing {
		if string(ancestor.ControllerName) != ourControllerName {
			out = append(out, ancestor)
		}
	}
	ours := make([]gwxv1a1.BackendAncestorStatus, 0, len(desired))
	for _, ancestor := range desired {
		if string(ancestor.ControllerName) == ourControllerName {
			ours = append(ours, ancestor)
		}
	}
	slices.SortFunc(ours, func(a, b gwxv1a1.BackendAncestorStatus) int {
		return compareParentReference(a.AncestorRef, b.AncestorRef)
	})
	return append(out, ours...)
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

func mergeGatewayStatus(existing gwv1.GatewayStatus, desired gwv1.GatewayStatus) gwv1.GatewayStatus {
	merged := desired
	merged.Addresses = mergeGatewayAddresses(existing.Addresses, desired.Addresses)
	merged.Conditions = mergeGatewayConditions(existing.Conditions, desired.Conditions)

	return merged
}

func mergeGatewayConditions(existing []metav1.Condition, desired []metav1.Condition) []metav1.Condition {
	out := append([]metav1.Condition(nil), desired...)
	existingAccepted := meta.FindStatusCondition(existing, string(gwv1.GatewayConditionAccepted))
	desiredAccepted := meta.FindStatusCondition(desired, string(gwv1.GatewayConditionAccepted))
	selectedAccepted := selectGatewayAcceptedCondition(existingAccepted, desiredAccepted)

	if selectedAccepted == nil {
		return out
	}

	replaceStatusCondition(&out, *selectedAccepted)
	if selectedAccepted == existingAccepted && isGatewayAcceptedInvalidParameters(existingAccepted) {
		preserveInvalidGatewayProgrammedCondition(&out, existing, desired, *existingAccepted)
	}

	return out
}

func selectGatewayAcceptedCondition(existingAccepted, desiredAccepted *metav1.Condition) *metav1.Condition {
	if isGatewayAcceptedInvalidParameters(existingAccepted) {
		if desiredAccepted == nil {
			return existingAccepted
		}
		existingAtLeastAsNew := existingAccepted.ObservedGeneration >= desiredAccepted.ObservedGeneration
		if isGatewayAcceptedDefaultSuccess(desiredAccepted) && existingAtLeastAsNew {
			return existingAccepted
		}
		if isGatewayAcceptedUnsupportedAddress(desiredAccepted) &&
			desiredAccepted.ObservedGeneration <= existingAccepted.ObservedGeneration {
			return existingAccepted
		}
	}

	if isGatewayAcceptedDefaultSuccess(existingAccepted) && isGatewayAcceptedInvalidParameters(desiredAccepted) &&
		desiredAccepted.ObservedGeneration <= existingAccepted.ObservedGeneration {
		return existingAccepted
	}

	return desiredAccepted
}

func preserveInvalidGatewayProgrammedCondition(
	out *[]metav1.Condition,
	existing []metav1.Condition,
	desired []metav1.Condition,
	accepted metav1.Condition,
) {
	desiredProgrammed := meta.FindStatusCondition(desired, string(gwv1.GatewayConditionProgrammed))
	if desiredProgrammed != nil && !isGatewayProgrammedDefaultSuccess(desiredProgrammed) {
		return
	}

	existingProgrammed := meta.FindStatusCondition(existing, string(gwv1.GatewayConditionProgrammed))
	if isGatewayProgrammedInvalid(existingProgrammed) {
		replaceStatusCondition(out, *existingProgrammed)
		return
	}

	meta.SetStatusCondition(out, metav1.Condition{
		Type:               string(gwv1.GatewayConditionProgrammed),
		Status:             metav1.ConditionFalse,
		ObservedGeneration: accepted.ObservedGeneration,
		Reason:             string(gwv1.GatewayReasonInvalid),
		Message:            accepted.Message,
	})
}

func replaceStatusCondition(conditions *[]metav1.Condition, condition metav1.Condition) {
	for i := range *conditions {
		if (*conditions)[i].Type == condition.Type {
			(*conditions)[i] = condition
			return
		}
	}
	*conditions = append(*conditions, condition)
}

func isGatewayAcceptedDefaultSuccess(condition *metav1.Condition) bool {
	return condition != nil &&
		condition.Type == string(gwv1.GatewayConditionAccepted) &&
		condition.Status == metav1.ConditionTrue &&
		condition.Reason == string(gwv1.GatewayReasonAccepted)
}

func isGatewayAcceptedInvalidParameters(condition *metav1.Condition) bool {
	return condition != nil &&
		condition.Type == string(gwv1.GatewayConditionAccepted) &&
		condition.Status == metav1.ConditionFalse &&
		condition.Reason == string(gwv1.GatewayReasonInvalidParameters)
}

func isGatewayAcceptedUnsupportedAddress(condition *metav1.Condition) bool {
	return condition != nil &&
		condition.Type == string(gwv1.GatewayConditionAccepted) &&
		condition.Status == metav1.ConditionFalse &&
		condition.Reason == string(gwv1.GatewayReasonUnsupportedAddress)
}

func isGatewayProgrammedDefaultSuccess(condition *metav1.Condition) bool {
	return condition != nil &&
		condition.Type == string(gwv1.GatewayConditionProgrammed) &&
		condition.Status == metav1.ConditionTrue &&
		condition.Reason == string(gwv1.GatewayReasonProgrammed)
}

func isGatewayProgrammedInvalid(condition *metav1.Condition) bool {
	return condition != nil &&
		condition.Type == string(gwv1.GatewayConditionProgrammed) &&
		condition.Status == metav1.ConditionFalse &&
		condition.Reason == string(gwv1.GatewayReasonInvalid)
}

func mergeGatewayAddresses(existing []gwv1.GatewayStatusAddress, desired []gwv1.GatewayStatusAddress) []gwv1.GatewayStatusAddress {
	// Addresses are computed from the generated Service by the gateway reconciler, so translated
	// status only replaces them when it explicitly publishes addresses.
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
