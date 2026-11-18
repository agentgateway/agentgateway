package translator

import (
	"fmt"
	"math/rand"
	"slices"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/test"
	"istio.io/istio/pkg/util/sets"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	agwir "github.com/agentgateway/agentgateway/controller/pkg/agentgateway/ir"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

var conflictBase = time.Date(2026, 11, 15, 10, 0, 0, 0, time.UTC)

func TestModelPrecedence(t *testing.T) {
	now := conflictBase.Add(123456789 * time.Nanosecond)
	m := func(ns, name string, alpha bool, created time.Time) *agentgateway.AgentgatewayModel {
		return testModel{ns: ns, name: name, alpha: alpha, created: created}.object()
	}
	cases := []struct {
		name           string
		earlier, later *agentgateway.AgentgatewayModel
	}{
		{"older timestamp beats smaller namespace/name", m("z", "z", false, now), m("a", "a", false, now.Add(time.Nanosecond))},
		{"sub-second difference is honored", m("z", "z", false, now), m("a", "a", false, now.Add(time.Millisecond))},
		{"zero timestamp is oldest", m("z", "z", false, time.Time{}), m("a", "a", false, now)},
		{"same timestamp: namespace", m("a", "z", false, now), m("b", "a", false, now)},
		{"same timestamp: name", m("a", "a", false, now), m("a", "b", false, now)},
		{"same timestamp, namespace and name: group then kind", m("a", "a", false, now), m("a", "a", true, now)},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			assert.True(t, modelPrecedes(tc.earlier, tc.later))
			assert.False(t, modelPrecedes(tc.later, tc.earlier), "order must be strict")
			assert.False(t, sameModel(tc.earlier, tc.later) && tc.earlier.Namespace != tc.later.Namespace)
		})
	}
	assert.False(t, sameModel(m("a", "a", false, now), m("a", "a", true, now)), "different kinds are different models")
}

func TestModelRouterNameKeepsRoutersDistinct(t *testing.T) {
	listener := RouteParentReference{ListenerKey: "default/gw.llm"}
	rule := RouteParentReference{ListenerKey: "default/gw.llm", ModelRouterKey: "/llm:router:httproute:default:r:models"}
	otherRule := RouteParentReference{ListenerKey: "default/gw.llm", ModelRouterKey: "/llm:router:httproute:default:r:other"}
	assert.Equal(t, "listener:default/gw.llm", modelRouterName(listener))
	assert.Equal(t, "rule:/llm:router:httproute:default:r:models", modelRouterName(rule))
	assert.NotEqual(t, modelRouterName(listener), modelRouterName(rule))
	assert.NotEqual(t, modelRouterName(rule), modelRouterName(otherRule))
	assert.Empty(t, modelRouterName(RouteParentReference{}))
}

func TestExactModelName(t *testing.T) {
	m := func(name string, match *string) *agentgateway.AgentgatewayModel {
		model := &agentgateway.AgentgatewayModel{Name: name}
		if match != nil {
			model.Spec.Match = &agentgateway.ModelMatch{Model: match}
		}
		return model
	}
	explicit, wild, empty := "gpt", "gpt-*", ""
	for _, tc := range []struct {
		model *agentgateway.AgentgatewayModel
		want  string
		exact bool
	}{
		{m("implicit", nil), "implicit", true},
		{m("ignored", &explicit), "gpt", true},
		{m("w", &wild), "gpt-*", false},
		{m("w", &[]string{"*"}[0]), "*", false},
		{m("w", &[]string{"*-latest"}[0]), "*-latest", false},
		{m("e", &empty), "", true},
		{m("x", &[]string{"GPT"}[0]), "GPT", true}, // names are case-sensitive
		{m("x", &[]string{"ünï/😀 model"}[0]), "ünï/😀 model", true},
	} {
		got, exact := exactModelName(tc.model)
		assert.Equal(t, tc.want, got)
		assert.Equal(t, tc.exact, exact)
	}
}

// --- Reactive (KRT) tests -------------------------------------------------------------------------------------

// listenersResolver resolves a fixed set of listeners. The optional hosts collection sets the hostnames of the
// accepting listeners, ("<namespace>/*" entries limit them to those namespaces), which lets tests change a
// model's eligibility from outside the model.
type listenersResolver struct {
	infos []*ParentInfo
	hosts krt.Collection[listenerHosts]
}

// listenerHosts is a comma separated list of "<namespace>/*" hostnames for the accepting listeners.
type listenerHosts struct{ Hosts string }

func (l listenerHosts) ResourceName() string { return "hosts" }

func (r listenersResolver) ParentsFor(ctx krt.HandlerContext, _ utils.TypedNamespacedName) []*ParentInfo {
	if r.hosts == nil {
		return r.infos
	}
	cfg := krt.FetchOne(ctx, r.hosts)
	if cfg == nil || cfg.Hosts == "" {
		return r.infos
	}
	out := make([]*ParentInfo, 0, len(r.infos))
	for _, info := range r.infos {
		clone := *info
		if info.SectionName != "restricted" {
			clone.Hostnames = strings.Split(cfg.Hosts, ",")
		}
		out = append(out, &clone)
	}
	return out
}

const (
	alphaGroup = "example.io"
	alphaKind  = "Alpha"
)

var alphaGVK = schema.GroupVersionKind{Group: alphaGroup, Version: "v1", Kind: alphaKind}

