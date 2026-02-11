package reports_test

import (
	"context"
	"testing"

	"istio.io/istio/pkg/test/util/assert"
	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/utils/ptr"
	"sigs.k8s.io/controller-runtime/pkg/client"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gwv1a2 "sigs.k8s.io/gateway-api/apis/v1alpha2"

	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/wellknown"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/reporter"
	"github.com/agentgateway/agentgateway/controller/pkg/reports"
)

const fakeCondition = "kgateway.dev/SomeCondition"

var ctx = context.Background()

func TestBuildGatewayStatus(t *testing.T) {
	t.Run("build all positive conditions with an empty report", func(t *testing.T) {
		gw := gw()
		rm := reports.NewReportMap()

		r := reports.NewReporter(&rm)
		// Initialize GatewayReporter to mimic translation loop.
		r.Gateway(gw)

		status := rm.BuildGWStatus(context.Background(), *gw, nil)

		assert.Equal(t, true, status != nil)
		assert.Equal(t, 2, len(status.Conditions))
		assert.Equal(t, 1, len(status.Listeners))
		assert.Equal(t, 4, len(status.Listeners[0].Conditions))
	})

	t.Run("preserve conditions set externally", func(t *testing.T) {
		gw := gw()
		gw.Status.Conditions = append(gw.Status.Conditions, metav1.Condition{
			Type:   "gateway.kgateway.dev/SomeCondition",
			Status: metav1.ConditionFalse,
		})
		rm := reports.NewReportMap()

		r := reports.NewReporter(&rm)
		// Initialize GatewayReporter to mimic translation loop.
		r.Gateway(gw)

		status := rm.BuildGWStatus(context.Background(), *gw, nil)

		assert.Equal(t, true, status != nil)
		assert.Equal(t, 3, len(status.Conditions)) // 2 from report, 1 from original status.
		assert.Equal(t, 1, len(status.Listeners))
		assert.Equal(t, 4, len(status.Listeners[0].Conditions))
	})

	t.Run("set negative gateway conditions from report and not add extra conditions", func(t *testing.T) {
		gw := gw()
		rm := reports.NewReportMap()
		r := reports.NewReporter(&rm)
		r.Gateway(gw).SetCondition(reporter.GatewayCondition{
			Type:   gwv1.GatewayConditionProgrammed,
			Status: metav1.ConditionFalse,
			Reason: gwv1.GatewayReasonAddressNotUsable,
		})
		status := rm.BuildGWStatus(context.Background(), *gw, nil)

		assert.Equal(t, true, status != nil)
		assert.Equal(t, 2, len(status.Conditions))
		assert.Equal(t, 1, len(status.Listeners))
		assert.Equal(t, 4, len(status.Listeners[0].Conditions))

		programmed := meta.FindStatusCondition(status.Conditions, string(gwv1.GatewayConditionProgrammed))
		assert.Equal(t, true, programmed != nil)
		assert.Equal(t, metav1.ConditionFalse, programmed.Status)
	})

	t.Run("set negative listener conditions from report and not add extra conditions", func(t *testing.T) {
		gw := gw()
		rm := reports.NewReportMap()
		r := reports.NewReporter(&rm)
		r.Gateway(gw).Listener(listener()).SetCondition(reporter.ListenerCondition{
			Type:   gwv1.ListenerConditionResolvedRefs,
			Status: metav1.ConditionFalse,
			Reason: gwv1.ListenerReasonInvalidRouteKinds,
		})
		status := rm.BuildGWStatus(context.Background(), *gw, nil)

		assert.Equal(t, true, status != nil)
		assert.Equal(t, 2, len(status.Conditions))
		assert.Equal(t, 1, len(status.Listeners))
		assert.Equal(t, 4, len(status.Listeners[0].Conditions))

		resolvedRefs := meta.FindStatusCondition(status.Listeners[0].Conditions, string(gwv1.ListenerConditionResolvedRefs))
		assert.Equal(t, true, resolvedRefs != nil)
		assert.Equal(t, metav1.ConditionFalse, resolvedRefs.Status)
	})

	t.Run("does not modify LastTransitionTime for existing conditions that have not changed", func(t *testing.T) {
		gw := gw()
		rm := reports.NewReportMap()

		r := reports.NewReporter(&rm)
		// Initialize GatewayReporter to mimic translation loop.
		r.Gateway(gw)

		status := rm.BuildGWStatus(context.Background(), *gw, nil)

		assert.Equal(t, true, status != nil)
		assert.Equal(t, 2, len(status.Conditions))
		assert.Equal(t, 1, len(status.Listeners))
		assert.Equal(t, 4, len(status.Listeners[0].Conditions))

		acceptedCond := meta.FindStatusCondition(status.Listeners[0].Conditions, string(gwv1.ListenerConditionAccepted))
		assert.Equal(t, true, acceptedCond != nil)
		oldTransitionTime := acceptedCond.LastTransitionTime

		gw.Status = *status
		status = rm.BuildGWStatus(context.Background(), *gw, nil)

		assert.Equal(t, true, status != nil)
		assert.Equal(t, 2, len(status.Conditions))
		assert.Equal(t, 1, len(status.Listeners))
		assert.Equal(t, 4, len(status.Listeners[0].Conditions))

		acceptedCond = meta.FindStatusCondition(status.Listeners[0].Conditions, string(gwv1.ListenerConditionAccepted))
		assert.Equal(t, true, acceptedCond != nil)
		newTransitionTime := acceptedCond.LastTransitionTime
		assert.Equal(t, oldTransitionTime, newTransitionTime)
	})
}

