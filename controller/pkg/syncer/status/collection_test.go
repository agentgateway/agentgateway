package status

import (
	"context"
	"testing"

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

	enqueueStatus(queue, model, wantStatus, nil)

	if queue.target.GroupVersionKind != wellknown.AgentgatewayModelGVK {
		t.Fatalf("GVK = %v, want %v", queue.target.GroupVersionKind, wellknown.AgentgatewayModelGVK)
	}
	if queue.target.Name != model.Name || queue.target.Namespace != model.Namespace {
		t.Fatalf("namespaced name = %s/%s, want %s/%s", queue.target.Namespace, queue.target.Name, model.Namespace, model.Name)
	}
	if queue.data != wantStatus {
		t.Fatalf("queued status = %p, want %p", queue.data, wantStatus)
	}
}

func TestEnqueueStatusMergesUpdatesToTheSameObject(t *testing.T) {
	model := &agentgateway.AgentgatewayModel{}
	model.Name = "model"
	model.Namespace = "default"
	model.ResourceVersion = "123"
	first := &captureQueue{}
	enqueueStatus(first, model, &agentgateway.AgentgatewayModelStatus{}, nil)

	model.ResourceVersion = "456"
	second := &captureQueue{}
	enqueueStatus(second, model, &agentgateway.AgentgatewayModelStatus{}, nil)

	// Both pushes must land on the same queue key: the key identifies the object, not one version of it. An
	// update that produced a new key would look like another target, so the pushes would not coalesce and the
	// object could have more than one status write in flight.
	queue := &WorkQueue{pending: map[Resource]any{}, processing: map[Resource]any{}}
	queue.Enqueue(first.target, first.data)
	queue.Enqueue(second.target, second.data)

	if got := queue.Length(); got != 1 {
		t.Fatalf("queue length after two updates to %v = %d, want 1", first.target, got)
	}
	if _, data, ok := queue.Dequeue(); !ok || data != second.data {
		t.Fatalf("dequeued status = %p, want the status of the newest update (%p)", data, second.data)
	}
}

func TestEnqueueStatusMergesUpdatesWhileProcessing(t *testing.T) {
	queue := &WorkQueue{pending: map[Resource]any{}, processing: map[Resource]any{}}
	model := &agentgateway.AgentgatewayModel{}
	model.Name = "model"
	model.Namespace = "default"
	push := func(version string, status *agentgateway.AgentgatewayModelStatus) {
		model.ResourceVersion = version
		captured := &captureQueue{}
		enqueueStatus(captured, model, status, nil)
		queue.Enqueue(captured.target, captured.data)
	}

	old := &agentgateway.AgentgatewayModelStatus{}
	newest := &agentgateway.AgentgatewayModelStatus{}
	push("123", old)
	key, data, ok := queue.Dequeue()
	if !ok || data != old {
		t.Fatalf("initial dequeue: ok = %t, status = %p, want %p", ok, data, old)
	}

	// Updates arriving before MarkDone must wait and retain only the latest status.
	push("456", &agentgateway.AgentgatewayModelStatus{})
	push("789", newest)
	if _, _, ok := queue.Dequeue(); ok {
		t.Fatal("object dequeued again while its previous write is still processing")
	}

	queue.MarkDone(key)
	nextKey, data, ok := queue.Dequeue()
	if !ok || nextKey != key || data != newest {
		t.Fatalf("follow-up dequeue: ok = %t, key = %v, status = %p; want key = %v, status = %p", ok, nextKey, data, key, newest)
	}
	queue.MarkDone(nextKey)
	if _, _, ok := queue.Dequeue(); ok {
		t.Fatal("unexpected extra write after processing the newest status")
	}
}