// conflictEnv wires the real model translation and attachment collections over static inputs, the way the
// syncer does, so the tests exercise the production KRT dependency graph.
type conflictEnv struct {
	models      krt.StaticCollection[*agentgateway.AgentgatewayModel]
	alphas      krt.StaticCollection[*agentgateway.AgentgatewayModel]
	nativeState krt.Collection[krt.ObjectWithStatus[*agentgateway.AgentgatewayModel, agentgateway.AgentgatewayModelStatus]]
	alphaState  krt.Collection[krt.ObjectWithStatus[*agentgateway.AgentgatewayModel, agentgateway.AgentgatewayModelStatus]]
	nativeOut   krt.Collection[agwir.AgwResource]
	alphaOut    krt.Collection[agwir.AgwResource]
	attach      krt.Collection[*plugins.RouteAttachment]
	hosts       krt.StaticCollection[listenerHosts]
	translated  atomic.Int64 // number of TranslateModel invocations
}

func newConflictEnv(t *testing.T, routes ...*gwv1.HTTPRoute) *conflictEnv {
	stop := test.NewStop(t)
	opts := krtutil.NewKrtOptions(stop, nil)
	gateway := types.NamespacedName{Namespace: "default", Name: "gw"}
	allowed := []gwv1.RouteGroupKind{
		{Group: new(gwv1.Group(wellknown.AgentgatewayModelGVK.Group)), Kind: gwv1.Kind(wellknown.AgentgatewayModelGVK.Kind)},
		{Group: new(gwv1.Group(alphaGroup)), Kind: alphaKind},
		{Group: new(gwv1.Group(wellknown.HTTPRouteGVK.Group)), Kind: gwv1.Kind(wellknown.HTTPRouteGVK.Kind)},
	}
	resolver := listenersResolver{infos: []*ParentInfo{
		{ParentGateway: gateway, ListenerKey: "default/gw.llm", SectionName: "llm", AllowedKinds: allowed},
		{ParentGateway: gateway, ListenerKey: "default/gw.llm2", SectionName: "llm2", AllowedKinds: allowed},
		// A listener that does not accept models: attachments to it are denied before any conflict check.
		{ParentGateway: gateway, ListenerKey: "default/gw.restricted", SectionName: "restricted", AllowedKinds: allowed[2:]},
	}}
	env := &conflictEnv{}
	env.hosts = krt.NewMutableCollection[listenerHosts](nil, nil, opts.ToOptions("test/Hosts")...)
	resolver.hosts = env.hosts.AsCollection()
	env.models = krt.NewMutableCollection[*agentgateway.AgentgatewayModel](nil, nil, opts.ToOptions("test/Models")...)
	env.alphas = krt.NewMutableCollection[*agentgateway.AgentgatewayModel](nil, nil, opts.ToOptions("test/Alphas")...)
	httpRoutes := krt.NewStaticCollection[*gwv1.HTTPRoute](nil, routes, opts.ToOptions("test/Routes")...)

	inputs := RouteContextInputs{
		Collections: &plugins.AgwCollections{
			HTTPRoutes:   httpRoutes,
			ModelSources: map[schema.GroupKind]plugins.ModelSource{alphaGVK.GroupKind(): {GVK: alphaGVK, Models: env.alphas.AsCollection()}},
		},
		RouteParents: resolver,
		Models:       env.models.AsCollection(),
		References:   plugins.ReferenceTypes{AllowedParentReferences: sets.New(wellknown.GatewayGVK.GroupKind())},
	}
	inputs.ParentRefs = RouteParentRefsCollection(inputs, httpRoutes,
		krt.NewStaticCollection[*gwv1.GRPCRoute](nil, nil), krt.NewStaticCollection[*gwv1.TCPRoute](nil, nil),
		krt.NewStaticCollection[*gwv1.TLSRoute](nil, nil), env.models.AsCollection(), opts)
	inputs.ModelNameIndex = NewModelNameIndex(inputs)

	translate := func(models krt.Collection[*agentgateway.AgentgatewayModel], name string) (
		krt.Collection[krt.ObjectWithStatus[*agentgateway.AgentgatewayModel, agentgateway.AgentgatewayModelStatus]], krt.Collection[agwir.AgwResource],
	) {
		return krt.NewStatusManyCollection(models, func(krtctx krt.HandlerContext, obj *agentgateway.AgentgatewayModel) (*agentgateway.AgentgatewayModelStatus, []agwir.AgwResource) {
			env.translated.Add(1)
			return TranslateModel(inputs.WithCtx(krtctx), obj)
		}, opts.ToOptions(name)...)
	}
	env.nativeState, env.nativeOut = translate(env.models.AsCollection(), "test/translate")
	env.alphaState, env.alphaOut = translate(env.alphas.AsCollection(), "test/translateAlpha")
	nativeAttach, _ := ModelReferences(inputs, env.models.AsCollection(), wellknown.AgentgatewayModelGVK, opts)
	alphaAttach, _ := ModelReferences(inputs, env.alphas.AsCollection(), alphaGVK, opts)
	env.attach = krt.JoinCollection([]krt.Collection[*plugins.RouteAttachment]{nativeAttach, alphaAttach}, opts.ToOptions("test/attach")...)
	return env
}

// testModel describes a model; the zero value attaches nothing and serves provider OpenAI.
type testModel struct {
	ns, name, match string
	created         time.Time
	sections        []string // direct Gateway listener parents
	httpRoutes      []string // "route" or "route/rule" HTTPRoute parents
	alpha           bool
	virtualRefs     []string // weighted targets by same-namespace native modelRef
	internal        bool
	allListeners    bool // one parentRef to the Gateway without a sectionName
}

func (m testModel) key() string {
	ns := m.ns
	if ns == "" {
		ns = "default"
	}
	return ns + "/" + m.name
}