func TestBuildRouteStatus(t *testing.T) {
	t.Run("build all positive route conditions with an empty report", func(t *testing.T) {
		tests := []struct {
			name string
			obj  client.Object
		}{
			{name: "regular httproute", obj: httpRoute()},
			{name: "regular tcproute", obj: tcpRoute()},
			{name: "regular tlsroute", obj: tlsRoute()},
			{name: "regular grpcroute", obj: grpcRoute()},
			{name: "delegatee route", obj: delegateeRoute()},
		}

		for _, tt := range tests {
			t.Run(tt.name, func(t *testing.T) {
				rm := reports.NewReportMap()

				r := reports.NewReporter(&rm)
				fakeTranslate(r, tt.obj)
				status := rm.BuildRouteStatus(ctx, tt.obj, wellknown.DefaultAgwControllerName)

				assert.Equal(t, true, status != nil)
				assert.Equal(t, 1, len(status.Parents))
				assert.Equal(t, 2, len(status.Parents[0].Conditions))
			})
		}
	})

	t.Run("preserve conditions set externally", func(t *testing.T) {
		tests := []struct {
			name string
			obj  client.Object
		}{
			{name: "regular httproute", obj: httpRoute(metav1.Condition{Type: fakeCondition})},
			{name: "regular tcproute", obj: tcpRoute(metav1.Condition{Type: fakeCondition})},
			{name: "regular tlsroute", obj: tlsRoute(metav1.Condition{Type: fakeCondition})},
			{name: "regular grpcroute", obj: grpcRoute(metav1.Condition{Type: fakeCondition})},
			{name: "delegatee route", obj: delegateeRoute(metav1.Condition{Type: fakeCondition})},
		}

		for _, tt := range tests {
			t.Run(tt.name, func(t *testing.T) {
				rm := reports.NewReportMap()

				r := reports.NewReporter(&rm)
				fakeTranslate(r, tt.obj)
				status := rm.BuildRouteStatus(ctx, tt.obj, wellknown.DefaultAgwControllerName)

				assert.Equal(t, true, status != nil)
				assert.Equal(t, 1, len(status.Parents))
				assert.Equal(t, 3, len(status.Parents[0].Conditions)) // 2 from report, 1 original.
			})
		}
	})

	t.Run("do not report for parentRefs that belong to other controllers", func(t *testing.T) {
		rm := reports.NewReportMap()
		r := reports.NewReporter(&rm)

		route := &gwv1.HTTPRoute{
			ObjectMeta: metav1.ObjectMeta{
				Name:      "route",
				Namespace: "default",
			},
			Spec: gwv1.HTTPRouteSpec{
				CommonRouteSpec: gwv1.CommonRouteSpec{
					ParentRefs: []gwv1.ParentReference{
						*parentRef(),
						*otherParentRef(),
					},
				},
			},
			Status: gwv1.HTTPRouteStatus{
				RouteStatus: gwv1.RouteStatus{
					Parents: []gwv1.RouteParentStatus{
						{
							ControllerName: "other.io/controller",
							ParentRef:      *otherParentRef(),
							Conditions: []metav1.Condition{
								{
									Type:   string(gwv1.RouteConditionAccepted),
									Status: metav1.ConditionTrue,
									Reason: string(gwv1.RouteConditionAccepted),
								},
							},
						},
					},
				},
			},
		}

		// Only translate our parentRef.
		r.Route(route).ParentRef(parentRef())

		status := rm.BuildRouteStatus(ctx, route, wellknown.DefaultAgwControllerName)

		assert.Equal(t, true, status != nil)
		// 1 parent is ours, 1 parent is other.
		assert.Equal(t, 2, len(status.Parents))
		// Ours will be first due to alphabetical ordering of controller name ('k' vs. 'o').
		assert.Equal(t, 2, len(status.Parents[0].Conditions))
	})

	t.Run("set negative route conditions from report and not add extra conditions", func(t *testing.T) {
		tests := []struct {
			name      string
			obj       client.Object
			parentRef *gwv1.ParentReference
		}{
			{name: "regular httproute", obj: httpRoute(), parentRef: parentRef()},
			{name: "regular tcproute", obj: tcpRoute(), parentRef: parentRef()},
			{name: "regular tlsroute", obj: tlsRoute(), parentRef: parentRef()},
			{name: "regular grpcroute", obj: grpcRoute(), parentRef: parentRef()},
			{name: "delegatee route", obj: delegateeRoute(), parentRef: parentRouteRef()},
		}

		for _, tt := range tests {
			t.Run(tt.name, func(t *testing.T) {
				rm := reports.NewReportMap()
				r := reports.NewReporter(&rm)
				r.Route(tt.obj).ParentRef(tt.parentRef).SetCondition(reporter.RouteCondition{
					Type:   gwv1.RouteConditionResolvedRefs,
					Status: metav1.ConditionFalse,
					Reason: gwv1.RouteReasonBackendNotFound,
				})

				status := rm.BuildRouteStatus(context.Background(), tt.obj, wellknown.DefaultAgwControllerName)

				assert.Equal(t, true, status != nil)
				assert.Equal(t, 1, len(status.Parents))
				assert.Equal(t, 2, len(status.Parents[0].Conditions))

				resolvedRefs := meta.FindStatusCondition(status.Parents[0].Conditions, string(gwv1.RouteConditionResolvedRefs))
				assert.Equal(t, true, resolvedRefs != nil)
				assert.Equal(t, metav1.ConditionFalse, resolvedRefs.Status)
			})
		}
	})

	t.Run("filter out multiple negative route conditions of the same type from report", func(t *testing.T) {
		tests := []struct {
			name      string
			obj       client.Object
			parentRef *gwv1.ParentReference
		}{
			{name: "regular httproute", obj: httpRoute(), parentRef: parentRef()},
			{name: "regular tcproute", obj: tcpRoute(), parentRef: parentRef()},
			{name: "regular tlsroute", obj: tlsRoute(), parentRef: parentRef()},
			{name: "regular grpcroute", obj: grpcRoute(), parentRef: parentRef()},
			{name: "delegatee route", obj: delegateeRoute(), parentRef: parentRouteRef()},
		}

		for _, tt := range tests {
			t.Run(tt.name, func(t *testing.T) {
				rm := reports.NewReportMap()
				r := reports.NewReporter(&rm)
				r.Route(tt.obj).ParentRef(tt.parentRef).SetCondition(reporter.RouteCondition{
					Type:   gwv1.RouteConditionResolvedRefs,
					Status: metav1.ConditionFalse,
					Reason: gwv1.RouteReasonBackendNotFound,
				})
				r.Route(tt.obj).ParentRef(tt.parentRef).SetCondition(reporter.RouteCondition{
					Type:   gwv1.RouteConditionResolvedRefs,
					Status: metav1.ConditionFalse,
					Reason: gwv1.RouteReasonBackendNotFound,
				})

				status := rm.BuildRouteStatus(context.Background(), tt.obj, wellknown.DefaultAgwControllerName)

				assert.Equal(t, true, status != nil)
				assert.Equal(t, 1, len(status.Parents))
				assert.Equal(t, 2, len(status.Parents[0].Conditions))

				resolvedRefs := meta.FindStatusCondition(status.Parents[0].Conditions, string(gwv1.RouteConditionResolvedRefs))
				assert.Equal(t, true, resolvedRefs != nil)
				assert.Equal(t, metav1.ConditionFalse, resolvedRefs.Status)
			})
		}
	})

	t.Run("do not modify LastTransitionTime for existing conditions that have not changed", func(t *testing.T) {
		tests := []struct {
			name string
			obj  client.Object
		}{
			{name: "regular httproute", obj: httpRoute()},
			{name: "regular tcproute", obj: tcpRoute()},
			{name: "regular tlsroute", obj: tlsRoute()},
			{name: "regular grpcroute", obj: grpcRoute()},
			{name: "delegatee route", obj: delegateeRoute()},
		}

		for _, tt := range tests {
			t.Run(tt.name, func(t *testing.T) {
				rm := reports.NewReportMap()

				r := reports.NewReporter(&rm)
				fakeTranslate(r, tt.obj)
				status := rm.BuildRouteStatus(context.Background(), tt.obj, wellknown.DefaultAgwControllerName)

				assert.Equal(t, true, status != nil)
				assert.Equal(t, 1, len(status.Parents))
				assert.Equal(t, 2, len(status.Parents[0].Conditions))

				resolvedRefs := meta.FindStatusCondition(status.Parents[0].Conditions, string(gwv1.RouteConditionResolvedRefs))
				assert.Equal(t, true, resolvedRefs != nil)
				oldTransitionTime := resolvedRefs.LastTransitionTime

				switch route := tt.obj.(type) {
				case *gwv1.HTTPRoute:
					route.Status.RouteStatus = *status
				case *gwv1a2.TCPRoute:
					route.Status.RouteStatus = *status
				case *gwv1a2.TLSRoute:
					route.Status.RouteStatus = *status
				case *gwv1.GRPCRoute:
					route.Status.RouteStatus = *status
				default:
					t.Fatalf("unsupported route type: %T", tt.obj)
				}

				status = rm.BuildRouteStatus(context.Background(), tt.obj, wellknown.DefaultAgwControllerName)

				assert.Equal(t, true, status != nil)
				assert.Equal(t, 1, len(status.Parents))
				assert.Equal(t, 2, len(status.Parents[0].Conditions))

				resolvedRefs = meta.FindStatusCondition(status.Parents[0].Conditions, string(gwv1.RouteConditionResolvedRefs))
				assert.Equal(t, true, resolvedRefs != nil)
				newTransitionTime := resolvedRefs.LastTransitionTime
				assert.Equal(t, oldTransitionTime, newTransitionTime)
			})
		}
	})

	t.Run("handle multiple ParentRefs on a route", func(t *testing.T) {
		tests := []struct {
			name string
			obj  client.Object
		}{
			{name: "regular HTTPRoute", obj: httpRoute()},
			{name: "regular TCPRoute", obj: tcpRoute()},
			{name: "regular tlsroute", obj: tlsRoute()},
			{name: "regular grpcroute", obj: grpcRoute()},
		}

		for _, tt := range tests {
			t.Run(tt.name, func(t *testing.T) {
				switch route := tt.obj.(type) {
				case *gwv1.HTTPRoute:
					route.Spec.ParentRefs = append(route.Spec.ParentRefs, gwv1.ParentReference{
						Name: "additional-gateway",
					})
				case *gwv1a2.TCPRoute:
					route.Spec.ParentRefs = append(route.Spec.ParentRefs, gwv1.ParentReference{
						Name: "additional-gateway",
					})
				case *gwv1a2.TLSRoute:
					route.Spec.ParentRefs = append(route.Spec.ParentRefs, gwv1.ParentReference{
						Name: "additional-gateway",
					})
				case *gwv1.GRPCRoute:
					route.Spec.ParentRefs = append(route.Spec.ParentRefs, gwv1.ParentReference{
						Name: "additional-gateway",
					})
				default:
					t.Fatalf("unsupported route type: %T", tt.obj)
				}

				rm := reports.NewReportMap()
				r := reports.NewReporter(&rm)
				fakeTranslate(r, tt.obj)

				status := rm.BuildRouteStatus(ctx, tt.obj, wellknown.DefaultAgwControllerName)

				assert.Equal(t, true, status != nil)
				assert.Equal(t, 2, len(status.Parents))
				for _, parent := range status.Parents {
					assert.Equal(t, 2, len(parent.Conditions))
				}
			})
		}
	})

	t.Run("associate multiple routes with shared and separate listeners", func(t *testing.T) {
		tests := []struct {
			name      string
			route1    client.Object
			route2    client.Object
			listener1 gwv1.Listener
			listener2 gwv1.Listener
		}{
			{
				name:      "HTTPRoutes with shared and separate listeners",
				route1:    httpRoute(),
				route2:    httpRoute(),
				listener1: gwv1.Listener{Name: "foo-http", Protocol: gwv1.HTTPProtocolType},
				listener2: gwv1.Listener{Name: "bar-http", Protocol: gwv1.HTTPProtocolType},
			},
			{
				name:      "TCPRoutes with shared and separate listeners",
				route1:    tcpRoute(),
				route2:    tcpRoute(),
				listener1: gwv1.Listener{Name: "foo-tcp", Protocol: gwv1.TCPProtocolType},
				listener2: gwv1.Listener{Name: "bar-tcp", Protocol: gwv1.TCPProtocolType},
			},
			{
				name:      "TLSRoutes with shared and separate listeners",
				route1:    tlsRoute(),
				route2:    tlsRoute(),
				listener1: gwv1.Listener{Name: "foo-tls", Protocol: gwv1.TLSProtocolType},
				listener2: gwv1.Listener{Name: "bar-tls", Protocol: gwv1.TLSProtocolType},
			},
			{
				name:      "GRPCRoutes with shared and separate listeners",
				route1:    grpcRoute(),
				route2:    grpcRoute(),
				listener1: gwv1.Listener{Name: "foo-grpc", Protocol: gwv1.HTTPProtocolType},
				listener2: gwv1.Listener{Name: "bar-grpc", Protocol: gwv1.HTTPProtocolType},
			},
		}

		for _, tt := range tests {
			t.Run(tt.name, func(t *testing.T) {
				gw := gw()
				gw.Spec.Listeners = []gwv1.Listener{tt.listener1, tt.listener2}

				switch r1 := tt.route1.(type) {
				case *gwv1.HTTPRoute:
					r1.Spec.ParentRefs[0].SectionName = ptr.To(gwv1.SectionName(tt.listener1.Name))
				case *gwv1a2.TCPRoute:
					r1.Spec.ParentRefs[0].SectionName = ptr.To(gwv1.SectionName(tt.listener1.Name))
				case *gwv1a2.TLSRoute:
					r1.Spec.ParentRefs[0].SectionName = ptr.To(gwv1.SectionName(tt.listener1.Name))
				case *gwv1.GRPCRoute:
					r1.Spec.ParentRefs[0].SectionName = ptr.To(gwv1.SectionName(tt.listener1.Name))
				}

				switch r2 := tt.route2.(type) {
				case *gwv1.HTTPRoute:
					r2.Spec.ParentRefs[0].SectionName = ptr.To(gwv1.SectionName(tt.listener2.Name))
				case *gwv1a2.TCPRoute:
					r2.Spec.ParentRefs[0].SectionName = ptr.To(gwv1.SectionName(tt.listener2.Name))
				case *gwv1a2.TLSRoute:
					r2.Spec.ParentRefs[0].SectionName = ptr.To(gwv1.SectionName(tt.listener2.Name))
				case *gwv1.GRPCRoute:
					r2.Spec.ParentRefs[0].SectionName = ptr.To(gwv1.SectionName(tt.listener2.Name))
				}

				rm := reports.NewReportMap()
				r := reports.NewReporter(&rm)

				fakeTranslate(r, tt.route1)
				fakeTranslate(r, tt.route2)

				status1 := rm.BuildRouteStatus(ctx, tt.route1, wellknown.DefaultAgwControllerName)
				status2 := rm.BuildRouteStatus(ctx, tt.route2, wellknown.DefaultAgwControllerName)

				assert.Equal(t, true, status1 != nil)
				assert.Equal(t, 2, len(status1.Parents[0].Conditions))
				assert.Equal(t, true, status2 != nil)
				assert.Equal(t, 2, len(status2.Parents[0].Conditions))
			})
		}
	})
}

