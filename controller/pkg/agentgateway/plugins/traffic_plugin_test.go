package plugins_test

import (
	"crypto/tls"
	"fmt"
	"testing"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
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
	jwks.BuildJwksConfigMapNamespacedNameFunc(jwks.DefaultJwksStorePrefix, "kgateway-system")
}

func TestTrafficPolicies(t *testing.T) {
	testutils.RunForDirectory(t, "testdata/trafficpolicy", func(t *testing.T, ctx plugins.PolicyCtx) (*gwv1.PolicyStatus, []plugins.AgwPolicy) {
		pol := testutils.GetTestResource(t, ctx.Collections.AgentgatewayPolicies)
		s, o := plugins.TranslateAgentgatewayPolicy(ctx.Krt, pol, ctx.Collections)
		return s, o
	})
}
