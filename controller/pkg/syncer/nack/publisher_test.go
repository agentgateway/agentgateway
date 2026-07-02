package nack

import (
	"fmt"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	appsv1 "k8s.io/api/apps/v1"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/types"
	"sigs.k8s.io/controller-runtime/pkg/client"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient/fake"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

var (
	testGateway      = types.NamespacedName{Name: "test-gw", Namespace: "default"}
	testTypeURL      = "type.googleapis.com/agentgateway.dev.resource.Resource"
	testErrorMessage = "test error"
	testNackEvent    = NackEvent{
		Gateway:   testGateway,
		TypeUrl:   testTypeURL,
		ErrorMsg:  testErrorMessage,
		Timestamp: time.Now(),
	}
)

// TestPublisher_PublishNackWorkloadPrecedence verifies that NACK events are recorded on the
// Gateway and on the backing workload resolved from default, GatewayClass, or Gateway parameters.
func TestPublisher_PublishNackWorkloadPrecedence(t *testing.T) {
	tests := []struct {
		name             string
		gatewayClassKind agentgateway.AgentgatewayParametersWorkloadKind
		gatewayKind      agentgateway.AgentgatewayParametersWorkloadKind
		workload         client.Object
		wantWorkloadKind string
	}{
		{
			name:             "default workload records on Deployment",
			workload:         deploymentWorkload(),
			wantWorkloadKind: wellknown.DeploymentGVK.Kind,
		},
		{
			name:             "GatewayClass DaemonSet default records on DaemonSet",
			gatewayClassKind: agentgateway.AgentgatewayParametersWorkloadDaemonSet,
			workload:         daemonSetWorkload(),
			wantWorkloadKind: wellknown.DaemonSetGVK.Kind,
		},
		{
			name:             "Gateway Deployment override records on Deployment",
			gatewayClassKind: agentgateway.AgentgatewayParametersWorkloadDaemonSet,
			gatewayKind:      agentgateway.AgentgatewayParametersWorkloadDeployment,
			workload:         deploymentWorkload(),
			wantWorkloadKind: wellknown.DeploymentGVK.Kind,
		},
		{
			name:             "Gateway DaemonSet override records on DaemonSet",
			gatewayClassKind: agentgateway.AgentgatewayParametersWorkloadDeployment,
			gatewayKind:      agentgateway.AgentgatewayParametersWorkloadDaemonSet,
			workload:         daemonSetWorkload(),
			wantWorkloadKind: wellknown.DaemonSetGVK.Kind,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			ctx := t.Context()
			gw, objects := publisherTestObjects(tt.gatewayClassKind, tt.gatewayKind, tt.workload)
			fakeClient := fake.NewClient(t, objects...)
			publisher := NewPublisher(fakeClient)
			recorder := &capturingRecorder{}
			publisher.eventRecorder = recorder

			fakeClient.RunAndWait(ctx.Done())
			fakeClient.WaitForCacheSync("test-publisher", ctx.Done(), publisher.HasSynced)

			workloadRef := publisher.workloadObjectReference(gw)
			require.NotNil(t, workloadRef)
			assert.Equal(t, tt.wantWorkloadKind, workloadRef.Kind)

			publisher.PublishNack(&testNackEvent)

			require.Len(t, recorder.events, 2)
			assert.Equal(t, wellknown.GatewayKind, recorder.events[0].kind)
			assert.Equal(t, tt.wantWorkloadKind, recorder.events[1].kind)
			for _, event := range recorder.events {
				assert.Equal(t, corev1.EventTypeWarning, event.eventType)
				assert.Equal(t, ReasonNack, event.reason)
				assert.Equal(t, testErrorMessage, event.message)
			}
		})
	}
}

// TestPublisher_PublishNackMissingWorkloadRecordsGatewayOnly verifies that a missing workload does
// not block recording the NACK event on the Gateway.
func TestPublisher_PublishNackMissingWorkloadRecordsGatewayOnly(t *testing.T) {
	ctx := t.Context()
	gw := &gwv1.Gateway{
		ObjectMeta: metav1.ObjectMeta{
			Name:      testGateway.Name,
			Namespace: testGateway.Namespace,
		},
	}

	fakeClient := fake.NewClient(t, gw)
	publisher := NewPublisher(fakeClient)
	recorder := &capturingRecorder{}
	publisher.eventRecorder = recorder

	fakeClient.RunAndWait(ctx.Done())
	fakeClient.WaitForCacheSync("test-publisher", ctx.Done(), publisher.HasSynced)

	publisher.PublishNack(&testNackEvent)

	require.Len(t, recorder.events, 1)
	assert.Equal(t, wellknown.GatewayKind, recorder.events[0].kind)
	assert.Equal(t, corev1.EventTypeWarning, recorder.events[0].eventType)
	assert.Equal(t, ReasonNack, recorder.events[0].reason)
	assert.Equal(t, testErrorMessage, recorder.events[0].message)
}