func TestBuildRouteStatusWithMissingParentReferences(t *testing.T) {
	tests := []struct {
		name  string
		route client.Object
	}{
		{name: "HTTPRoute with missing parent reference", route: httpRoute()},
		{name: "TCPRoute with missing parent reference", route: tcpRoute()},
		{name: "TLSRoute with missing parent reference", route: tlsRoute()},
		{name: "GRPCRoute with missing parent reference", route: grpcRoute()},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			switch r := tt.route.(type) {
			case *gwv1.HTTPRoute:
				r.Spec.ParentRefs = nil
			case *gwv1a2.TCPRoute:
				r.Spec.ParentRefs = nil
			case *gwv1a2.TLSRoute:
				r.Spec.ParentRefs = nil
			case *gwv1.GRPCRoute:
				r.Spec.ParentRefs = nil
			}

			rm := reports.NewReportMap()
			r := reports.NewReporter(&rm)

			fakeTranslate(r, tt.route)
			status := rm.BuildRouteStatus(ctx, tt.route, wellknown.DefaultAgwControllerName)

			assert.Equal(t, true, status != nil)
			assert.Equal(t, 0, len(status.Parents))
		})
	}
}

func TestBuildRouteStatusClearsStaleStatusOnEmptyRouteReportEntry(t *testing.T) {
	tests := []struct {
		name  string
		route client.Object
	}{
		{
			name: "HTTPRoute with stale status",
			route: httpRoute(
				metav1.Condition{Type: fakeCondition},
			),
		},
		{
			name: "TCPRoute with stale status",
			route: tcpRoute(
				metav1.Condition{Type: fakeCondition},
			),
		},
		{
			name: "TLSRoute with stale status",
			route: tlsRoute(
				metav1.Condition{Type: fakeCondition},
			),
		},
		{
			name: "GRPCRoute with stale status",
			route: grpcRoute(
				metav1.Condition{Type: fakeCondition},
			),
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			rm := reports.NewReportMap()
			r := reports.NewReporter(&rm)

			// Create empty route entry in report map.
			r.Route(tt.route)

			status := rm.BuildRouteStatus(ctx, tt.route, wellknown.DefaultAgwControllerName)

			assert.Equal(t, true, status != nil)
			assert.Equal(t, 0, len(status.Parents))
		})
	}
}

