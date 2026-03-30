//go:build e2e

package remotejwtauth

import (
	"path/filepath"
	"strings"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/fsutils"
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
)

//
// Use `go run hack/utils/jwt/jwt-generator.go`
// to generate jwks and a jwt signed by the key in it
//

var _ e2e.NewSuiteFunc = NewTestingSuite

const (
	namespace = "agentgateway-base"

	insecureRouteName      = "route-example-insecure"
	secureRouteName        = "route-secure"
	secureGatewayRouteName = "route-secure-gw"

	insecureRouteHost = "insecureroute.com"
	secureRouteHost   = "secureroute.com"
	secureGatewayHost = "securegateways.com"

	// jwt subject is "ignore@kgateway.dev"
	// could also retrieve these jwts from https://dummy-idp.default:8443/org-one/jwt and https://dummy-idp.default:8443/org-two/jwt
	jwtOrgOne = "eyJhbGciOiJSUzI1NiIsImtpZCI6IjUzNTAyMzEyMTkzMDYwMzg2OTIiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2tnYXRld2F5LmRldiIsInN1YiI6Imlnbm9yZUBrZ2F0ZXdheS5kZXYiLCJleHAiOjIwNzExNjM0MDcsIm5iZiI6MTc2MzU3OTQwNywiaWF0IjoxNzYzNTc5NDA3fQ.TsHCCdd0_629wibU4EviEi1-_UXaFUX1NuLgXCrC-tr7kqlcnUJIJC0WSab1EgXKtF8gTfwTUeQcAQNrunwngQU-K9DFcH5-2vnGeiXV3_X3SokkPq74ceRrCFEL2d7YNaGfhq_UNyvKRJsRz-pwdKK7QIPXALmWaUHn7EV7zU-CcPCKNwmt62P88qNp5HYSbgqz_WfnzIIH8LANpCC8fUqVedgTJMJ86E06pfDNUuuXe_fhjgMQXlfyDeUxIuzJunvS2qIqt4IYMzjcQbl2QI1QK3xz37tridSP_WVuuMUe2Lqo0oDjWVpxqPb5fb90W6a6khRP59Pf6qKMbQ9SQg"
	jwtOrgTwo = "eyJhbGciOiJSUzI1NiIsImtpZCI6IjI4OTk1NjQyMzcyMTQ2ODQ5NDciLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2tnYXRld2F5LmRldiIsInN1YiI6Imlnbm9yZUBrZ2F0ZXdheS5kZXYiLCJleHAiOjIwNzExNjM1MzIsIm5iZiI6MTc2MzU3OTUzMiwiaWF0IjoxNzYzNTc5NTMyfQ.kLazcb2o_zcVfJ7WECsQJdOaluxAJ-GdOkeuXUOJSeN8PvahjxfpftgeJjcGsp2sl-VIKXIuTLH6csHT_CBq7kI8bVKGDkk8qw3w8gem7MtiXKPMSYiYEHAoCCzsl8O-pGPF6G_PU-CfiWla8CIAjOewLzRmLeAYmwEiUYf8LQ7y6BbVDzvtxIQW3pTurHXFy0TZ6nUGqu_Xwh7uXe42WC0T-9LAI4zsGo5x_FKhlE_6N9_a7R0UIYFeRrbph_b1z47xTZ3YhZBmQmue2j1xR6hwRCnL7mOaCrxdte8SqXNUVA6vPSaiMTSkdmKyeRSzeTliDKiqAmP8eiIaqAoN5A"
	// sub "boom@kgateway.dev"
	jwtOrgFour = "eyJhbGciOiJSUzI1NiIsImtpZCI6IjI5MjkxMDAyNTE1MzE5NjM0MCIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJodHRwczovL2tnYXRld2F5LmRldiIsInN1YiI6ImJvb21Aa2dhdGV3YXkuZGV2IiwiZXhwIjoyMDczMTU2OTc5LCJuYmYiOjE3NjU1NzI5NzksImlhdCI6MTc2NTU3Mjk3OX0.juMOUmoChZEE_AQVZv3jwtZjytWfzN23-palLXA-DIsSa4-f-lmf3CQiwXz0n1YlSY_dt3rGO6OsDdkYn8wkYEVoQVh11crJvZ5FhpIlZlROOSp03KTW2mQ1XwGYRxffzdzBv65LrFYWK0iNQH2NKfqOzVo5xt3SLTJuxIvCE8-qnqXUWrADw3b2TIzE7SgN7xXzeRGwTpgltq4BswdkB0R5g_1xtbrcdFgT533vt3nCiumhqrBkmk4g02x3L1iSjDCnnwJX2YLHYfpUN0i7SooguTkta067lwBiOi3NOTQjRBOBlZmkoj6sz4YNQ9EwsD74pkNBW9pN-__2cVPBxw"
)

