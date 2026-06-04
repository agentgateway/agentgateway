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
	case "short":
		return &shortPrinter{}, nil
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
	b, err := json.MarshalIndent(v, "", "  ")
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

// shortPrinter uses pretty-printed JSON as the default human-readable format.
type shortPrinter struct{}

func (p *shortPrinter) Print(w io.Writer, v any) error {
	b, err := json.MarshalIndent(v, "", "  ")
	if err != nil {
		return err
	}
	fmt.Fprintf(w, "%s\n", string(b))
	return nil
}
