package cmd

import (
	"context"
	"fmt"
	"os"
	"strings"

	"github.com/spf13/cobra"
	"k8s.io/apimachinery/pkg/labels"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"sigs.k8s.io/controller-runtime/pkg/client"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/kubeutil"
	"github.com/agentgateway/agentgateway/controller/pkg/cli/printer"
	"github.com/agentgateway/agentgateway/controller/pkg/cli/resource"
)

type getFlags struct {
	namespace     string
	allNamespaces bool
	selector      string
	output        string
}

// BuildGetCmd returns the `agctl get` cobra command built dynamically
// from registered resource handlers.
func BuildGetCmd() *cobra.Command {
	f := &getFlags{}

	cmd := &cobra.Command{
		Use:       "get <resource> [name]",
		Short:     "List or get Agentgateway resources",
		Long:      buildGetLong(),
		Aliases:   []string{"g"},
		Args:      cobra.RangeArgs(1, 2),
		ValidArgs: collectAliases(),
		RunE: func(cmd *cobra.Command, args []string) error {
			return runGet(cmd.Context(), f, args)
		},
		SilenceUsage: true,
	}

	cmd.Flags().StringVarP(&f.namespace, "namespace", "n", "", "Kubernetes namespace")
	cmd.Flags().BoolVarP(&f.allNamespaces, "all-namespaces", "A", false, "List across all namespaces")
	cmd.Flags().StringVarP(&f.selector, "selector", "l", "", "Label selector filter")
	cmd.Flags().StringVarP(&f.output, "output", "o", "short", "Output format: short|wide|json|yaml")

	return cmd
}

func runGet(ctx context.Context, f *getFlags, args []string) error {
	resourceType := args[0]
	var resourceName string
	if len(args) == 2 {
		resourceName = args[1]
	}

	h, err := resource.Lookup(resourceType)
	if err != nil {
		return err
	}

	switch f.output {
	case "short", "wide", "json", "yaml":
	default:
		return fmt.Errorf("unsupported output format %q; use short, wide, json, or yaml", f.output)
	}

	c, err := resource.NewClient()
	if err != nil {
		return err
	}

	namespace := f.namespace
	if f.allNamespaces {
		namespace = ""
	} else if namespace == "" {
		namespace, err = kubeutil.LoadNamespace("")
		if err != nil {
			return err
		}
	}

	gvk := h.Mapping().GroupVersionKind
	gvr := h.Mapping().Resource

	if resourceName != "" {
		obj, err := getOne(ctx, c, gvk, gvr, namespace, resourceName)
		if err != nil {
			return err
		}
		return printObject(f.output, h, []client.Object{obj})
	}

	objs, err := listAll(ctx, c, gvk, gvr, namespace, f.selector)
	if err != nil {
		return err
	}
	if len(objs) == 0 {
		if namespace != "" {
			fmt.Fprintf(os.Stdout, "No resources found in %s namespace.\n", namespace)
		} else {
			fmt.Fprintln(os.Stdout, "No resources found.")
		}
		return nil
	}
	return printObject(f.output, h, objs)
}

func getOne(ctx context.Context, c client.Client, gvk schema.GroupVersionKind, _ schema.GroupVersionResource, namespace, name string) (client.Object, error) {
	obj, err := newObjectForGVK(gvk)
	if err != nil {
		return nil, err
	}
	key := client.ObjectKey{Namespace: namespace, Name: name}
	if err := c.Get(ctx, key, obj); err != nil {
		return nil, fmt.Errorf("failed to get %s/%s: %w", gvk.Kind, name, err)
	}
	// controller-runtime clears TypeMeta on typed Get responses; re-stamp it so
	// callers (e.g. PrintDescribe, json/yaml output) see the correct Kind/Group.
	obj.GetObjectKind().SetGroupVersionKind(gvk)
	return obj, nil
}

func listAll(ctx context.Context, c client.Client, gvk schema.GroupVersionKind, _ schema.GroupVersionResource, namespace, selector string) ([]client.Object, error) {
	listGVK := schema.GroupVersionKind{Group: gvk.Group, Version: gvk.Version, Kind: gvk.Kind + "List"}
	listObj, err := newListObjectForGVK(listGVK)
	if err != nil {
		return nil, err
	}

	var opts []client.ListOption
	if namespace != "" {
		opts = append(opts, client.InNamespace(namespace))
	}
	if selector != "" {
		sel, err := labels.Parse(selector)
		if err != nil {
			return nil, fmt.Errorf("invalid label selector %q: %w", selector, err)
		}
		opts = append(opts, client.MatchingLabelsSelector{Selector: sel})
	}

	if err := c.List(ctx, listObj, opts...); err != nil {
		return nil, fmt.Errorf("failed to list %s: %w", gvk.Kind, err)
	}

	return extractListItems(listObj)
}

