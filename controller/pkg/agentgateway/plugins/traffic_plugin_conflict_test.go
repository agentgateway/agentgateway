package plugins_test

import (
	"strings"
	"testing"

	"k8s.io/apimachinery/pkg/apis/meta/v1"
	k8stypes "k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/shared"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/testutils"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/wellknown"
)

func TestTranslateAgentgatewayPolicyRejectsRouteJWTWhenGatewayOAuth2Exists(t *testing.T) {
	gateway := testGateway()
	route := testHTTPRoute("default", "route", "gw")
	gatewayOAuth2 := testGatewayOAuth2Policy("default", "gateway-oauth2", "gw", "uid-gateway-oauth2")
	routeJWT := testRouteJWTPolicy("default", "route-jwt", "route", "uid-route-jwt")

	ctx := testutils.BuildMockPolicyContext(t, []any{
		gateway,
		route,
		gatewayOAuth2,
		routeJWT,
	})

	status, output := plugins.TranslateAgentgatewayPolicy(ctx.Krt, routeJWT, ctx.Collections)
	if len(output) != 0 {
		t.Fatalf("expected no translated policies on conflict, got %d", len(output))
	}

	accepted := findAcceptedCondition(t, status.Ancestors[0].Conditions)
	if accepted.Reason != string(shared.PolicyReasonInvalid) {
		t.Fatalf("expected invalid reason, got %q", accepted.Reason)
	}
	if !strings.Contains(accepted.Message, "invalid auth mode combination") {
		t.Fatalf("unexpected accepted message: %q", accepted.Message)
	}
}

func TestTranslateAgentgatewayPolicyRejectsGatewayOAuth2WhenRouteJWTExists(t *testing.T) {
	gateway := testGateway()
	route := testHTTPRoute("default", "route", "gw")
	gatewayOAuth2 := testGatewayOAuth2Policy("default", "gateway-oauth2", "gw", "uid-gateway-oauth2")
	routeJWT := testRouteJWTPolicy("default", "route-jwt", "route", "uid-route-jwt")

	ctx := testutils.BuildMockPolicyContext(t, []any{
		gateway,
		route,
		gatewayOAuth2,
		routeJWT,
	})

	status, output := plugins.TranslateAgentgatewayPolicy(ctx.Krt, gatewayOAuth2, ctx.Collections)
	if len(output) != 0 {
		t.Fatalf("expected no translated policies on conflict, got %d", len(output))
	}

	accepted := findAcceptedCondition(t, status.Ancestors[0].Conditions)
	if accepted.Reason != string(shared.PolicyReasonInvalid) {
		t.Fatalf("expected invalid reason, got %q", accepted.Reason)
	}
	if !strings.Contains(accepted.Message, "invalid auth mode combination") {
		t.Fatalf("unexpected accepted message: %q", accepted.Message)
	}
}

func TestTranslateAgentgatewayPolicyAllowsRouteJWTWhenGatewayOAuth2IsDifferentListener(t *testing.T) {
	gateway := testGateway()
	oauthListener := gwv1.SectionName("listener-a")
	routeListener := gwv1.SectionName("listener-b")
	route := testHTTPRouteWithListener("default", "route", "gw", &routeListener)
	gatewayOAuth2 := testGatewayOAuth2PolicyWithSection("default", "gateway-oauth2", "gw", &oauthListener, "uid-gateway-oauth2")
	routeJWT := testRouteJWTPolicy("default", "route-jwt", "route", "uid-route-jwt")

	ctx := testutils.BuildMockPolicyContext(t, []any{
		gateway,
		route,
		gatewayOAuth2,
		routeJWT,
	})

	status, output := plugins.TranslateAgentgatewayPolicy(ctx.Krt, routeJWT, ctx.Collections)
	if len(output) == 0 {
		t.Fatalf("expected translated policies when listeners do not overlap")
	}

	accepted := findAcceptedCondition(t, status.Ancestors[0].Conditions)
	if accepted.Reason != string(shared.PolicyReasonValid) {
		t.Fatalf("expected valid reason, got %q", accepted.Reason)
	}
}

func TestTranslateAgentgatewayPolicyRejectsRouteJWTSelectorWhenGatewayOAuth2Exists(t *testing.T) {
	gateway := testGateway()
	route := testHTTPRoute("default", "route", "gw")
	route.Labels = map[string]string{"team": "red"}
	gatewayOAuth2 := testGatewayOAuth2Policy("default", "gateway-oauth2", "gw", "uid-gateway-oauth2")
	routeJWT := testRouteJWTSelectorPolicy("default", "route-jwt-selector", map[string]string{"team": "red"}, "uid-route-jwt")

	ctx := testutils.BuildMockPolicyContext(t, []any{
		gateway,
		route,
		gatewayOAuth2,
		routeJWT,
	})

	status, output := plugins.TranslateAgentgatewayPolicy(ctx.Krt, routeJWT, ctx.Collections)
	if len(output) != 0 {
		t.Fatalf("expected no translated policies on selector conflict, got %d", len(output))
	}

	accepted := findAcceptedCondition(t, status.Ancestors[0].Conditions)
	if accepted.Reason != string(shared.PolicyReasonInvalid) {
		t.Fatalf("expected invalid reason, got %q", accepted.Reason)
	}
	if !strings.Contains(accepted.Message, "invalid auth mode combination") {
		t.Fatalf("unexpected accepted message: %q", accepted.Message)
	}
}

