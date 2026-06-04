package resource

import (
	"fmt"
	"io"
	"strings"
	"text/tabwriter"

	"sigs.k8s.io/controller-runtime/pkg/client"
)

// PrintTable writes objects as a tab-aligned table to w using the provided columns.
// If wide is true, columns with Wide=true are included.
func PrintTable(w io.Writer, cols []Column, objs []client.Object, wide bool) error {
	tw := tabwriter.NewWriter(w, 0, 0, 3, ' ', 0)

	// Header
	var headers []string
	for _, c := range cols {
		if !c.Wide || wide {
			headers = append(headers, c.Header)
		}
	}
	fmt.Fprintln(tw, strings.Join(headers, "\t"))

	// Rows
	for _, obj := range objs {
		var cells []string
		for _, c := range cols {
			if !c.Wide || wide {
				cells = append(cells, c.Field(obj))
			}
		}
		fmt.Fprintln(tw, strings.Join(cells, "\t"))
	}

	return tw.Flush()
}

// PrintDescribe writes a describe-style output for a single object.
func PrintDescribe(w io.Writer, obj client.Object, sections []Section) error {
	gvk := obj.GetObjectKind().GroupVersionKind()
	fmt.Fprintf(w, "Name:       %s\n", obj.GetName())
	fmt.Fprintf(w, "Namespace:  %s\n", obj.GetNamespace())
	fmt.Fprintf(w, "Kind:       %s\n", gvk.Kind)
	if gvk.Group != "" {
		fmt.Fprintf(w, "Group:      %s\n", gvk.Group)
	}

	labels := obj.GetLabels()
	if len(labels) > 0 {
		fmt.Fprintf(w, "Labels:\n")
		for k, v := range labels {
			fmt.Fprintf(w, "  %s=%s\n", k, v)
		}
	}

	annotations := obj.GetAnnotations()
	if len(annotations) > 0 {
		fmt.Fprintf(w, "Annotations:\n")
		for k, v := range annotations {
			fmt.Fprintf(w, "  %s=%s\n", k, v)
		}
	}

	for _, s := range sections {
		fmt.Fprintf(w, "\n%s:\n", s.Title)
		if s.Body == "" {
			fmt.Fprintf(w, "  <none>\n")
		} else {
			for _, line := range strings.Split(strings.TrimRight(s.Body, "\n"), "\n") {
				fmt.Fprintf(w, "  %s\n", line)
			}
		}
	}

	return nil
}
