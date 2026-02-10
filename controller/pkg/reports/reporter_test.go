package reports_test

import (
	"context"
	"fmt"

	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/utils/ptr"
	"sigs.k8s.io/controller-runtime/pkg/client"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gwv1a2 "sigs.k8s.io/gateway-api/apis/v1alpha2"

	. "github.com/onsi/ginkgo/v2"
	. "github.com/onsi/gomega"

	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/wellknown"
	"github.com/kgateway-dev/kgateway/v2/pkg/pluginsdk/reporter"
	"github.com/kgateway-dev/kgateway/v2/pkg/reports"
)

const fake_condition = "kgateway.dev/SomeCondition"

var ctx = context.Background()

var _ = Describe("Reporting Infrastructure", func() {
	BeforeEach(func() {
	})

	Describe("building gateway status", func() {
		It("should build all positive conditions with an empty report", func() {
			gw := gw()
			rm := reports.NewReportMap()

			reporter := reports.NewReporter(&rm)
			// initialize GatewayReporter to mimic translation loop (i.e. report gets initialized for all GWs)
			reporter.Gateway(gw)

			status := rm.BuildGWStatus(context.Background(), *gw, nil)

			Expect(status).NotTo(BeNil())
			Expect(status.Conditions).To(HaveLen(2))
			Expect(status.Listeners).To(HaveLen(1))
			Expect(status.Listeners[0].Conditions).To(HaveLen(4))
		})

		It("should preserve conditions set externally", func() {
			gw := gw()
			gw.Status.Conditions = append(gw.Status.Conditions, metav1.Condition{
				Type:   "gateway.kgateway.dev/SomeCondition",
				Status: metav1.ConditionFalse,
			})
			rm := reports.NewReportMap()

			reporter := reports.NewReporter(&rm)
			// initialize GatewayReporter to mimic translation loop (i.e. report gets initialized for all GWs)
			reporter.Gateway(gw)

			status := rm.BuildGWStatus(context.Background(), *gw, nil)

			Expect(status).NotTo(BeNil())
			Expect(status.Conditions).To(HaveLen(3)) // 2 from the report, 1 from the original status
			Expect(status.Listeners).To(HaveLen(1))
			Expect(status.Listeners[0].Conditions).To(HaveLen(4))
		})

		It("should correctly set negative gateway conditions from report and not add extra conditions", func() {
			gw := gw()
			rm := reports.NewReportMap()
			r := reports.NewReporter(&rm)
			r.Gateway(gw).SetCondition(reporter.GatewayCondition{
				Type:   gwv1.GatewayConditionProgrammed,
				Status: metav1.ConditionFalse,
				Reason: gwv1.GatewayReasonAddressNotUsable,
			})
			status := rm.BuildGWStatus(context.Background(), *gw, nil)

			Expect(status).NotTo(BeNil())
			Expect(status.Conditions).To(HaveLen(2))
			Expect(status.Listeners).To(HaveLen(1))
			Expect(status.Listeners[0].Conditions).To(HaveLen(4))

			programmed := meta.FindStatusCondition(status.Conditions, string(gwv1.GatewayConditionProgrammed))
			Expect(programmed.Status).To(Equal(metav1.ConditionFalse))
		})

		It("should correctly set negative listener conditions from report and not add extra conditions", func() {
			gw := gw()
			rm := reports.NewReportMap()
			r := reports.NewReporter(&rm)
			r.Gateway(gw).Listener(listener()).SetCondition(reporter.ListenerCondition{
				Type:   gwv1.ListenerConditionResolvedRefs,
				Status: metav1.ConditionFalse,
				Reason: gwv1.ListenerReasonInvalidRouteKinds,
			})
			status := rm.BuildGWStatus(context.Background(), *gw, nil)

			Expect(status).NotTo(BeNil())
			Expect(status.Conditions).To(HaveLen(2))
			Expect(status.Listeners).To(HaveLen(1))
			Expect(status.Listeners[0].Conditions).To(HaveLen(4))

			resolvedRefs := meta.FindStatusCondition(status.Listeners[0].Conditions, string(gwv1.ListenerConditionResolvedRefs))
			Expect(resolvedRefs.Status).To(Equal(metav1.ConditionFalse))
		})

		It("should not modify LastTransitionTime for existing conditions that have not changed", func() {
			gw := gw()
			rm := reports.NewReportMap()

			reporter := reports.NewReporter(&rm)
			// initialize GatewayReporter to mimic translation loop (i.e. report gets initialized for all GWs)
			reporter.Gateway(gw)

			status := rm.BuildGWStatus(context.Background(), *gw, nil)

			Expect(status).NotTo(BeNil())
			Expect(status.Conditions).To(HaveLen(2))
			Expect(status.Listeners).To(HaveLen(1))
			Expect(status.Listeners[0].Conditions).To(HaveLen(4))

			acceptedCond := meta.FindStatusCondition(status.Listeners[0].Conditions, string(gwv1.ListenerConditionAccepted))
			oldTransitionTime := acceptedCond.LastTransitionTime

			gw.Status = *status
			status = rm.BuildGWStatus(context.Background(), *gw, nil)

			Expect(status).NotTo(BeNil())
			Expect(status.Conditions).To(HaveLen(2))
			Expect(status.Listeners).To(HaveLen(1))
			Expect(status.Listeners[0].Conditions).To(HaveLen(4))

			acceptedCond = meta.FindStatusCondition(status.Listeners[0].Conditions, string(gwv1.ListenerConditionAccepted))
			newTransitionTime := acceptedCond.LastTransitionTime
			Expect(newTransitionTime).To(Equal(oldTransitionTime))
		})

		// TODO(Law): add multiple gws/listener tests
		// TODO(Law): add test confirming transitionTime change when status change
	})

	Describe("building route status", func() {
		DescribeTable("should build all positive route conditions with an empty report",
			func(obj client.Object) {
				rm := reports.NewReportMap()

				reporter := reports.NewReporter(&rm)
				fakeTranslate(reporter, obj)
				status := rm.BuildRouteStatus(ctx, obj, wellknown.DefaultAgwControllerName)

				Expect(status).NotTo(BeNil())
				Expect(status.Parents).To(HaveLen(1))
				Expect(status.Parents[0].Conditions).To(HaveLen(2))
			},
			Entry("regular httproute", httpRoute()),
			Entry("regular tcproute", tcpRoute()),
			Entry("regular tlsroute", tlsRoute()),
			Entry("regular grpcroute", grpcRoute()),
			Entry("delegatee route", delegateeRoute()),
		)

		DescribeTable("should preserve conditions set externally",
			func(obj client.Object) {
				rm := reports.NewReportMap()

				reporter := reports.NewReporter(&rm)
				fakeTranslate(reporter, obj)
				status := rm.BuildRouteStatus(ctx, obj, wellknown.DefaultAgwControllerName)

				Expect(status).NotTo(BeNil())
				Expect(status.Parents).To(HaveLen(1))
				Expect(status.Parents[0].Conditions).To(HaveLen(3)) // 2 from the report, 1 from the original status
			},
			Entry("regular httproute", httpRoute(
				metav1.Condition{
					Type: fake_condition,
				},
			)),
			Entry("regular tcproute", tcpRoute(
				metav1.Condition{
					Type: fake_condition,
				},
			)),
			Entry("regular tlsroute", tlsRoute(
				metav1.Condition{
					Type: fake_condition,
				},
			)),
			Entry("regular grpcroute", grpcRoute(
				metav1.Condition{
					Type: fake_condition,
				},
			)),
			Entry("delegatee route", delegateeRoute(
				metav1.Condition{
					Type: fake_condition,
				},
			)),
		)

		DescribeTable("should not report for parentRefs that belong to other controllers",
			func(obj client.Object) {
				rm := reports.NewReportMap()

				reporter := reports.NewReporter(&rm)

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

				// we only translate our parentRef
				reporter.Route(obj).ParentRef(parentRef())

				status := rm.BuildRouteStatus(ctx, route, wellknown.DefaultAgwControllerName)

				Expect(status).NotTo(BeNil())
				// 1 parent is ours, 1 parent is other
				Expect(status.Parents).To(HaveLen(2))
				// 2 default positive conditions for the single parentRef we "translated"
				// ours will be first due to alphabetical ordering of controller name ('k' vs. 'o')
				Expect(status.Parents[0].Conditions).To(HaveLen(2))
			},
			Entry("httproute", &gwv1.HTTPRoute{
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
			}),
		)

		DescribeTable("should correctly set negative route conditions from report and not add extra conditions",
			func(obj client.Object, parentRef *gwv1.ParentReference) {
				rm := reports.NewReportMap()
				r := reports.NewReporter(&rm)
				r.Route(obj).ParentRef(parentRef).SetCondition(reporter.RouteCondition{
					Type:   gwv1.RouteConditionResolvedRefs,
					Status: metav1.ConditionFalse,
					Reason: gwv1.RouteReasonBackendNotFound,
				})

				status := rm.BuildRouteStatus(context.Background(), obj, wellknown.DefaultAgwControllerName)

				Expect(status).NotTo(BeNil())
				Expect(status.Parents).To(HaveLen(1))
				Expect(status.Parents[0].Conditions).To(HaveLen(2))

				resolvedRefs := meta.FindStatusCondition(status.Parents[0].Conditions, string(gwv1.RouteConditionResolvedRefs))
				Expect(resolvedRefs.Status).To(Equal(metav1.ConditionFalse))
			},
			Entry("regular httproute", httpRoute(), parentRef()),
			Entry("regular tcproute", tcpRoute(), parentRef()),
			Entry("regular tlsroute", tlsRoute(), parentRef()),
			Entry("regular grpcroute", grpcRoute(), parentRef()),
			Entry("delegatee route", delegateeRoute(), parentRouteRef()),
		)

		DescribeTable("should filter out multiple negative route conditions of the same type from report",
			func(obj client.Object, parentRef *gwv1.ParentReference) {
				rm := reports.NewReportMap()
				r := reports.NewReporter(&rm)
				r.Route(obj).ParentRef(parentRef).SetCondition(reporter.RouteCondition{
					Type:   gwv1.RouteConditionResolvedRefs,
					Status: metav1.ConditionFalse,
					Reason: gwv1.RouteReasonBackendNotFound,
				})
				r.Route(obj).ParentRef(parentRef).SetCondition(reporter.RouteCondition{
					Type:   gwv1.RouteConditionResolvedRefs,
					Status: metav1.ConditionFalse,
					Reason: gwv1.RouteReasonBackendNotFound,
				})

				status := rm.BuildRouteStatus(context.Background(), obj, wellknown.DefaultAgwControllerName)

				Expect(status).NotTo(BeNil())
				Expect(status.Parents).To(HaveLen(1))
				Expect(status.Parents[0].Conditions).To(HaveLen(2))

				resolvedRefs := meta.FindStatusCondition(status.Parents[0].Conditions, string(gwv1.RouteConditionResolvedRefs))
				Expect(resolvedRefs.Status).To(Equal(metav1.ConditionFalse))
			},
			Entry("regular httproute", httpRoute(), parentRef()),
			Entry("regular tcproute", tcpRoute(), parentRef()),
			Entry("regular tlsroute", tlsRoute(), parentRef()),
			Entry("regular grpcroute", grpcRoute(), parentRef()),
			Entry("delegatee route", delegateeRoute(), parentRouteRef()),
		)

		DescribeTable("should not modify LastTransitionTime for existing conditions that have not changed",
			func(obj client.Object) {
				rm := reports.NewReportMap()

				reporter := reports.NewReporter(&rm)
				fakeTranslate(reporter, obj)
				status := rm.BuildRouteStatus(context.Background(), obj, wellknown.DefaultAgwControllerName)

				Expect(status).NotTo(BeNil())
				Expect(status.Parents).To(HaveLen(1))
				Expect(status.Parents[0].Conditions).To(HaveLen(2))

				resolvedRefs := meta.FindStatusCondition(status.Parents[0].Conditions, string(gwv1.RouteConditionResolvedRefs))
				oldTransitionTime := resolvedRefs.LastTransitionTime

				// Type assert the object to update the Status field based on its type
				switch route := obj.(type) {
				case *gwv1.HTTPRoute:
					route.Status.RouteStatus = *status
				case *gwv1a2.TCPRoute:
					route.Status.RouteStatus = *status
				case *gwv1a2.TLSRoute:
					route.Status.RouteStatus = *status
				case *gwv1.GRPCRoute:
					route.Status.RouteStatus = *status
				default:
					Fail(fmt.Sprintf("unsupported route type: %T", obj))
				}

				status = rm.BuildRouteStatus(context.Background(), obj, wellknown.DefaultAgwControllerName)

				Expect(status).NotTo(BeNil())
				Expect(status.Parents).To(HaveLen(1))
				Expect(status.Parents[0].Conditions).To(HaveLen(2))

				resolvedRefs = meta.FindStatusCondition(status.Parents[0].Conditions, string(gwv1.RouteConditionResolvedRefs))
				newTransitionTime := resolvedRefs.LastTransitionTime
				Expect(newTransitionTime).To(Equal(oldTransitionTime))
			},
			Entry("regular httproute", httpRoute()),
			Entry("regular tcproute", tcpRoute()),
			Entry("regular tlsroute", tlsRoute()),
			Entry("regular grpcroute", grpcRoute()),
			Entry("delegatee route", delegateeRoute()),
		)

		DescribeTable("should correctly handle multiple ParentRefs on a route",
			func(obj client.Object) {
				// Add an additional ParentRef to test multiple parent references handling
				switch route := obj.(type) {
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
					Fail(fmt.Sprintf("unsupported route type: %T", obj))
				}

				rm := reports.NewReportMap()
				reporter := reports.NewReporter(&rm)

				fakeTranslate(reporter, obj)

				status := rm.BuildRouteStatus(ctx, obj, wellknown.DefaultAgwControllerName)

				Expect(status).NotTo(BeNil())
				Expect(status.Parents).To(HaveLen(2))

				// Check that each parent has the correct number of conditions
				for _, parent := range status.Parents {
					Expect(parent.Conditions).To(HaveLen(2))
				}
			},
			Entry("regular HTTPRoute", httpRoute()),
			Entry("regular TCPRoute", tcpRoute()),
			Entry("regular tlsroute", tlsRoute()),
			Entry("regular grpcroute", grpcRoute()),
		)

		DescribeTable("should correctly associate multiple routes with shared and separate listeners",
			func(route1, route2 client.Object, listener1, listener2 gwv1.Listener) {
				gw := gw()
				gw.Spec.Listeners = []gwv1.Listener{listener1, listener2}

				// Assign the first listener to the first route's parent ref
				switch r1 := route1.(type) {
				case *gwv1.HTTPRoute:
					r1.Spec.ParentRefs[0].SectionName = ptr.To(gwv1.SectionName(listener1.Name))
				case *gwv1a2.TCPRoute:
					r1.Spec.ParentRefs[0].SectionName = ptr.To(gwv1.SectionName(listener1.Name))
				case *gwv1a2.TLSRoute:
					r1.Spec.ParentRefs[0].SectionName = ptr.To(gwv1.SectionName(listener1.Name))
				case *gwv1.GRPCRoute:
					r1.Spec.ParentRefs[0].SectionName = ptr.To(gwv1.SectionName(listener1.Name))
				}

				// Assign the second listener to the second route's parent ref
				switch r2 := route2.(type) {
				case *gwv1.HTTPRoute:
					r2.Spec.ParentRefs[0].SectionName = ptr.To(gwv1.SectionName(listener2.Name))
				case *gwv1a2.TCPRoute:
					r2.Spec.ParentRefs[0].SectionName = ptr.To(gwv1.SectionName(listener2.Name))
				case *gwv1a2.TLSRoute:
					r2.Spec.ParentRefs[0].SectionName = ptr.To(gwv1.SectionName(listener2.Name))
				case *gwv1.GRPCRoute:
					r2.Spec.ParentRefs[0].SectionName = ptr.To(gwv1.SectionName(listener2.Name))
				}

				rm := reports.NewReportMap()
				reporter := reports.NewReporter(&rm)

				fakeTranslate(reporter, route1)
				fakeTranslate(reporter, route2)

				status1 := rm.BuildRouteStatus(ctx, route1, wellknown.DefaultAgwControllerName)
				status2 := rm.BuildRouteStatus(ctx, route2, wellknown.DefaultAgwControllerName)

				Expect(status1).NotTo(BeNil())
				Expect(status1.Parents[0].Conditions).To(HaveLen(2))

				Expect(status2).NotTo(BeNil())
				Expect(status2.Parents[0].Conditions).To(HaveLen(2))
			},
			Entry("HTTPRoutes with shared and separate listeners",
				httpRoute(), httpRoute(),
				gwv1.Listener{Name: "foo-http", Protocol: gwv1.HTTPProtocolType},
				gwv1.Listener{Name: "bar-http", Protocol: gwv1.HTTPProtocolType},
			),
			Entry("TCPRoutes with shared and separate listeners",
				tcpRoute(), tcpRoute(),
				gwv1.Listener{Name: "foo-tcp", Protocol: gwv1.TCPProtocolType},
				gwv1.Listener{Name: "bar-tcp", Protocol: gwv1.TCPProtocolType},
			),
			Entry("TLSRoutes with shared and separate listeners",
				tlsRoute(), tlsRoute(),
				gwv1.Listener{Name: "foo-tls", Protocol: gwv1.TLSProtocolType},
				gwv1.Listener{Name: "bar-tls", Protocol: gwv1.TLSProtocolType},
			),
			Entry("GRPCRoutes with shared and separate listeners",
				grpcRoute(), grpcRoute(),
				gwv1.Listener{Name: "foo-grpc", Protocol: gwv1.HTTPProtocolType},
				gwv1.Listener{Name: "bar-grpc", Protocol: gwv1.HTTPProtocolType},
			),
		)
	})

	DescribeTable("should handle routes with missing parent references gracefully",
		func(route client.Object) {
			// Remove ParentRefs from the route.
			switch r := route.(type) {
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
			reporter := reports.NewReporter(&rm)

			fakeTranslate(reporter, route)
			status := rm.BuildRouteStatus(ctx, route, wellknown.DefaultAgwControllerName)

			Expect(status).NotTo(BeNil())
			Expect(status.Parents).To(BeEmpty())
		},
		Entry("HTTPRoute with missing parent reference", httpRoute()),
		Entry("TCPRoute with missing parent reference", tcpRoute()),
		Entry("TLSRoute with missing parent reference", tlsRoute()),
		Entry("GRPCRoute with missing parent reference", grpcRoute()),
	)

	DescribeTable("should clear any stale status on empty route report entry",
		func(route client.Object) {
			rm := reports.NewReportMap()
			reporter := reports.NewReporter(&rm)

			// create empty route entry in report map
			reporter.Route(route)

			status := rm.BuildRouteStatus(ctx, route, wellknown.DefaultAgwControllerName)

			Expect(status).NotTo(BeNil())
			Expect(status.Parents).To(BeEmpty())
		},
		Entry("HTTPRoute with stale status", httpRoute(
			metav1.Condition{Type: fake_condition},
		)),
		Entry("TCPRoute with stale status", tcpRoute(
			metav1.Condition{Type: fake_condition},
		)),
		Entry("TLSRoute with stale status", tlsRoute(
			metav1.Condition{Type: fake_condition},
		)),
		Entry("GRPCRoute with stale status", grpcRoute(
			metav1.Condition{Type: fake_condition},
		)),
	)
})

// fakeTranslate mimics the translation loop and reports for the provided route
// along with all parentRefs defined in the route
func fakeTranslate(reporter reporter.Reporter, obj client.Object) {
	// translation will call Route() and ParentRef() for routes it translates out
	// we use the same pattern here to establish reports that would reflect translation
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
	gw := &gwv1.Gateway{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: "default",
			Name:      "kgateway-gtw",
		},
	}
	gw.Spec.Listeners = append(gw.Spec.Listeners, *listener())
	return gw
}

func listener() *gwv1.Listener {
	return &gwv1.Listener{
		Name: "http",
	}
}
