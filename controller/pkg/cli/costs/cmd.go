package costs

import (
	"context"
	"fmt"
	"os"
	"slices"
	"strings"

	"github.com/spf13/cobra"
)

func Command() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "costs",
		Short: "Manage model cost catalogs",
		Long: `Manage agentgateway model cost catalogs.

Use subcommands to import catalog data from supported sources.`,
	}
	cmd.AddCommand(importCmd())
	return cmd
}

type importFlags struct {
	providers []string
	source    string
	out       string
	pretty    bool
	legacy    bool
}

type importOptions struct {
	providers []string
	legacy    bool
}

var importSources = map[string]func(ctx context.Context, opts importOptions) (*ModelCatalog, []string, error){}

func importSourceNames() []string {
	names := make([]string, 0, len(importSources))
	for name := range importSources {
		names = append(names, name)
	}
	slices.Sort(names)
	return names
}

func importSourceList() string {
	return strings.Join(importSourceNames(), ", ")
}

func importCmd() *cobra.Command {
	f := &importFlags{
		source: modelsDevSourceName,
	}
	cmd := &cobra.Command{
		Use:   "import",
		Short: "Import model costs",
		Long: `Import a model cost catalog.

Examples:
	agctl costs import > catalog.json
	agctl costs import --out ./costs/catalog.json
	agctl costs import --source models.dev --providers anthropic,google,openai`,
		Args:         cobra.NoArgs,
		SilenceUsage: true,
		RunE: func(cmd *cobra.Command, args []string) error {
			return runImport(cmd, f)
		},
	}

	cmd.Flags().StringVar(&f.source, "source", f.source, "import source ("+importSourceList()+")")
	cmd.Flags().StringSliceVar(&f.providers, "providers", nil, "source provider ids to import (default: every provider the proxy supports)")
	cmd.Flags().BoolVar(&f.legacy, "legacy", false, "include deprecated models")
	cmd.Flags().BoolVar(&f.pretty, "pretty", false, "pretty-print the output JSON")
	cmd.Flags().StringVarP(&f.out, "out", "o", f.out, "output catalog path (default: stdout)")

	return cmd
}

func runImport(cmd *cobra.Command, f *importFlags) error {
	ctx := cmd.Context()
	if f.source == "" {
		return fmt.Errorf("source is required; pass --source with one of: %s", importSourceList())
	}
	src, ok := importSources[f.source]
	if !ok {
		return fmt.Errorf("unsupported source %q (supported sources: %s)", f.source, importSourceList())
	}

	cat, warns, err := src(ctx, importOptions{
		providers: f.providers,
		legacy:    f.legacy,
	})
	if err != nil {
		return err
	}
	if err := cat.Validate(); err != nil {
		return fmt.Errorf("invalid catalog: %w", err)
	}
	for _, w := range warns {
		fmt.Fprintln(cmd.ErrOrStderr(), "warning:", w)
	}

	data, err := marshalCatalog(cat, f.pretty)
	if err != nil {
		return err
	}

	if dest := f.out; dest == "" {
		if _, err := cmd.OutOrStdout().Write(data); err != nil {
			return err
		}
	} else if err := os.WriteFile(dest, data, 0o600); err != nil {
		return fmt.Errorf("write %s: %w", dest, err)
	}
	fmt.Fprintf(cmd.ErrOrStderr(), "imported %d providers\n", len(cat.Providers))
	return nil
}
