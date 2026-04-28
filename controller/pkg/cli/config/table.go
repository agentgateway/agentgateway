package config

import (
	"fmt"
	"io"

	"github.com/goccy/go-json"
	"sigs.k8s.io/yaml"
)

func printData(w io.Writer, format string, raw any) {
	var b []byte
	var err error
	if format == yamlOutput {
		b, err = yaml.Marshal(raw)
	} else {
		b, err = json.MarshalIndent(raw, "", "  ")
	}
	if err != nil {
		fmt.Fprintf(w, "error: %v\n", err)
	} else {
		fmt.Fprintf(w, "%s\n", string(b))
	}
}
