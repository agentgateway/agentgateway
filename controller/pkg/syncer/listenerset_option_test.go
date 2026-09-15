package syncer

import (
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/slices"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/translator"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
)

var testGatewayParent = types.NamespacedName{Namespace: "default", Name: "example"}

// Only "other" carries the label, so a selector policy distinguishes it from the Gateway's own
// namespace.
var testNamespaces = []*corev1.Namespace{
	{Name: "default"},
	{Name: "other", Labels: map[string]string{"team": "platform"}},
}

// baseListenerSet is dotted so fixtures can exercise InternalGatewayName not being injective.
var baseListenerSet = testListenerSet("default", "ls.one", "http")

func testGateway(from *gwv1.FromNamespaces) *gwv1.Gateway {
	gw := &gwv1.Gateway{Namespace: "default", Name: "example"}
	if from != nil {
		gw.Spec.AllowedListeners = &gwv1.AllowedListeners{
			Namespaces: &gwv1.ListenerNamespaces{From: from},
		}
	}
	return gw
}

func testListenerSet(namespace, name, section string) translator.ListenerSet {
	return translator.ListenerSet{
		Name:          utils.InternalGatewayName(namespace, name, section),
		Parent:        types.NamespacedName{Namespace: namespace, Name: name},
		GatewayParent: testGatewayParent,
		Valid:         true,
		ParentInfo: plugins.ParentInfo{
			ParentGateway: testGatewayParent,
			ListenerKey:   utils.InternalGatewayName(namespace, name, section),
			SectionName:   gwv1.SectionName(section),
			Port:          8080,
			Protocol:      gwv1.HTTPProtocolType,
		},
	}
}

func staticExtra(sets ...translator.ListenerSet) AgentgatewaySyncerOption {
	return WithExtraListenerSets(func(agw *plugins.AgwCollections, krtopts krtutil.KrtOptions) krt.Collection[translator.ListenerSet] {
		return krt.NewStaticCollection(nil, sets, krtopts.ToOptions("Extra")...)
	})
}

type joinFixture struct {
	admitted krt.Collection[translator.ListenerSet]
	rejected krt.Collection[RejectedListenerSet]
	base     krt.Collection[translator.ListenerSet]
}

func newJoinFixture(
	t *testing.T,
	gw *gwv1.Gateway,
	listenerSets []*gwv1.ListenerSet,
	opts ...AgentgatewaySyncerOption,
) joinFixture {
	krtopts := krtutil.NewKrtOptions(t.Context().Done(), nil)
	gateways := []*gwv1.Gateway{}
	if gw != nil {
		gateways = append(gateways, gw)
	}
	cfg := processAgentgatewaySyncerOptions(opts...)
	s := &Syncer{
		extraListenerSets:        cfg.ExtraListenerSets,
		allowedListenersResolver: cfg.AllowedListenersResolver,
		agwCollections: &plugins.AgwCollections{
			KrtOpts:      krtopts,
			Gateways:     krt.NewStaticCollection(nil, gateways, krtopts.ToOptions("Gateways")...),
			ListenerSets: krt.NewStaticCollection(nil, listenerSets, krtopts.ToOptions("ListenerSets")...),
			Namespaces:   krt.NewStaticCollection(nil, testNamespaces, krtopts.ToOptions("Namespaces")...),
		},
	}
	base := krt.NewStaticCollection(nil, []translator.ListenerSet{baseListenerSet}, krtopts.ToOptions("Base")...)
	admitted, rejected := s.joinExtraListenerSets(base, krtopts)
	admitted.WaitUntilSynced(krtopts.Stop)
	rejected.WaitUntilSynced(krtopts.Stop)
	return joinFixture{admitted: admitted, rejected: rejected, base: base}
}

func (f joinFixture) names() []string {
	return slices.Map(f.admitted.List(), translator.ListenerSet.ResourceName)
}

func TestJoinExtraListenerSetsNoOpWhenUnset(t *testing.T) {
	t.Run("option unset", func(t *testing.T) {
		f := newJoinFixture(t, testGateway(ptr.Of(gwv1.NamespacesFromAll)), nil)
		assert.True(t, f.base == f.admitted)
		assert.Empty(t, f.rejected.List())
	})

	t.Run("builder returns nil", func(t *testing.T) {
		f := newJoinFixture(t, testGateway(ptr.Of(gwv1.NamespacesFromAll)), nil,
			WithExtraListenerSets(func(agw *plugins.AgwCollections, krtopts krtutil.KrtOptions) krt.Collection[translator.ListenerSet] {
				return nil
			}))
		assert.True(t, f.base == f.admitted)
		assert.Empty(t, f.rejected.List())
	})
}

// A selector policy is evaluated against the contribution's namespace, not the Gateway's.
func TestJoinExtraListenerSetsHonoursNamespaceSelector(t *testing.T) {
	gw := testGateway(ptr.Of(gwv1.NamespacesFromSelector))
	gw.Spec.AllowedListeners.Namespaces.Selector = &metav1.LabelSelector{
		MatchLabels: map[string]string{"team": "platform"},
	}

	f := newJoinFixture(t, gw, nil, staticExtra(testListenerSet("other", "b", "http")))
	assert.ElementsMatch(t, []string{baseListenerSet.Name, "other/b.http"}, f.names())
	assert.Empty(t, f.rejected.List())
}

