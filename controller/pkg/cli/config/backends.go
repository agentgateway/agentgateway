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
	Services []backendService  `json:"services"`
	Backends []topLevelBackend `json:"backends"`
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
		Name        string `json:"name"`
	} `json:"endpoint"`
	Info backendEndpointInfo `json:"info"`
}

type backendEndpointInfo struct {
	Health              *float64 `json:"health"`
	RequestLatency      *float64 `json:"requestLatency"`
	RequestLatencySnake *float64 `json:"request_latency"`
	TotalRequests       *int64   `json:"totalRequests"`
	TotalRequestsSnake  *int64   `json:"total_requests"`
}

type topLevelBackend struct {
	Backend map[string]topLevelBackendVariant `json:"backend"`
}

type topLevelBackendVariant struct {
	Name      string          `json:"name"`
	Namespace string          `json:"namespace"`
	Target    json.RawMessage `json:"target"`
}

type backendTargetProviders struct {
	Providers []backendEndpoint `json:"providers"`
}

type backendRow struct {
	Type      string  `json:"type" yaml:"type"`
	Name      string  `json:"name" yaml:"name"`
	Namespace string  `json:"namespace" yaml:"namespace"`
	Endpoint  string  `json:"endpoint" yaml:"endpoint"`
	Health    string  `json:"health" yaml:"health"`
	Requests  int64   `json:"requests" yaml:"requests"`
	LatencyMS float64 `json:"latencyMs" yaml:"latencyMs"`
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
				row := buildBackendRow("Service", service.Name, service.Namespace, endpointName, state)
				if !showAll && row.Requests == 0 {
					continue
				}
				rows = append(rows, row)
			}
		}
	}
	for _, backend := range dump.Backends {
		for _, variant := range backend.Backend {
			for _, provider := range variant.providers() {
				endpointNames := make([]string, 0, len(provider.Active))
				for endpointName := range provider.Active {
					endpointNames = append(endpointNames, endpointName)
				}
				sort.Strings(endpointNames)

				for _, endpointName := range endpointNames {
					row := buildBackendRow("Backend", variant.Name, variant.Namespace, endpointName, provider.Active[endpointName])
					if !showAll && row.Requests == 0 {
						continue
					}
					rows = append(rows, row)
				}
			}
		}
	}

	sort.SliceStable(rows, func(i, j int) bool {
		if rows[i].Type != rows[j].Type {
			return rows[i].Type < rows[j].Type
		}
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

func (v topLevelBackendVariant) providers() []backendEndpoint {
	var target backendTargetProviders
	if err := json.Unmarshal(v.Target, &target); err != nil {
		return nil
	}
	return target.Providers
}

func buildBackendRow(backendType, name, namespace, endpointName string, state backendEndpointState) backendRow {
	row := backendRow{
		Type:      backendType,
		Name:      name,
		Namespace: namespace,
		Endpoint:  formatEndpointName(endpointName, namespace),
	}
	if row.Endpoint == "" {
		row.Endpoint = formatEndpointName(state.Endpoint.Name, namespace)
	}
	if row.Endpoint == "" {
		row.Endpoint = formatEndpointName(state.Endpoint.WorkloadUID, namespace)
	}
	if state.Info.Health != nil {
		row.Health = formatFloat(*state.Info.Health)
	}
	if requestLatency := state.Info.requestLatency(); requestLatency != nil {
		row.LatencyMS = *requestLatency * 1000
	}
	if totalRequests := state.Info.totalRequests(); totalRequests != nil {
		row.Requests = *totalRequests
	}
	return row
}

func (i backendEndpointInfo) requestLatency() *float64 {
	if i.RequestLatency != nil {
		return i.RequestLatency
	}
	return i.RequestLatencySnake
}

func (i backendEndpointInfo) totalRequests() *int64 {
	if i.TotalRequests != nil {
		return i.TotalRequests
	}
	return i.TotalRequestsSnake
}

func printBackendTable(w io.Writer, rows []backendRow) {
	tw := tabwriter.NewWriter(w, 0, 0, 2, ' ', 0)
	fmt.Fprintln(tw, "TYPE\tNAME\tNAMESPACE\tENDPOINT\tHEALTH\tREQUESTS\tLATENCY")
	for _, row := range rows {
		fmt.Fprintf(tw, "%s\t%s\t%s\t%s\t%s\t%d\t%s\n", row.Type, row.Name, row.Namespace, row.Endpoint, row.Health, row.Requests, formatLatencyMS(row))
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
