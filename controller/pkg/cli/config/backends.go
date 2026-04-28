package config

import (
	"fmt"
	"io"
	"sort"
	"strconv"
	"strings"
	"text/tabwriter"

	"github.com/goccy/go-json"
	"github.com/spf13/cobra"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/flag"
)

type backendConfigDump struct {
	Services []backendService `json:"services"`
}

type backendService struct {
	Name      string            `json:"name"`
	Namespace string            `json:"namespace"`
	Endpoints []backendEndpoint `json:"endpoints"`
}

type backendEndpoint struct {
	Active map[string]backendEndpointState `json:"active"`
}

type backendEndpointState struct {
	Endpoint struct {
		WorkloadUID string `json:"workloadUid"`
	} `json:"endpoint"`
	Info struct {
		Health         *float64 `json:"health"`
		RequestLatency *float64 `json:"requestLatency"`
		TotalRequests  *int64   `json:"totalRequests"`
	} `json:"info"`
}

type backendRow struct {
	Name      string
	Namespace string
	Endpoint  string
	Health    string
	Requests  int64
	LatencyMS float64
}

func backendsCommand(common *commonFlags) flag.Command {
	var showAll bool

	return flag.Command{
		Use:     "backends",
		Aliases: []string{"b", "be"},
		Short:   "Retrieve Agentgateway backend endpoint status",
		Long:    "Retrieve Agentgateway backend endpoint status.",
		AddFlags: func(cmd *cobra.Command) {
			cmd.Flags().BoolVar(&showAll, "all", false, "Show endpoints with zero requests")
		},
		Args: func(cmd *cobra.Command, args []string) error {
			return common.validateArgs(cmd, args)
		},
		RunE: func(cmd *cobra.Command, args []string) error {
			source, err := loadConfigDumpSource(cmd.Context(), common, args)
			if err != nil {
				return err
			}

			rows, err := parseBackendRows(source.ConfigDump, showAll)
			if err != nil {
				return err
			}
			if common.outputFormat == shortOutput {
				printBackendTable(cmd.OutOrStdout(), rows)
			} else {
				printData(cmd.OutOrStdout(), common.outputFormat, rows)
			}

			return nil
		},
	}
}

func parseBackendRows(raw json.RawMessage, showAll bool) ([]backendRow, error) {
	var dump backendConfigDump
	if err := json.Unmarshal(raw, &dump); err != nil {
		return nil, fmt.Errorf("failed to parse config dump services: %w", err)
	}

	rows := make([]backendRow, 0)
	for _, service := range dump.Services {
		for _, endpoints := range service.Endpoints {
			endpointNames := make([]string, 0, len(endpoints.Active))
			for endpointName := range endpoints.Active {
				endpointNames = append(endpointNames, endpointName)
			}
			sort.Strings(endpointNames)

			for _, endpointName := range endpointNames {
				state := endpoints.Active[endpointName]
				row := backendRow{
					Name:      service.Name,
					Namespace: service.Namespace,
					Endpoint:  formatEndpointName(endpointName, service.Namespace),
				}
				if row.Endpoint == "" {
					row.Endpoint = formatEndpointName(state.Endpoint.WorkloadUID, service.Namespace)
				}
				if state.Info.Health != nil {
					row.Health = formatFloat(*state.Info.Health)
				}
				if state.Info.RequestLatency != nil {
					row.LatencyMS = *state.Info.RequestLatency * 1000
				}
				if state.Info.TotalRequests != nil {
					row.Requests = *state.Info.TotalRequests
				}
				if !showAll && row.Requests == 0 {
					continue
				}
				rows = append(rows, row)
			}
		}
	}

	sort.SliceStable(rows, func(i, j int) bool {
		if rows[i].Namespace != rows[j].Namespace {
			return rows[i].Namespace < rows[j].Namespace
		}
		if rows[i].Name != rows[j].Name {
			return rows[i].Name < rows[j].Name
		}
		return rows[i].Endpoint < rows[j].Endpoint
	})

	return rows, nil
}

func printBackendTable(w io.Writer, rows []backendRow) {
	tw := tabwriter.NewWriter(w, 0, 0, 2, ' ', 0)
	fmt.Fprintln(tw, "NAME\tNAMESPACE\tENDPOINT\tHEALTH\tREQUESTS\tLATENCY")
	for _, row := range rows {
		fmt.Fprintf(tw, "%s\t%s\t%s\t%s\t%d\t%s\n", row.Name, row.Namespace, row.Endpoint, row.Health, row.Requests, formatLatencyMS(row))
	}
	_ = tw.Flush()
}

func formatLatencyMS(row backendRow) string {
	if row.Requests == 0 {
		return ""
	}
	return formatFloat(row.LatencyMS) + "ms"
}

func formatFloat(value float64) string {
	return strconv.FormatFloat(value, 'f', 2, 64)
}

func formatEndpointName(endpoint, namespace string) string {
	parts := strings.Split(strings.TrimLeft(endpoint, "/"), "/")
	for i, part := range parts {
		if part == "Pod" {
			parts = parts[i+1:]
			break
		}
	}
	if len(parts) >= 2 && parts[0] == namespace {
		parts = parts[1:]
	}
	return strings.Join(parts, "/")
}
