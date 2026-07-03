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

func Command() *cobra.Command {
	f := &importFlags{
		source: awsMDSourceName,
	}
	cmd := &cobra.Command{
		Use:   "bedrock",
		Short: "Import Bedrock runtime model list",
		Long: `Import the list of models served by bedrock-runtime (InvokeModel/Converse).

Scrapes the AWS Bedrock endpoint availability Markdown page and extracts
canonical base model IDs for all models that support bedrock-runtime.
The output JSON is suitable for embedding in the proxy as the runtime
model allowlist.

Examples:
  agctl import bedrock > bedrock_runtime_models.json
  agctl import bedrock --pretty --out crates/agentgateway/src/llm/bedrock_runtime_models.json`,
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

	if dest := f.out; dest == "" {
		if _, err := cmd.OutOrStdout().Write(data); err != nil {
			return err
		}
	} else if err := os.WriteFile(dest, data, 0o600); err != nil {
		return fmt.Errorf("write %s: %w", dest, err)
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
