package status

import (
	"context"
	"testing"

	"k8s.io/apimachinery/pkg/runtime/schema"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

type captureQueue struct {
	target Resource
	data   any
}

func (c *captureQueue) Push(target Resource, data any) {
	c.target = target
	c.data = data
}

func (*captureQueue) Run(context.Context) {}

func TestEnqueueStatusAgentgatewayModelWithoutTypeMeta(t *testing.T) {
	queue := &captureQueue{}
	model := &agentgateway.AgentgatewayModel{}
	model.Name = "model"
	model.Namespace = "default"
	model.ResourceVersion = "123"
	wantStatus := &agentgateway.AgentgatewayModelStatus{}

	enqueueStatus(queue, model, wantStatus, []schema.GroupVersionKind(nil))

	if queue.target.GroupVersionKind != wellknown.AgentgatewayModelGVK {
		t.Fatalf("GVK = %v, want %v", queue.target.GroupVersionKind, wellknown.AgentgatewayModelGVK)
	}
	if queue.target.Name != model.Name || queue.target.Namespace != model.Namespace {
		t.Fatalf("namespaced name = %s/%s, want %s/%s", queue.target.Namespace, queue.target.Name, model.Namespace, model.Name)
	}
	if queue.target.ResourceVersion != model.ResourceVersion {
		t.Fatalf("resource version = %q, want %q", queue.target.ResourceVersion, model.ResourceVersion)
	}
	if queue.data != wantStatus {
		t.Fatalf("queued status = %p, want %p", queue.data, wantStatus)
	}
}
