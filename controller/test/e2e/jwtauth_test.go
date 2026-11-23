//go:build e2e

package e2e_test

import (
	"fmt"
	"net/http"
	"strings"
	"testing"

	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/assertions"
)

//
// Use `go run hack/utils/jwt/jwt-generator.go`
// to generate jwks and a jwt signed by the key in it
//

const (
	// jwt subject is "ignore@agentgateway.dev"
	jwt1 = "eyJhbGciOiJSUzI1NiIsImtpZCI6IjUzNTg0ODg0MTQ2NzkzMTE2NDQiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2FnZW50Z2F0ZXdheS5kZXYiLCJzdWIiOiJpZ25vcmVAYWdlbnRnYXRld2F5LmRldiIsImV4cCI6MjA4NTMxNzExNSwibmJmIjoxNzc3NzMzMTE1LCJpYXQiOjE3Nzc3MzMxMTV9.dCvD5WQYRYTcHlULa9WisRTxJYTYINbJGX_QCk9x_nA6NcDETxtYXpFe6zivWkBzkEDLby9U0JfcrdeuNc2fVWlm1VjWSzFBCdf15xQBTmqfblC1Fd_0KsW17lUA01lq-p4yomV4XGPLYWTx9TiQ2zOrQSmKkIWzWRouI-eTWBpnkP6x3cQkjXWZPgZoCRyxkOXXyJTkGP5JxlaeJb3J_v94i53ZYt9jDC2gXN5HZz7IZB-IWaZSlBbCgaAl3EJtg06npQZQtlYs-QkacmA9MZMYTTZS5xB3AaqVWltEau9zbJnkqpzVH1DmsOwvT-hiJVXZoqfGHw7vvMFrbQbK-g"
	jwt2 = "eyJhbGciOiJSUzI1NiIsImtpZCI6IjExNzA4NjQ1NDE4MzI5NzA3ODkiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2FnZW50Z2F0ZXdheS5kZXYiLCJzdWIiOiJpZ25vcmVAYWdlbnRnYXRld2F5LmRldiIsImV4cCI6MjA4NTMxNzExNCwibmJmIjoxNzc3NzMzMTE0LCJpYXQiOjE3Nzc3MzMxMTR9.n1nH82Kcn3uCnnFUcol5e0yNM5M9jZijjZtPtjtJQiuVRqB6nHGeFLLCEtjbpgzYjK_Saxyv91aCFHNkbin0dHJOFf9HaxdmH_DrAycZtbUp8Runj8VoZeOUtlU7qvutbi7vKRO_I11EoNOjpA4PIi9IJouEgdjKeP9eZTt4TDrfYKME8DXa-OqvrHYRqgntjg7_i_6k23qhlTO1GFCXRWNc9pmMSSFML_nt0xpUxIHJ8SifvPrujtQ3NIB4iEM9d4XTNk7-sCfHPAyk5tFFZTO_mxOiNthxbqB1jeyS_ZHGhTDEJ9ww78yqpkc4sxwT-2NEPcgSUCQ_k_PMMxpd9g"
	jwt3 = "eyJhbGciOiJSUzI1NiIsImtpZCI6Ijc5NzEzODI2NzkxNzg1NDk2MjAiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2FnZW50Z2F0ZXdheS5kZXYiLCJzdWIiOiJpZ25vcmVAYWdlbnRnYXRld2F5LmRldiIsImV4cCI6MjA4NTMxNzExNCwibmJmIjoxNzc3NzMzMTE0LCJpYXQiOjE3Nzc3MzMxMTR9.SPuJpi6W_UM-cUWDYw3AcIGRGIGSjjogeqWzf-_rrHZ7FsOY4566FmKaqxai0T3a6z4TYj30qIItgftQEVXrFxXVkMLLN7PoPSmiqp2T8FOmPZODOKio_IVwfOPlc99I9y0_cGsyEOsilxm1qje0gRovqUyVd3wWnsoknf3YWLbBWwNCWawteumDBAN4A7CVncDXKNNjk_uXdUwO-ah_Cwao-nLdU2GPiVGtP-V3_5ClK-khWvk8qthEuTOkZ0jeRTcMNQKHkTONALqLsnXEhZOOFjQ8d-ueTk2tYduSqJ8uiiF9Uvzz-tNVrC1-nvXcpKb0Ob3YnMH1VycK1invNA"
	jwt4 = "eyJhbGciOiJSUzI1NiIsImtpZCI6IjcwNzMwMjQzNTI5MTkzMjkwOTQiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2FnZW50Z2F0ZXdheS5kZXYiLCJzdWIiOiJpZ25vcmVAYWdlbnRnYXRld2F5LmRldiIsImV4cCI6MjA4NTMxNzExNSwibmJmIjoxNzc3NzMzMTE1LCJpYXQiOjE3Nzc3MzMxMTV9.BZqclslF020OmjLY8ZmLhtx-LCqwUxn1Wsdq7SeUtzZ7NI64MwH37Bxd2z9AGSVOhliBB8otcRdiWRHMhHfaKu4l9NDYpsmFWIYViiuZQd4OUPtS5d2NmRXAl4noZ5EzmtMHrTYhv1wBB8bWQGs20mimTjdcdJbmzHcEqmNMMHxX93Wk25xAn4habR8b8Z2HlxlU-MZj40gL_iPsH088e8gf-Qb4JCqrQc4_UI8EpsO4vWk42gwJGU9ZLDFDt6mWs88OWMgs0c0DB82lX5xyVZtmFyVmq1p7mW9Ez9olUg64iOBIhdnv7560Ilc6_9AwJ9zU2fcDGaBP0ZaF1vxOsg"
	// jwt subject is "boom@agentgateway.dev"
	jwt5 = "eyJhbGciOiJSUzI1NiIsImtpZCI6IjcwNzMwMjQzNTI5MTkzMjkwOTQiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2FnZW50Z2F0ZXdheS5kZXYiLCJzdWIiOiJib29tQGFnZW50Z2F0ZXdheS5kZXYiLCJleHAiOjIwODUzMTcxMTUsIm5iZiI6MTc3NzczMzExNSwiaWF0IjoxNzc3NzMzMTE1fQ.MS9PaXb81m8tBEs1qtTBD6LSD8lTYJuP2ygvmrzwnwiYLb7-QbLJUwtxwCSxu6icwOU50OHQiFsyLnYnmpACvJ0Nc3co_a2q4lThUNuUyLxwxqJWRRFiFqF78hv3E3A3Nrdpuvk5qF4M8yqusPcpOd6dhAwwlSoEM8_2q5__PuNNFIx6Z37LS507rKcmYfk7kCvpBbddi5n9tyYcHpvZEckPhNdWn_E7yyEi_WrIhAq1OcgrwbS2JFrLoeUap2FrpSVvkk-dfRzR2QreTehc4WihFCPTPc0edhHeb0AW8wfsyjSQvq4DkXw_SWMdonRWqxQYqnYiDv1v49bC-ro6Xg"
	// TODO: generate these tokens in the test if more are needed.
	// signed by the jwks in testdata/jwtauth/secured-route-with-actors.yaml, subject is "ignore@agentgateway.dev"
	jwtNoActors = "eyJhbGciOiJSUzI1NiIsImtpZCI6Ijc5NTE0NzExNTk5NTY3NTY4NDMiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2FnZW50Z2F0ZXdheS5kZXYiLCJzdWIiOiJpZ25vcmVAYWdlbnRnYXRld2F5LmRldiIsImV4cCI6MjEwMTkwMzY2MCwibmJmIjoxNzk0MzE5NjYwLCJpYXQiOjE3OTQzMTk2NjB9.XmWQzzo7uwUOTqNGlSctgjIYBiwNUsgf7-83ZcjF9cHFrSErU6sh41L2RaXo-HsgVY1x5DH8i9qXcfYdd5SIUzwHh_KnYVOUu2I0WElcOIcQrP9VwG9CljAF0HeAUsam5esDXpgH5JjvociJPv8JhQfX7xXw_WAdqI_c4kMO3hjM436lKjuXcqjg4W26LTxKoLFGq1XE6GD0DIWH3rKXQk6RdJ6zg5a9AZ4yjz_W63LFLqYLxgUyHR4Hjt51BzV8mG10NlhAx4GgB32SICOoZ408DcwcYVSOSVeY8G5eY_nvcxrj6vzqbClPmaF_bpS2YGmfa2JIY29PnHFapprDgA"
	// act chains, current actor first: [agent-a], [agent-a, agent-b], [agent-a, agent-b, agent-c]
	jwtActorsA   = "eyJhbGciOiJSUzI1NiIsImtpZCI6Ijc5NTE0NzExNTk5NTY3NTY4NDMiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2FnZW50Z2F0ZXdheS5kZXYiLCJzdWIiOiJpZ25vcmVAYWdlbnRnYXRld2F5LmRldiIsImV4cCI6MjEwMTkwMzY2MCwibmJmIjoxNzk0MzE5NjYwLCJpYXQiOjE3OTQzMTk2NjAsImFjdCI6eyJzdWIiOiJhZ2VudC1hIn19.DAMU6ubyYH9BEmBoY7D7j20teg65rIr79Tygp_kGSIC6G4XQA93Dj4SADdHR7qCrZIAu6yBDwZvtUxXbCmuh7cge8tW096fdvj9ilqRsc-9T9nNk1HwAmrAWr-jhlfGhYGoIbD-oBj4MrOyToAhyO4PAZvslP7or_YFrxcIF0VbshIZTtg3FEbw6BQwlzEWYcYfj3oHaMK-Z16ihTMg8CAucAsydieSdkjQHvNHCuzEoCeNR3NrhOtE1uXzBzm7Jl1rOcZ4y3lBqQLNrR79d2Drz81yZeIEEXkEV2ic5QtBpFWbkJ3qyucScpD83Vm1tFN01PEFlPwF2MCJmJpyEfg"
	jwtActorsAB  = "eyJhbGciOiJSUzI1NiIsImtpZCI6Ijc5NTE0NzExNTk5NTY3NTY4NDMiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2FnZW50Z2F0ZXdheS5kZXYiLCJzdWIiOiJpZ25vcmVAYWdlbnRnYXRld2F5LmRldiIsImV4cCI6MjEwMTkwMzY2MCwibmJmIjoxNzk0MzE5NjYwLCJpYXQiOjE3OTQzMTk2NjAsImFjdCI6eyJhY3QiOnsic3ViIjoiYWdlbnQtYiJ9LCJzdWIiOiJhZ2VudC1hIn19.l36G9L5smE2EPLgUxLp2htd8LhYhGYRqVx29DvztzMgFzPb7S5jhxKGSx61dakbsioqT0Qh_SHxOTdgp_kLSa-8T3orRNch5sjszoksAgxeMSLFf3NJA6XGTmXcaOOzyt0n_5daAJK4W4NzFXuTF6GuGwysTQCNhtBkrpLC1tYamDYO_wS-SEb0Z5yyNxljKolXOVfWizuXH6u77h3Pw5-5fSYs4KwKPMq31ingPg75fQeC79QyOLMXeykQnSgtHzWKcOS1sXDbkKGzccVatbw22EFuxtr0dvtsjVYgyNAcrDHLGdPOj90Hh-lBMjPiMlFcrxhLPt7Dk_XDMQQAtcw"
	jwtActorsABC = "eyJhbGciOiJSUzI1NiIsImtpZCI6Ijc5NTE0NzExNTk5NTY3NTY4NDMiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2FnZW50Z2F0ZXdheS5kZXYiLCJzdWIiOiJpZ25vcmVAYWdlbnRnYXRld2F5LmRldiIsImV4cCI6MjEwMTkwMzY2MCwibmJmIjoxNzk0MzE5NjYwLCJpYXQiOjE3OTQzMTk2NjAsImFjdCI6eyJhY3QiOnsiYWN0Ijp7InN1YiI6ImFnZW50LWMifSwic3ViIjoiYWdlbnQtYiJ9LCJzdWIiOiJhZ2VudC1hIn19.U989v5saO9wGHxq4Xc-F1t263nnFilqiucytWZoaGkQPE030MlSRTfDOLPGLqhpDLrH-yUh-i7SAN8jn08wfHV-0GLAlktWBZkYsUE9ReWyAkbWCXXjDVK4Xb7fTeOAeoH2wH7pxlvGn5Ula5XPh_MfKn5RETawDuCzUdrL0KrDI5XBrqT4ovwEPEclTkV7ziCBvIxIrNYTHj3j9G5HU6kEs557uxu_Ccx4ulgAvJU3-sZaH6bVoZTKlOFGz9GhS38VpskcaAFX7JiX754fNjQpUnmqjU4Phy1Nfle8y62n4jy9hNMVacgbxQn-toYz7cbh21okzUl4eDGwP5L0pDQ"
)

