package handlers

import (
	"fmt"
	"strings"

	"k8s.io/apimachinery/pkg/api/meta"
	"sigs.k8s.io/controller-runtime/pkg/client"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	agwv1a1 "github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/cli/resource"
)

func init() {
	resource.Register(&agentgatewayPolicyHandler{})
	resource.Register(&agentgatewayParametersHandler{})
	resource.Register(&agentgatewayBackendHandler{})
}

// ─── AgentgatewayPolicy ──────────────────────────────────────────────────────

type agentgatewayPolicyHandler struct{}

func (h *agentgatewayPolicyHandler) Mapping() meta.RESTMapping {
	return resource.Mapping(resource.GVR("agentgateway.dev", "v1alpha1", "agentgatewaypolicies"), "AgentgatewayPolicy")
}

func (h *agentgatewayPolicyHandler) Aliases() []string {
	return []string{"agentgatewaypolicies", "agentgatewaypolicy", "agpol"}
}

func (h *agentgatewayPolicyHandler) Columns() []resource.Column {
	return []resource.Column{
		{Header: "NAMESPACE", Field: func(obj client.Object) string { return obj.GetNamespace() }},
		{Header: "NAME", Field: func(obj client.Object) string { return obj.GetName() }},
		{Header: "ACCEPTED", Field: func(obj client.Object) string {
			if p, ok := obj.(*agwv1a1.AgentgatewayPolicy); ok {
				return policyAccepted(p.Status)
			}
			return ""
		}},
		{Header: "ATTACHED", Field: func(obj client.Object) string {
			if p, ok := obj.(*agwv1a1.AgentgatewayPolicy); ok {
				return policyAttached(p.Status)
			}
			return ""
		}},
		{Header: "AGE", Field: func(obj client.Object) string { return age(obj.GetCreationTimestamp()) }},
		{Header: "TARGET", Wide: true, Field: func(obj client.Object) string {
			if p, ok := obj.(*agwv1a1.AgentgatewayPolicy); ok {
				return policyTargets(p)
			}
			return ""
		}},
	}
}

func (h *agentgatewayPolicyHandler) DescribeExtra(obj client.Object) ([]resource.Section, error) {
	p, ok := obj.(*agwv1a1.AgentgatewayPolicy)
	if !ok {
		return nil, nil
	}
	var sections []resource.Section

	if len(p.Spec.TargetRefs) > 0 {
		var b strings.Builder
		for _, t := range p.Spec.TargetRefs {
			fmt.Fprintf(&b, "group=%s kind=%s name=%s\n", t.Group, t.Kind, t.Name)
		}
		sections = append(sections, resource.Section{Title: "Target References", Body: b.String()})
	}

	if len(p.Status.Ancestors) > 0 {
		var b strings.Builder
		for _, a := range p.Status.Ancestors {
			kind := "Gateway"
			if a.AncestorRef.Kind != nil {
				kind = string(*a.AncestorRef.Kind)
			}
			fmt.Fprintf(&b, "%s/%s:\n", kind, a.AncestorRef.Name)
			for _, c := range a.Conditions {
				fmt.Fprintf(&b, "  %-20s %s  %s\n", c.Type, c.Status, c.Message)
			}
		}
		sections = append(sections, resource.Section{Title: "Status", Body: b.String()})
	}

	return sections, nil
}

// ─── AgentgatewayParameters ──────────────────────────────────────────────────

type agentgatewayParametersHandler struct{}

func (h *agentgatewayParametersHandler) Mapping() meta.RESTMapping {
	return resource.Mapping(resource.GVR("agentgateway.dev", "v1alpha1", "agentgatewayparameters"), "AgentgatewayParameters")
}

func (h *agentgatewayParametersHandler) Aliases() []string {
	return []string{"agentgatewayparameters", "agentgatewayparameter", "agpar"}
}

func (h *agentgatewayParametersHandler) Columns() []resource.Column {
	return []resource.Column{
		{Header: "NAMESPACE", Field: func(obj client.Object) string { return obj.GetNamespace() }},
		{Header: "NAME", Field: func(obj client.Object) string { return obj.GetName() }},
		{Header: "AGE", Field: func(obj client.Object) string { return age(obj.GetCreationTimestamp()) }},
	}
}

