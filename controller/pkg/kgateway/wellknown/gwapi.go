package wellknown

import (
	apiextv1 "k8s.io/apiextensions-apiserver/pkg/apis/apiextensions/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
	inf "sigs.k8s.io/gateway-api-inference-extension/api/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gwv1a2 "sigs.k8s.io/gateway-api/apis/v1alpha2"
	gwv1b1 "sigs.k8s.io/gateway-api/apis/v1beta1"
	gwxv1a1 "sigs.k8s.io/gateway-api/apisx/v1alpha1"
)

const (
	// Group string for Gateway API resources
	GatewayGroup      = gwv1.GroupName
	XListenerSetGroup = gwxv1a1.GroupName

	// Kind strings
	ServiceKind          = "Service"
	ConfigMapKind        = "ConfigMap"
	SecretKind           = "Secret"
	HTTPRouteKind        = "HTTPRoute"
	TCPRouteKind         = "TCPRoute"
	TLSRouteKind         = "TLSRoute"
	GRPCRouteKind        = "GRPCRoute"
	GatewayKind          = "Gateway"
	GatewayClassKind     = "GatewayClass"
	ReferenceGrantKind   = "ReferenceGrant"
	BackendTLSPolicyKind = "BackendTLSPolicy"

	// Kind string for XListenerSet resource
	XListenerSetKind = "XListenerSet"

	// Kind string for InferencePool resource
	InferencePoolKind = "InferencePool"

	// Gateway API CRD names
	TCPRouteCRDName = "tcproutes.gateway.networking.k8s.io"
)

var (
	GatewayGVK = schema.GroupVersionKind{
		Group:   GatewayGroup,
		Version: gwv1.GroupVersion.Version,
		Kind:    GatewayKind,
	}
	GatewayGVR = schema.GroupVersionResource{
		Group:    GatewayGroup,
		Version:  gwv1.GroupVersion.Version,
		Resource: "gateways",
	}
	GatewayClassGVK = schema.GroupVersionKind{
		Group:   GatewayGroup,
		Version: gwv1.GroupVersion.Version,
		Kind:    GatewayClassKind,
	}
	GatewayClassGVR = schema.GroupVersionResource{
		Group:    GatewayGroup,
		Version:  gwv1.GroupVersion.Version,
		Resource: "gatewayclasses",
	}
	HTTPRouteGVK = schema.GroupVersionKind{
		Group:   GatewayGroup,
		Version: gwv1.GroupVersion.Version,
		Kind:    HTTPRouteKind,
	}
	HTTPRouteGVR = schema.GroupVersionResource{
		Group:    GatewayGroup,
		Version:  gwv1.GroupVersion.Version,
		Resource: "httproutes",
	}
	TLSRouteGVK = schema.GroupVersionKind{
		Group:   GatewayGroup,
		Version: gwv1a2.GroupVersion.Version,
		Kind:    TLSRouteKind,
	}
	TLSRouteGVR = schema.GroupVersionResource{
		Group:    GatewayGroup,
		Version:  gwv1a2.GroupVersion.Version,
		Resource: "tlsroutes",
	}
	TCPRouteGVK = schema.GroupVersionKind{
		Group:   GatewayGroup,
		Version: gwv1a2.GroupVersion.Version,
		Kind:    TCPRouteKind,
	}
	TCPRouteGVR = schema.GroupVersionResource{
		Group:    GatewayGroup,
		Version:  gwv1a2.GroupVersion.Version,
		Resource: "tcproutes",
	}
	GRPCRouteGVK = schema.GroupVersionKind{
		Group:   GatewayGroup,
		Version: gwv1.GroupVersion.Version,
		Kind:    GRPCRouteKind,
	}
	GRPCRouteGVR = schema.GroupVersionResource{
		Group:    GatewayGroup,
		Version:  gwv1.GroupVersion.Version,
		Resource: "grpcroutes",
	}
	ReferenceGrantGVK = schema.GroupVersionKind{
		Group:   GatewayGroup,
		Version: gwv1b1.GroupVersion.Version,
		Kind:    ReferenceGrantKind,
	}
	ReferenceGrantGVR = schema.GroupVersionResource{
		Group:    GatewayGroup,
		Version:  gwv1b1.GroupVersion.Version,
		Resource: "referencegrants",
	}
	BackendTLSPolicyGVK = schema.GroupVersionKind{
		Group:   GatewayGroup,
		Version: gwv1.GroupVersion.Version,
		Kind:    BackendTLSPolicyKind,
	}
	InferencePoolGVK = schema.GroupVersionKind{
		Group:   inf.GroupVersion.Group,
		Version: inf.GroupVersion.Version,
		Kind:    InferencePoolKind,
	}
	InferencePoolGVR = schema.GroupVersionResource{
		Group:    inf.GroupVersion.Group,
		Version:  inf.GroupVersion.Version,
		Resource: "inferencepools",
	}
	BackendTLSPolicyGVR = schema.GroupVersionResource{
		Group:    GatewayGroup,
		Version:  gwv1.GroupVersion.Version,
		Resource: "backendtlspolicies",
	}

	TCPRouteCRD = apiextv1.CustomResourceDefinition{
		ObjectMeta: metav1.ObjectMeta{
			Name: TCPRouteCRDName,
		},
	}

	XListenerSetGVK = schema.GroupVersionKind{
		Group:   XListenerSetGroup,
		Version: gwxv1a1.GroupVersion.Version,
		Kind:    XListenerSetKind,
	}
	XListenerSetGVR = schema.GroupVersionResource{
		Group:    XListenerSetGroup,
		Version:  gwxv1a1.GroupVersion.Version,
		Resource: "xlistenersets",
	}
)
