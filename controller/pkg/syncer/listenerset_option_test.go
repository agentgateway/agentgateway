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

func TestWithExtraListenerSets(t *testing.T) {
	assert.Nil(t, processAgentgatewaySyncerOptions().ExtraListenerSets)
	assert.Nil(t, processAgentgatewaySyncerOptions(WithExtraListenerSets(nil)).ExtraListenerSets)
	assert.NotNil(t, processAgentgatewaySyncerOptions(staticExtra()).ExtraListenerSets)
}

var testGatewayParent = types.NamespacedName{Namespace: "default", Name: "example"}

// baseListenerSet is the Gateway API ListenerSet listener every fixture starts from. Its parent
// name is dotted, as object names may be, so that fixtures can exercise the fact that
// utils.InternalGatewayName is not injective.
var baseListenerSet = testListenerSet("default", "ls.one", "http")

// testGateway is the parent Gateway, with allowedListeners admitting from the given namespaces.
func testGateway(from *gwv1.FromNamespaces) *gwv1.Gateway {
	gw := &gwv1.Gateway{ObjectMeta: metav1.ObjectMeta{Namespace: "default", Name: "example"}}
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
			SectionName:   gwv1.SectionName(section),
			Port:          8080,
			Protocol:      gwv1.HTTPProtocolType,
		},
	}
}

func staticExtra(sets ...translator.ListenerSet) AgentgatewaySyncerOption {
	return WithExtraListenerSets(func(agw *plugins.AgwCollections) krt.Collection[translator.ListenerSet] {
		return krt.NewStaticCollection(nil, sets, agw.KrtOpts.ToOptions("Extra")...)
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
			Namespaces:   krt.NewStaticCollection[*corev1.Namespace](nil, nil, krtopts.ToOptions("Namespaces")...),
		},
	}
	base := krt.NewStaticCollection(nil, []translator.ListenerSet{baseListenerSet}, krtopts.ToOptions("Base")...)
	admitted, rejected := s.joinExtraListenerSets(base, krtopts)
	admitted.WaitUntilSynced(krtopts.Stop)
	if rejected != nil {
		rejected.WaitUntilSynced(krtopts.Stop)
	}
	return joinFixture{admitted: admitted, rejected: rejected, base: base}
}

func (f joinFixture) names() []string {
	return slices.Map(f.admitted.List(), translator.ListenerSet.ResourceName)
}

// GatewayTransformationFunc reads listener sets only through an index, so the index is the path
// that has to show the Gateway API listener winning, not just List and GetKey.
func (f joinFixture) indexedNames() []string {
	idx := krt.NewIndex(f.admitted, "gatewayParent", func(o translator.ListenerSet) []types.NamespacedName {
		return []types.NamespacedName{o.GatewayParent}
	})
	return slices.Map(idx.Lookup(testGatewayParent), translator.ListenerSet.ResourceName)
}

func TestJoinExtraListenerSetsNoOpWhenUnset(t *testing.T) {
	t.Run("option unset", func(t *testing.T) {
		f := newJoinFixture(t, testGateway(ptr.Of(gwv1.NamespacesFromAll)), nil)
		assert.True(t, f.base == f.admitted)
		assert.Nil(t, f.rejected)
	})

	t.Run("builder returns nil", func(t *testing.T) {
		f := newJoinFixture(t, testGateway(ptr.Of(gwv1.NamespacesFromAll)), nil,
			WithExtraListenerSets(func(agw *plugins.AgwCollections) krt.Collection[translator.ListenerSet] {
				return nil
			}))
		assert.True(t, f.base == f.admitted)
		assert.Nil(t, f.rejected)
	})
}

func TestJoinExtraListenerSetsAdmits(t *testing.T) {
	f := newJoinFixture(t, testGateway(ptr.Of(gwv1.NamespacesFromAll)), nil,
		staticExtra(testListenerSet("other", "b", "http")))

	assert.ElementsMatch(t, []string{baseListenerSet.Name, "other/b.http"}, f.names())
	assert.ElementsMatch(t, []string{baseListenerSet.Name, "other/b.http"}, f.indexedNames())
	assert.Empty(t, f.rejected.List())
}

