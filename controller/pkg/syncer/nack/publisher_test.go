package nack

import (
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"k8s.io/apimachinery/pkg/types"
	"k8s.io/client-go/tools/record"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/apiclient/fake"
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
		Name:      testGateway.Name,
		Namespace: testGateway.Namespace,
	}

	fakeClient := fake.NewClient(t, gw)

	publisher := NewPublisher(fakeClient)
	fakeRecorder := record.NewFakeRecorder(10)
	publisher.eventRecorder = fakeRecorder

	fakeClient.RunAndWait(ctx.Done())

	fakeClient.WaitForCacheSync("test-publisher", ctx.Done(), publisher.HasSynced)

	for _, tt := range []struct {
		name    string
		reason  string
		publish func()
	}{
		{name: "rejection", reason: ReasonNack, publish: func() { publisher.PublishNack(&testNackEvent) }},
		{name: "warning", reason: "AgentGatewayConfigWarning", publish: func() { publisher.PublishWarning(testGateway, testErrorMessage) }},
	} {
		t.Run(tt.name, func(t *testing.T) {
			tt.publish()
			select {
			case event := <-fakeRecorder.Events:
				assert.Contains(t, event, "Warning")
				assert.Contains(t, event, tt.reason)
				assert.Contains(t, event, testErrorMessage)
			default:
				t.Fatal("Expected event to be recorded but none was found")
			}
		})
	}
}
