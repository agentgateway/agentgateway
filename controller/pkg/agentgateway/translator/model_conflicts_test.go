package translator

import (
	"testing"
	"time"

	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

func TestDuplicateModelNamePrecedence(t *testing.T) {
	listener := modelRouterKey{listener: "default/gw.llm"}
	extra := modelRouterKey{listener: "default/gw.extra"}
	route := modelRouterKey{httpRoute: "/llm:router:httproute:default:tenant1:models"}
	otherRoute := modelRouterKey{httpRoute: "/llm:router:httproute:default:tenant1:other"}
	customGVK := schema.GroupVersionKind{Group: "custom.example.com", Version: "v1", Kind: "CatalogModel"}

	older := conflictModel("default", "a", "2024-01-01T00:00:00Z", "gpt", schema.GroupVersionKind{})
	newer := conflictModel("default", "b", "2024-01-02T00:00:00Z", "gpt", schema.GroupVersionKind{})
	alsoNew := conflictModel("default", "c", "2024-01-03T00:00:00Z", "gpt", schema.GroupVersionKind{})
	namedA := conflictModel("default", "a", "2024-06-01T00:00:00Z", "claude", schema.GroupVersionKind{})
	namedB := conflictModel("default", "b", "2024-06-01T00:00:00Z", "claude", schema.GroupVersionKind{})
	nsA := conflictModel("a", "m", "2024-06-01T00:00:00Z", "haiku", schema.GroupVersionKind{})
	nsB := conflictModel("b", "m", "2024-06-01T00:00:00Z", "haiku", schema.GroupVersionKind{})
	kindBuiltin := conflictModel("default", "a", "2024-06-01T00:00:00Z", "gpt", schema.GroupVersionKind{})
	kindCustom := conflictModel("default", "a", "2024-06-01T00:00:00Z", "gpt", customGVK)
	customOlder := conflictModel("default", "a", "2024-01-01T00:00:00Z", "gpt", customGVK)
	builtinNewer := conflictModel("default", "b", "2024-01-02T00:00:00Z", "gpt", schema.GroupVersionKind{})
	byName := conflictModel("default", "gpt", "2024-01-01T00:00:00Z", "", schema.GroupVersionKind{})
	explicit := conflictModel("default", "alias", "2024-01-02T00:00:00Z", "gpt", schema.GroupVersionKind{})
	wildA := conflictModel("default", "wild-a", "2024-01-01T00:00:00Z", "gpt-*", schema.GroupVersionKind{})
	wildB := conflictModel("default", "wild-b", "2024-01-02T00:00:00Z", "gpt-*", schema.GroupVersionKind{})
	otherName := conflictModel("default", "other", "2024-01-01T00:00:00Z", "not-gpt", schema.GroupVersionKind{})

	tests := []struct {
		name   string
		model  *agentgateway.AgentgatewayModel
		claims []modelClaim
		router modelRouterKey
		winner string // empty when model keeps the router
	}{
		{
			name:   "oldest timestamp wins",
			model:  newer,
			claims: []modelClaim{{model: older, router: listener}, {model: newer, router: listener}, {model: alsoNew, router: listener}},
			router: listener,
			winner: "default/a",
		},
		{
			name:   "oldest model keeps the router",
			model:  older,
			claims: []modelClaim{{model: older, router: listener}, {model: newer, router: listener}},
			router: listener,
		},
		{
			name:   "same timestamp then namespace",
			model:  nsB,
			claims: []modelClaim{{model: nsB, router: listener}, {model: nsA, router: listener}},
			router: listener,
			winner: "a/m",
		},
		{
			name:   "same timestamp and namespace then name",
			model:  namedB,
			claims: []modelClaim{{model: namedB, router: listener}, {model: namedA, router: listener}},
			router: listener,
			winner: "default/a",
		},
		{
			name:   "same metadata then group and kind",
			model:  kindCustom,
			claims: []modelClaim{{model: kindCustom, router: listener}, {model: kindBuiltin, router: listener}},
			router: listener,
			winner: "default/a",
		},
		{
			name:   "other listener is a different router",
			model:  newer,
			claims: []modelClaim{{model: older, router: listener}, {model: newer, router: listener}, {model: newer, router: extra}},
			router: extra,
		},
		{
			name:   "http route rule is one router",
			model:  newer,
			claims: []modelClaim{{model: older, router: route}, {model: newer, router: route}, {model: newer, router: otherRoute}},
			router: route,
			winner: "default/a",
		},
		{
			name:   "other http route rule is unaffected",
			model:  newer,
			claims: []modelClaim{{model: older, router: route}, {model: newer, router: route}, {model: newer, router: otherRoute}},
			router: otherRoute,
		},
		{
			name:   "omitted match.model uses the object name",
			model:  explicit,
			claims: []modelClaim{{model: byName, router: listener}, {model: explicit, router: listener}},
			router: listener,
			winner: "default/gpt",
		},
		{
			name:   "different names do not conflict",
			model:  newer,
			claims: []modelClaim{{model: otherName, router: listener}, {model: newer, router: listener}},
			router: listener,
		},
		{
			name:   "wildcard patterns are out of scope",
			model:  wildB,
			claims: []modelClaim{{model: wildA, router: listener}, {model: wildB, router: listener}},
			router: listener,
		},
		{
			name:   "exact name does not conflict with a wildcard",
			model:  newer,
			claims: []modelClaim{{model: wildA, router: listener}, {model: newer, router: listener}},
			router: listener,
		},
		{
			name:   "older custom kind wins across model kinds",
			model:  builtinNewer,
			claims: []modelClaim{{model: customOlder, router: listener}, {model: builtinNewer, router: listener}},
			router: listener,
			winner: "CatalogModel default/a",
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			losers := losingModelRouters(tt.model, tt.claims)
			got := ""
			if winner := losers[tt.router]; winner != nil {
				got = modelConflictIdentity(winner)
			}
			if got != tt.winner {
				t.Fatalf("winner = %q, want %q", got, tt.winner)
			}
		})
	}
}