func TestJoinExtraListenerSetsRejects(t *testing.T) {
	// Name and ListenerKey agree, so only the derivation check can catch this.
	undrivedName := testListenerSet("other", "b", "http")
	undrivedName.Name = "other/zzz.http"
	undrivedName.ParentInfo.ListenerKey = undrivedName.Name

	// "default/ls" + "." + "one.http" is the same string as "default/ls.one" + "." + "http".
	dottedSection := testListenerSet("default", "ls", "one.http")

	mismatchedListenerKey := testListenerSet("other", "b", "http")
	mismatchedListenerKey.ParentInfo.ListenerKey = "other/b.https"

	emptySection := testListenerSet("other", "b", "")

	// Does not collide by name, so only the parent-is-a-ListenerSet check catches it.
	realListenerSets := []*gwv1.ListenerSet{
		{Namespace: "default", Name: "real"},
	}

	cases := []struct {
		name         string
		gateway      *gwv1.Gateway
		listenerSets []*gwv1.ListenerSet
		extra        translator.ListenerSet
		reason       gwv1.ListenerSetConditionReason
	}{
		{
			name:    "name not derived from parent",
			gateway: testGateway(ptr.Of(gwv1.NamespacesFromAll)),
			extra:   undrivedName,
			reason:  gwv1.ListenerSetReasonInvalid,
		},
		{
			name:         "parent is a listener set",
			gateway:      testGateway(ptr.Of(gwv1.NamespacesFromAll)),
			listenerSets: realListenerSets,
			extra:        testListenerSet("default", "real", "http"),
			reason:       gwv1.ListenerSetReasonInvalid,
		},
		{
			name:    "dotted section name derives a name a listener set listener owns",
			gateway: testGateway(ptr.Of(gwv1.NamespacesFromAll)),
			extra:   dottedSection,
			reason:  gwv1.ListenerSetReasonInvalid,
		},
		{
			name:    "listener key does not match name",
			gateway: testGateway(ptr.Of(gwv1.NamespacesFromAll)),
			extra:   mismatchedListenerKey,
			reason:  gwv1.ListenerSetReasonInvalid,
		},
		{
			name:    "empty section name",
			gateway: testGateway(ptr.Of(gwv1.NamespacesFromAll)),
			extra:   emptySection,
			reason:  gwv1.ListenerSetReasonInvalid,
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			f := newJoinFixture(t, tc.gateway, tc.listenerSets, staticExtra(tc.extra))

			assert.Equal(t, []string{baseListenerSet.Name}, f.names())

			require.Len(t, f.rejected.List(), 1)
			got := f.rejected.List()[0]
			assert.Equal(t, tc.reason, got.Reason)
			assert.NotEmpty(t, got.Message)
			assert.Equal(t, tc.extra.Name, got.ListenerSet.Name)
		})
	}
}

// No parent Gateway means the gate cannot be evaluated: not admitted, not reported.
func TestJoinExtraListenerSetsDropsUnparentedContributions(t *testing.T) {
	orphan := testListenerSet("other", "b", "http")
	orphan.GatewayParent = types.NamespacedName{Namespace: "default", Name: "missing"}
	orphan.ParentInfo.ParentGateway = orphan.GatewayParent

	f := newJoinFixture(t, testGateway(ptr.Of(gwv1.NamespacesFromAll)), nil, staticExtra(orphan))
	assert.Equal(t, []string{baseListenerSet.Name}, f.names())
	assert.Empty(t, f.rejected.List())
}

func TestWithAllowedListenersResolver(t *testing.T) {
	extra := staticExtra(testListenerSet("other", "b", "http"))

	// No spec.allowedListeners, as on a CRD without the field.
	gw := testGateway(nil)
	gw.Annotations = map[string]string{"example.com/allowed-listeners": "All"}

	t.Run("resolver supplies the policy from elsewhere", func(t *testing.T) {
		f := newJoinFixture(t, gw, nil, extra, WithAllowedListenersResolver(
			func(gw *gwv1.Gateway) *gwv1.AllowedListeners {
				if gw.Annotations["example.com/allowed-listeners"] != "All" {
					return nil
				}
				return &gwv1.AllowedListeners{Namespaces: &gwv1.ListenerNamespaces{From: ptr.Of(gwv1.NamespacesFromAll)}}
			},
		))
		assert.ElementsMatch(t, []string{baseListenerSet.Name, "other/b.http"}, f.names())
		assert.Empty(t, f.rejected.List())
	})

	t.Run("resolver returning nil still denies", func(t *testing.T) {
		f := newJoinFixture(t, gw, nil, extra, WithAllowedListenersResolver(
			func(gw *gwv1.Gateway) *gwv1.AllowedListeners { return nil },
		))
		assert.Equal(t, []string{baseListenerSet.Name}, f.names())
		require.Len(t, f.rejected.List(), 1)
		assert.Equal(t, gwv1.ListenerSetReasonNotAllowed, f.rejected.List()[0].Reason)
	})

	t.Run("explicit spec denial is not overridden", func(t *testing.T) {
		denied := testGateway(ptr.Of(gwv1.NamespacesFromNone))
		f := newJoinFixture(t, denied, nil, extra, WithAllowedListenersResolver(
			func(gw *gwv1.Gateway) *gwv1.AllowedListeners {
				return &gwv1.AllowedListeners{Namespaces: &gwv1.ListenerNamespaces{From: ptr.Of(gwv1.NamespacesFromAll)}}
			},
		))
		assert.Equal(t, []string{baseListenerSet.Name}, f.names())
		require.Len(t, f.rejected.List(), 1)
		assert.Equal(t, gwv1.ListenerSetReasonNotAllowed, f.rejected.List()[0].Reason)
	})
}