func TestJwtAuth(tt *testing.T) {
	t := New(tt)

	t.Run("RoutePolicy", func(t base.Test) {
		testJwtAuthRoutePolicy(t)
	})
	t.Run("NoAudienceClaim", func(t base.Test) {
		t.Apply(manifest("jwtauth", "secured-route.yaml"))
		t.HTTPRouteAccepted("route-secure", base.Namespace)
		// The provider does not configure audiences, so a JWT without aud is valid.
		assertJwtResponse(t, "secureroute.com", jwt1, http.StatusOK)
	})
	t.Run("RoutePolicyWithRBAC", func(t base.Test) {
		testJwtAuthRoutePolicyWithRbac(t)
	})
	t.Run("GatewayPolicy", func(t base.Test) {
		testJwtAuthGatewayPolicy(t)
	})
	t.Run("GatewayPolicyWithRBAC", func(t base.Test) {
		testJwtAuthGatewayPolicyWithRbac(t)
	})
	t.Run("RoutePolicyWithActors", func(t base.Test) {
		testJwtAuthRoutePolicyWithActors(t)
	})
	t.Run("InvalidInlineJwks", func(t base.Test) {
		testJwtAuthInvalidInlineJwks(t)
	})
}

func testJwtAuthInvalidInlineJwks(t base.Test) {
	t.Apply(
		manifest("jwtauth", "invalid-inline.yaml"),
	)

	t.HTTPRouteAccepted("route-invalid-jwks", base.Namespace)

	assertions.EventuallyAgwPolicyStatus(t, "route-invalid-inline-jwks-policy", base.Namespace, func(status gwv1.PolicyStatus) error {
		for _, ancestor := range status.Ancestors {
			for _, condition := range ancestor.Conditions {
				if condition.Type == agentgateway.PolicyConditionAccepted &&
					condition.Status == metav1.ConditionTrue &&
					condition.Reason == agentgateway.PolicyReasonPartiallyValid &&
					strings.Contains(condition.Message, "invalid inline") {
					return nil
				}
			}
		}
		return fmt.Errorf("policy status does not report the invalid inline JWKS as PartiallyValid: %+v", status)
	})
	assertJwtResponse(t, "invalidjwksroute.com", "", http.StatusUnauthorized)

	// assert that we are still accepting new config by checking secure route
	assertJwtResponse(t, "secureroute.com", jwt1, http.StatusNotFound)
	t.Apply(manifest("jwtauth", "secured-route.yaml"))
	t.HTTPRouteAccepted("route-secure", base.Namespace)
	assertJwtResponse(t, "secureroute.com", jwt1, http.StatusOK)
}

