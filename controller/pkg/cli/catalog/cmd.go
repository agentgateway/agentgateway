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
	sources          []string
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

// defaultImportSources merge (in order) when --source is unset: models.dev rates + Bedrock tags.
func defaultImportSources() []string {
	return []string{modelsDevSourceName, bedrockMantleSourceName}
}

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
		sources: defaultImportSources(),
	}
	cmd := &cobra.Command{
		Use:   "import",
		Short: "Import a model catalog",
		Long: `Import a model catalog.

Multiple sources are merged in order, so later sources overlay earlier ones (e.g. models.dev
supplies pricing and aws-bedrock-mantle overlays Bedrock endpoint tags onto it).

Examples:
	agctl catalog import --out ./costs/catalog.json
	agctl catalog import --source models.dev --providers anthropic,google,openai
	agctl catalog import --source models.dev,aws-bedrock-mantle --overlay ./catalog/model-catalog-overrides.yaml --out ./catalog/model-catalog.json --pretty`,
		Args:         cobra.NoArgs,
		SilenceUsage: true,
		RunE: func(cmd *cobra.Command, args []string) error {
			return runImport(cmd, f)
		},
	}

	cmd.Flags().StringSliceVar(&f.sources, "source", f.sources, "import sources to merge, in order ("+importSourceList()+")")
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
	if len(f.sources) == 0 {
		return fmt.Errorf("at least one source is required; pass --source with any of: %s", importSourceList())
	}

	merged := &ModelCatalog{Providers: map[string]Provider{}}
	var warns []string
	for _, name := range f.sources {
		src, ok := importSources[name]
		if !ok {
			return fmt.Errorf("unsupported source %q (supported sources: %s)", name, importSourceList())
		}
		cat, w, err := src(ctx, importOptions{
			providers:        f.providers,
			excludeProviders: f.excludeProviders,
			legacy:           f.legacy,
		})
		if err != nil {
			return fmt.Errorf("source %q: %w", name, err)
		}
		warns = append(warns, w...)
		merged.overlayWith(cat)
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
		merged.overlayWith(&overlay)
	}
	if merged.Metadata == nil {
		merged.Metadata = &CatalogMetadata{
			GeneratedAt: time.Now().UTC().Truncate(time.Second),
		}
	}
	if err := merged.Validate(); err != nil {
		return fmt.Errorf("invalid catalog: %w", err)
	}
	for _, w := range warns {
		fmt.Fprintln(cmd.ErrOrStderr(), "warning:", w)
	}

	data, err := marshalCatalog(merged, f.pretty)
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
	fmt.Fprintf(cmd.ErrOrStderr(), "imported %d providers\n", len(merged.Providers))
	return nil
}
