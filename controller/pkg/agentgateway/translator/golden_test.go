package translator_test

import (
	"fmt"
	"strings"
	"testing"

	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/slices"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/ir"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/testutils"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/agentgatewaysyncer"
)

func TestReferences(t *testing.T) {
	testutils.RunForDirectory(t, "testdata/references", func(t *testing.T, ctx plugins.PolicyCtx) (any, []ir.AgwResource) {
		sq, ri := testutils.Syncer(t, ctx, "")
		r := ri.Outputs.Resources.List()
		r = slices.FilterInPlace(r, func(resource ir.AgwResource) bool {
			x := ir.GetAgwResourceName(resource.Resource)
			return strings.HasPrefix(x, "policy/") || strings.HasPrefix(x, "backend/")
		})
		return sq.Dump(), slices.SortBy(r, func(a ir.AgwResource) string {
			return a.ResourceName()
		})
	})
}

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
		sq, ri := testutils.Syncer(t, ctx, "Gateway", "ListenerSet")
		r := ri.Outputs.Resources.List()
		return sq.Dump(), slices.SortBy(r, func(a ir.AgwResource) string {
			return a.ResourceName()
		})
	})
}

func TestBackends(t *testing.T) {
	testutils.RunForDirectory(t, "testdata/backends", func(t *testing.T, ctx plugins.PolicyCtx) (any, []any) {
		dummyRoutes := setupDummyAncestorMapping(ctx)
		ctx.Collections.HTTPRoutes = krt.JoinCollection([]krt.Collection[*gwv1.HTTPRoute]{ctx.Collections.HTTPRoutes, krt.NewStaticCollection(nil, dummyRoutes)})
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
			if r.Resource.GetBind() != nil || r.Resource.GetRoute() != nil || r.Resource.GetListener() != nil {
				// Not relevant to our tests here, just auto-gen stuff
				continue
			}
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

// We will only include backends that are referenced by a Gateway. So we build the HTTPRoute our selves.
func setupDummyAncestorMapping(ctx plugins.PolicyCtx) []*gwv1.HTTPRoute {
	bes := []controllers.Object{}
	for _, v := range ctx.Collections.Backends.List() {
		bes = append(bes, v)
	}
	for _, v := range ctx.Collections.Services.List() {
		bes = append(bes, v)
	}
	for _, v := range ctx.Collections.ServiceEntries.List() {
		bes = append(bes, v)
	}
	for _, v := range ctx.Collections.InferencePools.List() {
		bes = append(bes, v)
	}
	dummyRoutes := []*gwv1.HTTPRoute{}
	for idx, backend := range bes {
		dummyRoutes = append(dummyRoutes, &gwv1.HTTPRoute{
			TypeMeta: metav1.TypeMeta{},
			ObjectMeta: metav1.ObjectMeta{
				Name:      fmt.Sprintf("dummy-%d", idx),
				Namespace: "default",
			},
			Spec: gwv1.HTTPRouteSpec{
				CommonRouteSpec: gwv1.CommonRouteSpec{
					ParentRefs: []gwv1.ParentReference{{
						Name: "basic",
					}},
				},
				Rules: []gwv1.HTTPRouteRule{{
					BackendRefs: []gwv1.HTTPBackendRef{{
						BackendRef: gwv1.BackendRef{
							BackendObjectReference: gwv1.BackendObjectReference{
								Group:     ptr.Of(gwv1.Group(backend.GetObjectKind().GroupVersionKind().Group)),
								Kind:      ptr.Of(gwv1.Kind(backend.GetObjectKind().GroupVersionKind().Kind)),
								Name:      gwv1.ObjectName(backend.GetName()),
								Namespace: ptr.Of(gwv1.Namespace(backend.GetNamespace())),
								Port:      nil,
							},
						},
					}},
				}},
			},
		})
	}
	return dummyRoutes
}