func (m testModel) object() *agentgateway.AgentgatewayModel {
	ns, _, _ := strings.Cut(m.key(), "/")
	model := &agentgateway.AgentgatewayModel{Namespace: ns, Name: m.name, CreationTimestamp: metav1.NewTime(m.created)}
	if m.alpha {
		model.SetGroupVersionKind(alphaGVK)
	}
	for _, s := range m.sections {
		model.Spec.ParentRefs = append(model.Spec.ParentRefs, gwv1.ParentReference{
			Group: new(gwv1.Group(wellknown.GatewayGVK.Group)), Kind: new(gwv1.Kind(wellknown.GatewayGVK.Kind)),
			Name: "gw", Namespace: new(gwv1.Namespace("default")), SectionName: new(gwv1.SectionName(s)),
		})
	}
	if m.allListeners {
		model.Spec.ParentRefs = append(model.Spec.ParentRefs, gwv1.ParentReference{
			Group: new(gwv1.Group(wellknown.GatewayGVK.Group)), Kind: new(gwv1.Kind(wellknown.GatewayGVK.Kind)),
			Name: "gw", Namespace: new(gwv1.Namespace("default")),
		})
	}
	for _, r := range m.httpRoutes {
		route, rule, hasRule := strings.Cut(r, "/")
		ref := gwv1.ParentReference{
			Group: new(gwv1.Group(wellknown.HTTPRouteGVK.Group)), Kind: new(gwv1.Kind(wellknown.HTTPRouteGVK.Kind)), Name: gwv1.ObjectName(route),
		}
		if hasRule {
			ref.SectionName = new(gwv1.SectionName(rule))
		}
		model.Spec.ParentRefs = append(model.Spec.ParentRefs, ref)
	}
	if m.match != "" {
		model.Spec.Match = &agentgateway.ModelMatch{Model: new(m.match)}
	}
	if len(m.virtualRefs) > 0 {
		vm := &agentgateway.VirtualModel{Weighted: &agentgateway.WeightedModelRouting{}}
		for _, ref := range m.virtualRefs {
			vm.Weighted.Targets = append(vm.Weighted.Targets, agentgateway.WeightedModelTarget{
				ModelRef: agentgateway.ModelObjectReference{Name: gwv1.ObjectName(ref)},
			})
		}
		model.Spec.VirtualModel = vm
	} else {
		model.Spec.Provider = new(agentgateway.ModelProviderOpenAI)
	}
	if m.internal {
		model.Spec.Visibility = agentgateway.ModelVisibilityInternal
	}
	return model
}

func (e *conflictEnv) put(models ...testModel) {
	for _, m := range models {
		if m.alpha {
			e.alphas.UpdateObject(m.object())
		} else {
			e.models.UpdateObject(m.object())
		}
	}
}

func (e *conflictEnv) setHosts(hosts string) { e.hosts.UpdateObject(listenerHosts{Hosts: hosts}) }

func (e *conflictEnv) remove(models ...testModel) {
	for _, m := range models {
		if m.alpha {
			e.alphas.DeleteObject(m.key())
		} else {
			e.models.DeleteObject(m.key())
		}
	}
}

// modelRoute is a programmed ModelRoute reduced to what the tests assert on.
type modelRoute struct {
	key, listener, router, match string
	invalidTargets               []string
}

type snapshot struct {
	routes   []modelRoute
	backends []string
}

func (e *conflictEnv) snapshot() snapshot {
	var s snapshot
	for _, col := range []krt.Collection[agwir.AgwResource]{e.nativeOut, e.alphaOut} {
		for _, r := range col.List() {
			if mr := r.Resource.GetModelRoute(); mr != nil {
				route := modelRoute{key: mr.GetKey(), listener: strings.TrimPrefix(mr.GetListenerKey(), "default/gw."), router: mr.GetRouterKey(), match: mr.GetMatch().GetModel()}
				for _, target := range mr.GetVirtualModel().GetWeighted().GetTargets() {
					if target.GetInvalid() {
						route.invalidTargets = append(route.invalidTargets, target.GetModel())
					}
				}
				s.routes = append(s.routes, route)
			}
			if be := r.Resource.GetBackend(); be != nil {
				s.backends = append(s.backends, be.GetKey())
			}
		}
	}
	slices.SortFunc(s.routes, func(a, b modelRoute) int { return strings.Compare(a.key, b.key) })
	slices.Sort(s.backends)
	return s
}

// routesOf returns the programmed routes of a model: keys are "<ns>/<name>.<suffix>", optionally behind a kind prefix.
func (s snapshot) routesOf(m testModel) []modelRoute {
	var out []modelRoute
	for _, r := range s.routes {
		if strings.HasPrefix(r.key, m.key()+".") || strings.HasPrefix(r.key, alphaGroup+"/"+alphaKind+"/"+m.key()+".") && m.alpha {
			if m.alpha == strings.HasPrefix(r.key, alphaGroup+"/") {
				out = append(out, r)
			}
		}
	}
	return out
}

func (s snapshot) backendsOf(m testModel) []string {
	var out []string
	for _, b := range s.backends {
		if strings.HasPrefix(b, m.key()+"/") && !m.alpha {
			out = append(out, b)
		}
		if m.alpha && strings.Contains(b, alphaKind) && strings.Contains(b, "/"+m.name+"/") {
			out = append(out, b)
		}
	}
	return out
}

func (s snapshot) listeners(m testModel) []string {
	var out []string
	for _, r := range s.routesOf(m) {
		out = append(out, r.listener)
	}
	slices.Sort(out)
	return out
}

type parentStatus struct{ accepted, reason, message, resolved, resolvedMessage string }

// statusOf returns the status of a model keyed by parentRef sectionName (or name for non-listener parents).
func (e *conflictEnv) statusOf(m testModel) map[string]parentStatus {
	col := e.nativeState
	if m.alpha {
		col = e.alphaState
	}
	out := map[string]parentStatus{}
	for _, item := range col.List() {
		if item.Obj.Namespace+"/"+item.Obj.Name != m.key() {
			continue
		}
		for _, p := range item.Status.Parents {
			ps := parentStatus{}
			for _, c := range p.Conditions {
				switch c.Type {
				case string(gwv1.RouteConditionAccepted):
					ps.accepted, ps.reason, ps.message = string(c.Status), c.Reason, c.Message
				case string(gwv1.RouteConditionResolvedRefs):
					ps.resolved, ps.resolvedMessage = string(c.Status), c.Message
				}
			}
			section := ""
			if p.ParentRef.SectionName != nil {
				section = string(*p.ParentRef.SectionName)
			}
			out[section] = ps
		}
	}
	return out
}

