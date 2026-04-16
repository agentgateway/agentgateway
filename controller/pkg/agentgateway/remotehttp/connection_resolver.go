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
	proxyURL    string
	proxyTLS    *resolvedTLS
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
			nil,
			nil,
			backend.Spec.Policies,
		)
		if err != nil {
			return nil, fmt.Errorf("error setting tls options; backend: %s, policy: %s, %w", backendNN, types.NamespacedName{Namespace: defaultNS, Name: parentName}, err)
		}

		var connectHost string
		switch {
		case backend.Spec.Static.UnixPath != nil:
			connectHost = "unix://" + *backend.Spec.Static.UnixPath
		case backend.Spec.Static.Host != nil && backend.Spec.Static.Port != nil:
			connectHost = fmt.Sprintf("%s:%d", *backend.Spec.Static.Host, *backend.Spec.Static.Port)
		default:
			return nil, fmt.Errorf("static backend %s must specify either unixPath or both host and port", backendNN)
		}

		conn := &connection{
			connectHost: connectHost,
			tls:         resolvedTLS,
		}

		if backend.Spec.Policies != nil && backend.Spec.Policies.Tunnel != nil {
			proxy, err := r.resolveTunnelProxy(krtctx, refNamespace, backend.Spec.Policies.Tunnel.BackendRef)
			if err != nil {
				return nil, fmt.Errorf("error resolving tunnel proxy for backend %s: %w", backendNN, err)
			}
			if proxy.tls != nil {
				conn.proxyURL = "https://" + proxy.host
			} else {
				conn.proxyURL = "http://" + proxy.host
			}
			conn.proxyTLS = proxy.tls
		}

		return conn, nil
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

type tunnelProxy struct {
	host string
	tls  *resolvedTLS
}

// resolveTunnelProxy resolves a tunnel BackendRef to a proxy host:port and
// optional TLS configuration. Only static backends and services are supported;
// the proxy backend itself must not chain another tunnel.
func (r *defaultResolver) resolveTunnelProxy(
	krtctx krt.HandlerContext,
	defaultNS string,
	backendRef gwv1.BackendObjectReference,
) (*tunnelProxy, error) {
	kind := ptr.OrDefault(backendRef.Kind, wellknown.ServiceKind)
	group := ptr.OrDefault(backendRef.Group, "")
	refNamespace := string(ptr.OrDefault(backendRef.Namespace, gwv1.Namespace(defaultNS)))

	switch {
	case string(kind) == wellknown.AgentgatewayBackendGVK.Kind && string(group) == wellknown.AgentgatewayBackendGVK.Group:
		nn := types.NamespacedName{Name: string(backendRef.Name), Namespace: refNamespace}
		backend := ptr.Flatten(krt.FetchOne(krtctx, r.backends, krt.FilterObjectName(nn)))
		if backend == nil {
			return nil, fmt.Errorf("tunnel proxy backend %s not found", nn)
		}
		if backend.Spec.Static == nil {
			return nil, fmt.Errorf("only static backends are supported for tunnel proxy; backend: %s", nn)
		}
		if backend.Spec.Static.UnixPath != nil {
			return nil, fmt.Errorf("unix domain socket backends are not supported as tunnel proxies; backend: %s", nn)
		}
		if backend.Spec.Static.Host == nil {
			return nil, fmt.Errorf("host is required for TCP tunnel proxy backend: %s", nn)
		}
		var port int32
		if p := ptr.OrEmpty(backendRef.Port); p != 0 {
			port = int32(p)
		} else if backend.Spec.Static.Port != nil {
			port = *backend.Spec.Static.Port
		} else {
			return nil, fmt.Errorf("port is required for TCP tunnel proxy backend: %s", nn)
		}

		proxyTLS, err := r.resolveTLS(
			krtctx,
			refNamespace,
			string(group),
			string(kind),
			string(backendRef.Name),
			nil,
			nil,
			backend.Spec.Policies,
		)
		if err != nil {
			return nil, fmt.Errorf("error resolving tls for tunnel proxy backend %s: %w", nn, err)
		}

		return &tunnelProxy{
			host: fmt.Sprintf("%s:%d", *backend.Spec.Static.Host, port),
			tls:  proxyTLS,
		}, nil

	case string(kind) == wellknown.ServiceKind && string(group) == "":
		host := kubeutils.GetServiceHostname(string(backendRef.Name), refNamespace)
		port := ptr.OrEmpty(backendRef.Port)
		if port == 0 {
			return nil, fmt.Errorf("port is required for Service tunnel proxy backend %s/%s", backendRef.Name, refNamespace)
		}
		return &tunnelProxy{
			host: fmt.Sprintf("%s:%d", host, port),
		}, nil

	default:
		return nil, fmt.Errorf("unsupported backend kind %s.%s for tunnel proxy", group, kind)
	}
}
