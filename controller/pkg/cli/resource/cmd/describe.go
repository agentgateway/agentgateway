package cmd

import (
	"context"
	"fmt"

	"github.com/spf13/cobra"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/resource"
)

type describeFlags struct {
	namespace string
}

// BuildDescribeCmd returns the `agctl describe` cobra command.
func BuildDescribeCmd() *cobra.Command {
	f := &describeFlags{}

	cmd := &cobra.Command{
		Use:          "describe <resource> <name>",
		Short:        "Show detailed information about a resource",
		Aliases:      []string{"desc"},
		Args:         cobra.ExactArgs(2),
		ValidArgs:    collectAliases(),
		SilenceUsage: true,
		RunE: func(cmd *cobra.Command, args []string) error {
			return runDescribe(cmd.Context(), f, args, cmd)
		},
	}

	cmd.Flags().StringVarP(&f.namespace, "namespace", "n", "", "Kubernetes namespace")
	return cmd
}

func runDescribe(ctx context.Context, f *describeFlags, args []string, cmd *cobra.Command) error {
	resourceType := args[0]
	name := args[1]

	h, err := resource.Lookup(resourceType)
	if err != nil {
		return err
	}

	c, err := resource.NewClient()
	if err != nil {
		return err
	}

	namespace := f.namespace
	if namespace == "" {
		namespace, err = resolveNamespace()
		if err != nil {
			return err
		}
	}

	gvk := h.Mapping().GroupVersionKind
	gvr := h.Mapping().Resource
	obj, err := getOne(ctx, c, gvk, gvr, namespace, name)
	if err != nil {
		return err
	}

	sections, err := h.DescribeExtra(obj)
	if err != nil {
		return fmt.Errorf("describe: %w", err)
	}

	return resource.PrintDescribe(cmd.OutOrStdout(), obj, sections)
}
