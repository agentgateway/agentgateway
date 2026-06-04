package handlers

import (
	"fmt"
	"strings"

	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"sigs.k8s.io/controller-runtime/pkg/client"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gwv1b1 "sigs.k8s.io/gateway-api/apis/v1beta1"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/resource"
)

func init() {
	resource.Register(&gatewayClassHandler{})
	resource.Register(&gatewayHandler{})
	resource.Register(&httpRouteHandler{})
	resource.Register(&referenceGrantHandler{})
}

// ─── GatewayClass ────────────────────────────────────────────────────────────

type gatewayClassHandler struct{}

func (h *gatewayClassHandler) Mapping() meta.RESTMapping {
	return resource.Mapping(resource.GVR("gateway.networking.k8s.io", "v1", "gatewayclasses"), "GatewayClass")
}

func (h *gatewayClassHandler) Aliases() []string {
	return []string{"gatewayclasses", "gatewayclass"}
}

func (h *gatewayClassHandler) Columns() []resource.Column {
	return []resource.Column{
		{Header: "NAME", Field: func(obj client.Object) string { return obj.GetName() }},
		{Header: "CONTROLLER", Field: func(obj client.Object) string {
			if gc, ok := obj.(*gwv1.GatewayClass); ok {
				return string(gc.Spec.ControllerName)
			}
			return ""
		}},
		{Header: "ACCEPTED", Field: func(obj client.Object) string {
			if gc, ok := obj.(*gwv1.GatewayClass); ok {
				return conditionStatus(gc.Status.Conditions, "Accepted")
			}
			return ""
		}},
		{Header: "AGE", Field: func(obj client.Object) string { return age(obj.GetCreationTimestamp()) }},
		{Header: "DESCRIPTION", Wide: true, Field: func(obj client.Object) string {
			if gc, ok := obj.(*gwv1.GatewayClass); ok && gc.Spec.Description != nil {
				return *gc.Spec.Description
			}
			return ""
		}},
	}
}

func (h *gatewayClassHandler) DescribeExtra(obj client.Object) ([]resource.Section, error) {
	gc, ok := obj.(*gwv1.GatewayClass)
	if !ok {
		return nil, nil
	}
	var sections []resource.Section

	sections = append(sections, resource.Section{
		Title: "Controller",
		Body:  string(gc.Spec.ControllerName),
	})

	if len(gc.Status.Conditions) > 0 {
		sections = append(sections, resource.Section{
			Title: "Conditions",
			Body:  formatConditions(gc.Status.Conditions),
		})
	}

	return sections, nil
}

// ─── Gateway ──────────────────────────────────────────────────────────────────

type gatewayHandler struct{}

func (h *gatewayHandler) Mapping() meta.RESTMapping {
	return resource.Mapping(resource.GVR("gateway.networking.k8s.io", "v1", "gateways"), "Gateway")
}

func (h *gatewayHandler) Aliases() []string {
	return []string{"gateways", "gateway"}
}

func (h *gatewayHandler) Columns() []resource.Column {
	return []resource.Column{
		{Header: "NAMESPACE", Field: func(obj client.Object) string { return obj.GetNamespace() }},
		{Header: "NAME", Field: func(obj client.Object) string { return obj.GetName() }},
		{Header: "CLASS", Field: func(obj client.Object) string {
			if gw, ok := obj.(*gwv1.Gateway); ok {
				return string(gw.Spec.GatewayClassName)
			}
			return ""
		}},
		{Header: "ADDRESS", Field: func(obj client.Object) string {
			if gw, ok := obj.(*gwv1.Gateway); ok {
				return gatewayAddress(gw)
			}
			return ""
		}},
		{Header: "PROGRAMMED", Field: func(obj client.Object) string {
			if gw, ok := obj.(*gwv1.Gateway); ok {
				return conditionStatus(gw.Status.Conditions, "Programmed")
			}
			return ""
		}},
		{Header: "AGE", Field: func(obj client.Object) string { return age(obj.GetCreationTimestamp()) }},
		{Header: "ROUTES", Wide: true, Field: func(obj client.Object) string {
			if gw, ok := obj.(*gwv1.Gateway); ok {
				return fmt.Sprintf("%d", gatewayRouteCount(gw))
			}
			return ""
		}},
	}
}

