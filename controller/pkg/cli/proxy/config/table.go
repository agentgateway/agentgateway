package config

import (
	"fmt"
	"io"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/printer"
)

func printData(w io.Writer, format string, v any) {
	p, err := printer.New(format)
	if err != nil {
		fmt.Fprintf(w, "error: %v\n", err)
		return
	}
	if err := p.Print(w, v); err != nil {
		fmt.Fprintf(w, "error: %v\n", err)
	}
}
