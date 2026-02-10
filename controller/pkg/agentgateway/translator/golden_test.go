package translator_test

import (
	"strings"
	"testing"

	"istio.io/istio/pkg/slices"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/ir"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/testutils"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/agentgatewaysyncer"
)

func TestRouteCollection(t *testing.T) {
	testutils.RunForDirectory(t, "testdata/routes", func(t *testing.T, ctx plugins.PolicyCtx) (any, []ir.AgwResource) {
		sq, ri := testutils.Syncer(t, ctx, "HTTPRoute", "GRPCRoute", "TCPRoute", "TLSRoute", "InferencePool")
		r := ri.Outputs.Resources.List()
		r = slices.FilterInPlace(r, func(resource ir.AgwResource) bool {
			x := ir.GetAgwResourceName(resource.Resource)
			return strings.HasPrefix(x, "route/") || strings.HasPrefix(x, "tcp_route/") || strings.HasPrefix(x, "policy/")
		})
		return sq.Dump(), slices.SortBy(r, func(a ir.AgwResource) string {
			return a.ResourceName()
		})
	})
}

func TestGatewayCollection(t *testing.T) {
	testutils.RunForDirectory(t, "testdata/gateways", func(t *testing.T, ctx plugins.PolicyCtx) (any, []ir.AgwResource) {
		sq, ri := testutils.Syncer(t, ctx, "Gateway")
		r := ri.Outputs.Resources.List()
		return sq.Dump(), slices.SortBy(r, func(a ir.AgwResource) string {
			return a.ResourceName()
		})
	})
}

func TestBackends(t *testing.T) {
	testutils.RunForDirectory(t, "testdata/backends", func(t *testing.T, ctx plugins.PolicyCtx) (any, []any) {
		sq, ri := testutils.Syncer(t, ctx, "AgentgatewayBackend", "BackendTLSPolicy", "InferencePool")
		r := ri.Outputs.Resources.List()
		r = slices.SortBy(r, func(a ir.AgwResource) string {
			return a.ResourceName()
		})
		a := ri.Outputs.Addresses.List()
		a = slices.SortBy(a, func(a agentgatewaysyncer.Address) string {
			return a.ResourceName()
		})
		res := []any{}
		for _, r := range r {
			res = append(res, r)
		}
		for _, a := range a {
			if a.Service != nil {
				res = append(res, a.Service.Service)
			}
			if a.Workload != nil {
				res = append(res, a.Workload.Workload)
			}
		}
		return sq.Dump(), res
	})
}