func (h *gatewayHandler) DescribeExtra(obj client.Object) ([]resource.Section, error) {
	gw, ok := obj.(*gwv1.Gateway)
	if !ok {
		return nil, nil
	}
	var sections []resource.Section

	sections = append(sections, resource.Section{
		Title: "GatewayClass",
		Body:  string(gw.Spec.GatewayClassName),
	})

	if len(gw.Spec.Listeners) > 0 {
		var b strings.Builder
		for _, l := range gw.Spec.Listeners {
			fmt.Fprintf(&b, "%s  port=%d  protocol=%s", l.Name, l.Port, l.Protocol)
			if l.Hostname != nil {
				fmt.Fprintf(&b, "  hostname=%s", *l.Hostname)
			}
			b.WriteByte('\n')
		}
		sections = append(sections, resource.Section{Title: "Listeners", Body: b.String()})
	}

	if len(gw.Status.Conditions) > 0 {
		sections = append(sections, resource.Section{
			Title: "Conditions",
			Body:  formatConditions(gw.Status.Conditions),
		})
	}

	return sections, nil
}

// ─── HTTPRoute ────────────────────────────────────────────────────────────────

type httpRouteHandler struct{}

func (h *httpRouteHandler) Mapping() meta.RESTMapping {
	return resource.Mapping(resource.GVR("gateway.networking.k8s.io", "v1", "httproutes"), "HTTPRoute")
}

func (h *httpRouteHandler) Aliases() []string {
	return []string{"httproutes", "httproute"}
}

func (h *httpRouteHandler) Columns() []resource.Column {
	return []resource.Column{
		{Header: "NAMESPACE", Field: func(obj client.Object) string { return obj.GetNamespace() }},
		{Header: "NAME", Field: func(obj client.Object) string { return obj.GetName() }},
		{Header: "HOSTNAMES", Field: func(obj client.Object) string {
			if r, ok := obj.(*gwv1.HTTPRoute); ok {
				return httpRouteHostnames(r)
			}
			return ""
		}},
		{Header: "PARENT", Field: func(obj client.Object) string {
			if r, ok := obj.(*gwv1.HTTPRoute); ok {
				return httpRouteParents(r)
			}
			return ""
		}},
		{Header: "AGE", Field: func(obj client.Object) string { return age(obj.GetCreationTimestamp()) }},
	}
}

func (h *httpRouteHandler) DescribeExtra(obj client.Object) ([]resource.Section, error) {
	r, ok := obj.(*gwv1.HTTPRoute)
	if !ok {
		return nil, nil
	}
	var sections []resource.Section

	if len(r.Spec.Hostnames) > 0 {
		var hostnames []string
		for _, h := range r.Spec.Hostnames {
			hostnames = append(hostnames, string(h))
		}
		sections = append(sections, resource.Section{Title: "Hostnames", Body: strings.Join(hostnames, "\n")})
	}

	if len(r.Spec.ParentRefs) > 0 {
		var b strings.Builder
		for _, p := range r.Spec.ParentRefs {
			ns := ""
			if p.Namespace != nil {
				ns = string(*p.Namespace) + "/"
			}
			fmt.Fprintf(&b, "%s%s\n", ns, p.Name)
		}
		sections = append(sections, resource.Section{Title: "Parent References", Body: b.String()})
	}

	if len(r.Status.RouteStatus.Parents) > 0 {
		var b strings.Builder
		for _, ps := range r.Status.RouteStatus.Parents {
			fmt.Fprintf(&b, "%s: %s\n", ps.ParentRef.Name, conditionStatus(ps.Conditions, "Accepted"))
		}
		sections = append(sections, resource.Section{Title: "Parent Status", Body: b.String()})
	}

	if len(r.Spec.Rules) > 0 {
		var b strings.Builder
		for i, rule := range r.Spec.Rules {
			fmt.Fprintf(&b, "Rule %d:\n", i+1)
			for _, m := range rule.Matches {
				if m.Path != nil && m.Path.Value != nil {
					fmt.Fprintf(&b, "  path: %s\n", *m.Path.Value)
				}
			}
			for _, be := range rule.BackendRefs {
				portStr := ""
				if be.Port != nil {
					portStr = fmt.Sprintf(":%d", *be.Port)
				}
				fmt.Fprintf(&b, "  backend: %s%s\n", be.Name, portStr)
			}
		}
		sections = append(sections, resource.Section{Title: "Rules", Body: b.String()})
	}

	return sections, nil
}

// ─── ReferenceGrant ──────────────────────────────────────────────────────────

type referenceGrantHandler struct{}

func (h *referenceGrantHandler) Mapping() meta.RESTMapping {
	return resource.Mapping(resource.GVR("gateway.networking.k8s.io", "v1beta1", "referencegrants"), "ReferenceGrant")
}

func (h *referenceGrantHandler) Aliases() []string {
	return []string{"referencegrants", "referencegrant"}
}