func (h *agentgatewayParametersHandler) DescribeExtra(obj client.Object) ([]resource.Section, error) {
	_, ok := obj.(*agwv1a1.AgentgatewayParameters)
	if !ok {
		return nil, nil
	}
	return []resource.Section{
		{Title: "Spec", Body: "Use -o yaml for full parameters spec"},
	}, nil
}

// ─── AgentgatewayBackend ─────────────────────────────────────────────────────

type agentgatewayBackendHandler struct{}

func (h *agentgatewayBackendHandler) Mapping() meta.RESTMapping {
	return resource.Mapping(resource.GVR("agentgateway.dev", "v1alpha1", "agentgatewaybackends"), "AgentgatewayBackend")
}

func (h *agentgatewayBackendHandler) Aliases() []string {
	return []string{"agentgatewaybackends", "agentgatewaybackend", "agbe"}
}

func (h *agentgatewayBackendHandler) Columns() []resource.Column {
	return []resource.Column{
		{Header: "NAMESPACE", Field: func(obj client.Object) string { return obj.GetNamespace() }},
		{Header: "NAME", Field: func(obj client.Object) string { return obj.GetName() }},
		{Header: "TYPE", Field: func(obj client.Object) string {
			if b, ok := obj.(*agwv1a1.AgentgatewayBackend); ok {
				return backendType(b)
			}
			return ""
		}},
		{Header: "ACCEPTED", Field: func(obj client.Object) string {
			if b, ok := obj.(*agwv1a1.AgentgatewayBackend); ok {
				return backendAccepted(b)
			}
			return ""
		}},
		{Header: "AGE", Field: func(obj client.Object) string { return age(obj.GetCreationTimestamp()) }},
	}
}

func (h *agentgatewayBackendHandler) DescribeExtra(obj client.Object) ([]resource.Section, error) {
	b, ok := obj.(*agwv1a1.AgentgatewayBackend)
	if !ok {
		return nil, nil
	}
	var sections []resource.Section

	sections = append(sections, resource.Section{
		Title: "Type",
		Body:  backendType(b),
	})

	if len(b.Status.Conditions) > 0 {
		var buf strings.Builder
		for _, c := range b.Status.Conditions {
			fmt.Fprintf(&buf, "%-20s %-8s %s\n", c.Type, c.Status, c.Message)
		}
		sections = append(sections, resource.Section{Title: "Conditions", Body: buf.String()})
	}

	return sections, nil
}

// ─── helpers ─────────────────────────────────────────────────────────────────

func policyAccepted(status gwv1.PolicyStatus) string {
	return findPolicyCondition(status, "Accepted")
}

func policyAttached(status gwv1.PolicyStatus) string {
	if s := findPolicyCondition(status, "Attached"); s != "Unknown" {
		return s
	}
	return findPolicyCondition(status, "ResolvedRefs")
}

func findPolicyCondition(status gwv1.PolicyStatus, condType string) string {
	for _, a := range status.Ancestors {
		for _, c := range a.Conditions {
			if c.Type == condType {
				return string(c.Status)
			}
		}
	}
	return "Unknown"
}

func policyTargets(p *agwv1a1.AgentgatewayPolicy) string {
	var parts []string
	for _, t := range p.Spec.TargetRefs {
		parts = append(parts, string(t.Kind)+"/"+string(t.Name))
	}
	for _, t := range p.Spec.TargetSelectors {
		parts = append(parts, string(t.Kind)+"[selector]")
	}
	return strings.Join(parts, ",")
}

func backendType(b *agwv1a1.AgentgatewayBackend) string {
	switch {
	case b.Spec.Static != nil:
		return "static"
	case b.Spec.AI != nil:
		return "ai"
	case b.Spec.MCP != nil:
		return "mcp"
	case b.Spec.A2A != nil:
		return "a2a"
	case b.Spec.DynamicForwardProxy != nil:
		return "dynamic-forward-proxy"
	case b.Spec.Aws != nil:
		return "aws"
	default:
		return "unknown"
	}
}

func backendAccepted(b *agwv1a1.AgentgatewayBackend) string {
	for _, c := range b.Status.Conditions {
		if c.Type == "Accepted" {
			return string(c.Status)
		}
	}
	return "Unknown"
}
