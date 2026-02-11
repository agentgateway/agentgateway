//go:build e2e

package common

import (
	"context"
	"fmt"
	"net/http"
	"testing"
	"time"

	"istio.io/istio/pkg/log"
	"istio.io/istio/pkg/test/util/assert"
	"istio.io/istio/pkg/test/util/retry"
	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

func SetupBaseConfig(ctx context.Context, t *testing.T, installation *e2e.TestInstallation, manifests ...string) {
	for _, s := range log.Scopes() {
		s.SetOutputLevel(log.DebugLevel)
	}
	err := installation.ClusterContext.IstioClient.ApplyYAMLFiles("", manifests...)
	assert.NoError(t, err)
}

func SetupBaseGateway(ctx context.Context, installation *e2e.TestInstallation, name types.NamespacedName) {
	address := installation.Assertions.EventuallyGatewayAddress(
		ctx,
		name.Name,
		name.Namespace,
	)
	BaseGateway = Gateway{
		NamespacedName: name,
		Address:        address,
	}
}

type Gateway struct {
	types.NamespacedName
	Address string
}

var BaseGateway Gateway

func (g *Gateway) Send(t *testing.T, match *matchers.HttpResponse, opts ...curl.Option) {
	resp := g.SendWithResponse(t, match, opts...)
	_ = resp.Body.Close()
}

func (g *Gateway) SendWithResponse(t *testing.T, match *matchers.HttpResponse, opts ...curl.Option) http.Response {
	fullOpts := append([]curl.Option{curl.WithHost(g.Address)}, opts...)
	var passedRes http.Response
	retry.UntilSuccessOrFail(t, func() error {
		r, err := curl.ExecuteRequest(fullOpts...)
		if err != nil {
			return err
		}
		mm := matchers.HaveHttpResponse(match)
		success, err := mm.Match(r)
		if err != nil {
			r.Body.Close()
			return err
		}
		if !success {
			r.Body.Close()
			return fmt.Errorf("match failed: %v", mm.FailureMessage(r))
		}
		passedRes = *r
		return nil
	}, retry.Timeout(time.Second*300))
	return passedRes
}