func testJwtAuthRoutePolicy(t base.Test) {
	t.Apply(
		manifest("jwtauth", "insecure-route.yaml"),
		manifest("jwtauth", "secured-route.yaml"),
	)

	t.HTTPRouteAccepted("route-example-insecure", base.Namespace)
	// verify unprotected route works
	assertJwtResponse(t, "insecureroute.com", "", http.StatusOK)

	t.HTTPRouteAccepted("route-secure", base.Namespace)
	// verify a provider with a single key in jwks works
	assertJwtResponse(t, "secureroute.com", jwt1, http.StatusOK)
	// verify a provider with multiple keys in jwks works
	assertJwtResponse(t, "secureroute.com", jwt2, http.StatusOK)
	assertJwtResponse(t, "secureroute.com", jwt3, http.StatusOK)
	// verify invalid/missing tokens are caught
	assertJwtResponse(t, "secureroute.com", "nosuchkey", http.StatusUnauthorized)
	assertJwtResponse(t, "secureroute.com", "", http.StatusUnauthorized)
}

func testJwtAuthRoutePolicyWithRbac(t base.Test) {
	t.Apply(manifest("jwtauth", "secured-route-with-rbac.yaml"))

	t.HTTPRouteAccepted("route-secure", base.Namespace)
	// jwt subject matches rbac policy
	assertJwtResponse(t, "secureroute.com", jwt4, http.StatusOK)
	// jwt subject doesn't match rbac policy
	assertJwtResponse(t, "secureroute.com", jwt5, http.StatusForbidden)
}

