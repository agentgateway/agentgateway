package reporter

import (
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"sigs.k8s.io/controller-runtime/pkg/client"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
)

const (
	PolicyAcceptedMsg = "Policy accepted"

	PolicyAttachedMsg = "Attached to all targets"
)

type PolicyCondition struct {
	Type               string
	Status             metav1.ConditionStatus
	Reason             string
	Message            string
	ObservedGeneration int64
}

type PolicyKey struct {
	Group     string
	Kind      string
	Namespace string
	Name      string
}

func (p PolicyKey) DisplayString() string {
	return p.Kind + "/" + p.Namespace + "/" + p.Name
}

type GatewayCondition struct {
	Type    gwv1.GatewayConditionType
	Status  metav1.ConditionStatus
	Reason  gwv1.GatewayConditionReason
	Message string
}

type ListenerCondition struct {
	Type    gwv1.ListenerConditionType
	Status  metav1.ConditionStatus
	Reason  gwv1.ListenerConditionReason
	Message string
}

type RouteCondition struct {
	Type    gwv1.RouteConditionType
	Status  metav1.ConditionStatus
	Reason  gwv1.RouteConditionReason
	Message string
}

type Reporter interface {
	Gateway(gateway *gwv1.Gateway) GatewayReporter
	ListenerSet(listenerSet client.Object) ListenerSetReporter
	Route(obj metav1.Object) RouteReporter
}

type GatewayReporter interface {
	Listener(listener *gwv1.Listener) ListenerReporter
	ListenerName(listenerName string) ListenerReporter
	SetCondition(condition GatewayCondition)
}

type ListenerSetReporter interface {
	Listener(listener *gwv1.Listener) ListenerReporter
	ListenerName(listenerName string) ListenerReporter
	SetCondition(condition GatewayCondition)
}

type ListenerReporter interface {
	SetCondition(ListenerCondition)
	SetSupportedKinds([]gwv1.RouteGroupKind)
	SetAttachedRoutes(n uint)
}

type RouteReporter interface {
	ParentRef(parentRef *gwv1.ParentReference) ParentRefReporter
}

type ParentRefReporter interface {
	SetCondition(condition RouteCondition)
}