func TestJoinExtraListenerSetsRejects(t *testing.T) {
	aliasedName := testListenerSet("default", "b", "http")
	aliasedName.Name = baseListenerSet.Name

	// SectionName may contain dots, so this derives baseListenerSet's name from a different
	// parent: "default/ls" + "." + "one.http" is the same string as "default/ls.one" + "." +
	// "http". It passes both identity checks and is caught only by the collision check.
	dottedSection := testListenerSet("default", "ls", "one.http")

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
			extra:   aliasedName,
			reason:  gwv1.ListenerSetReasonInvalid,
		},
		{
			name:    "parent is a gateway api listener set",
			gateway: testGateway(ptr.Of(gwv1.NamespacesFromAll)),
			listenerSets: []*gwv1.ListenerSet{
				{ObjectMeta: metav1.ObjectMeta{Namespace: "default", Name: "real"}},
			},
			extra:  testListenerSet("default", "real", "http"),
			reason: gwv1.ListenerSetReasonInvalid,
		},
		{
			name:    "dotted section name derives a name a listener set listener owns",
			gateway: testGateway(ptr.Of(gwv1.NamespacesFromAll)),
			extra:   dottedSection,
			reason:  gwv1.ListenerSetReasonInvalid,
		},
		{
			name:    "gateway allows no listener attachment",
			gateway: testGateway(nil),
			extra:   testListenerSet("other", "b", "http"),
			reason:  gwv1.ListenerSetReasonNotAllowed,
		},
		{
			name:    "gateway allows only its own namespace",
			gateway: testGateway(ptr.Of(gwv1.NamespacesFromSame)),
			extra:   testListenerSet("other", "b", "http"),
			reason:  gwv1.ListenerSetReasonNotAllowed,
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			f := newJoinFixture(t, tc.gateway, tc.listenerSets, staticExtra(tc.extra))

			// The Gateway API listener set is the only one left, on every read path.
			assert.Equal(t, []string{baseListenerSet.Name}, f.names())
			assert.Equal(t, []string{baseListenerSet.Name}, f.indexedNames())

			require.Len(t, f.rejected.List(), 1)
			got := f.rejected.List()[0]
			assert.Equal(t, tc.reason, got.Reason)
			assert.NotEmpty(t, got.Message)
			assert.Equal(t, tc.extra.Name, got.ListenerSet.Name)
		})
	}
}

func TestWithAllowedListenersResolver(t *testing.T) {
	assert.Nil(t, processAgentgatewaySyncerOptions().AllowedListenersResolver)
	assert.Nil(t, processAgentgatewaySyncerOptions(WithAllowedListenersResolver(nil)).AllowedListenersResolver)

	extra := staticExtra(testListenerSet("other", "b", "http"))

	// The Gateway carries no spec.allowedListeners, as it cannot on a CRD without the field.
	gw := testGateway(nil)
	gw.Annotations = map[string]string{"example.com/allowed-listeners": "All"}

	t.Run("default reads the spec field", func(t *testing.T) {
		f := newJoinFixture(t, gw, nil, extra)
		assert.Equal(t, []string{baseListenerSet.Name}, f.names())
		require.Len(t, f.rejected.List(), 1)
		assert.Equal(t, gwv1.ListenerSetReasonNotAllowed, f.rejected.List()[0].Reason)
	})

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
		f := newJoinFixture(t, testGateway(ptr.Of(gwv1.NamespacesFromAll)), nil, extra, WithAllowedListenersResolver(
			func(gw *gwv1.Gateway) *gwv1.AllowedListeners { return nil },
		))
		assert.Equal(t, []string{baseListenerSet.Name}, f.names())
		require.Len(t, f.rejected.List(), 1)
		assert.Equal(t, gwv1.ListenerSetReasonNotAllowed, f.rejected.List()[0].Reason)
	})
}