func testJwtAuthRoutePolicyWithActors(t base.Test) {
	t.Apply(manifest("jwtauth", "secured-route-with-actors.yaml"))

	t.HTTPRouteAccepted("route-actors", base.Namespace)
	assertJwtResponse(t, "actorsroute.com", jwtActorsA, http.StatusOK)
	assertJwtResponse(t, "actorsroute.com", jwtActorsAB, http.StatusOK)
	// agent-c deep in the chain is not allowed
	assertJwtResponse(t, "actorsroute.com", jwtActorsABC, http.StatusForbidden)
	// no delegation
	assertJwtResponse(t, "actorsroute.com", jwtNoActors, http.StatusForbidden)
}

func testJwtAuthGatewayPolicy(t base.Test) {
	t.Apply(manifest("jwtauth", "secured-gateway-policy.yaml"))

	t.HTTPRouteAccepted("route-secure-gw", base.Namespace)
	// verify a provider with a single key in jwks works
	assertJwtResponse(t, "securegateways.com", jwt1, http.StatusOK)
	// verify a provider with multiple keys in jwks works
	assertJwtResponse(t, "securegateways.com", jwt2, http.StatusOK)
	assertJwtResponse(t, "securegateways.com", jwt3, http.StatusOK)
	assertJwtResponse(t, "securegateways.com", "nosuchkey", http.StatusUnauthorized)
	// verify invalid/missing tokens are caught
	assertJwtResponse(t, "securegateways.com", "", http.StatusUnauthorized)
}

func testJwtAuthGatewayPolicyWithRbac(t base.Test) {
	t.Apply(manifest("jwtauth", "secured-gateway-policy-with-rbac.yaml"))

	t.HTTPRouteAccepted("route-secure-gw", base.Namespace)
	// jwt subject matches rbac policy
	assertJwtResponse(t, "securegateways.com", jwt4, http.StatusOK)
	// jwt subject doesn't match rbac policy
	assertJwtResponse(t, "securegateways.com", jwt5, http.StatusForbidden)
}

func assertJwtResponse(t base.Test, host, token string, status int) {
	opts := []curl.Option{}
	if token != "" {
		opts = append(opts, curl.WithHeader("Authorization", "Bearer "+token))
	}
	t.Send(host, base.Expect(status), opts...)
}
