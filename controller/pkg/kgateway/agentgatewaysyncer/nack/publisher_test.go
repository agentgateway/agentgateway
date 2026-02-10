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

	"github.com/kgateway-dev/kgateway/v2/pkg/apiclient/fake"
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

func TestPublisher_PublishNack(t *testing.T) {
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

	publisher.PublishNack(&testNackEvent)

	select {
	case event := <-fakeRecorder.Events:
		assert.Contains(t, event, "Warning")
		assert.Contains(t, event, ReasonNack)
		assert.Contains(t, event, testErrorMessage)
	default:
		t.Fatal("Expected event to be recorded but none was found")
	}
}