func (e *conflictEnv) attachedModels() []string {
	var out []string
	for _, a := range e.attach.List() {
		out = append(out, a.From.Name+"@"+a.ListenerName)
	}
	slices.Sort(out)
	return out
}

func eventually(t *testing.T, what string, cond func() bool) {
	t.Helper()
	require.Eventually(t, cond, 10*time.Second, 5*time.Millisecond, what)
}

// converges waits for a state, lets queued events drain, then re-checks it still holds, since reconciliation must be
// idempotent and an intermediate state must not be mistaken for the final one.
func (e *conflictEnv) converges(t *testing.T, what string, check func(snapshot) bool) {
	t.Helper()
	defer func() {
		if t.Failed() {
			snap := e.snapshot()
			for _, r := range snap.routes {
				t.Logf("route %+v", r)
			}
			t.Logf("backends %v", snap.backends)
		}
	}()
	eventually(t, what, func() bool { return check(e.snapshot()) })
	e.quiesce(0) // let any queued events settle, then require the state to still hold
	require.True(t, check(e.snapshot()), "%s: state was not stable", what)
}

func TestDuplicateModelNameLifecycle(t *testing.T) {
	e := newConflictEnv(t)
	a := testModel{name: "a", match: "gpt", created: conflictBase, sections: []string{"llm"}}
	b := testModel{name: "b", match: "gpt", created: conflictBase.Add(time.Second), sections: []string{"llm"}}

	// Adding a newer duplicate rejects it; no route and no generated backend of the loser is programmed.
	e.put(a, b)
	e.converges(t, "a wins over newer b", func(s snapshot) bool {
		return len(s.routes) == 1 && len(s.routesOf(a)) == 1 && len(s.routesOf(b)) == 0 &&
			len(s.backendsOf(a)) == 1 && len(s.backendsOf(b)) == 0 && len(s.backends) == 1
	})
	assert.Equal(t, parentStatus{accepted: "True", reason: "Accepted", message: "Successfully accepted AgentgatewayModel", resolved: "True", resolvedMessage: ""}, e.statusOf(a)["llm"])
	loser := e.statusOf(b)["llm"]
	assert.Equal(t, "False", loser.accepted)
	assert.Equal(t, "Conflicted", loser.reason)
	assert.Equal(t, `match.model "gpt" conflicts with default/a on listener default/gw.llm`, loser.message)
	assert.Equal(t, "True", loser.resolved, "ResolvedRefs is independent of name conflicts")
	assert.Equal(t, []string{"a@llm"}, e.attachedModels(), "a rejected attachment is not counted")

	// Adding an older duplicate replaces the winner.
	c := testModel{name: "c", match: "gpt", created: conflictBase.Add(-time.Hour), sections: []string{"llm"}}
	e.put(c)
	e.converges(t, "older c replaces the winner", func(s snapshot) bool {
		return len(s.routes) == 1 && len(s.routesOf(c)) == 1 && len(s.backends) == 1 && len(s.backendsOf(c)) == 1
	})
	assert.Equal(t, "Conflicted", e.statusOf(a)["llm"].reason)
	assert.Contains(t, e.statusOf(a)["llm"].message, "default/c")
	assert.Equal(t, []string{"c@llm"}, e.attachedModels())

	// Deleting the winner promotes the next eligible model, and a stale conflict message does not linger.
	e.remove(c)
	e.converges(t, "a is promoted", func(s snapshot) bool { return len(s.routes) == 1 && len(s.routesOf(a)) == 1 })
	assert.Equal(t, "True", e.statusOf(a)["llm"].accepted)
	e.remove(a)
	e.converges(t, "b is promoted", func(s snapshot) bool {
		return len(s.routes) == 1 && len(s.routesOf(b)) == 1 && len(s.backendsOf(b)) == 1
	})
	assert.Equal(t, parentStatus{accepted: "True", reason: "Accepted", message: "Successfully accepted AgentgatewayModel", resolved: "True"}, e.statusOf(b)["llm"])

	// Renaming the winner releases the old name.
	e.put(a)
	e.converges(t, "a retakes gpt", func(s snapshot) bool { return len(s.routesOf(a)) == 1 && len(s.routesOf(b)) == 0 })
	renamed := a
	renamed.match = "gpt-renamed"
	e.put(renamed)
	e.converges(t, "rename releases the name", func(s snapshot) bool {
		return len(s.routes) == 2 && len(s.routesOf(b)) == 1 && s.routesOf(a)[0].match == "gpt-renamed"
	})
	assert.Equal(t, "True", e.statusOf(b)["llm"].accepted)

	// Moving a model to another parent recalculates the conflict on both routers.
	e.put(a) // gpt again, older than b
	e.converges(t, "a wins gpt again", func(s snapshot) bool { return len(s.routesOf(b)) == 0 && len(s.routesOf(a)) == 1 })
	moved := a
	moved.sections = []string{"llm2"}
	e.put(moved)
	e.converges(t, "moving a away restores b", func(s snapshot) bool {
		return slices.Equal(s.listeners(moved), []string{"llm2"}) && slices.Equal(s.listeners(b), []string{"llm"})
	})
	assert.Equal(t, "True", e.statusOf(b)["llm"].accepted)
}

