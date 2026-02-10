package schemes

import (
	"fmt"

	istionetworkingv1 "istio.io/client-go/pkg/apis/networking/v1"
	istiosecurityv1 "istio.io/client-go/pkg/apis/security/v1"
	appsv1 "k8s.io/api/apps/v1"
	corev1 "k8s.io/api/core/v1"
	discoveryv1 "k8s.io/api/discovery/v1"
	rbacv1 "k8s.io/api/rbac/v1"
	apiextensionsv1 "k8s.io/apiextensions-apiserver/pkg/apis/apiextensions/v1"
	"k8s.io/apimachinery/pkg/runtime"
	inf "sigs.k8s.io/gateway-api-inference-extension/api/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gwv1a2 "sigs.k8s.io/gateway-api/apis/v1alpha2"
	gwv1b1 "sigs.k8s.io/gateway-api/apis/v1beta1"
	gwxv1a1 "sigs.k8s.io/gateway-api/apisx/v1alpha1"

	agwv1a1 "github.com/kgateway-dev/kgateway/v2/api/v1alpha1/agentgateway"
)

// SchemeBuilder contains all the Schemes for registering the CRDs with which kgateway interacts.
// We share one SchemeBuilder as there's no harm in registering all I/O types internally.
var SchemeBuilder = runtime.SchemeBuilder{
	// K8s Gateway API resources
	gwv1.Install,
	gwv1b1.Install,
	gwv1a2.Install,
	gwxv1a1.Install,

	// Kubernetes Core resources
	corev1.AddToScheme,
	appsv1.AddToScheme,
	discoveryv1.AddToScheme,

	// Register the apiextensions API group
	apiextensionsv1.AddToScheme,

	// agentgateway API resources
	agwv1a1.Install,

	// Istio resources
	istionetworkingv1.AddToScheme,
	istiosecurityv1.AddToScheme,
}

func AddToScheme(s *runtime.Scheme) error {
	return SchemeBuilder.AddToScheme(s)
}

// DefaultScheme returns a scheme with all the types registered for kgateway.
// We intentionally do not perform this operation in an init!!
// See https://github.com/kgateway-dev/kgateway/pull/9692 for context
func DefaultScheme() *runtime.Scheme {
	s := runtime.NewScheme()
	_ = AddToScheme(s)
	return s
}

// GatewayScheme unconditionally includes the default and required Gateway API schemes.
// Use the Default scheme with AddGatewayV1A2Scheme to conditionally add the v1alpha2 scheme.
func GatewayScheme() *runtime.Scheme {
	s := DefaultScheme()
	if err := gwv1a2.Install(s); err != nil {
		panic(fmt.Sprintf("Failed to install gateway v1alpha2 scheme: %v", err))
	}
	if err := gwv1b1.Install(s); err != nil {
		panic(fmt.Sprintf("Failed to install gateway v1beta1 scheme: %v", err))
	}
	if err := gwxv1a1.Install(s); err != nil {
		panic(fmt.Sprintf("Failed to install gateway experimental v1alpha1 scheme: %v", err))
	}
	return s
}

// InferExtScheme unconditionally includes the default, Gateway API, and Inference Extension schemes.
// Use the Default scheme with AddInferExtV1A2Scheme to conditionally add the v1alpha2 scheme.
func InferExtScheme() *runtime.Scheme {
	s := GatewayScheme()
	// Required to deploy RBAC resources for endpoint picker extension.
	if err := rbacv1.AddToScheme(s); err != nil {
		panic(fmt.Sprintf("Failed to add RBAC v1 scheme: %v", err))
	}
	if err := inf.Install(s); err != nil {
		panic(fmt.Sprintf("Failed to add Gateway API Inference Extension v1alpha2 scheme: %v", err))
	}
	return s
}
