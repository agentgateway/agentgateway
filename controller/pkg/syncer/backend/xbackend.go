package agentgatewaybackend

import (
	"fmt"
	"strings"

	"istio.io/istio/pilot/pkg/model/kstatus"
	"istio.io/istio/pkg/config"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/slices"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gwxv1a1 "sigs.k8s.io/gateway-api/apisx/v1alpha1"

	"github.com/agentgateway/agentgateway/api"
	agwir "github.com/agentgateway/agentgateway/controller/pkg/agentgateway/ir"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/translator"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

func TranslateXBackend(
	krtctx krt.HandlerContext,
	agw *plugins.AgwCollections,
	backend *gwxv1a1.XBackend,
	references plugins.ReferenceIndex,
	grants plugins.ReferenceGrantChecker,
) (*gwxv1a1.BackendStatus, []agwir.AgwResource) {
	gateways := references.LookupGatewaysForBackend(krtctx, utils.TypedNamespacedName{
		NamespacedName: config.NamespacedName(backend),
		Kind:           wellknown.XBackendKind,
	}).UnsortedList()
	slices.SortFunc(gateways, func(a, b types.NamespacedName) int { return strings.Compare(a.String(), b.String()) })

	translated, err := buildXBackend(krtctx, agw, backend, grants)
	status := buildXBackendStatus(backend, agw.ControllerName, gateways, err)
	if err != nil {
		logger.Error("failed to translate XBackend", "backend", backend.Name, "namespace", backend.Namespace, "err", err)
		return status, nil
	}

	resources := make([]agwir.AgwResource, 0, len(gateways))
	for _, gateway := range gateways {
		resources = append(resources, translator.ToResourceForGateway(gateway, &api.Resource{
			Kind: &api.Resource_Backend{Backend: translated},
		}))
	}
	return status, resources
}

func buildXBackend(
	krtctx krt.HandlerContext,
	agw *plugins.AgwCollections,
	backend *gwxv1a1.XBackend,
	grants plugins.ReferenceGrantChecker,
) (*api.Backend, error) {
	if backend.Spec.Type != gwxv1a1.BackendTypeExternalHostname || backend.Spec.ExternalHostname == nil {
		return nil, fmt.Errorf("only ExternalHostname XBackends are supported")
	}

	inlinePolicies, err := xBackendProtocolPolicies(backend.Spec.Protocol, backend.Spec.TLS)
	if err != nil {
		return nil, err
	}
	if backend.Spec.TLS != nil && backend.Spec.TLS.Mode != gwxv1a1.BackendTLSModeNone {
		tls, err := translateXBackendTLS(krtctx, agw, backend, grants)
		if err != nil {
			return nil, err
		}
		inlinePolicies = append(inlinePolicies, &api.BackendPolicySpec{
			Kind: &api.BackendPolicySpec_BackendTls{BackendTls: tls},
		})
	}
	if backend.Spec.Protocol != nil && *backend.Spec.Protocol == gwxv1a1.BackendProtocolMCP {
		return &api.Backend{
			Key:  backend.Namespace + "/" + backend.Name,
			Name: plugins.ResourceName(backend),
			Kind: &api.Backend_Mcp{Mcp: &api.MCPBackend{
				Targets: []*api.MCPTarget{{
					Name: "default",
					Backend: &api.BackendReference{
						Kind: &api.BackendReference_Service_{Service: &api.BackendReference_Service{
							Hostname:  string(backend.Spec.ExternalHostname.Hostname),
							Namespace: backend.Namespace,
						}},
						Port: uint32(backend.Spec.Port.Port), //nolint:gosec // G115: validated 1-65535 by the Gateway API CRD
					},
					Protocol: api.MCPTarget_STREAMABLE_HTTP,
				}},
				StatefulMode: api.MCPBackend_STATEFUL,
				PrefixMode:   api.MCPBackend_CONDITIONAL,
				FailureMode:  api.MCPBackend_FAIL_CLOSED,
			}},
			InlinePolicies: inlinePolicies,
		}, nil
	}

	return &api.Backend{
		Key:  backend.Namespace + "/" + backend.Name,
		Name: plugins.ResourceName(backend),
		Kind: &api.Backend_Static{Static: &api.StaticBackend{
			Host: string(backend.Spec.ExternalHostname.Hostname),
			Port: int32(backend.Spec.Port.Port),
		}},
		InlinePolicies: inlinePolicies,
	}, nil
}

func xBackendProtocolPolicies(protocol *gwxv1a1.BackendProtocol, tls *gwxv1a1.BackendTLS) ([]*api.BackendPolicySpec, error) {
	if protocol == nil || *protocol == gwxv1a1.BackendProtocolHTTP || *protocol == gwxv1a1.BackendProtocolTCP {
		return nil, nil
	}
	if *protocol == gwxv1a1.BackendProtocolMCP {
		return nil, nil
	}
	if *protocol == gwxv1a1.BackendProtocolH2C && tls != nil && tls.Mode != gwxv1a1.BackendTLSModeNone {
		return nil, fmt.Errorf("XBackend protocol H2C cannot be combined with TLS mode %s", tls.Mode)
	}

	var version api.BackendPolicySpec_BackendHTTP_HttpVersion
	switch *protocol {
	case gwxv1a1.BackendProtocolHTTP11:
		version = api.BackendPolicySpec_BackendHTTP_HTTP1
	case gwxv1a1.BackendProtocolHTTP2, gwxv1a1.BackendProtocolH2C, gwxv1a1.BackendProtocolGRPC:
		version = api.BackendPolicySpec_BackendHTTP_HTTP2
	default:
		return nil, fmt.Errorf("unsupported XBackend protocol %q", *protocol)
	}
	return []*api.BackendPolicySpec{{
		Kind: &api.BackendPolicySpec_BackendHttp{BackendHttp: &api.BackendPolicySpec_BackendHTTP{Version: version}},
	}}, nil
}

func translateXBackendTLS(
	krtctx krt.HandlerContext,
	agw *plugins.AgwCollections,
	backend *gwxv1a1.XBackend,
	grants plugins.ReferenceGrantChecker,
) (*api.BackendPolicySpec_BackendTLS, error) {
	tls := backend.Spec.TLS
	result := &api.BackendPolicySpec_BackendTLS{
		Hostname: new(string(tls.Validation.Hostname)),
		VerifySubjectAltNames: slices.Map(tls.Validation.SubjectAltNames, func(san gwv1.SubjectAltName) string {
			if san.Type == gwv1.URISubjectAltNameType {
				return string(san.URI)
			}
			return string(san.Hostname)
		}),
	}

	if wellKnown := tls.Validation.WellKnownCACertificates; wellKnown != nil {
		if *wellKnown != gwv1.WellKnownCACertificatesSystem {
			return nil, fmt.Errorf("unsupported wellKnownCACertificates %q", *wellKnown)
		}
	} else {
		var roots strings.Builder
		for _, ref := range tls.Validation.CACertificateRefs {
			if ref.Group != gwv1.Group(wellknown.ConfigMapGVK.Group) || ref.Kind != gwv1.Kind(wellknown.ConfigMapKind) {
				return nil, fmt.Errorf("CA certificate reference %s must refer to a core ConfigMap", ref.Name)
			}
			configMap := ptr.Flatten(krt.FetchOne(krtctx, agw.ConfigMaps, krt.FilterObjectName(types.NamespacedName{
				Namespace: backend.Namespace,
				Name:      string(ref.Name),
			})))
			if configMap == nil {
				return nil, fmt.Errorf("CA certificate ConfigMap %s/%s not found", backend.Namespace, ref.Name)
			}
			root, err := plugins.GetCACertFromConfigMap(configMap)
			if err != nil {
				return nil, fmt.Errorf("invalid CA certificate ConfigMap %s/%s: %w", backend.Namespace, ref.Name, err)
			}
			if roots.Len() > 0 {
				roots.WriteByte('\n')
			}
			roots.WriteString(root)
		}
		result.Root = []byte(roots.String())
	}

	if tls.Mode == gwxv1a1.BackendTLSModeClientAndServer {
		ref := tls.ClientCertificateRef
		if ref == nil {
			return nil, fmt.Errorf("clientCertificateRef is required for ClientAndServer TLS")
		}
		if ptr.OrDefault(ref.Group, gwv1.Group("")) != gwv1.Group(wellknown.SecretGVK.Group) || ptr.OrDefault(ref.Kind, gwv1.Kind(wellknown.SecretKind)) != gwv1.Kind(wellknown.SecretKind) {
			return nil, fmt.Errorf("clientCertificateRef must refer to a core Secret")
		}
		secretNamespace := string(ptr.OrDefault(ref.Namespace, gwv1.Namespace(backend.Namespace)))
		secretName := types.NamespacedName{Namespace: secretNamespace, Name: string(ref.Name)}
		if secretNamespace != backend.Namespace && (grants == nil || !grants.SecretAllowed(krtctx, wellknown.XBackendGVK, secretName, backend.Namespace)) {
			return nil, fmt.Errorf("client certificate Secret %s is not permitted by a ReferenceGrant", secretName)
		}
		secret := ptr.Flatten(krt.FetchOne(krtctx, agw.Secrets, krt.FilterObjectName(secretName)))
		if secret == nil {
			return nil, fmt.Errorf("client certificate Secret %s not found", secretName)
		}
		if _, err := plugins.ValidateTlsSecretData(secret.Name, secret.Namespace, secret.Data); err != nil {
			return nil, fmt.Errorf("invalid client certificate Secret %s: %w", secretName, err)
		}
		result.Cert = secret.Data[corev1.TLSCertKey]
		result.Key = secret.Data[corev1.TLSPrivateKeyKey]
	}
	return result, nil
}

func buildXBackendStatus(
	backend *gwxv1a1.XBackend,
	controllerName string,
	gateways []types.NamespacedName,
	translationErr error,
) *gwxv1a1.BackendStatus {
	status := &gwxv1a1.BackendStatus{}
	for _, gateway := range gateways {
		condition := metav1.Condition{
			Type:               "Accepted",
			Status:             metav1.ConditionTrue,
			Reason:             "Accepted",
			Message:            "Backend successfully accepted",
			ObservedGeneration: backend.Generation,
			LastTransitionTime: metav1.Now(),
		}
		if translationErr != nil {
			condition.Status = metav1.ConditionFalse
			condition.Reason = "Invalid"
			condition.Message = translationErr.Error()
		}
		ancestorRef := gwv1.ParentReference{
			Group:     new(gwv1.Group(wellknown.GatewayGVK.Group)),
			Kind:      new(gwv1.Kind(wellknown.GatewayKind)),
			Namespace: new(gwv1.Namespace(gateway.Namespace)),
			Name:      gwv1.ObjectName(gateway.Name),
		}
		var existingConditions []metav1.Condition
		for _, existing := range backend.Status.Ancestors {
			if string(existing.ControllerName) == controllerName && existing.AncestorRef.Name == ancestorRef.Name && ptr.Equal(existing.AncestorRef.Namespace, ancestorRef.Namespace) {
				existingConditions = existing.Conditions
				break
			}
		}
		status.Ancestors = append(status.Ancestors, gwxv1a1.BackendAncestorStatus{
			ControllerName: gwv1.GatewayController(controllerName),
			AncestorRef:    ancestorRef,
			Conditions:     kstatus.UpdateConditionIfChanged(existingConditions, condition),
		})
	}
	return status
}