// fakeTranslate mimics the translation loop and reports for the provided route
// along with all parentRefs defined in the route.
func fakeTranslate(reporter reporter.Reporter, obj client.Object) {
	switch route := obj.(type) {
	case *gwv1.HTTPRoute:
		routeReporter := reporter.Route(route)
		for _, pr := range route.Spec.ParentRefs {
			routeReporter.ParentRef(&pr)
		}
	case *gwv1a2.TCPRoute:
		routeReporter := reporter.Route(route)
		for _, pr := range route.Spec.ParentRefs {
			routeReporter.ParentRef(&pr)
		}
	case *gwv1a2.TLSRoute:
		routeReporter := reporter.Route(route)
		for _, pr := range route.Spec.ParentRefs {
			routeReporter.ParentRef(&pr)
		}
	case *gwv1.GRPCRoute:
		routeReporter := reporter.Route(route)
		for _, pr := range route.Spec.ParentRefs {
			routeReporter.ParentRef(&pr)
		}
	}
}

func httpRoute(conditions ...metav1.Condition) client.Object {
	route := &gwv1.HTTPRoute{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "route",
			Namespace: "default",
		},
	}
	route.Spec.CommonRouteSpec.ParentRefs = append(route.Spec.CommonRouteSpec.ParentRefs, *parentRef())
	if len(conditions) > 0 {
		route.Status.Parents = append(route.Status.Parents, gwv1.RouteParentStatus{
			ParentRef:      *parentRef(),
			Conditions:     conditions,
			ControllerName: wellknown.DefaultAgwControllerName,
		})
	}
	return route
}