type testingSuite struct {
	*base.BaseTestingSuite
}

var (
	setup = base.TestCase{
		Manifests: []string{
			getTestFile("common.yaml"),
		},
	}

	testCases = map[string]*base.TestCase{
		"TestRoutePolicySvc":                    manifestCase(secureRoutePolicyManifestSvc),
		"TestRoutePolicySvcCaCert":              manifestWithCABundleCase(secureRoutePolicyManifestSvc),
		"TestRoutePolicyBackend":                manifestCase(insecureRouteManifest, secureRoutePolicyManifestBackend),
		"TestRoutePolicyBackendAndTlsPolicy":    manifestCase(secureRoutePolicyManifestBackendAndTLSPolicy),
		"TestRoutePolicyWithRbac":               manifestCase(secureRoutePolicyWithRBACManifest),
		"TestGatewayPolicySvc":                  manifestCase(secureGatewayPolicyManifestSvc),
		"TestGatewayPolicySvcCaCert":            manifestWithCABundleCase(secureGatewayPolicyManifestSvc),
		"TestGatewayPolicyBackend":              manifestCase(secureGatewayPolicyManifestBackend),
		"TestGatewayPolicyBackendWithTlsPolicy": manifestCase(secureGatewayPolicyManifestBackendAndTLSPolicy),
		"TestGatewayPolicyWithRbac":             manifestCase(secureGatewayPolicyWithRBACManifest),
	}

	insecureRouteManifest                          = getTestFile("insecure-route.yaml")
	secureGatewayPolicyManifestBackend             = getTestFile("secured-gateway-policy-with-backend.yaml")
	secureGatewayPolicyManifestBackendAndTLSPolicy = getTestFile("secured-gateway-policy-with-backend-and-ref.yaml")
	secureGatewayPolicyManifestSvc                 = getTestFile("secured-gateway-policy-with-svc.yaml")
	secureGatewayPolicyWithRBACManifest            = getTestFile("secured-gateway-policy-with-rbac.yaml")
	secureRoutePolicyManifestBackend               = getTestFile("secured-route-with-backend.yaml")
	secureRoutePolicyManifestBackendAndTLSPolicy   = getTestFile("secured-route-with-backend-and-ref.yaml")
	secureRoutePolicyManifestSvc                   = getTestFile("secured-route-with-svc.yaml")
	secureRoutePolicyWithRBACManifest              = getTestFile("secured-route-with-rbac.yaml")
)

func manifestCase(manifests ...string) *base.TestCase {
	return &base.TestCase{Manifests: manifests}
}

func manifestWithCABundleCase(manifest string) *base.TestCase {
	return &base.TestCase{
		ManifestsWithTransform: map[string]func(string) string{
			manifest: withCABundleTLS,
		},
	}
}

func withCABundleTLS(content string) string {
	return strings.Replace(
		content,
		"    tls:\n      insecureSkipVerify: All",
		"    tls:\n      caCertificateRefs:\n      - name: ca",
		1,
	)
}

func getTestFile(filename string) string {
	return filepath.Join(fsutils.MustGetThisDir(), "testdata", filename)
}