func (h *referenceGrantHandler) Columns() []resource.Column {
	return []resource.Column{
		{Header: "NAMESPACE", Field: func(obj client.Object) string { return obj.GetNamespace() }},
		{Header: "NAME", Field: func(obj client.Object) string { return obj.GetName() }},
		{Header: "FROM", Field: func(obj client.Object) string {
			if rg, ok := obj.(*gwv1b1.ReferenceGrant); ok {
				return referenceGrantFrom(rg)
			}
			return ""
		}},
		{Header: "TO", Field: func(obj client.Object) string {
			if rg, ok := obj.(*gwv1b1.ReferenceGrant); ok {
				return referenceGrantTo(rg)
			}
			return ""
		}},
		{Header: "AGE", Field: func(obj client.Object) string { return age(obj.GetCreationTimestamp()) }},
	}
}

func (h *referenceGrantHandler) DescribeExtra(obj client.Object) ([]resource.Section, error) {
	rg, ok := obj.(*gwv1b1.ReferenceGrant)
	if !ok {
		return nil, nil
	}
	var sections []resource.Section

	var from strings.Builder
	for _, f := range rg.Spec.From {
		fmt.Fprintf(&from, "group=%s kind=%s namespace=%s\n", f.Group, f.Kind, f.Namespace)
	}
	sections = append(sections, resource.Section{Title: "From", Body: from.String()})

	var to strings.Builder
	for _, t := range rg.Spec.To {
		fmt.Fprintf(&to, "group=%s kind=%s", t.Group, t.Kind)
		if t.Name != nil {
			fmt.Fprintf(&to, " name=%s", *t.Name)
		}
		to.WriteByte('\n')
	}
	sections = append(sections, resource.Section{Title: "To", Body: to.String()})

	return sections, nil
}

// ─── helpers ─────────────────────────────────────────────────────────────────

func conditionStatus(conditions []metav1.Condition, condType string) string {
	for _, c := range conditions {
		if c.Type == condType {
			return string(c.Status)
		}
	}
	return "Unknown"
}

func age(t metav1.Time) string {
	if t.IsZero() {
		return "<unknown>"
	}
	d := metav1.Now().Sub(t.Time)
	switch {
	case d.Hours() >= 24*365:
		return fmt.Sprintf("%dy", int(d.Hours()/(24*365)))
	case d.Hours() >= 24:
		return fmt.Sprintf("%dd", int(d.Hours()/24))
	case d.Hours() >= 1:
		return fmt.Sprintf("%dh", int(d.Hours()))
	case d.Minutes() >= 1:
		return fmt.Sprintf("%dm", int(d.Minutes()))
	default:
		return fmt.Sprintf("%ds", int(d.Seconds()))
	}
}

func formatConditions(conditions []metav1.Condition) string {
	var b strings.Builder
	for _, c := range conditions {
		fmt.Fprintf(&b, "%-20s %-8s %s\n", c.Type, c.Status, c.Message)
	}
	return b.String()
}

func gatewayAddress(gw *gwv1.Gateway) string {
	var addrs []string
	for _, a := range gw.Status.Addresses {
		addrs = append(addrs, a.Value)
	}
	if len(addrs) == 0 {
		return "<none>"
	}
	return strings.Join(addrs, ",")
}

func gatewayRouteCount(gw *gwv1.Gateway) int {
	count := 0
	for _, l := range gw.Status.Listeners {
		count += int(l.AttachedRoutes)
	}
	return count
}

func httpRouteHostnames(r *gwv1.HTTPRoute) string {
	if len(r.Spec.Hostnames) == 0 {
		return "*"
	}
	var h []string
	for _, hn := range r.Spec.Hostnames {
		h = append(h, string(hn))
	}
	return strings.Join(h, ",")
}

func httpRouteParents(r *gwv1.HTTPRoute) string {
	var parents []string
	for _, p := range r.Spec.ParentRefs {
		ns := r.Namespace
		if p.Namespace != nil {
			ns = string(*p.Namespace)
		}
		parents = append(parents, ns+"/"+string(p.Name))
	}
	return strings.Join(parents, ",")
}

func referenceGrantFrom(rg *gwv1b1.ReferenceGrant) string {
	var parts []string
	for _, f := range rg.Spec.From {
		parts = append(parts, string(f.Kind)+"/"+string(f.Namespace))
	}
	return strings.Join(parts, ",")
}

func referenceGrantTo(rg *gwv1b1.ReferenceGrant) string {
	var parts []string
	for _, t := range rg.Spec.To {
		name := "*"
		if t.Name != nil {
			name = string(*t.Name)
		}
		parts = append(parts, string(t.Kind)+"/"+name)
	}
	return strings.Join(parts, ",")
}