func tcpRoute(conditions ...metav1.Condition) client.Object {
	route := &gwv1a2.TCPRoute{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "route",
			Namespace: "default",
		},
	}
	route.Spec.CommonRouteSpec.ParentRefs = append(route.Spec.CommonRouteSpec.ParentRefs, *parentRef())
	if len(conditions) > 0 {
		route.Status.Parents = append(route.Status.Parents, gwv1.RouteParentStatus{
			ParentRef:      *parentRef(),
			Conditions:     conditions,
			ControllerName: wellknown.DefaultAgwControllerName,
		})
	}
	return route
}

func tlsRoute(conditions ...metav1.Condition) client.Object {
	route := &gwv1a2.TLSRoute{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "route",
			Namespace: "default",
		},
	}
	route.Spec.CommonRouteSpec.ParentRefs = append(route.Spec.CommonRouteSpec.ParentRefs, *parentRef())
	if len(conditions) > 0 {
		route.Status.Parents = append(route.Status.Parents, gwv1.RouteParentStatus{
			ParentRef:      *parentRef(),
			Conditions:     conditions,
			ControllerName: wellknown.DefaultAgwControllerName,
		})
	}
	return route
}

func grpcRoute(conditions ...metav1.Condition) client.Object {
	route := &gwv1.GRPCRoute{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "route",
			Namespace: "default",
		},
	}
	route.Spec.CommonRouteSpec.ParentRefs = append(route.Spec.CommonRouteSpec.ParentRefs, *parentRef())
	if len(conditions) > 0 {
		route.Status.Parents = append(route.Status.Parents, gwv1.RouteParentStatus{
			ParentRef:      *parentRef(),
			Conditions:     conditions,
			ControllerName: wellknown.DefaultAgwControllerName,
		})
	}
	return route
}

