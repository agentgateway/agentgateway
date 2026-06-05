package nack

import (
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	appsv1 "k8s.io/api/apps/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	"k8s.io/client-go/tools/record"
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

func TestPublisher_PublishNackDeploymentWorkload(t *testing.T) {
	ctx := t.Context()

	// Ensure involved objects exist so UID lookups succeed
	gw := &gwv1.Gateway{
		ObjectMeta: metav1.ObjectMeta{
			Name:      testGateway.Name,
			Namespace: testGateway.Namespace,
		},
	}
	dep := &appsv1.Deployment{
		ObjectMeta: metav1.ObjectMeta{
			Name:      testGateway.Name,
			Namespace: testGateway.Namespace,
		},
	}

	fakeClient := fake.NewClient(t, gw, dep)

	publisher := NewPublisher(fakeClient)
	fakeRecorder := record.NewFakeRecorder(10)
	publisher.eventRecorder = fakeRecorder

	fakeClient.RunAndWait(ctx.Done())

	fakeClient.WaitForCacheSync("test-publisher", ctx.Done(), publisher.HasSynced)

	workloadRef := publisher.workloadObjectReference(gw)
	assert.NotNil(t, workloadRef)
	assert.Equal(t, wellknown.DeploymentGVK.Kind, workloadRef.Kind)

	publisher.PublishNack(&testNackEvent)

	assertRecordedEvent(t, fakeRecorder, "gateway", "Warning", ReasonNack, testErrorMessage)
	assertRecordedEvent(t, fakeRecorder, "workload", "Warning", ReasonNack, testErrorMessage)
}

func TestPublisher_PublishNackDaemonSetWorkload(t *testing.T) {
	ctx := t.Context()

	paramNamespace := gwv1.Namespace(testGateway.Namespace)
	gw := &gwv1.Gateway{
		ObjectMeta: metav1.ObjectMeta{
			Name:      testGateway.Name,
			Namespace: testGateway.Namespace,
		},
		Spec: gwv1.GatewaySpec{GatewayClassName: "agentgateway"},
	}
	gwc := &gwv1.GatewayClass{
		ObjectMeta: metav1.ObjectMeta{Name: "agentgateway"},
		Spec: gwv1.GatewayClassSpec{
			ParametersRef: &gwv1.ParametersReference{
				Group:     agentgateway.GroupName,
				Kind:      gwv1.Kind(wellknown.AgentgatewayParametersGVK.Kind),
				Name:      "daemonset-agwp",
				Namespace: &paramNamespace,
			},
		},
	}
	agwp := &agentgateway.AgentgatewayParameters{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "daemonset-agwp",
			Namespace: testGateway.Namespace,
		},
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersConfigs: agentgateway.AgentgatewayParametersConfigs{
				Workload: &agentgateway.AgentgatewayParametersWorkload{
					Kind: agentgateway.AgentgatewayParametersWorkloadDaemonSet,
				},
			},
		},
	}
	ds := &appsv1.DaemonSet{
		ObjectMeta: metav1.ObjectMeta{
			Name:      testGateway.Name,
			Namespace: testGateway.Namespace,
		},
	}

	fakeClient := fake.NewClient(t, gw, gwc, agwp, ds)

	publisher := NewPublisher(fakeClient)
	fakeRecorder := record.NewFakeRecorder(10)
	publisher.eventRecorder = fakeRecorder

	fakeClient.RunAndWait(ctx.Done())

	fakeClient.WaitForCacheSync("test-publisher", ctx.Done(), publisher.HasSynced)

	workloadRef := publisher.workloadObjectReference(gw)
	assert.NotNil(t, workloadRef)
	assert.Equal(t, wellknown.DaemonSetGVK.Kind, workloadRef.Kind)

	publisher.PublishNack(&testNackEvent)

	assertRecordedEvent(t, fakeRecorder, "gateway", "Warning", ReasonNack, testErrorMessage)
	assertRecordedEvent(t, fakeRecorder, "workload", "Warning", ReasonNack, testErrorMessage)
}

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
	fakeRecorder := record.NewFakeRecorder(10)
	publisher.eventRecorder = fakeRecorder

	fakeClient.RunAndWait(ctx.Done())
	fakeClient.WaitForCacheSync("test-publisher", ctx.Done(), publisher.HasSynced)

	publisher.PublishNack(&testNackEvent)

	assertRecordedEvent(t, fakeRecorder, "gateway", "Warning", ReasonNack, testErrorMessage)

	select {
	case event := <-fakeRecorder.Events:
		t.Fatalf("expected no workload event, got %q", event)
	default:
	}
}

func assertRecordedEvent(
	t *testing.T,
	recorder *record.FakeRecorder,
	eventType string,
	contains ...string,
) {
	t.Helper()

	select {
	case event := <-recorder.Events:
		for _, expected := range contains {
			assert.Contains(t, event, expected)
		}
	default:
		t.Fatalf("Expected %s event to be recorded but none was found", eventType)
	}
}
