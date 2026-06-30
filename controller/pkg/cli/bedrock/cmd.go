package bedrock

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"slices"
	"strings"

	"github.com/spf13/cobra"
)

// RuntimeTable is the data model for runtime_models.json.
type RuntimeTable struct {
	Source string   `json:"source"`
	Models []string `json:"models"`
}

func Command() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "bedrock",
		Short: "Manage AWS Bedrock runtime model tables",
		Long: `Manage agentgateway AWS Bedrock runtime model tables.

Use subcommands to import the set of models served by bedrock-runtime.`,
	}
	cmd.AddCommand(importCmd())
	return cmd
}

type importFlags struct {
	source string
	out    string
	pretty bool
}

var importSources = map[string]func(ctx context.Context) (*RuntimeTable, []string, error){}

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
		source: awsDocsSourceName,
	}
	cmd := &cobra.Command{
		Use:   "import",
		Short: "Import Bedrock runtime model list",
		Long: `Import the list of models served by bedrock-runtime (InvokeModel/Converse).

Scrapes the AWS Bedrock endpoint availability docs page and follows model-card
links to collect canonical base model IDs. The output JSON is suitable for
embedding in the proxy as the runtime model allowlist.

Examples:
	agctl bedrock import > bedrock_runtime_models.json
	agctl bedrock import --pretty --out crates/agentgateway/src/llm/bedrock_runtime_models.json`,
		Args:         cobra.NoArgs,
		SilenceUsage: true,
		RunE: func(cmd *cobra.Command, args []string) error {
			return runImport(cmd, f)
		},
	}
	cmd.Flags().StringVar(&f.source, "source", f.source, "import source ("+importSourceList()+")")
	cmd.Flags().BoolVar(&f.pretty, "pretty", false, "pretty-print the output JSON")
	cmd.Flags().StringVarP(&f.out, "out", "o", "", "output path (default: stdout)")
	return cmd
}

func runImport(cmd *cobra.Command, f *importFlags) error {
	ctx := cmd.Context()
	src, ok := importSources[f.source]
	if !ok {
		return fmt.Errorf("unsupported source %q (supported: %s)", f.source, importSourceList())
	}

	table, warns, err := src(ctx)
	if err != nil {
		return err
	}
	for _, w := range warns {
		fmt.Fprintln(cmd.ErrOrStderr(), "warning:", w)
	}

	data, err := marshalTable(table, f.pretty)
	if err != nil {
		return err
	}

	if f.out == "" {
		_, err = cmd.OutOrStdout().Write(data)
		return err
	}
	if err := os.WriteFile(f.out, data, 0o600); err != nil {
		return fmt.Errorf("write %s: %w", f.out, err)
	}
	fmt.Fprintf(cmd.ErrOrStderr(), "imported %d runtime models\n", len(table.Models))
	return nil
}

func marshalTable(t *RuntimeTable, pretty bool) ([]byte, error) {
	marshal := json.Marshal
	if pretty {
		marshal = func(v any) ([]byte, error) { return json.MarshalIndent(v, "", "  ") }
	}
	data, err := marshal(t)
	if err != nil {
		return nil, fmt.Errorf("marshal table: %w", err)
	}
	return append(data, '\n'), nil
}