func findAcceptedCondition(t *testing.T, conds []v1.Condition) v1.Condition {
	t.Helper()
	for _, cond := range conds {
		if cond.Type == string(shared.PolicyConditionAccepted) {
			return cond
		}
	}
	t.Fatalf("condition %q not found", shared.PolicyConditionAccepted)
	return v1.Condition{}
}

func testGateway() *gwv1.Gateway {
	return &gwv1.Gateway{
		ObjectMeta: v1.ObjectMeta{
			Namespace: "default",
			Name:      "gw",
		},
	}
}

func testHTTPRoute(namespace, name, gatewayName string) *gwv1.HTTPRoute {
	return testHTTPRouteWithListener(namespace, name, gatewayName, nil)
}

func testHTTPRouteWithListener(namespace, name, gatewayName string, sectionName *gwv1.SectionName) *gwv1.HTTPRoute {
	return &gwv1.HTTPRoute{
		ObjectMeta: v1.ObjectMeta{
			Namespace: namespace,
			Name:      name,
		},
		Spec: gwv1.HTTPRouteSpec{
			CommonRouteSpec: gwv1.CommonRouteSpec{
				ParentRefs: []gwv1.ParentReference{
					{
						Group: ptrTo(gwv1.Group(wellknown.GatewayGVK.Group)),
						Kind:  ptrTo(gwv1.Kind(wellknown.GatewayGVK.Kind)),
						Name:  gwv1.ObjectName(gatewayName),
						SectionName: func() *gwv1.SectionName {
							if sectionName == nil {
								return nil
							}
							copied := *sectionName
							return &copied
						}(),
					},
				},
			},
		},
	}
}

func testGatewayOAuth2Policy(namespace, name, gatewayName string, uid k8stypes.UID) *agentgateway.AgentgatewayPolicy {
	return testGatewayOAuth2PolicyWithSection(namespace, name, gatewayName, nil, uid)
}

func testGatewayOAuth2PolicyWithSection(
	namespace,
	name,
	gatewayName string,
	sectionName *gwv1.SectionName,
	uid k8stypes.UID,
) *agentgateway.AgentgatewayPolicy {
	return &agentgateway.AgentgatewayPolicy{
		ObjectMeta: v1.ObjectMeta{
			Namespace: namespace,
			Name:      name,
			UID:       uid,
		},
		Spec: agentgateway.AgentgatewayPolicySpec{
			TargetRefs: []shared.LocalPolicyTargetReferenceWithSectionName{
				{
					LocalPolicyTargetReference: shared.LocalPolicyTargetReference{
						Group: gwv1.Group(wellknown.GatewayGVK.Group),
						Kind:  gwv1.Kind(wellknown.GatewayGVK.Kind),
						Name:  gwv1.ObjectName(gatewayName),
					},
					SectionName: func() *gwv1.SectionName {
						if sectionName == nil {
							return nil
						}
						copied := *sectionName
						return &copied
					}(),
				},
			},
			Traffic: &agentgateway.Traffic{
				OAuth2: &agentgateway.OAuth2{},
			},
		},
	}
}

func testRouteJWTPolicy(namespace, name, routeName string, uid k8stypes.UID) *agentgateway.AgentgatewayPolicy {
	return &agentgateway.AgentgatewayPolicy{
		ObjectMeta: v1.ObjectMeta{
			Namespace: namespace,
			Name:      name,
			UID:       uid,
		},
		Spec: agentgateway.AgentgatewayPolicySpec{
			TargetRefs: []shared.LocalPolicyTargetReferenceWithSectionName{
				{
					LocalPolicyTargetReference: shared.LocalPolicyTargetReference{
						Group: gwv1.Group(wellknown.HTTPRouteGVK.Group),
						Kind:  gwv1.Kind(wellknown.HTTPRouteGVK.Kind),
						Name:  gwv1.ObjectName(routeName),
					},
				},
			},
			Traffic: &agentgateway.Traffic{
				JWTAuthentication: &agentgateway.JWTAuthentication{
					Providers: []agentgateway.JWTProvider{
						{
							Issuer: "https://issuer.example.com",
							JWKS: agentgateway.JWKS{
								Inline: ptrTo(`{"keys":[{"kid":"k1"}]}`),
							},
						},
					},
				},
			},
		},
	}
}

func testRouteJWTSelectorPolicy(
	namespace string,
	name string,
	matchLabels map[string]string,
	uid k8stypes.UID,
) *agentgateway.AgentgatewayPolicy {
	return &agentgateway.AgentgatewayPolicy{
		ObjectMeta: v1.ObjectMeta{
			Namespace: namespace,
			Name:      name,
			UID:       uid,
		},
		Spec: agentgateway.AgentgatewayPolicySpec{
			TargetSelectors: []shared.LocalPolicyTargetSelectorWithSectionName{
				{
					LocalPolicyTargetSelector: shared.LocalPolicyTargetSelector{
						Group:       gwv1.Group(wellknown.HTTPRouteGVK.Group),
						Kind:        gwv1.Kind(wellknown.HTTPRouteGVK.Kind),
						MatchLabels: matchLabels,
					},
				},
			},
			Traffic: &agentgateway.Traffic{
				JWTAuthentication: &agentgateway.JWTAuthentication{
					Providers: []agentgateway.JWTProvider{
						{
							Issuer: "https://issuer.example.com",
							JWKS: agentgateway.JWKS{
								Inline: ptrTo(`{"keys":[{"kid":"k1"}]}`),
							},
						},
					},
				},
			},
		},
	}
}

func ptrTo[T any](v T) *T {
	return &v
}
