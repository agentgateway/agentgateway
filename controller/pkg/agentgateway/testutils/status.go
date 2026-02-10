package testutils

import (
	gocmp "cmp"
	"context"
	"encoding/json"
	"sync"

	"istio.io/istio/pilot/pkg/config/kube/crd"
	"istio.io/istio/pkg/config/schema/gvk"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/slices"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/agentgatewaysyncer/status"
)

var _ status.WorkerQueue = &TestStatusQueue{}

type TestStatusQueue struct {
	mu           sync.Mutex
	state        map[status.Resource]any
	includeKinds []string
}

func (t *TestStatusQueue) Push(target status.Resource, data any) {
	t.mu.Lock()
	defer t.mu.Unlock()
	t.state[target] = data
}

func (t *TestStatusQueue) Run(ctx context.Context) {
}

func (t *TestStatusQueue) Dump() []any {
	t.mu.Lock()
	defer t.mu.Unlock()
	objs := []crd.IstioKind{}
	for k, v := range t.state {
		statusj, _ := json.Marshal(v)
		if len(t.includeKinds) > 0 && !slices.Contains(t.includeKinds, k.Kind) {
			continue
		}
		obj := crd.IstioKind{
			TypeMeta: metav1.TypeMeta{
				Kind:       k.Kind,
				APIVersion: k.GroupVersion().String(),
			},
			ObjectMeta: metav1.ObjectMeta{
				Name:      k.Name,
				Namespace: k.Namespace,
			},
			Spec:   nil,
			Status: ptr.Of(json.RawMessage(statusj)),
		}
		objs = append(objs, obj)
	}
	slices.SortFunc(objs, func(a, b crd.IstioKind) int {
		ord := []string{gvk.GatewayClass.Kind, gvk.Gateway.Kind, gvk.HTTPRoute.Kind, gvk.GRPCRoute.Kind, gvk.TLSRoute.Kind, gvk.TCPRoute.Kind}
		if r := gocmp.Compare(slices.Index(ord, a.Kind), slices.Index(ord, b.Kind)); r != 0 {
			return r
		}
		if r := a.CreationTimestamp.Time.Compare(b.CreationTimestamp.Time); r != 0 {
			return r
		}
		if r := gocmp.Compare(a.Namespace, b.Namespace); r != 0 {
			return r
		}
		return gocmp.Compare(a.Name, b.Name)
	})
	return slices.Map(objs, func(e crd.IstioKind) any {
		return e
	})
}
