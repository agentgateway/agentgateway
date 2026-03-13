package oidcstore

import (
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	corev1 "k8s.io/api/core/v1"
	discovery "k8s.io/api/discovery/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gwv1a2 "sigs.k8s.io/gateway-api/apis/v1alpha2"
	gwv1b1 "sigs.k8s.io/gateway-api/apis/v1beta1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/identity/oidc"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
)

func TestOIDCStorePolicyControllerEmitsOwnerScopedEventsForSharedSource(t *testing.T) {
	opts := krtutil.NewKrtOptions(t.Context().Done(), nil)
	policies := krt.NewStaticCollection[*agentgateway.AgentgatewayPolicy](nil, nil, opts.ToOptions("AgentgatewayPolicies")...)

	controller := NewOIDCStorePolicyController(&plugins.AgwCollections{
		KrtOpts:              opts,
		AgentgatewayPolicies: policies,
		Namespaces:           krt.NewStaticCollection[*corev1.Namespace](nil, nil, opts.ToOptions("Namespaces")...),
		Nodes:                krt.NewStaticCollection[*corev1.Node](nil, nil, opts.ToOptions("Nodes")...),
		Pods:                 krt.NewStaticCollection[*corev1.Pod](nil, nil, opts.ToOptions("Pods")...),
		Services:             krt.NewStaticCollection[*corev1.Service](nil, nil, opts.ToOptions("Services")...),
		Secrets:              krt.NewStaticCollection[*corev1.Secret](nil, nil, opts.ToOptions("Secrets")...),
		ConfigMaps:           krt.NewStaticCollection[*corev1.ConfigMap](nil, nil, opts.ToOptions("ConfigMaps")...),
		EndpointSlices:       krt.NewStaticCollection[*discovery.EndpointSlice](nil, nil, opts.ToOptions("EndpointSlices")...),
		GatewayClasses:       krt.NewStaticCollection[*gwv1.GatewayClass](nil, nil, opts.ToOptions("GatewayClasses")...),
		Gateways:             krt.NewStaticCollection[*gwv1.Gateway](nil, nil, opts.ToOptions("Gateways")...),
		HTTPRoutes:           krt.NewStaticCollection[*gwv1.HTTPRoute](nil, nil, opts.ToOptions("HTTPRoutes")...),
		GRPCRoutes:           krt.NewStaticCollection[*gwv1.GRPCRoute](nil, nil, opts.ToOptions("GRPCRoutes")...),
		TCPRoutes:            krt.NewStaticCollection[*gwv1a2.TCPRoute](nil, nil, opts.ToOptions("TCPRoutes")...),
		TLSRoutes:            krt.NewStaticCollection[*gwv1.TLSRoute](nil, nil, opts.ToOptions("TLSRoutes")...),
		ReferenceGrants:      krt.NewStaticCollection[*gwv1b1.ReferenceGrant](nil, nil, opts.ToOptions("ReferenceGrants")...),
		BackendTLSPolicies:   krt.NewStaticCollection[*gwv1.BackendTLSPolicy](nil, nil, opts.ToOptions("BackendTLSPolicies")...),
		ListenerSets:         krt.NewStaticCollection[*gwv1.ListenerSet](nil, nil, opts.ToOptions("ListenerSets")...),
		Backends:             krt.NewStaticCollection[*agentgateway.AgentgatewayBackend](nil, nil, opts.ToOptions("Backends")...),
	})
	controller.Init(t.Context())
	events := make(chan krt.Event[oidc.ProviderSource], 4)
	reg := controller.sources.Register(func(event krt.Event[oidc.ProviderSource]) {
		events <- event
	})
	require.True(t, reg.WaitUntilSynced(t.Context().Done()))

	policyA := testOIDCOAuth2Policy("default", "oauth2-a", "https://issuer.example.com")
	policyB := testOIDCOAuth2Policy("default", "oauth2-b", "https://issuer.example.com")
	wantKey, err := oidc.CanonicalSourceKey("https://issuer.example.com", "default", nil)
	require.NoError(t, err)

	policies.UpdateObject(policyA)
	expectSharedSourceEvent(t, events, policySourceOwnerKey(policyA.Name, policyA.Namespace), wantKey, false)

	policies.UpdateObject(policyB)
	expectSharedSourceEvent(t, events, policySourceOwnerKey(policyB.Name, policyB.Namespace), wantKey, false)
	assert.Eventually(t, func() bool {
		return len(controller.sources.List()) == 2
	}, time.Second, 10*time.Millisecond)

	policies.DeleteObject(krt.GetKey(policyA))
	assert.Eventually(t, func() bool {
		return len(controller.sources.List()) == 1
	}, time.Second, 10*time.Millisecond)
	expectSharedSourceEvent(t, events, policySourceOwnerKey(policyA.Name, policyA.Namespace), wantKey, true)

	policies.DeleteObject(krt.GetKey(policyB))
	expectSharedSourceEvent(t, events, policySourceOwnerKey(policyB.Name, policyB.Namespace), wantKey, true)
	assert.Eventually(t, func() bool {
		return len(controller.sources.List()) == 0
	}, time.Second, 10*time.Millisecond)
}

func expectSharedSourceEvent(
	t *testing.T,
	events <-chan krt.Event[oidc.ProviderSource],
	wantOwner string,
	wantKey string,
	deleted bool,
) {
	t.Helper()
	assert.Eventually(t, func() bool {
		select {
		case event := <-events:
			if deleted {
				return event.Event == controllers.EventDelete &&
					event.Old != nil &&
					event.Old.ResourceKey == wantKey &&
					event.Old.OwnerKey == wantOwner
			}
			return event.Event == controllers.EventAdd &&
				event.New != nil &&
				event.New.ResourceKey == wantKey &&
				event.New.OwnerKey == wantOwner
		default:
			return false
		}
	}, time.Second, 10*time.Millisecond)
}

func testOIDCOAuth2Policy(namespace, name, issuer string) *agentgateway.AgentgatewayPolicy {
	return &agentgateway.AgentgatewayPolicy{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: namespace,
			Name:      name,
		},
		Spec: agentgateway.AgentgatewayPolicySpec{
			Traffic: &agentgateway.Traffic{
				OAuth2: &agentgateway.OAuth2{
					Issuer: ptr.Of(agentgateway.LongString(issuer)),
				},
			},
		},
	}
}