func TestDuplicateModelNamePrecedenceAndScope(t *testing.T) {
	e := newConflictEnv(t)
	// Timestamp tie: smallest namespace/name wins, whatever the insertion order.
	q := testModel{name: "q", match: "tie", created: conflictBase, sections: []string{"llm"}}
	p := testModel{name: "p", match: "tie", created: conflictBase, sections: []string{"llm"}}
	// Defaulted metadata.name versus an older explicit match.model.
	explicit := testModel{name: "explicit", match: "gpt", created: conflictBase, sections: []string{"llm"}}
	implicit := testModel{name: "gpt", created: conflictBase.Add(time.Second), sections: []string{"llm"}}
	// Case, whitespace and unicode are significant: these are distinct names.
	upper := testModel{name: "upper", match: "GPT", created: conflictBase.Add(time.Hour), sections: []string{"llm"}}
	uni := testModel{name: "uni", match: "gpt-é", created: conflictBase.Add(time.Hour), sections: []string{"llm"}}
	uni2 := testModel{name: "uni2", match: "gpt-e", created: conflictBase.Add(time.Hour), sections: []string{"llm"}}
	// Same name on a different listener does not conflict; a multi-parent model loses only where it conflicts.
	other := testModel{name: "other", match: "gpt", created: conflictBase.Add(time.Hour), sections: []string{"llm2"}}
	oldShared := testModel{name: "old-shared", match: "shared", created: conflictBase, sections: []string{"llm"}}
	multi := testModel{name: "multi", match: "shared", created: conflictBase.Add(2 * time.Hour), sections: []string{"llm", "llm2"}}
	// Different namespaces: tie broken by namespace first.
	nsB := testModel{ns: "zz", name: "same", match: "xns", created: conflictBase, sections: []string{"llm"}}
	nsA := testModel{ns: "default", name: "same2", match: "xns", created: conflictBase, sections: []string{"llm"}}
	// Wildcard behavior is unchanged: both are programmed.
	w1 := testModel{name: "w1", match: "wild-*", created: conflictBase, sections: []string{"llm"}}
	w2 := testModel{name: "w2", match: "wild-*", created: conflictBase.Add(time.Second), sections: []string{"llm"}}
	// An attachment that is not eligible never claims a router.
	ghost := testModel{name: "ghost", match: "claimed", created: conflictBase.Add(-time.Hour), sections: []string{"missing"}}
	claimed := testModel{name: "real", match: "claimed", created: conflictBase, sections: []string{"llm"}}
	// Internal and public concrete models share one name space.
	internal := testModel{name: "internal", match: "vis", created: conflictBase, sections: []string{"llm"}, internal: true}
	public := testModel{name: "public", match: "vis", created: conflictBase.Add(time.Second), sections: []string{"llm"}}
	virtual := testModel{name: "virt", match: "vis", created: conflictBase.Add(2 * time.Second), sections: []string{"llm"}, virtualRefs: []string{"internal"}}

	e.put(q, p, explicit, implicit, upper, uni, uni2, other, oldShared, multi, nsB, nsA, w1, w2, ghost, claimed, internal, public, virtual)
	e.converges(t, "winners", func(s snapshot) bool {
		wanted := []testModel{p, explicit, oldShared, upper, uni, uni2, other, nsA, w1, w2, claimed, internal}
		unwanted := []testModel{q, implicit, nsB, ghost, public, virtual}
		for _, m := range wanted {
			if len(s.routesOf(m)) != 1 {
				return false
			}
		}
		for _, m := range unwanted {
			if len(s.routesOf(m)) != 0 || len(s.backendsOf(m)) != 0 {
				return false
			}
		}
		return slices.Equal(s.listeners(multi), []string{"llm2"}) // multi loses only on llm
	})
	for _, m := range []testModel{q, implicit, nsB, public, virtual} {
		assert.Equal(t, "Conflicted", e.statusOf(m)["llm"].reason, m.name)
		assert.Equal(t, "False", e.statusOf(m)["llm"].accepted, m.name)
	}
	assert.Contains(t, e.statusOf(q)["llm"].message, `conflicts with default/p on listener default/gw.llm`)
	assert.Contains(t, e.statusOf(implicit)["llm"].message, `match.model "gpt" conflicts with default/explicit`)
	assert.Contains(t, e.statusOf(nsB)["llm"].message, `conflicts with default/same2`)
	// Existing invalid attachments keep their original reason rather than becoming conflicts.
	assert.Equal(t, "False", e.statusOf(ghost)["missing"].accepted)
	assert.NotEqual(t, "Conflicted", e.statusOf(ghost)["missing"].reason)
	// Multi-parent: conflicted on one parentRef, accepted on the other.
	assert.Equal(t, "Conflicted", e.statusOf(multi)["llm"].reason)
	assert.Equal(t, "True", e.statusOf(multi)["llm2"].accepted)
	// Attachment counts follow what is programmed.
	assert.NotContains(t, e.attachedModels(), "q@llm")
	assert.Contains(t, e.attachedModels(), "p@llm")
	assert.Contains(t, e.attachedModels(), "multi@llm2")
	assert.NotContains(t, e.attachedModels(), "multi@llm")
}

