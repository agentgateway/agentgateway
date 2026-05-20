//go:build e2e

package e2e_test

import (
	"net/http"
	"testing"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
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
)

func TestJwtAuth(tt *testing.T) {
	t := New(tt)

	t.Run("RoutePolicy", func(t base.Test) {
		testJwtAuthRoutePolicy(t)
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
