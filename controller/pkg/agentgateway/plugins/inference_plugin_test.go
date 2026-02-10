package plugins

import (
	"fmt"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	corev1 "k8s.io/api/core/v1"
	meta "k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"istio.io/istio/pkg/kube/krt"
	"k8s.io/apimachinery/pkg/types"
	inf "sigs.k8s.io/gateway-api-inference-extension/api/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/wellknown"
)

func TestBuildInferencePoolStatus_MergesByControllerAndSortsGateways(t *testing.T) {
	controllerName := "example.com/controller"
	otherController := "other.example.com/controller"
	pool := newTestInferencePool()
	pool.Status.Parents = []inf.ParentStatus{
		{
			ParentRef: inf.ParentReference{
				Kind:      inf.Kind(wellknown.GatewayKind),
				Namespace: inf.Namespace("zns"),
				Name:      inf.ObjectName("foreign-gateway"),
			},
			ControllerName: inf.ControllerName(otherController),
		},
		{
			ParentRef: inf.ParentReference{
				Kind: inf.Kind(defaultInferencePoolStatusKind),
				Name: inf.ObjectName(defaultInferencePoolStatusName),
			},
			ControllerName: inf.ControllerName(controllerName),
		},
		{
			ParentRef: inf.ParentReference{
				Kind:      inf.Kind(wellknown.GatewayKind),
				Namespace: inf.Namespace("old-ns"),
				Name:      inf.ObjectName("old-gateway"),
			},
			ControllerName: inf.ControllerName(controllerName),
		},
	}

	attachedGateways := map[types.NamespacedName]struct{}{
		{Namespace: "b-ns", Name: "b-gw"}: {},
		{Namespace: "a-ns", Name: "a-gw"}: {},
	}

	status := buildInferencePoolStatus(pool, controllerName, attachedGateways, nil)
	require.NotNil(t, status)
	require.Len(t, status.Parents, 3)

	// Non-ours is preserved.
	assert.Equal(t, inf.ControllerName(otherController), status.Parents[0].ControllerName)
	assert.Equal(t, inf.ObjectName("foreign-gateway"), status.Parents[0].ParentRef.Name)

	// Ours are replaced and sorted deterministically by namespace/name.
	assert.Equal(t, inf.ControllerName(controllerName), status.Parents[1].ControllerName)
	assert.Equal(t, inf.Namespace("a-ns"), status.Parents[1].ParentRef.Namespace)
	assert.Equal(t, inf.ObjectName("a-gw"), status.Parents[1].ParentRef.Name)
	assert.Equal(t, inf.ControllerName(controllerName), status.Parents[2].ControllerName)
	assert.Equal(t, inf.Namespace("b-ns"), status.Parents[2].ParentRef.Namespace)
	assert.Equal(t, inf.ObjectName("b-gw"), status.Parents[2].ParentRef.Name)
}

func TestBuildInferencePoolStatus_FallbacksToStatusParentWhenNoGateways(t *testing.T) {
	pool := newTestInferencePool()

	status := buildInferencePoolStatus(pool, "example.com/controller", map[types.NamespacedName]struct{}{}, nil)
	require.NotNil(t, status)
	require.Len(t, status.Parents, 1)

	parent := status.Parents[0]
	assert.Equal(t, inf.ParentReference{
		Kind: inf.Kind(defaultInferencePoolStatusKind),
		Name: inf.ObjectName(defaultInferencePoolStatusName),
	}, parent.ParentRef)
}

func TestBuildInferencePoolStatus_InvalidRefSetsResolvedRefsFalse(t *testing.T) {
	pool := newTestInferencePool()

	status := buildInferencePoolStatus(
		pool,
		"example.com/controller",
		map[types.NamespacedName]struct{}{},
		fmt.Errorf("bad endpoint picker ref"),
	)
	require.NotNil(t, status)
	require.Len(t, status.Parents, 1)

	parent := status.Parents[0]
	accepted := meta.FindStatusCondition(parent.Conditions, string(inf.InferencePoolConditionAccepted))
	require.NotNil(t, accepted)
	assert.Equal(t, metav1.ConditionTrue, accepted.Status)

	resolved := meta.FindStatusCondition(parent.Conditions, string(inf.InferencePoolConditionResolvedRefs))
	require.NotNil(t, resolved)
	assert.Equal(t, metav1.ConditionFalse, resolved.Status)
	assert.Equal(t, string(inf.InferencePoolReasonInvalidExtensionRef), resolved.Reason)
	assert.Equal(t, "error: bad endpoint picker ref", resolved.Message)
}