func printObject(format string, h resource.ResourceHandler, objs []client.Object) error {
	switch format {
	case "json", "yaml":
		p, _ := printer.New(format)
		if len(objs) == 1 {
			return p.Print(os.Stdout, objs[0])
		}
		return p.Print(os.Stdout, objs)
	default:
		return resource.PrintTable(os.Stdout, h.Columns(), objs, format == "wide")
	}
}

func newObjectForGVK(gvk schema.GroupVersionKind) (client.Object, error) {
	obj, err := resource.SchemeForClient().New(gvk)
	if err != nil {
		return nil, fmt.Errorf("unrecognized resource type %s/%s: %w", gvk.Group, gvk.Kind, err)
	}
	co, ok := obj.(client.Object)
	if !ok {
		return nil, fmt.Errorf("type %T does not implement client.Object", obj)
	}
	return co, nil
}

func newListObjectForGVK(gvk schema.GroupVersionKind) (client.ObjectList, error) {
	obj, err := resource.SchemeForClient().New(gvk)
	if err != nil {
		return nil, fmt.Errorf("unrecognized list type %s/%s: %w", gvk.Group, gvk.Kind, err)
	}
	lo, ok := obj.(client.ObjectList)
	if !ok {
		return nil, fmt.Errorf("type %T does not implement client.ObjectList", obj)
	}
	return lo, nil
}

func extractListItems(listObj client.ObjectList) ([]client.Object, error) {
	scheme := resource.SchemeForClient()
	// Resolve item GVK once, outside the loop.
	gvks, _, err := scheme.ObjectKinds(listObj)
	if err != nil || len(gvks) == 0 {
		return nil, fmt.Errorf("failed to determine GVK for list object: %w", err)
	}
	// Item GVK is the list GVK without the "List" suffix.
	itemKind := strings.TrimSuffix(gvks[0].Kind, "List")
	if itemKind == gvks[0].Kind {
		return nil, fmt.Errorf("list GVK kind %q does not end in \"List\"", gvks[0].Kind)
	}
	itemGVK := schema.GroupVersionKind{
		Group:   gvks[0].Group,
		Version: gvks[0].Version,
		Kind:    itemKind,
	}

	raw, err := runtime.DefaultUnstructuredConverter.ToUnstructured(listObj)
	if err != nil {
		return nil, fmt.Errorf("failed to extract list items: %w", err)
	}
	rawItems, _ := raw["items"].([]any)
	result := make([]client.Object, 0, len(rawItems))
	for i, item := range rawItems {
		m, ok := item.(map[string]any)
		if !ok {
			return nil, fmt.Errorf("item %d in list is not a map", i)
		}
		obj, err := scheme.New(itemGVK)
		if err != nil {
			return nil, fmt.Errorf("failed to create object for %s: %w", itemGVK.Kind, err)
		}
		if err := runtime.DefaultUnstructuredConverter.FromUnstructured(m, obj); err != nil {
			return nil, fmt.Errorf("failed to decode item %d (%s): %w", i, itemGVK.Kind, err)
		}
		co, ok := obj.(client.Object)
		if !ok {
			return nil, fmt.Errorf("type %T does not implement client.Object", obj)
		}
		co.GetObjectKind().SetGroupVersionKind(itemGVK)
		result = append(result, co)
	}
	return result, nil
}

func resolveNamespace() (string, error) {
	return kubeutil.LoadNamespace("")
}

func collectAliases() []string {
	var all []string
	for _, h := range resource.All() {
		all = append(all, h.Aliases()...)
	}
	return all
}

func buildGetLong() string {
	var b strings.Builder
	b.WriteString("List or get Agentgateway resources.\n\nAvailable resources:")
	for _, h := range resource.All() {
		aliases := h.Aliases()
		if len(aliases) > 0 {
			fmt.Fprintf(&b, "\n  %-35s (%s)", aliases[0], strings.Join(aliases[1:], ", "))
		}
	}
	return b.String()
}