func TestDuplicateModelNameAcrossKinds(t *testing.T) {
	// Identical namespace, name and timestamp: group then kind decide, deterministically.
	native := testModel{name: "same", match: "gpt", created: conflictBase, sections: []string{"llm"}}
	alpha := testModel{name: "same", match: "gpt", created: conflictBase, sections: []string{"llm"}, alpha: true}
	for _, order := range [][]testModel{{native, alpha}, {alpha, native}} {
		e := newConflictEnv(t)
		e.put(order...)
		e.converges(t, "native wins", func(s snapshot) bool {
			return len(s.routes) == 1 && len(s.routesOf(native)) == 1 && len(s.routesOf(alpha)) == 0
		})
		assert.Equal(t, "Conflicted", e.statusOf(alpha)["llm"].reason)
		assert.Equal(t, `match.model "gpt" conflicts with AgentgatewayModel default/same on listener default/gw.llm`, e.statusOf(alpha)["llm"].message)
	}

	// An older model of another kind wins, and deleting it promotes the other kind.
	e := newConflictEnv(t)
	olderAlpha := testModel{name: "x", match: "gpt", created: conflictBase.Add(-time.Hour), sections: []string{"llm"}, alpha: true}
	e.put(native, olderAlpha)
	e.converges(t, "older Alpha wins", func(s snapshot) bool {
		return len(s.routes) == 1 && len(s.routesOf(olderAlpha)) == 1 && len(s.backendsOf(native)) == 0
	})
	assert.Contains(t, e.statusOf(native)["llm"].message, "conflicts with Alpha default/x")
	e.remove(olderAlpha)
	e.converges(t, "native promoted after the other kind is deleted", func(s snapshot) bool {
		return len(s.routes) == 1 && len(s.routesOf(native)) == 1
	})
	assert.Equal(t, "True", e.statusOf(native)["llm"].accepted)
}

func TestVirtualModelDoesNotReachLosingConcreteModel(t *testing.T) {
	e := newConflictEnv(t)
	winner := testModel{name: "a", match: "gpt", created: conflictBase, sections: []string{"llm"}}
	loser := testModel{name: "b", match: "gpt", created: conflictBase.Add(time.Second), sections: []string{"llm"}}
	// "gpt" would resolve to the winner at runtime, so referencing the loser by modelRef must not be emitted.
	viaLoser := testModel{name: "via-loser", created: conflictBase.Add(time.Minute), sections: []string{"llm"}, virtualRefs: []string{"b", "a"}}
	viaWinner := testModel{name: "via-winner", created: conflictBase.Add(time.Minute), sections: []string{"llm"}, virtualRefs: []string{"a"}}
	e.put(winner, loser, viaLoser, viaWinner)
	e.converges(t, "virtual targets", func(s snapshot) bool {
		r := s.routesOf(viaLoser)
		return len(r) == 1 && slices.Equal(r[0].invalidTargets, []string{"b"}) && len(s.routesOf(loser)) == 0 && len(s.routesOf(viaWinner)) == 1
	})
	st := e.statusOf(viaLoser)["llm"]
	assert.Equal(t, "True", st.accepted, "the virtual model itself is served")
	assert.Equal(t, "False", st.resolved)
	assert.Contains(t, st.resolvedMessage, `model target default/b is not served on listener default/gw.llm: match.model "gpt" is held by default/a`)
	assert.Equal(t, "True", e.statusOf(viaWinner)["llm"].resolved)
	assert.Empty(t, e.snapshot().routesOf(viaWinner)[0].invalidTargets)
	for _, route := range e.snapshot().routes {
		assert.NotContains(t, route.invalidTargets, "gpt", "the ambiguous model name must never be forwarded")
	}

	// Deleting the winner promotes the loser, and the reference becomes valid again.
	e.remove(winner)
	e.converges(t, "reference recovers", func(s snapshot) bool {
		r := s.routesOf(viaLoser)
		return len(s.routesOf(loser)) == 1 && len(r) == 1 && slices.Equal(r[0].invalidTargets, []string{"a"}) // only the deleted target is invalid now
	})
}

func httpRouteWithModelRules(rules ...string) *gwv1.HTTPRoute {
	route := &gwv1.HTTPRoute{Namespace: "default", Name: "r"}
	route.Spec.ParentRefs = []gwv1.ParentReference{{Name: "gw", SectionName: new(gwv1.SectionName("llm"))}}
	for _, name := range rules {
		route.Spec.Rules = append(route.Spec.Rules, gwv1.HTTPRouteRule{
			Name: new(gwv1.SectionName(name)),
			Matches: []gwv1.HTTPRouteMatch{{Path: &gwv1.HTTPPathMatch{
				Type: new(gwv1.PathMatchPathPrefix), Value: new("/" + name),
			}}},
			BackendRefs: []gwv1.HTTPBackendRef{{
				Group: new(gwv1.Group(wellknown.AgentgatewayModelGVK.Group)), Kind: new(gwv1.Kind(wellknown.AgentgatewayModelGVK.Kind)), Name: "*",
			}},
		})
	}
	return route
}

func TestDuplicateModelNameHTTPRouteRules(t *testing.T) {
	e := newConflictEnv(t, httpRouteWithModelRules("one", "two"))
	one1 := testModel{name: "one1", match: "gpt", created: conflictBase, httpRoutes: []string{"r/one"}}
	one2 := testModel{name: "one2", match: "gpt", created: conflictBase.Add(time.Second), httpRoutes: []string{"r/one"}}
	two1 := testModel{name: "two1", match: "gpt", created: conflictBase.Add(time.Hour), httpRoutes: []string{"r/two"}}
	// The same name on a listener model is a different router namespace (the HTTPRoute rule vs the listener).
	direct := testModel{name: "direct", match: "gpt", created: conflictBase.Add(-time.Hour), sections: []string{"llm2"}}
	e.put(one1, one2, two1, direct)
	e.converges(t, "per-rule routers are independent", func(s snapshot) bool {
		return len(s.routesOf(one1)) == 1 && len(s.routesOf(one2)) == 0 && len(s.routesOf(two1)) == 1 && len(s.routesOf(direct)) == 1
	})
	s := e.snapshot()
	assert.Contains(t, s.routesOf(one1)[0].router, ":one")
	assert.Contains(t, s.routesOf(two1)[0].router, ":two")
	assert.NotEqual(t, s.routesOf(one1)[0].router, s.routesOf(two1)[0].router)
	st := e.statusOf(one2)["one"]
	assert.Equal(t, "False", st.accepted)
	assert.Equal(t, "Conflicted", st.reason)
	assert.Equal(t, `match.model "gpt" conflicts with default/one1 on HTTPRoute default/r rule one`, st.message)
	assert.Equal(t, "True", e.statusOf(two1)["two"].accepted)
	assert.Equal(t, "True", e.statusOf(one1)["one"].accepted)

	e.remove(one1)
	e.converges(t, "one2 promoted", func(s snapshot) bool { return len(s.routesOf(one2)) == 1 && len(s.routesOf(two1)) == 1 })
}

