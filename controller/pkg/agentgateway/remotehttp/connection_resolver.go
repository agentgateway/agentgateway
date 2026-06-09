package remotehttp

import (
	"fmt"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"k8s.io/apimachinery/pkg/runtime/schema"
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
		return r.resolveBackendConnection(
			krtctx,
			types.NamespacedName{Namespace: defaultNS, Name: parentName},
			refNamespace,
			schema.GroupKind{Group: string(group), Kind: string(kind)},
			backendRef,
		)
	}
}

func (r *defaultResolver) resolveBackendConnection(
	krtctx krt.HandlerContext,
	policy types.NamespacedName,
	refNamespace string,
	gk schema.GroupKind,
	backendRef gwv1.BackendObjectReference,
) (*connection, error) {
	backendNN := types.NamespacedName{Name: string(backendRef.Name), Namespace: refNamespace}
	if r.backendResolvers[gk] == nil {
		return nil, fmt.Errorf("unsupported backend kind %s.%s for policy %s", gk.Group, gk.Kind, policy)
	}
	backend, err := r.resolveBackend(krtctx, backendNN, gk)
	if err != nil {
		return nil, err
	}
	if backend == nil {
		return nil, fmt.Errorf("backend %s not found, policy %s", backendNN, policy)
	}
	if backend.Static == nil {
		return nil, fmt.Errorf("only static backends are supported; backend: %s, policy: %s", backendNN, policy)
	}

	resolvedTLS, err := r.resolveTLS(
		krtctx,
		refNamespace,
		gk.Group,
		gk.Kind,
		string(backendRef.Name),
		nil,
		nil,
		backend.Policies,
	)
	if err != nil {
		return nil, fmt.Errorf("error setting tls options; backend: %s, policy: %s, %w", backendNN, policy, err)
	}

	var connectHost string
	if backend.Static.UnixPath != nil {
		connectHost = "unix://" + *backend.Static.UnixPath
	} else {
		connectHost = fmt.Sprintf("%s:%d", backend.Static.Host, backend.Static.Port)
	}

	conn := &connection{
		connectHost: connectHost,
		tls:         resolvedTLS,
	}

	if backend.Policies != nil && backend.Policies.Tunnel != nil {
		proxy, err := r.resolveTunnelProxy(krtctx, refNamespace, backend.Policies.Tunnel.BackendRef)
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
}

func (r *defaultResolver) resolveBackend(krtctx krt.HandlerContext, nn types.NamespacedName, gk schema.GroupKind) (*ResolvedBackend, error) {
	resolver := r.backendResolvers[gk]
	if resolver == nil {
		return nil, fmt.Errorf("unsupported backend kind %s.%s", gk.Group, gk.Kind)
	}
	backend, ok, err := resolver(krtctx, nn)
	if err != nil {
		return nil, fmt.Errorf("error resolving backend %s: %w", nn, err)
	}
	if !ok {
		return nil, nil
	}
	return backend, nil
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
	case string(kind) != wellknown.ServiceKind || string(group) != "":
		nn := types.NamespacedName{Name: string(backendRef.Name), Namespace: refNamespace}
		gk := schema.GroupKind{Group: string(group), Kind: string(kind)}
		if r.backendResolvers[gk] == nil {
			return nil, fmt.Errorf("unsupported backend kind %s.%s for tunnel proxy", group, kind)
		}
		backend, err := r.resolveBackend(krtctx, nn, gk)
		if err != nil {
			return nil, err
		}
		if backend == nil {
			return nil, fmt.Errorf("tunnel proxy backend %s not found", nn)
		}
		if backend.Static == nil {
			return nil, fmt.Errorf("only static backends are supported for tunnel proxy; backend: %s", nn)
		}
		if backend.Static.UnixPath != nil {
			return nil, fmt.Errorf("unix domain socket backends are not supported as tunnel proxies; backend: %s", nn)
		}
		var port int32
		if p := ptr.OrEmpty(backendRef.Port); p != 0 {
			port = int32(p)
		} else if backend.Static.Port != 0 {
			port = backend.Static.Port
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
			backend.Policies,
		)
		if err != nil {
			return nil, fmt.Errorf("error resolving tls for tunnel proxy backend %s: %w", nn, err)
		}

		return &tunnelProxy{
			host: fmt.Sprintf("%s:%d", backend.Static.Host, port),
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
	}
	return nil, fmt.Errorf("unsupported backend kind %s.%s for tunnel proxy", group, kind)
}
