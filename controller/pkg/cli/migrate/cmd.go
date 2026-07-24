package migrate

import (
	"context"
	"fmt"
	"io"
	"maps"
	"slices"
	"strings"

	"github.com/spf13/cobra"
	"github.com/spf13/pflag"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/kubeutil"
	"github.com/agentgateway/agentgateway/controller/pkg/cli/printer"
)

// Migration is selectable via --apply <id>; migrations self-register into registry via init().
type Migration struct {
	ID string
	// RegisterFlags adds flags to the command; nil if none.
	RegisterFlags func(fs *pflag.FlagSet)
	Run           func(ctx context.Context, out, status io.Writer, kubeClient kubeutil.CLIClient, namespace string, write bool) error
}

var registry = map[string]Migration{}

func migrationIDList() string {
	return strings.Join(slices.Sorted(maps.Keys(registry)), ", ")
}

type migrateFlags struct {
	namespace string
	apply     []string
	write     bool
}

func Command() *cobra.Command {
	f := &migrateFlags{}

	cmd := &cobra.Command{
		Use:   "migrate",
		Short: "Migrate agentgateway resources to newer configurations",
		Long: `Migrate agentgateway resources to newer configurations.

Prints the changes as YAML by default; pass --write to apply them to the cluster.

Available migrations:
  ` + migrationIDList(),
		Example: `agctl migrate --apply virtualkeys-to-configmap -n my-namespace | kubectl apply -f -
agctl migrate --apply virtualkeys-to-configmap -n my-namespace > migration.yaml
agctl migrate --apply virtualkeys-to-configmap -n my-namespace --write`,
		Args:         cobra.NoArgs,
		SilenceUsage: true,
		RunE: func(cmd *cobra.Command, args []string) error {
			return runMigrate(cmd, f)
		},
	}

	cmd.Flags().StringVarP(&f.namespace, "namespace", "n", "", "Namespace to migrate resources in")
	cmd.Flags().StringSliceVar(&f.apply, "apply", nil, "migrations to run, comma-separated ("+migrationIDList()+")")
	cmd.Flags().BoolVar(&f.write, "write", false, "apply the changes to the cluster (default: print YAML)")

	for _, m := range registry {
		if m.RegisterFlags != nil {
			m.RegisterFlags(cmd.Flags())
		}
	}

	return cmd
}

func runMigrate(cmd *cobra.Command, f *migrateFlags) error {
	if len(f.apply) == 0 {
		return fmt.Errorf("--apply is required; pass one or more of: %s", migrationIDList())
	}

	ctx := cmd.Context()
	namespace, err := kubeutil.LoadNamespace(f.namespace)
	if err != nil {
		return err
	}
	kubeClient, err := kubeutil.NewCLIClient()
	if err != nil {
		return err
	}
	out := cmd.OutOrStdout()
	status := cmd.ErrOrStderr()

	for _, id := range f.apply {
		m, ok := registry[id]
		if !ok {
			return fmt.Errorf("unknown migration %q (available: %s)", id, migrationIDList())
		}
		fmt.Fprintf(status, "== %s ==\n", id)
		if err := m.Run(ctx, out, status, kubeClient, namespace, f.write); err != nil {
			return fmt.Errorf("migration %q: %w", id, err)
		}
	}

	return nil
}

// serverManagedFields are read-time metadata stripped before printing, so output stays git-clean.
var serverManagedFields = [][]string{
	{"metadata", "resourceVersion"},
	{"metadata", "uid"},
	{"metadata", "generation"},
	{"metadata", "creationTimestamp"},
	{"metadata", "managedFields"},
	{"metadata", "selfLink"},
	{"status"},
}

// printYAML writes a "---"-prefixed doc; unstructured objects get serverManagedFields stripped first.
func printYAML(out io.Writer, v any) error {
	if _, err := fmt.Fprintln(out, "---"); err != nil {
		return err
	}
	if u, ok := v.(*unstructured.Unstructured); ok {
		clean := u.DeepCopy()
		for _, path := range serverManagedFields {
			unstructured.RemoveNestedField(clean.Object, path...)
		}
		v = clean
	}
	p, err := printer.New("yaml")
	if err != nil {
		return err
	}
	return p.Print(out, v)
}
