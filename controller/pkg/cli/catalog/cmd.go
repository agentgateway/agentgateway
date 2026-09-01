package catalog

import (
	"context"
	"fmt"
	"os"
	"slices"
	"strings"
	"time"

	"github.com/spf13/cobra"
	"sigs.k8s.io/yaml"
)

func Command() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "catalog",
		Short: "Manage model catalogs",
		Long: `Manage agentgateway model catalogs.

Use subcommands to import catalog data from supported sources.`,
	}
	cmd.AddCommand(importCmd())
	return cmd
}

type importFlags struct {
	providers        []string
	excludeProviders []string
	source           string
	overlay          string
	out              string
	pretty           bool
	legacy           bool
}

type importOptions struct {
	providers        []string
	excludeProviders []string
	legacy           bool
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
		Short: "Import a model catalog",
		Long: `Import a model catalog.

Examples:
	agctl catalog import > catalog.json
	agctl catalog import --overlay ./catalog/model-catalog-overrides.yaml --out ./catalog/model-catalog.json --pretty
	agctl catalog import --source models.dev --providers anthropic,google,openai`,
		Args:         cobra.NoArgs,
		SilenceUsage: true,
		RunE: func(cmd *cobra.Command, args []string) error {
			return runImport(cmd, f)
		},
	}

	cmd.Flags().StringVar(&f.source, "source", f.source, "import source ("+importSourceList()+")")
	cmd.Flags().StringVar(&f.overlay, "overlay", "", "YAML catalog to merge over imported data")
	cmd.Flags().StringSliceVar(&f.providers, "providers", nil, "source provider ids to import (default: every provider the proxy supports)")
	cmd.Flags().StringSliceVar(&f.excludeProviders, "exclude-providers", nil, "source provider ids to omit")
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
		providers:        f.providers,
		excludeProviders: f.excludeProviders,
		legacy:           f.legacy,
	})
	if err != nil {
		return err
	}
	if f.overlay != "" {
		overlayData, err := os.ReadFile(f.overlay)
		if err != nil {
			return fmt.Errorf("read overlay %s: %w", f.overlay, err)
		}
		var overlay ModelCatalog
		if err := yaml.UnmarshalStrict(overlayData, &overlay); err != nil {
			return fmt.Errorf("parse overlay %s: %w", f.overlay, err)
		}
		if err := overlay.Validate(); err != nil {
			return fmt.Errorf("invalid overlay %s: %w", f.overlay, err)
		}
		cat.overlayWith(&overlay)
	}
	cat.Metadata = &CatalogMetadata{
		Source:      f.source,
		GeneratedAt: time.Now().UTC().Truncate(time.Second),
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
	} else if err := os.WriteFile(dest, data, 0o644); err != nil { //nolint:gosec // Catalog data is non-sensitive.
		return fmt.Errorf("write %s: %w", dest, err)
	}
	fmt.Fprintf(cmd.ErrOrStderr(), "imported %d providers\n", len(cat.Providers))
	return nil
}