func TestDuplicateModelNameConvergesIndependentOfOrderAndRestart(t *testing.T) {
	const n = 16
	models := make([]testModel, n)
	for i := range models {
		models[i] = testModel{
			name: fmt.Sprintf("m%02d", i), match: "gpt", sections: []string{"llm"},
			created: conflictBase.Add(time.Duration((i*7)%n) * time.Second), // distinct timestamps, scrambled
		}
	}
	oldest := slices.MinFunc(models, func(a, b testModel) int { return a.created.Compare(b.created) })

	for run := range 5 {
		e := newConflictEnv(t)                                // a fresh environment models a controller restart / cold start
		order := rand.New(rand.NewSource(int64(run))).Perm(n) //nolint:gosec // deterministic shuffle of test inputs
		var wg sync.WaitGroup
		for _, idx := range order { // concurrent creation
			wg.Go(func() { e.put(models[idx]) })
		}
		wg.Wait()
		e.converges(t, "single winner", func(s snapshot) bool {
			return len(s.routes) == 1 && len(s.routesOf(oldest)) == 1 && len(s.backends) == 1
		})
		assert.Equal(t, []string{oldest.name + "@llm"}, e.attachedModels())

		// Rapid create/delete/update churn of the winner and others ends in the same, stable state.
		for i := range 30 {
			e.remove(oldest)
			e.put(oldest)
			e.put(models[(i*5)%n])
		}
		e.converges(t, "stable after churn", func(s snapshot) bool { return len(s.routes) == 1 && len(s.routesOf(oldest)) == 1 })
	}
}

// quiesce waits until no model translation has run for a while. It only reads an atomic counter.
func (e *conflictEnv) quiesce(minTranslations int64) {
	last, stable := int64(-1), 0
	for stable < 8 {
		time.Sleep(5 * time.Millisecond)
		cur := e.translated.Load()
		if cur == last && cur >= minTranslations {
			stable++
		} else {
			stable, last = 0, cur
		}
	}
}

// A competitor changing anything that affects where it is served, without changing its name, must
// retranslate the models it competes with: the index only tracks the name key.
func TestCompetitorChangesRetranslateSameNameModels(t *testing.T) {
	e := newConflictEnv(t)
	a := testModel{name: "a", match: "gpt", created: conflictBase, sections: []string{"llm"}}
	b := testModel{name: "b", match: "gpt", created: conflictBase.Add(time.Second), sections: []string{"llm"}}
	e.put(a, b)
	e.converges(t, "a wins", func(s snapshot) bool { return len(s.routesOf(a)) == 1 && len(s.routesOf(b)) == 0 })

	// The winner moves to another router: b is released.
	moved := a
	moved.sections = []string{"llm2"}
	e.put(moved)
	e.converges(t, "winner moved", func(s snapshot) bool {
		return len(s.routesOf(b)) == 1 && slices.Equal(s.listeners(moved), []string{"llm2"})
	})

	// The winner moves back: b loses again.
	e.put(a)
	e.converges(t, "winner moved back", func(s snapshot) bool { return len(s.routesOf(b)) == 0 && len(s.routesOf(a)) == 1 })

	// The winner becomes ineligible (unknown listener): it no longer claims the router.
	ineligible := a
	ineligible.sections = []string{"missing"}
	e.put(ineligible)
	e.converges(t, "winner ineligible", func(s snapshot) bool { return len(s.routesOf(b)) == 1 && len(s.routesOf(a)) == 0 })
	assert.Equal(t, "True", e.statusOf(b)["llm"].accepted)
	assert.Equal(t, "False", e.statusOf(a)["missing"].accepted)
	assert.NotEqual(t, "Conflicted", e.statusOf(a)["missing"].reason, "an ineligible attachment keeps its own reason")

	// The winner becomes eligible again but is now newer than b: b keeps the router.
	newer := a
	newer.created = conflictBase.Add(time.Hour)
	e.put(newer)
	e.converges(t, "newer winner loses", func(s snapshot) bool { return len(s.routesOf(b)) == 1 && len(s.routesOf(a)) == 0 })
	assert.Equal(t, "Conflicted", e.statusOf(a)["llm"].reason)

	// A third model joining and leaving only affects its own name.
	c := testModel{name: "c", match: "gpt", created: conflictBase.Add(-time.Hour), sections: []string{"llm"}}
	e.put(c)
	e.converges(t, "older c wins", func(s snapshot) bool { return len(s.routesOf(c)) == 1 && len(s.routesOf(b)) == 0 })
	e.remove(c)
	e.converges(t, "c gone", func(s snapshot) bool { return len(s.routesOf(b)) == 1 && len(s.routesOf(c)) == 0 })
}

