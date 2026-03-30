package remotehttp

import (
	"strconv"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/slices"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
)

func (r *defaultResolver) serviceTargetSectionMatcher(
	refPort *gwv1.PortNumber,
	defaultPort string,
) targetSectionMatcher {
	candidates := make([]string, 0, 1)
	appendPort := func(port int32) {
		candidates = append(candidates, strconv.FormatInt(int64(port), 10))
	}

	if port := ptr.OrEmpty(refPort); port != 0 {
		appendPort(int32(port))
	} else if defaultPort != "" {
		if parsed, err := strconv.ParseInt(defaultPort, 10, 32); err == nil {
			appendPort(int32(parsed))
		}
	}

	return newTargetSectionMatcher(slices.FilterDuplicates(candidates))
}

func (r *defaultResolver) backendTLSServiceTargetSectionMatcher(
	krtctx krt.HandlerContext,
	namespace, name string,
	refPort *gwv1.PortNumber,
	defaultPort string,
) targetSectionMatcher {
	candidates := make([]string, 0, 2)
	appendPort := func(port int32) {
		candidates = append(candidates, strconv.FormatInt(int64(port), 10))
		if portName := r.servicePortName(krtctx, namespace, name, port); portName != "" {
			candidates = append(candidates, portName)
		}
	}

	if port := ptr.OrEmpty(refPort); port != 0 {
		appendPort(int32(port))
	} else if defaultPort != "" {
		if parsed, err := strconv.ParseInt(defaultPort, 10, 32); err == nil {
			appendPort(int32(parsed))
		}
	}

	return newTargetSectionMatcher(slices.FilterDuplicates(candidates))
}

func (r *defaultResolver) servicePortName(
	krtctx krt.HandlerContext,
	namespace, name string,
	port int32,
) string {
	svc := ptr.Flatten(krt.FetchOne(krtctx, r.services, krt.FilterObjectName(types.NamespacedName{
		Name:      name,
		Namespace: namespace,
	})))
	if svc == nil {
		return ""
	}
	for _, svcPort := range svc.Spec.Ports {
		if svcPort.Port == port {
			return svcPort.Name
		}
	}
	return ""
}
