package printer

import (
	"fmt"
	"io"

	"github.com/goccy/go-json"
	"sigs.k8s.io/yaml"
)

type Printer interface {
	Print(w io.Writer, v any) error
}

func New(format string) (Printer, error) {
	switch format {
	case "pretty":
		return &prettyPrinter{}, nil
	case "json":
		return &jsonPrinter{}, nil
	case "yaml":
		return &yamlPrinter{}, nil
	default:
		return nil, fmt.Errorf("output format %q not supported", format)
	}
}

type jsonPrinter struct{}

func (p *jsonPrinter) Print(w io.Writer, v any) error {
	b, err := json.Marshal(v)
	if err != nil {
		return err
	}
	fmt.Fprintf(w, "%s\n", string(b))
	return nil
}

type yamlPrinter struct{}

func (p *yamlPrinter) Print(w io.Writer, v any) error {
	b, err := yaml.Marshal(v)
	if err != nil {
		return err
	}
	fmt.Fprintf(w, "%s\n", string(b))
	return nil
}

// prettyPrinter uses indented JSON for human-readable output.
type prettyPrinter struct{}

func (p *prettyPrinter) Print(w io.Writer, v any) error {
	b, err := json.MarshalIndent(v, "", "  ")
	if err != nil {
		return err
	}
	fmt.Fprintf(w, "%s\n", string(b))
	return nil
}
