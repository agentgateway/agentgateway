// Generate a list of all metrics defined in project source files.
// go run ./pkg/metrics/cmd/findmetrics/main.go .
// from the root of the project.
package main

import (
	"flag"
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

const defaultNamespace = "kgateway"

type metricInfo struct {
	File      string
	Namespace string
	Subsystem string
	Name      string
	Type      string
	Help      string
	Labels    []string
}

func (m metricInfo) fullName() string {
	parts := []string{}

	if m.Namespace != "" {
		parts = append(parts, m.Namespace)
	}

	if m.Subsystem != "" {
		parts = append(parts, m.Subsystem)
	}

	parts = append(parts, m.Name)

	return strings.Join(parts, "_")
}

func main() {
	outputMarkdown := flag.Bool("markdown", false, "Generate markdown output")

	flag.Parse()

	if len(flag.Args()) < 1 {
		fmt.Fprintf(os.Stderr, "Usage: %s <file_or_directory>\n", os.Args[0])
		os.Exit(1)
	}

	target := flag.Args()[0]

	metrics, err := findMetrics(target)
	if err != nil {
		fmt.Fprintf(os.Stderr, "Error: %v\n", err)
		os.Exit(1)
	}

	sort.Slice(metrics, func(i, j int) bool {
		return metrics[i].fullName() < metrics[j].fullName()
	})

	if outputMarkdown != nil && *outputMarkdown {
		fmt.Println("Name|Type|Labels|Help")
		fmt.Println("--|--|--|--")
		for _, metric := range metrics {
			labelsStr := strings.Join(metric.Labels, ", ")
			fmt.Println(metric.fullName() + "|" + metric.Type + "|" + labelsStr + "|" + metric.Help)
		}

		return
	}

	for _, metric := range metrics {
		labelsStr := strings.Join(metric.Labels, ", ")
		fmt.Println("--------------------------------------------------------------")
		fmt.Println("Name:\t", metric.fullName())
		fmt.Println("Type:\t", metric.Type)
		fmt.Println("Help:\t", metric.Help)
		fmt.Println("Labels:\t", labelsStr)
		fmt.Println("File:\t", metric.File)
	}
}

func findMetrics(target string) ([]metricInfo, error) {
	info, err := os.Stat(target)
	if err != nil {
		return nil, err
	}

	var files []string

	if info.IsDir() {
		err := filepath.Walk(target, func(path string, info os.FileInfo, err error) error {
			if err != nil {
				return err
			}

			if strings.HasSuffix(path, ".go") && !strings.HasSuffix(path, "_test.go") {
				files = append(files, path)
			}

			return nil
		})

		if err != nil {
			return nil, err
		}
	} else if strings.HasSuffix(target, ".go") {
		files = append(files, target)
	} else {
		return nil, fmt.Errorf("target must be a .go file or a directory")
	}

	var allMetrics []metricInfo

	for _, file := range files {
		metrics, err := parseFile(file)
		if err != nil {
			return nil, fmt.Errorf("error parsing %s: %v", file, err)
		}

		allMetrics = append(allMetrics, metrics...)
	}

	return allMetrics, nil
}

func parseFile(filename string) ([]metricInfo, error) {
	fset := token.NewFileSet()

	node, err := parser.ParseFile(fset, filename, nil, parser.ParseComments)
	if err != nil {
		return nil, err
	}

	// Build a map of constants for namespace, subsystem, and label resolution.
	constants := make(map[string]string)

	ast.Inspect(node, func(n ast.Node) bool {
		if vs, ok := n.(*ast.ValueSpec); ok {
			for i, name := range vs.Names {
				if i < len(vs.Values) {
					if lit, ok := vs.Values[i].(*ast.BasicLit); ok && lit.Kind == token.STRING {
						constants[name.Name] = strings.Trim(lit.Value, `"`)
					}
				}
			}
		}

		return true
	})

	var metrics []metricInfo

	ast.Inspect(node, func(n ast.Node) bool {
		if call, ok := n.(*ast.CallExpr); ok {
			if sel, ok := call.Fun.(*ast.SelectorExpr); ok {
				if ident, ok := sel.X.(*ast.Ident); ok {
					if ident.Name == "metrics" {
						var metricType string

						switch sel.Sel.Name {
						case "NewCounter":
							metricType = "counter"
						case "NewGauge":
							metricType = "gauge"
						case "NewHistogram":
							metricType = "histogram"
						default:
							return true
						}

						if len(call.Args) >= 2 {
							metric := extractMetricInfo(call, metricType, constants)
							if metric.Name != "" {
								metric.File = filename
								metrics = append(metrics, metric)
							}
						}
					}
				}
			}
		}

		return true
	})

	return metrics, nil
}

func extractMetricInfo(call *ast.CallExpr, metricType string, constants map[string]string) metricInfo {
	metric := metricInfo{Type: metricType}

	// First argument should be the opts struct.
	if len(call.Args) >= 1 {
		if comp, ok := call.Args[0].(*ast.CompositeLit); ok {
			for _, elt := range comp.Elts {
				if kv, ok := elt.(*ast.KeyValueExpr); ok {
					if ident, ok := kv.Key.(*ast.Ident); ok {
						switch ident.Name {
						case "Name":
							if lit, ok := kv.Value.(*ast.BasicLit); ok && lit.Kind == token.STRING {
								metric.Name = strings.Trim(lit.Value, `"`)
							}
						case "Help":
							if lit, ok := kv.Value.(*ast.BasicLit); ok && lit.Kind == token.STRING {
								metric.Help = strings.Trim(lit.Value, `"`)
							}
						case "Namespace":
							if lit, ok := kv.Value.(*ast.BasicLit); ok && lit.Kind == token.STRING {
								metric.Namespace = strings.Trim(lit.Value, `"`)
							} else if ident, ok := kv.Value.(*ast.Ident); ok {
								if val, exists := constants[ident.Name]; exists {
									metric.Namespace = val
								}
							}
						case "Subsystem":
							if lit, ok := kv.Value.(*ast.BasicLit); ok && lit.Kind == token.STRING {
								metric.Subsystem = strings.Trim(lit.Value, `"`)
							} else if ident, ok := kv.Value.(*ast.Ident); ok {
								if val, exists := constants[ident.Name]; exists {
									metric.Subsystem = val
								}
							}
						}
					}
				}
			}
		}
	}

	if metric.Namespace == "" {
		metric.Namespace = defaultNamespace
	}

	// Second argument should be the labels slice.
	if len(call.Args) >= 2 {
		if comp, ok := call.Args[1].(*ast.CompositeLit); ok {
			for _, elt := range comp.Elts {
				if lit, ok := elt.(*ast.BasicLit); ok && lit.Kind == token.STRING {
					label := strings.Trim(lit.Value, `"`)
					metric.Labels = append(metric.Labels, label)
				} else if ident, ok := elt.(*ast.Ident); ok {
					if val, exists := constants[ident.Name]; exists {
						metric.Labels = append(metric.Labels, val)
					} else {
						metric.Labels = append(metric.Labels, ident.Name)
					}
				}
			}
		}
	}

	return metric
}
