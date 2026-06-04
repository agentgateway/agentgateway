package cmd

import (
	"context"
	"fmt"

	"github.com/spf13/cobra"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	"sigs.k8s.io/controller-runtime/pkg/client"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	agwv1a1 "github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/cli/resource"
)

type analyzeFlags struct {
	namespace     string
	allNamespaces bool
}

// BuildAnalyzeCmd returns the `agctl analyze` cobra command.
// It performs pure read-only validation of resources against the cluster state.
func BuildAnalyzeCmd() *cobra.Command {
	f := &analyzeFlags{}

	cmd := &cobra.Command{
		Use:   "analyze",
		Short: "Validate resources for broken references and misconfigurations",
		Long: `Validate Agentgateway resources by reading cluster state.

Checks performed:
  - HTTPRoute parent refs resolve to existing Gateways
  - AgentgatewayPolicy targetRefs point at existing resources
  - AgentgatewayBackend static host/port sanity

No cluster-side execution; pure Kubernetes API reads.`,
		SilenceUsage: true,
		RunE: func(cmd *cobra.Command, args []string) error {
			return runAnalyze(cmd.Context(), f, cmd)
		},
	}

	cmd.Flags().StringVarP(&f.namespace, "namespace", "n", "", "Kubernetes namespace (default: current context namespace)")
	cmd.Flags().BoolVarP(&f.allNamespaces, "all-namespaces", "A", false, "Analyze resources across all namespaces")
	return cmd
}

type finding struct {
	kind      string
	namespace string
	name      string
	message   string
}

func runAnalyze(ctx context.Context, f *analyzeFlags, cmd *cobra.Command) error {
	c, err := resource.NewClient()
	if err != nil {
		return err
	}

	namespace := f.namespace
	if !f.allNamespaces && namespace == "" {
		namespace, err = resolveNamespace()
		if err != nil {
			return err
		}
	}

	var findings []finding

	findings = append(findings, analyzeHTTPRoutes(ctx, c, namespace)...)
	findings = append(findings, analyzeAgentgatewayPolicies(ctx, c, namespace)...)
	findings = append(findings, analyzeAgentgatewayBackends(ctx, c, namespace)...)

	if len(findings) == 0 {
		fmt.Fprintln(cmd.OutOrStdout(), "No issues found.")
		return nil
	}

	for _, f := range findings {
		ns := f.namespace
		if ns != "" {
			fmt.Fprintf(cmd.OutOrStdout(), "[%s] %s/%s: %s\n", f.kind, ns, f.name, f.message)
		} else {
			fmt.Fprintf(cmd.OutOrStdout(), "[%s] %s: %s\n", f.kind, f.name, f.message)
		}
	}
	return fmt.Errorf("%d issue(s) found", len(findings))
}

func analyzeHTTPRoutes(ctx context.Context, c client.Client, namespace string) []finding {
	var routes gwv1.HTTPRouteList
	var opts []client.ListOption
	if namespace != "" {
		opts = append(opts, client.InNamespace(namespace))
	}
	if err := c.List(ctx, &routes, opts...); err != nil {
		return []finding{{kind: "HTTPRoute", message: fmt.Sprintf("failed to list: %v", err)}}
	}

	// Cache gateway existence checks to avoid N+1 API calls when multiple
	// routes reference the same parent gateway. Only cache definitive results
	// (found or not-found); transient errors are not cached so retries are possible.
	type gwResult struct{ missing bool; err error }
	gwCache := map[client.ObjectKey]gwResult{}

	var out []finding
	for _, route := range routes.Items {
		for _, ref := range route.Spec.ParentRefs {
			gwNamespace := route.Namespace
			if ref.Namespace != nil {
				gwNamespace = string(*ref.Namespace)
			}
			key := client.ObjectKey{Namespace: gwNamespace, Name: string(ref.Name)}

			result, cached := gwCache[key]
			if !cached {
				var gw gwv1.Gateway
				err := c.Get(ctx, key, &gw)
				result = gwResult{missing: apierrors.IsNotFound(err)}
				if !apierrors.IsNotFound(err) {
					result.err = err
				}
				// Only cache definitive outcomes; transient errors are left uncached.
				if err == nil || apierrors.IsNotFound(err) {
					gwCache[key] = result
				}
			}

			if result.missing {
				out = append(out, finding{
					kind:      "HTTPRoute",
					namespace: route.Namespace,
					name:      route.Name,
					message:   fmt.Sprintf("parentRef Gateway %s/%s not found", gwNamespace, ref.Name),
				})
			} else if result.err != nil {
				out = append(out, finding{
					kind:      "HTTPRoute",
					namespace: route.Namespace,
					name:      route.Name,
					message:   fmt.Sprintf("error checking parentRef Gateway %s/%s: %v", gwNamespace, ref.Name, result.err),
				})
			}
		}
	}
	return out
}