func TestDenyConflictingModelParents(t *testing.T) {
	older := conflictModel("default", "a", "2024-01-01T00:00:00Z", "gpt", schema.GroupVersionKind{})
	newer := conflictModel("default", "b", "2024-01-02T00:00:00Z", "gpt", schema.GroupVersionKind{})
	routeKey := "/llm:router:httproute:default:tenant1:models"
	section := gwv1.SectionName("models")
	routeParent := func(listener string) RouteParentReference {
		return RouteParentReference{
			ListenerKey:    listener,
			ModelRouterKey: routeKey,
			ParentKey: utils.TypedNamespacedName{
				Kind:           wellknown.HTTPRouteKind,
				NamespacedName: types.NamespacedName{Namespace: "default", Name: "tenant1"},
			},
			ParentSection: section,
			OriginalReference: gwv1.ParentReference{
				Name:        "tenant1",
				SectionName: &section,
			},
		}
	}
	parents := []RouteParentReference{
		routeParent("default/gw.http"),
		routeParent("default/gw.https"),
		{ListenerKey: "default/gw.extra"},
	}
	claims := []modelClaim{
		{model: older, router: modelRouterKey{httpRoute: routeKey}},
		{model: newer, router: modelRouterKey{httpRoute: routeKey}},
		{model: newer, router: modelRouterKey{listener: "default/gw.extra"}},
	}

	got := denyConflictingModelParents(newer, parents, claims)
	if parents[0].DeniedReason != nil || parents[1].DeniedReason != nil || parents[2].DeniedReason != nil {
		t.Fatal("conflict rejection mutated the caller's parent slice")
	}
	want := `match.model "gpt" conflicts with default/a on HTTPRoute default/tenant1 rule models`
	for _, parent := range got[:2] {
		if parent.DeniedReason == nil || parent.DeniedReason.Reason != ParentErrorModelConflict || parent.DeniedReason.Message != want {
			t.Fatalf("parent %s denial = %#v, want %q", parent.ListenerKey, parent.DeniedReason, want)
		}
	}
	if got[2].DeniedReason != nil {
		t.Fatalf("other router denied: %#v", got[2].DeniedReason)
	}

	listenerParent := RouteParentReference{ListenerKey: "default/gw.llm"}
	denied := denyConflictingModelParents(newer, []RouteParentReference{listenerParent}, []modelClaim{
		{model: older, router: modelRouterKey{listener: listenerParent.ListenerKey}},
		{model: newer, router: modelRouterKey{listener: listenerParent.ListenerKey}},
	})
	wantListener := `match.model "gpt" conflicts with default/a on listener default/gw.llm`
	if denied[0].DeniedReason == nil || denied[0].DeniedReason.Message != wantListener {
		t.Fatalf("listener denial = %#v, want %q", denied[0].DeniedReason, wantListener)
	}
}

func TestRejectDuplicateModelNamesSkipsWildcards(t *testing.T) {
	match := agentgateway.LongString("gpt-*")
	model := &agentgateway.AgentgatewayModel{
		ObjectMeta: metav1.ObjectMeta{Namespace: "default", Name: "wild"},
		Spec:       agentgateway.AgentgatewayModelSpec{Match: &agentgateway.ModelMatch{Model: &match}},
	}
	parents := []RouteParentReference{{ListenerKey: "default/gw.llm"}}
	got := rejectDuplicateModelNames(RouteContext{}, model, parents)
	if len(got) != 1 || got[0].DeniedReason != nil || got[0].ListenerKey != parents[0].ListenerKey {
		t.Fatalf("wildcard parents = %#v", got)
	}
}

func conflictModel(namespace, name, created, match string, gvk schema.GroupVersionKind) *agentgateway.AgentgatewayModel {
	parsed, err := time.Parse(time.RFC3339, created)
	if err != nil {
		panic(err)
	}
	model := &agentgateway.AgentgatewayModel{
		ObjectMeta: metav1.ObjectMeta{
			Namespace:         namespace,
			Name:              name,
			CreationTimestamp: metav1.NewTime(parsed),
		},
	}
	if match != "" {
		pattern := agentgateway.LongString(match)
		model.Spec.Match = &agentgateway.ModelMatch{Model: &pattern}
	}
	if gvk.Kind != "" {
		model.APIVersion = gvk.GroupVersion().String()
		model.Kind = gvk.Kind
	}
	return model
}
