package plugins_test

import (
	"crypto/tls"
	"fmt"
	"strings"
	"testing"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/slices"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/ir"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/jwks"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/jwks_url"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/testutils"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils"
)

type jwksUrlFactoryForTesting struct{}

func (f *jwksUrlFactoryForTesting) BuildJwksUrlAndTlsConfig(krtctx krt.HandlerContext, policyName, defaultNS string, remoteProvider *agentgateway.RemoteJWKS) (string, *tls.Config, error) {
	ref := remoteProvider.BackendRef

	refName := string(ref.Name)
	refNamespace := string(ptr.OrDefault(ref.Namespace, gwv1.Namespace(defaultNS)))
	host := kubeutils.GetServiceHostname(refName, refNamespace)
	var fqdn string
	if port := ptr.OrEmpty(ref.Port); port != 0 {
		fqdn = fmt.Sprintf("%s:%d", host, port)
	} else {
		fqdn = host
	}

	return fmt.Sprintf("http://%s/%s", fqdn, remoteProvider.JwksPath), nil, nil
}

func init() {
	jwks_url.JwksUrlBuilderFactory = func() jwks_url.JwksUrlBuilder { return &jwksUrlFactoryForTesting{} }
	jwks.BuildJwksConfigMapNamespacedNameFunc(jwks.DefaultJwksStorePrefix, "agentgateway-system")
}

func TestTrafficPolicies(t *testing.T) {
	policyTest(t, "testdata/trafficpolicy")
}

func TestBackendPolicies(t *testing.T) {
	policyTest(t, "testdata/backendpolicy")
}

func TestFrontendPolicies(t *testing.T) {
	policyTest(t, "testdata/frontendpolicy")
}

func policyTest(t *testing.T, folder string) {
	t.Helper()
	testutils.RunForDirectory(t, folder, func(t *testing.T, ctx plugins.PolicyCtx) (any, []ir.AgwResource) {
		sq, ri := testutils.Syncer(t, ctx, "AgentgatewayPolicy")
		r := ri.Outputs.Resources.List()
		r = slices.FilterInPlace(r, func(resource ir.AgwResource) bool {
			x := ir.GetAgwResourceName(resource.Resource)
			return strings.HasPrefix(x, "policy/")
		})
		return sq.Dump(), slices.SortBy(r, func(a ir.AgwResource) string {
			return a.ResourceName()
		})
	})
}
