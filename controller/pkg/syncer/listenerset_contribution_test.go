package syncer_test

import (
	"encoding/json"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/kube/krt"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/testutils"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/translator"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
	"github.com/agentgateway/agentgateway/controller/pkg/syncer"
)

const gatewayClassYAML = `
apiVersion: gateway.networking.k8s.io/v1
kind: GatewayClass
metadata:
  name: agentgateway
spec:
  controllerName: agentgateway.dev/agentgateway
`

const gatewayYAML = `
apiVersion: gateway.networking.k8s.io/v1
kind: Gateway
metadata:
  name: example
  namespace: default
spec:
  gatewayClassName: agentgateway
  listeners:
    - name: http
      protocol: HTTP
      port: 8080
  allowedListeners:
    namespaces:
      from: All
`

var exampleGateway = types.NamespacedName{Namespace: "default", Name: "example"}

// Dated older than every Gateway API ListenerSet here, so it takes listener precedence.
func contributedListenerSet(name, section string, port gwv1.PortNumber, protocol gwv1.ProtocolType, internal bool) *translator.ListenerSet {
	key := utils.InternalGatewayName("default", name, section)
	return &translator.ListenerSet{
		Name:          key,
		ParentObject:  utils.TypedNamespacedName{Kind: "ListenerSet", NamespacedName: types.NamespacedName{Namespace: "default", Name: name}},
		ParentGateway: exampleGateway,
		Valid:         true,
		ParentInfo: plugins.ParentInfo{
			ParentGateway:     exampleGateway,
			ListenerKey:       key,
			SectionName:       gwv1.SectionName(section),
			Port:              port,
			Protocol:          protocol,
			Internal:          internal,
			CreationTimestamp: metav1.NewTime(time.Date(2026, 1, 1, 0, 0, 0, 0, time.UTC)),
		},
	}
}

func syncerWithContributedListenerSets(t *testing.T, inputs []any, sets ...*translator.ListenerSet) *syncer.Syncer {
	_, s := syncerAndStatus(t, inputs, sets...)
	return s
}

func syncerAndStatus(t *testing.T, inputs []any, sets ...*translator.ListenerSet) (*testutils.TestStatusQueue, *syncer.Syncer) {
	ctx := testutils.BuildMockPolicyContext(t, inputs)
	sq, s := testutils.SyncerWithOptions(t, ctx, []string{"ListenerSet"}, syncer.WithExtraListenerSets(
		func(agw *plugins.AgwCollections, krtopts krtutil.KrtOptions) krt.Collection[*translator.ListenerSet] {
			return krt.NewStaticCollection(nil, sets, krtopts.ToOptions("test/ContributedListenerSets")...)
		},
	))
	return sq, s
}

func binds(s *syncer.Syncer) map[string]*api.Bind {
	out := map[string]*api.Bind{}
	for _, r := range s.Outputs.Resources.List() {
		if b := r.Resource.GetBind(); b != nil {
			out[b.GetKey()] = b
		}
	}
	return out
}

func listenerKeys(s *syncer.Syncer) []string {
	out := []string{}
	for _, r := range s.Outputs.Resources.List() {
		if l := r.Resource.GetListener(); l != nil {
			out = append(out, l.GetKey())
		}
	}
	return out
}

func TestContributedListenerSetsAreArbitrated(t *testing.T) {
	free := contributedListenerSet("free", "free", 8081, gwv1.HTTPProtocolType, false)
	contested := contributedListenerSet("contested", "contested", 8080, gwv1.TCPProtocolType, false)
	s := syncerWithContributedListenerSets(t, []any{gatewayClassYAML, gatewayYAML}, free, contested)

	b := binds(s)
	require.Contains(t, b, "8081/default/example")
	assert.Equal(t, api.Bind_HTTP, b["8081/default/example"].GetProtocol())
	assert.Contains(t, listenerKeys(s), free.Name)

	// The contested listener lost, so it does not pick the bind protocol.
	require.Contains(t, b, "8080/default/example")
	assert.Equal(t, api.Bind_HTTP, b["8080/default/example"].GetProtocol())
}