func TestTranslatePoliciesForInferencePool_ValidAndInvalid(t *testing.T) {
	controllerName := "example.com/controller"
	pool := newTestInferencePool()
	svcIndex := newServicesByNamespaceIndex(newEPPService("default", "epp", 9002, corev1.ProtocolTCP, corev1.ServiceTypeClusterIP))

	status, policies := translatePoliciesForInferencePool(nil, controllerName, nil, nil, svcIndex, pool)
	require.NotNil(t, status)
	require.Len(t, policies, 2)
	require.Len(t, status.Parents, 1)

	group := inf.Group("example.com")
	pool.Spec.EndpointPickerRef.Group = &group
	status, policies = translatePoliciesForInferencePool(nil, controllerName, nil, nil, svcIndex, pool)
	require.NotNil(t, status)
	require.Empty(t, policies)
	resolved := meta.FindStatusCondition(status.Parents[0].Conditions, string(inf.InferencePoolConditionResolvedRefs))
	require.NotNil(t, resolved)
	assert.Equal(t, metav1.ConditionFalse, resolved.Status)
}

func TestValidateInferencePoolEndpointPickerRef_ServiceMustExist(t *testing.T) {
	pool := newTestInferencePool()
	err := validateInferencePoolEndpointPickerRef(nil, pool, newServicesByNamespaceIndex())
	require.Error(t, err)
	assert.Contains(t, err.Error(), "Service default/epp not found")
}

func TestValidateInferencePoolEndpointPickerRef_ExternalNameServiceRejected(t *testing.T) {
	pool := newTestInferencePool()
	svcIndex := newServicesByNamespaceIndex(newEPPService("default", "epp", 9002, corev1.ProtocolTCP, corev1.ServiceTypeExternalName))
	err := validateInferencePoolEndpointPickerRef(nil, pool, svcIndex)
	require.Error(t, err)
	assert.Contains(t, err.Error(), "must not be ExternalName")
}

func TestValidateInferencePoolEndpointPickerRef_PortMustBeTCPOnService(t *testing.T) {
	pool := newTestInferencePool()
	svcIndex := newServicesByNamespaceIndex(newEPPService("default", "epp", 9002, corev1.ProtocolUDP, corev1.ServiceTypeClusterIP))
	err := validateInferencePoolEndpointPickerRef(nil, pool, svcIndex)
	require.Error(t, err)
	assert.Contains(t, err.Error(), "must reference a TCP Service port")
}

func TestValidateInferencePoolEndpointPickerRef_TargetPortsMustBeSingle(t *testing.T) {
	pool := newTestInferencePool()
	pool.Spec.TargetPorts = append(pool.Spec.TargetPorts, inf.Port{Number: 9000})
	svcIndex := newServicesByNamespaceIndex(newEPPService("default", "epp", 9002, corev1.ProtocolTCP, corev1.ServiceTypeClusterIP))
	err := validateInferencePoolEndpointPickerRef(nil, pool, svcIndex)
	require.Error(t, err)
	assert.Contains(t, err.Error(), "targetPorts must contain exactly one")
}

func newTestInferencePool() *inf.InferencePool {
	return &inf.InferencePool{
		ObjectMeta: metav1.ObjectMeta{
			Name:       "test-pool",
			Namespace:  "default",
			Generation: 3,
		},
		Spec: inf.InferencePoolSpec{
			Selector: inf.LabelSelector{
				MatchLabels: map[inf.LabelKey]inf.LabelValue{
					"app": "model",
				},
			},
			TargetPorts: []inf.Port{{Number: 8000}},
			EndpointPickerRef: inf.EndpointPickerRef{
				Kind: inf.Kind(wellknown.ServiceKind),
				Name: inf.ObjectName("epp"),
				Port: &inf.Port{Number: 9002},
			},
		},
	}
}

func newEPPService(namespace, name string, port int32, protocol corev1.Protocol, serviceType corev1.ServiceType) *corev1.Service {
	return &corev1.Service{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: namespace,
			Name:      name,
		},
		Spec: corev1.ServiceSpec{
			Type: serviceType,
			Ports: []corev1.ServicePort{
				{
					Port:     port,
					Protocol: protocol,
				},
			},
		},
	}
}

func newServicesByNamespaceIndex(services ...*corev1.Service) krt.Index[string, *corev1.Service] {
	col := krt.NewStaticCollection[*corev1.Service](nil, services)
	return krt.NewNamespaceIndex(col)
}
