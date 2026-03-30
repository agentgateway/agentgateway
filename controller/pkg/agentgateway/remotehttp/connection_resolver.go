package remotehttp

import (
	"fmt"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

type connection struct {
	connectHost string
	tls         *resolvedTLS
}

func (r *defaultResolver) resolveConnection(
	krtctx krt.HandlerContext,
	parentName, defaultNS string,
	backendRef gwv1.BackendObjectReference,
	defaultPort string,
) (*connection, error) {
	kind := ptr.OrDefault(backendRef.Kind, wellknown.ServiceKind)
	group := ptr.OrDefault(backendRef.Group, "")
	refNamespace := string(ptr.OrDefault(backendRef.Namespace, gwv1.Namespace(defaultNS)))

	switch {
	case string(kind) == wellknown.AgentgatewayBackendGVK.Kind && string(group) == wellknown.AgentgatewayBackendGVK.Group:
		backendNN := types.NamespacedName{Name: string(backendRef.Name), Namespace: refNamespace}
		backend := ptr.Flatten(krt.FetchOne(krtctx, r.backends, krt.FilterObjectName(backendNN)))
		if backend == nil {
			return nil, fmt.Errorf("backend %s not found, policy %s", backendNN, types.NamespacedName{Namespace: defaultNS, Name: parentName})
		}
		if backend.Spec.Static == nil {
			return nil, fmt.Errorf("only static backends are supported; backend: %s, policy: %s", backendNN, types.NamespacedName{Namespace: defaultNS, Name: parentName})
		}

		resolvedTLS, err := r.resolveTLS(
			krtctx,
			refNamespace,
			string(group),
			string(kind),
			string(backendRef.Name),
			newTargetSectionMatcher(nil),
			newTargetSectionMatcher(nil),
			backend.Spec.Policies,
		)
		if err != nil {
			return nil, fmt.Errorf("error setting tls options; backend: %s, policy: %s, %w", backendNN, types.NamespacedName{Namespace: defaultNS, Name: parentName}, err)
		}

		return &connection{
			connectHost: fmt.Sprintf("%s:%d", backend.Spec.Static.Host, backend.Spec.Static.Port),
			tls:         resolvedTLS,
		}, nil
	case string(kind) == wellknown.ServiceKind && string(group) == "":
		resolvedTLS, err := r.resolveTLS(
			krtctx,
			refNamespace,
			string(group),
			string(kind),
			string(backendRef.Name),
			r.serviceTargetSectionMatcher(backendRef.Port, defaultPort),
			r.backendTLSServiceTargetSectionMatcher(krtctx, refNamespace, string(backendRef.Name), backendRef.Port, defaultPort),
			nil,
		)
		if err != nil {
			return nil, fmt.Errorf("error setting tls options; service %s/%s, policy: %s, %w", backendRef.Name, refNamespace, types.NamespacedName{Namespace: defaultNS, Name: parentName}, err)
		}

		connectHost := kubeutils.GetServiceHostname(string(backendRef.Name), refNamespace)
		if port := ptr.OrEmpty(backendRef.Port); port != 0 {
			connectHost = fmt.Sprintf("%s:%d", connectHost, port)
		} else if defaultPort != "" {
			connectHost = fmt.Sprintf("%s:%s", connectHost, defaultPort)
		}

		return &connection{
			connectHost: connectHost,
			tls:         resolvedTLS,
		}, nil
	default:
		return nil, fmt.Errorf("unsupported backend kind %s.%s for policy %s", group, kind, types.NamespacedName{Namespace: defaultNS, Name: parentName})
	}
}