func analyzeAgentgatewayPolicies(ctx context.Context, c client.Client, namespace string) []finding {
	var policies agwv1a1.AgentgatewayPolicyList
	var opts []client.ListOption
	if namespace != "" {
		opts = append(opts, client.InNamespace(namespace))
	}
	if err := c.List(ctx, &policies, opts...); err != nil {
		return []finding{{kind: "AgentgatewayPolicy", message: fmt.Sprintf("failed to list: %v", err)}}
	}

	var out []finding
	for _, pol := range policies.Items {
		for _, ref := range pol.Spec.TargetRefs {
			targetName := string(ref.Name)
			targetKind := string(ref.Kind)
			targetGroup := string(ref.Group)

			if err := checkTargetExists(ctx, c, targetGroup, targetKind, pol.Namespace, targetName); err != nil {
				out = append(out, finding{
					kind:      "AgentgatewayPolicy",
					namespace: pol.Namespace,
					name:      pol.Name,
					message:   fmt.Sprintf("targetRef %s/%s %q: %v", targetGroup, targetKind, targetName, err),
				})
			}
		}
		for _, sel := range pol.Spec.TargetSelectors {
			targetKind := string(sel.Kind)
			targetGroup := string(sel.Group)
			if !isSupportedTargetKind(targetKind) {
				out = append(out, finding{
					kind:      "AgentgatewayPolicy",
					namespace: pol.Namespace,
					name:      pol.Name,
					message:   fmt.Sprintf("targetSelector %s/%s: unknown kind — cannot verify target existence", targetGroup, targetKind),
				})
			}
		}
	}
	return out
}

func analyzeAgentgatewayBackends(ctx context.Context, c client.Client, namespace string) []finding {
	var backends agwv1a1.AgentgatewayBackendList
	var opts []client.ListOption
	if namespace != "" {
		opts = append(opts, client.InNamespace(namespace))
	}
	if err := c.List(ctx, &backends, opts...); err != nil {
		return []finding{{kind: "AgentgatewayBackend", message: fmt.Sprintf("failed to list: %v", err)}}
	}

	var out []finding
	for _, be := range backends.Items {
		if be.Spec.Static != nil {
			if be.Spec.Static.Host == "" && be.Spec.Static.UnixPath == nil {
				out = append(out, finding{
					kind:      "AgentgatewayBackend",
					namespace: be.Namespace,
					name:      be.Name,
					message:   "static backend must specify host+port or unixPath",
				})
			}
		}
	}
	return out
}

// isSupportedTargetKind reports whether the given kind is one that
// checkTargetExists can verify.
func isSupportedTargetKind(kind string) bool {
	switch kind {
	case "Gateway", "HTTPRoute", "AgentgatewayBackend":
		return true
	default:
		return false
	}
}

// checkTargetExists verifies that a referenced resource exists.
// Supports Gateway, HTTPRoute, and AgentgatewayBackend.
// Returns an error for unknown kinds so callers can surface a warning finding.
func checkTargetExists(ctx context.Context, c client.Client, _, kind, namespace, name string) error {
	key := client.ObjectKey{Namespace: namespace, Name: name}
	var obj client.Object

	switch kind {
	case "Gateway":
		obj = &gwv1.Gateway{}
	case "HTTPRoute":
		obj = &gwv1.HTTPRoute{}
	case "AgentgatewayBackend":
		obj = &agwv1a1.AgentgatewayBackend{}
	default:
		return fmt.Errorf("unknown kind %q — cannot verify target existence", kind)
	}

	if err := c.Get(ctx, key, obj); err != nil {
		if apierrors.IsNotFound(err) {
			return fmt.Errorf("not found")
		}
		return err
	}
	return nil
}