func parentRef() *gwv1.ParentReference {
	return &gwv1.ParentReference{
		Name: "kgateway-gtw",
	}
}

func otherParentRef() *gwv1.ParentReference {
	return &gwv1.ParentReference{
		Name: "other-gtw",
	}
}

func delegateeRoute(conditions ...metav1.Condition) client.Object {
	route := &gwv1.HTTPRoute{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "child-route",
			Namespace: "default",
		},
	}
	route.Spec.CommonRouteSpec.ParentRefs = append(route.Spec.CommonRouteSpec.ParentRefs, *parentRouteRef())
	if len(conditions) > 0 {
		route.Status.Parents = append(route.Status.Parents, gwv1.RouteParentStatus{
			ParentRef:      *parentRouteRef(),
			Conditions:     conditions,
			ControllerName: wellknown.DefaultAgwControllerName,
		})
	}
	return route
}

func parentRouteRef() *gwv1.ParentReference {
	return &gwv1.ParentReference{
		Group:     ptr.To(gwv1.Group("gateway.networking.k8s.io")),
		Kind:      ptr.To(gwv1.Kind("HTTPRoute")),
		Name:      "parent-route",
		Namespace: ptr.To(gwv1.Namespace("default")),
	}
}

func gw() *gwv1.Gateway {
	g := &gwv1.Gateway{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: "default",
			Name:      "kgateway-gtw",
		},
	}
	g.Spec.Listeners = append(g.Spec.Listeners, *listener())
	return g
}

func listener() *gwv1.Listener {
	return &gwv1.Listener{
		Name: "http",
	}
}