// The same dependency tracking must work across model kinds: changes to a model of one kind retranslate
// same-name models of another.
func TestCrossKindCompetitorChanges(t *testing.T) {
	e := newConflictEnv(t)
	alpha := testModel{name: "x", match: "gpt", created: conflictBase, sections: []string{"llm"}, alpha: true}
	native := testModel{name: "y", match: "gpt", created: conflictBase.Add(time.Second), sections: []string{"llm"}}
	e.put(alpha, native)
	e.converges(t, "alpha wins", func(s snapshot) bool { return len(s.routesOf(alpha)) == 1 && len(s.routesOf(native)) == 0 })

	moved := alpha
	moved.sections = []string{"llm2"}
	e.put(moved)
	e.converges(t, "alpha moved", func(s snapshot) bool { return len(s.routesOf(native)) == 1 && len(s.routesOf(alpha)) == 1 })

	e.put(alpha)
	e.converges(t, "alpha moved back", func(s snapshot) bool { return len(s.routesOf(native)) == 0 && len(s.routesOf(alpha)) == 1 })

	renamed := alpha
	renamed.match = "other"
	e.put(renamed)
	e.converges(t, "alpha renamed", func(s snapshot) bool { return len(s.routesOf(native)) == 1 && len(s.routesOf(alpha)) == 1 })
	assert.Equal(t, "True", e.statusOf(native)["llm"].accepted)

	e.put(alpha)
	e.converges(t, "alpha takes the name back", func(s snapshot) bool { return len(s.routesOf(native)) == 0 })
	e.remove(alpha)
	e.converges(t, "alpha deleted", func(s snapshot) bool { return len(s.routesOf(native)) == 1 && len(s.routes) == 1 })
	assert.Equal(t, []string{"y@llm"}, e.attachedModels())
}

// A parentRef spanning several listeners reports the conflict, not whichever denial sorts last, when the
// model lost on every listener that would otherwise accept it.
func TestMixedListenerStatusPrefersConflict(t *testing.T) {
	e := newConflictEnv(t)
	a := testModel{name: "a", match: "gpt", created: conflictBase, allListeners: true}
	b := testModel{name: "b", match: "gpt", created: conflictBase.Add(time.Second), allListeners: true}
	e.put(a, b)
	e.converges(t, "a wins on both accepting listeners", func(s snapshot) bool {
		return slices.Equal(s.listeners(a), []string{"llm", "llm2"}) && len(s.routesOf(b)) == 0
	})
	st := e.statusOf(b)[""]
	assert.Equal(t, "False", st.accepted)
	assert.Equal(t, "Conflicted", st.reason, "the restricted listener's denial must not mask the conflict")
	assert.Contains(t, st.message, `match.model "gpt" conflicts with default/a`)
	assert.Equal(t, "True", e.statusOf(a)[""].accepted)

	// Once the winner is gone the same parentRef is accepted again, with no stale conflict message.
	e.remove(a)
	e.converges(t, "b promoted on both listeners", func(s snapshot) bool { return slices.Equal(s.listeners(b), []string{"llm", "llm2"}) })
	assert.Equal(t, parentStatus{accepted: "True", reason: "Accepted", message: "Successfully accepted AgentgatewayModel", resolved: "True"}, e.statusOf(b)[""])
}

// Unrelated model changes must not fan out: only the changed model, and models sharing its exact name, are
// retranslated.
func TestModelUpdateFanout(t *testing.T) {
	e := newConflictEnv(t)
	const n = 50
	for i := range n {
		e.put(testModel{name: fmt.Sprintf("m%02d", i), created: conflictBase, sections: []string{"llm"}})
	}
	e.quiesce(n)
	delta := func(f func()) int64 {
		before := e.translated.Load()
		f()
		e.quiesce(before + 1)
		return e.translated.Load() - before
	}
	assert.LessOrEqual(t, delta(func() { e.put(testModel{name: "other", created: conflictBase, sections: []string{"llm"}}) }), int64(3), "a new unrelated name")
	assert.LessOrEqual(t, delta(func() {
		e.put(testModel{name: "m07", created: conflictBase, sections: []string{"llm"}, internal: true})
	}), int64(3), "an update of one model")
	// A same-name competitor retranslates exactly the models sharing the name.
	assert.LessOrEqual(t, delta(func() {
		e.put(testModel{name: "dup", match: "m03", created: conflictBase.Add(time.Hour), sections: []string{"llm"}})
	}), int64(6), "a competitor of m03")
}

// A competitor can lose eligibility through something other than its own object, such as a listener's
// allowed namespaces. The model it was beating is then retranslated only through the dependency created when
// its translation resolved the competitor's parents: its own resolved parents are unchanged.
func TestCompetitorEligibilityChangeFromOutsideRetranslatesLoser(t *testing.T) {
	e := newConflictEnv(t)
	winner := testModel{ns: "c", name: "winner", match: "gpt", created: conflictBase, sections: []string{"llm"}}
	loser := testModel{ns: "l", name: "loser", match: "gpt", created: conflictBase.Add(time.Second), sections: []string{"llm"}}
	e.setHosts("c/*,l/*")
	e.put(winner, loser)
	e.converges(t, "both namespaces allowed: oldest wins", func(s snapshot) bool { return len(s.routesOf(winner)) == 1 && len(s.routesOf(loser)) == 0 })
	assert.Equal(t, "Conflicted", e.statusOf(loser)["llm"].reason)

	e.setHosts("l/*") // only the loser's namespace may attach; its own resolved parents do not change
	e.converges(t, "winner no longer eligible: loser promoted", func(s snapshot) bool { return len(s.routesOf(winner)) == 0 && len(s.routesOf(loser)) == 1 })
	assert.Equal(t, "True", e.statusOf(loser)["llm"].accepted)
	assert.Equal(t, "False", e.statusOf(winner)["llm"].accepted)
	assert.NotEqual(t, "Conflicted", e.statusOf(winner)["llm"].reason)

	e.setHosts("c/*,l/*") // eligibility is restored
	e.converges(t, "winner eligible again", func(s snapshot) bool { return len(s.routesOf(winner)) == 1 && len(s.routesOf(loser)) == 0 })
	assert.Equal(t, "Conflicted", e.statusOf(loser)["llm"].reason)
}