func publisherTestObjects(
	gatewayClassKind agentgateway.AgentgatewayParametersWorkloadKind,
	gatewayKind agentgateway.AgentgatewayParametersWorkloadKind,
	workload client.Object,
) (*gwv1.Gateway, []client.Object) {
	objects := []client.Object{}
	gw := &gwv1.Gateway{
		ObjectMeta: metav1.ObjectMeta{
			Name:      testGateway.Name,
			Namespace: testGateway.Namespace,
		},
	}
	if gatewayClassKind != "" {
		gw.Spec.GatewayClassName = "agentgateway"
		classParamsName := "class-agwp"
		paramNamespace := gwv1.Namespace(testGateway.Namespace)
		objects = append(objects,
			&gwv1.GatewayClass{
				ObjectMeta: metav1.ObjectMeta{Name: "agentgateway"},
				Spec: gwv1.GatewayClassSpec{
					ParametersRef: &gwv1.ParametersReference{
						Group:     agentgateway.GroupName,
						Kind:      gwv1.Kind(wellknown.AgentgatewayParametersGVK.Kind),
						Name:      classParamsName,
						Namespace: &paramNamespace,
					},
				},
			},
			agentgatewayParameters(classParamsName, gatewayClassKind),
		)
	}
	if gatewayKind != "" {
		gatewayParamsName := "gateway-agwp"
		gw.Spec.Infrastructure = &gwv1.GatewayInfrastructure{
			ParametersRef: &gwv1.LocalParametersReference{
				Group: agentgateway.GroupName,
				Kind:  gwv1.Kind(wellknown.AgentgatewayParametersGVK.Kind),
				Name:  gatewayParamsName,
			},
		}
		objects = append(objects, agentgatewayParameters(gatewayParamsName, gatewayKind))
	}
	objects = append(objects, gw, workload)
	return gw, objects
}

func agentgatewayParameters(
	name string,
	kind agentgateway.AgentgatewayParametersWorkloadKind,
) *agentgateway.AgentgatewayParameters {
	return &agentgateway.AgentgatewayParameters{
		ObjectMeta: metav1.ObjectMeta{
			Name:      name,
			Namespace: testGateway.Namespace,
		},
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersConfigs: agentgateway.AgentgatewayParametersConfigs{
				Workload: &agentgateway.AgentgatewayParametersWorkload{Kind: kind},
			},
		},
	}
}

func deploymentWorkload() *appsv1.Deployment {
	return &appsv1.Deployment{
		ObjectMeta: metav1.ObjectMeta{
			Name:      testGateway.Name,
			Namespace: testGateway.Namespace,
		},
	}
}

func daemonSetWorkload() *appsv1.DaemonSet {
	return &appsv1.DaemonSet{
		ObjectMeta: metav1.ObjectMeta{
			Name:      testGateway.Name,
			Namespace: testGateway.Namespace,
		},
	}
}

type capturedEvent struct {
	kind      string
	eventType string
	reason    string
	message   string
}

type capturingRecorder struct {
	events []capturedEvent
}

func (r *capturingRecorder) Event(object runtime.Object, eventType, reason, message string) {
	r.record(object, eventType, reason, message)
}

func (r *capturingRecorder) Eventf(
	object runtime.Object,
	eventType string,
	reason string,
	messageFmt string,
	args ...any,
) {
	r.record(object, eventType, reason, fmt.Sprintf(messageFmt, args...))
}

func (r *capturingRecorder) AnnotatedEventf(
	object runtime.Object,
	_ map[string]string,
	eventType string,
	reason string,
	messageFmt string,
	args ...any,
) {
	r.record(object, eventType, reason, fmt.Sprintf(messageFmt, args...))
}

func (r *capturingRecorder) record(object runtime.Object, eventType, reason, message string) {
	ref, ok := object.(*corev1.ObjectReference)
	if !ok {
		r.events = append(r.events, capturedEvent{eventType: eventType, reason: reason, message: message})
		return
	}
	r.events = append(r.events, capturedEvent{
		kind:      ref.Kind,
		eventType: eventType,
		reason:    reason,
		message:   message,
	})
}
