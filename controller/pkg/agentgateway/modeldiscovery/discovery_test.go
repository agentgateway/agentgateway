package modeldiscovery

import (
	"errors"
	"io"
	"net/http"
	"strings"
	"sync"
	"testing"
	"time"

	"istio.io/istio/pkg/kube/krt"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	inf "sigs.k8s.io/gateway-api-inference-extension/api/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
)

func TestParseAnchor(t *testing.T) {
	model := discoveryAnchor()
	cfg, enabled, err := ParseAnchor(model)
	if err != nil {
		t.Fatal(err)
	}
	if !enabled {
		t.Fatal("discovery should be enabled")
	}
	if cfg.Pool.Namespace != "default" || cfg.Pool.Name != "llama-pool" {
		t.Fatalf("pool = %v, want default/llama-pool", cfg.Pool)
	}
	if cfg.Path != defaultPath || cfg.Interval != defaultInterval || cfg.StaleAfter != defaultStaleAfter {
		t.Fatalf("unexpected defaults: %#v", cfg)
	}

	model.Spec.Match = &agentgateway.ModelMatch{}
	if _, _, err := ParseAnchor(model); err == nil || !strings.Contains(err.Error(), "spec.match") {
		t.Fatalf("error = %v, want match validation", err)
	}

	model = discoveryAnchor()
	model.Annotations[PathAnnotation] = "http://metadata.invalid/models"
	if _, _, err := ParseAnchor(model); err == nil || !strings.Contains(err.Error(), "absolute path") {
		t.Fatalf("error = %v, want path validation", err)
	}
}

func TestParseResponse(t *testing.T) {
	models, err := parseResponse(strings.NewReader(`{
		"object": "list",
		"data": [
			{"id": "z-model", "created": 20, "owned_by": "runtime"},
			{"id": "a-model", "created": 10},
			{"id": "a-model", "created": 15}
		]
	}`))
	if err != nil {
		t.Fatal(err)
	}
	if len(models) != 2 {
		t.Fatalf("models = %d, want 2", len(models))
	}
	if models[0].ID != "a-model" || models[0].Created != 10 || models[1].ID != "z-model" {
		t.Fatalf("unexpected models: %#v", models)
	}

	if _, err := parseResponse(strings.NewReader(`{"data":[{"id":""}]}`)); err == nil {
		t.Fatal("empty model ID should fail")
	}
}

func TestControllerRetainsAndExpiresLastKnownGood(t *testing.T) {
	now := time.Unix(1_700_000_000, 0)
	transport := &mutableTransport{
		body: `{"object":"list","data":[{"id":"tweet-summary","created":42}]}`,
	}
	models := krt.NewStaticCollection[*agentgateway.AgentgatewayModel](nil, []*agentgateway.AgentgatewayModel{discoveryAnchor()})
	pools := krt.NewStaticCollection[*inf.InferencePool](nil, []*inf.InferencePool{testPool()})
	pods := krt.NewStaticCollection[*corev1.Pod](nil, []*corev1.Pod{readyPod()})
	c := &Controller{
		models:     models,
		pools:      pools,
		pods:       pods,
		discovered: krt.NewStaticCollection[Model](nil, nil),
		client:     &http.Client{Transport: transport},
		now:        func() time.Time { return now },
		states:     map[types.NamespacedName]anchorState{},
	}

	c.reconcile(t.Context())
	if got := c.discovered.List(); len(got) != 1 || got[0].ID != "tweet-summary" {
		t.Fatalf("discovered models = %#v", got)
	}

	transport.setError(errors.New("runtime unavailable"))
	now = now.Add(defaultInterval + time.Second)
	c.reconcile(t.Context())
	if got := c.discovered.List(); len(got) != 1 {
		t.Fatalf("transient failure removed last-known-good models: %#v", got)
	}

	now = now.Add(defaultStaleAfter)
	c.reconcile(t.Context())
	if got := c.discovered.List(); len(got) != 0 {
		t.Fatalf("expired models = %#v, want empty", got)
	}
}

func discoveryAnchor() *agentgateway.AgentgatewayModel {
	provider := agentgateway.ModelProviderCustom
	group := "inference.networking.k8s.io"
	kind := "InferencePool"
	return &agentgateway.AgentgatewayModel{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: "default",
			Name:      "llama-models",
			Annotations: map[string]string{
				EnabledAnnotation: "enabled",
			},
			CreationTimestamp: metav1.NewTime(time.Unix(100, 0)),
		},
		Spec: agentgateway.AgentgatewayModelSpec{
			Provider: &provider,
			Custom: &agentgateway.CustomProviderSettings{
				BackendRef: &agentgateway.LocalBackendObjectReference{
					Group: &group,
					Kind:  &kind,
					Name:  "llama-pool",
				},
				Formats: []agentgateway.ProviderFormatConfig{{
					Type: agentgateway.ProviderFormatCompletions,
				}},
			},
		},
	}
}

func testPool() *inf.InferencePool {
	return &inf.InferencePool{
		ObjectMeta: metav1.ObjectMeta{Namespace: "default", Name: "llama-pool"},
		Spec: inf.InferencePoolSpec{
			Selector: inf.LabelSelector{MatchLabels: map[inf.LabelKey]inf.LabelValue{
				"app": "llama",
			}},
			TargetPorts: []inf.Port{{Number: 8000}},
		},
	}
}

func readyPod() *corev1.Pod {
	return &corev1.Pod{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: "default",
			Name:      "llama-0",
			Labels:    map[string]string{"app": "llama"},
		},
		Status: corev1.PodStatus{
			Phase: corev1.PodRunning,
			PodIP: "10.0.0.1",
			Conditions: []corev1.PodCondition{{
				Type:   corev1.PodReady,
				Status: corev1.ConditionTrue,
			}},
		},
	}
}

type mutableTransport struct {
	mu   sync.Mutex
	body string
	err  error
}

func (m *mutableTransport) RoundTrip(*http.Request) (*http.Response, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.err != nil {
		return nil, m.err
	}
	return &http.Response{
		StatusCode: http.StatusOK,
		Body:       io.NopCloser(strings.NewReader(m.body)),
		Header:     make(http.Header),
	}, nil
}

func (m *mutableTransport) setError(err error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.err = err
}