const newerListenerSetYAML = `
apiVersion: gateway.networking.k8s.io/v1
kind: ListenerSet
metadata:
  name: newer
  namespace: default
  creationTimestamp: "2026-01-02T00:00:00Z"
spec:
  parentRef:
    name: example
    kind: Gateway
    group: gateway.networking.k8s.io
  listeners:
    - name: shared
      protocol: HTTP
      port: 9090
      allowedRoutes:
        namespaces:
          from: All
`

const gatewayNoAllowedListenersYAML = `
apiVersion: gateway.networking.k8s.io/v1
kind: Gateway
metadata:
  name: example
  namespace: default
spec:
  gatewayClassName: agentgateway
  listeners:
    - name: http
      protocol: HTTP
      port: 8080
`

func TestContributedListenerSetsRequireAllowedListeners(t *testing.T) {
	free := contributedListenerSet("free", "free", 8081, gwv1.HTTPProtocolType, false)
	s := syncerWithContributedListenerSets(t, []any{gatewayClassYAML, gatewayNoAllowedListenersYAML}, free)

	assert.NotContains(t, binds(s), "8081/default/example")
	assert.NotContains(t, listenerKeys(s), free.Name)

	require.Len(t, s.Outputs.RejectedListenerSets.List(), 1)
	rejection := s.Outputs.RejectedListenerSets.List()[0]
	assert.Equal(t, gwv1.ListenerSetReasonNotAllowed, rejection.Reason)
	assert.Equal(t, free.Name, rejection.ListenerSet.Name)
}

func TestContributedListenerSetsTakeListenerPrecedence(t *testing.T) {
	older := contributedListenerSet("older", "older", 9090, gwv1.HTTPProtocolType, true)
	sq, s := syncerAndStatus(t, []any{gatewayClassYAML, gatewayYAML, newerListenerSetYAML}, older)

	b := binds(s)
	require.Contains(t, b, "9090/default/example")
	assert.Equal(t, api.Bind_INTERNAL, b["9090/default/example"].GetMode())

	keys := listenerKeys(s)
	assert.Contains(t, keys, older.Name)
	assert.NotContains(t, keys, "default/newer.shared")

	// Losing is user-visible: the ListenerSet must say why its listener is gone.
	dump, err := json.Marshal(sq.Dump())
	require.NoError(t, err)
	assert.Contains(t, string(dump), `"type":"Conflicted","status":"True","lastTransitionTime"`)
	assert.Contains(t, string(dump), "BindModeConflict")
}

func TestContributedListenerSetsCannotAliasAGatewayListener(t *testing.T) {
	aliases := contributedListenerSet("example", "http", 8080, gwv1.TCPProtocolType, false)
	s := syncerWithContributedListenerSets(t, []any{gatewayClassYAML, gatewayYAML}, aliases)

	b := binds(s)
	require.Contains(t, b, "8080/default/example")
	assert.Equal(t, api.Bind_HTTP, b["8080/default/example"].GetProtocol())
	// Exactly one listener, so nothing replaced the Gateway's own.
	assert.Equal(t, []string{"default/example.http"}, listenerKeys(s))

	require.Len(t, s.Outputs.RejectedListenerSets.List(), 1)
	assert.Equal(t, gwv1.ListenerSetReasonInvalid, s.Outputs.RejectedListenerSets.List()[0].Reason)
}

func TestContributedListenerSetsCannotCrossGatewayBinds(t *testing.T) {
	crosses := contributedListenerSet("free", "free", 8081, gwv1.HTTPProtocolType, false)
	crosses.ParentInfo.ParentGateway = types.NamespacedName{Namespace: "default", Name: "elsewhere"}
	s := syncerWithContributedListenerSets(t, []any{gatewayClassYAML, gatewayYAML}, crosses)

	assert.NotContains(t, binds(s), "8081/default/example")
	assert.NotContains(t, listenerKeys(s), crosses.Name)

	require.Len(t, s.Outputs.RejectedListenerSets.List(), 1)
	assert.Equal(t, gwv1.ListenerSetReasonInvalid, s.Outputs.RejectedListenerSets.List()[0].Reason)
}
