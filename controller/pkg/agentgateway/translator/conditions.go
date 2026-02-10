package translator

import (
	"istio.io/istio/pilot/pkg/model/kstatus"
	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/maps"
	"istio.io/istio/pkg/slices"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/wellknown"
)

// ParentErrorReason is the error string for a ParentError (reason parent could not be referenced)
type ParentErrorReason string

const (
	ParentErrorNotAccepted       = ParentErrorReason(gwv1.RouteReasonNoMatchingParent)
	ParentErrorNotAllowed        = ParentErrorReason(gwv1.RouteReasonNotAllowedByListeners)
	ParentErrorNoHostname        = ParentErrorReason(gwv1.RouteReasonNoMatchingListenerHostname)
)

// ConfigErrorReason is the error string for a ConfigError (reason configuration is invalid)
type ConfigErrorReason = string

const (
	// InvalidTLS indicates an issue with TLS settings
	InvalidTLS = ConfigErrorReason(gwv1.ListenerReasonInvalidCertificateRef)
	// InvalidListenerRefNotPermitted indicates a listener reference was not permitted
	InvalidListenerRefNotPermitted = ConfigErrorReason(gwv1.ListenerReasonRefNotPermitted)
)

// ParentError represents that a parent could not be referenced
type ParentError struct {
	Reason  ParentErrorReason
	Message string
}

// ConfigError represents an invalid configuration that will be reported back to the user.
type ConfigError struct {
	Reason  ConfigErrorReason
	Message string
}

type Condition struct {
	// Reason defines the Reason to report on success. Ignored if error is set
	Reason string
	// Message defines the Message to report on success. Ignored if error is set
	Message string
	// Status defines the Status to report on success. The inverse will be set if error is set
	// If not set, will default to StatusTrue
	Status metav1.ConditionStatus
	// Error defines an Error state; the reason and message will be replaced with that of the Error and
	// the status inverted
	Error *ConfigError
	// SetOnce, if enabled, will only set the condition if it is not yet present or set to this reason
	SetOnce string
}

// SetConditions sets the existingConditions with the new conditions
func SetConditions(generation int64, existingConditions []metav1.Condition, conditions map[string]*Condition) []metav1.Condition {
	// Sort keys for deterministic ordering
	for _, k := range slices.Sort(maps.Keys(conditions)) {
		cond := conditions[k]
		setter := kstatus.UpdateConditionIfChanged
		if cond.SetOnce != "" {
			setter = func(conditions []metav1.Condition, condition metav1.Condition) []metav1.Condition {
				return kstatus.CreateCondition(conditions, condition, cond.SetOnce)
			}
		}
		// A condition can be "negative polarity" (ex: ListenerInvalid) or "positive polarity" (ex:
		// ListenerValid), so in order to determine the status we should set each `condition` defines its
		// default positive status. When there is an error, we will invert that. Example: If we have
		// condition ListenerInvalid, the status will be set to StatusFalse. If an error is reported, it
		// will be inverted to StatusTrue to indicate listeners are invalid. See
		// https://github.com/kubernetes/community/blob/master/contributors/devel/sig-architecture/api-conventions.md#typical-status-properties
		// for more information
		if cond.Error != nil {
			existingConditions = setter(existingConditions, metav1.Condition{
				Type:               k,
				Status:             kstatus.InvertStatus(cond.Status),
				ObservedGeneration: generation,
				LastTransitionTime: metav1.Now(),
				Reason:             cond.Error.Reason,
				Message:            cond.Error.Message,
			})
		} else {
			status := cond.Status
			if status == "" {
				status = kstatus.StatusTrue
			}
			existingConditions = setter(existingConditions, metav1.Condition{
				Type:               k,
				Status:             status,
				ObservedGeneration: generation,
				LastTransitionTime: metav1.Now(),
				Reason:             cond.Reason,
				Message:            cond.Message,
			})
		}
	}
	return existingConditions
}

func reportListenerCondition(index int, l gwv1.Listener, obj controllers.Object,
	statusListeners []gwv1.ListenerStatus, conditions map[string]*Condition,
) []gwv1.ListenerStatus {
	for index >= len(statusListeners) {
		statusListeners = append(statusListeners, gwv1.ListenerStatus{})
	}
	cond := statusListeners[index].Conditions
	supported, valid := GenerateSupportedKinds(l)
	if !valid {
		conditions[string(gwv1.ListenerConditionResolvedRefs)] = &Condition{
			Reason:  string(gwv1.ListenerReasonInvalidRouteKinds),
			Status:  metav1.ConditionFalse,
			Message: "Invalid route kinds",
		}
	}
	statusListeners[index] = gwv1.ListenerStatus{
		Name:           l.Name,
		SupportedKinds: supported,
		Conditions:     SetConditions(obj.GetGeneration(), cond, conditions),
	}
	return statusListeners
}

// GenerateSupportedKinds returns the supported kinds for the listener.
func GenerateSupportedKinds(l gwv1.Listener) ([]gwv1.RouteGroupKind, bool) {
	supported := []gwv1.RouteGroupKind{}
	switch l.Protocol {
	case gwv1.HTTPProtocolType, gwv1.HTTPSProtocolType:
		// Only terminate allowed, so its always HTTP
		supported = []gwv1.RouteGroupKind{
			toRouteKind(wellknown.HTTPRouteGVK),
			toRouteKind(wellknown.GRPCRouteGVK),
		}
	case gwv1.TCPProtocolType:
		supported = []gwv1.RouteGroupKind{toRouteKind(wellknown.TCPRouteGVK)}
	case gwv1.TLSProtocolType:
		if l.TLS != nil && l.TLS.Mode != nil && *l.TLS.Mode == gwv1.TLSModePassthrough {
			supported = []gwv1.RouteGroupKind{toRouteKind(wellknown.TLSRouteGVK)}
		} else {
			supported = []gwv1.RouteGroupKind{toRouteKind(wellknown.TCPRouteGVK)}
		}
		// UDP route not support
	}
	if l.AllowedRoutes != nil && len(l.AllowedRoutes.Kinds) > 0 {
		// We need to filter down to only ones we actually support
		intersection := []gwv1.RouteGroupKind{}
		for _, s := range supported {
			for _, kind := range l.AllowedRoutes.Kinds {
				if routeGroupKindEqual(s, kind) {
					intersection = append(intersection, s)
					break
				}
			}
		}
		return intersection, len(intersection) == len(l.AllowedRoutes.Kinds)
	}
	return supported, true
}
