package status

import (
	"sync"

	"istio.io/istio/pkg/config"
	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/log"
	"istio.io/istio/pkg/slices"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gwv1a2 "sigs.k8s.io/gateway-api/apis/v1alpha2"
	gwxv1a1 "sigs.k8s.io/gateway-api/apisx/v1alpha1"

	"github.com/kgateway-dev/kgateway/v2/api/v1alpha1/agentgateway"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/wellknown"
)

type NamedStatus[T any] struct {
	Name   types.NamespacedName
	Status T
}

type StatusRegistration = func(statusWriter WorkerQueue) krt.HandlerRegistration

// StatusCollections stores a variety of collections that can write status.
// These can be fed into a queue which can be dynamically changed (to handle leader election)
type StatusCollections struct {
	mu           sync.Mutex
	constructors []func(statusWriter WorkerQueue) krt.HandlerRegistration
	active       []krt.HandlerRegistration
	queue        WorkerQueue
	// extraGVKs tracks extra GVKs to sync statuses for
	extraGVKs []schema.GroupVersionKind
}

// NewStatusCollections creates a StatusCollections with an optional immutable set of extra GVKs.
// extraGVKs should be provided at construction time and not modified after.
func NewStatusCollections(extraGVKs []schema.GroupVersionKind) *StatusCollections {
	return &StatusCollections{
		extraGVKs: extraGVKs,
	}
}

func (s *StatusCollections) Register(sr StatusRegistration) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.constructors = append(s.constructors, sr)
}

func (s *StatusCollections) UnsetQueue() {
	// Now we are disabled
	s.queue = nil
	for _, act := range s.active {
		act.UnregisterHandler()
	}
	s.active = nil
}

func (s *StatusCollections) SetQueue(queue WorkerQueue) []krt.Syncer {
	s.mu.Lock()
	defer s.mu.Unlock()
	// Now we are enabled!
	s.queue = queue
	// Register all constructors
	s.active = slices.Map(s.constructors, func(reg StatusRegistration) krt.HandlerRegistration {
		return reg(queue)
	})
	return slices.Map(s.active, func(e krt.HandlerRegistration) krt.Syncer {
		return e
	})
}

// RegisterStatus takes a status collection and registers it to be managed by the status queue.
// krt.ObjectWithStatus, in theory, can contain anything in the "object" field. This function requires it to contain
// the current live *status*, and a passed in getStatus to extract it from the object.
// It will then compare the live status to the desired status to determine whether to write or not.
func RegisterStatus[I controllers.Object, IS any](s *StatusCollections, statusCol krt.StatusCollection[I, IS], getStatus func(I) IS) {
	reg := func(statusWriter WorkerQueue) krt.HandlerRegistration {
		h := statusCol.Register(func(o krt.Event[krt.ObjectWithStatus[I, IS]]) {
			l := o.Latest()
			liveStatus := getStatus(l.Obj)
			if krt.Equal(liveStatus, l.Status) {
				// We want the same status we already have! No need for a write so skip this.
				// Note: the Equals() function on ObjectWithStatus does not compare these. It only compares "old live + desired" == "new live + desired".
				// So if either the live OR the desired status changes, this callback will trigger.
				// Here, we do smarter filtering and can just check whether we meet the desired state.
				log.Debugf("suppress change for %v %v", l.ResourceName(), l.Obj.GetResourceVersion())
				return
			}
			status := &l.Status
			if o.Event == controllers.EventDelete {
				// if the object is being deleted, we should not reset status
				var empty IS
				status = &empty
			}
			enqueueStatus(statusWriter, l.Obj, status, s.extraGVKs)
			log.Debugf("Enqueued status update for %v %v: %v", l.ResourceName(), l.Obj.GetResourceVersion(), status)
		})
		return h
	}
	s.Register(reg)
}

func enqueueStatus[T any](sw WorkerQueue, obj controllers.Object, ws T, extraGVKs []schema.GroupVersionKind) {
	res := Resource{
		GroupVersionKind: schema.GroupVersionKind{},
		NamespacedName:   config.NamespacedName(obj),
		ResourceVersion:  obj.GetResourceVersion(),
	}
	switch obj.(type) {
	case *gwv1.Gateway:
		res.GroupVersionKind = wellknown.GatewayGVK
	case *gwv1.HTTPRoute:
		res.GroupVersionKind = wellknown.HTTPRouteGVK
	case *gwv1a2.TCPRoute:
		res.GroupVersionKind = wellknown.TCPRouteGVK
	case *gwv1a2.TLSRoute:
		res.GroupVersionKind = wellknown.TLSRouteGVK
	case *gwv1.GRPCRoute:
		res.GroupVersionKind = wellknown.GRPCRouteGVK
	case *agentgateway.AgentgatewayPolicy:
		res.GroupVersionKind = wellknown.AgentgatewayPolicyGVK
	case *agentgateway.AgentgatewayBackend:
		res.GroupVersionKind = wellknown.AgentgatewayBackendGVK
	case *gwxv1a1.XListenerSet:
		res.GroupVersionKind = wellknown.XListenerSetGVK
	default:
		// Prefer the object's own GVK if available
		if gvk := obj.GetObjectKind().GroupVersionKind(); !gvk.Empty() {
			res.GroupVersionKind = schema.GroupVersionKind{
				Group:   gvk.Group,
				Version: gvk.Version,
				Kind:    gvk.Kind,
			}
		} else if extraGVKs != nil {
			// Fallback: map external types by their concrete Kind using provided extraGVKs
			kind := obj.GetObjectKind().GroupVersionKind().Kind
			if kind != "" {
				for _, mapped := range extraGVKs {
					if mapped.Kind == kind {
						res.GroupVersionKind = mapped
						break
					}
				}
			}
		}
	}
	if res.GroupVersionKind.Empty() {
		log.Warnf("enqueueStatus unknown external type %T", obj)
	} else {
		sw.Push(res, ws)
	}
}
