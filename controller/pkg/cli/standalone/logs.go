package standalone

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"slices"
	"strings"
	"text/tabwriter"
	"time"

	"github.com/spf13/cobra"
	"sigs.k8s.io/yaml"
)

type logFilters struct {
	HTTPStatus    []int64  `json:"httpStatus,omitempty"`
	Provider      []string `json:"provider,omitempty"`
	RequestModel  []string `json:"requestModel,omitempty"`
	ResponseModel []string `json:"responseModel,omitempty"`
	TraceID       string   `json:"traceId,omitempty"`
}

func (f *logFilters) addFlags(cmd *cobra.Command) {
	cmd.Flags().Int64SliceVar(&f.HTTPStatus, "status", nil, "Filter by HTTP status (repeatable)")
	// pflag prints "(default [])" for empty int slices.
	cmd.Flags().Lookup("status").DefValue = ""
	cmd.Flags().StringSliceVar(&f.Provider, "provider", nil, "Filter by LLM provider (repeatable)")
	cmd.Flags().StringSliceVar(&f.RequestModel, "model", nil, "Filter by requested model (repeatable)")
	cmd.Flags().StringSliceVar(&f.ResponseModel, "response-model", nil, "Filter by response model (repeatable)")
	cmd.Flags().StringVar(&f.TraceID, "trace-id", "", "Filter by trace ID")
}

type logEntry struct {
	ID          string   `json:"id"`
	CompletedAt string   `json:"completedAt"`
	DurationMs  int64    `json:"durationMs"`
	HTTPStatus  *int64   `json:"httpStatus"`
	Error       *string  `json:"error"`
	Cost        *float64 `json:"cost"`
	GenAI       struct {
		ProviderName  *string `json:"providerName"`
		RequestModel  *string `json:"requestModel"`
		ResponseModel *string `json:"responseModel"`
	} `json:"genAi"`
	Usage struct {
		InputTokens  *int64 `json:"inputTokens"`
		OutputTokens *int64 `json:"outputTokens"`
	} `json:"usage"`
}

func logsCommand(c *client) *cobra.Command {
	var (
		output     string
		follow     bool
		limit      int64
		since      time.Duration
		attributes bool
		filters    logFilters
	)
	cmd := &cobra.Command{
		Use:   "logs [ID]",
		Short: "Display request logs",
		Long: `Display request logs from the standalone agentgateway request log database.

With no ID, prints the most recent requests, oldest first. With an ID, prints
that request including its stored prompt and completion.`,
		Example: `  agctl standalone logs
  agctl standalone logs -f --model gpt-4o
  agctl standalone logs --since 1h --status 429 --status 500
  agctl standalone logs 0198f5c2-...`,
		Args:         cobra.MaximumNArgs(1),
		SilenceUsage: true,
		RunE: func(cmd *cobra.Command, args []string) error {
			w := cmd.OutOrStdout()
			if len(args) == 1 {
				if follow {
					return fmt.Errorf("--follow cannot be used with an ID")
				}
				var result struct {
					Log json.RawMessage `json:"log"`
				}
				body := map[string]any{"id": args[0], "includePayload": true}
				if err := c.do(cmd.Context(), http.MethodPost, "/api/logs/get", body, &result); err != nil {
					return err
				}
				if len(result.Log) == 0 || bytes.Equal(result.Log, []byte("null")) {
					return fmt.Errorf("log %s not found", args[0])
				}
				if output == "table" {
					output = "yaml"
				}
				return printRaw(w, result.Log, output)
			}

			search := map[string]any{
				"limit":             limit,
				"filters":           filters,
				"includeAttributes": attributes,
			}
			if since > 0 {
				search["timeRange"] = map[string]any{"from": time.Now().Add(-since).UTC().Format(time.RFC3339)}
			}
			var result struct {
				Logs []json.RawMessage `json:"logs"`
			}
			if err := c.do(cmd.Context(), http.MethodPost, "/api/logs/search", search, &result); err != nil {
				return err
			}
			// Search returns newest first; print oldest first so follow output continues in order.
			slices.Reverse(result.Logs)
			if !follow {
				if output == "table" {
					tw := tabwriter.NewWriter(w, 0, 4, 2, ' ', 0)
					fmt.Fprintln(tw, strings.Join(logHeader, "\t"))
					for _, raw := range result.Logs {
						fmt.Fprintln(tw, strings.Join(logRow(raw), "\t"))
					}
					return tw.Flush()
				}
				data, err := json.Marshal(map[string]any{"logs": result.Logs})
				if err != nil {
					return err
				}
				return printRaw(w, data, output)
			}

			if output == "table" {
				printFollowRow(w, logHeader)
			}
			print := func(raw json.RawMessage) error {
				switch output {
				case "table":
					printFollowRow(w, logRow(raw))
					return nil
				case "json":
					var compact bytes.Buffer
					if err := json.Compact(&compact, raw); err != nil {
						return err
					}
					fmt.Fprintf(w, "%s\n", compact.Bytes())
					return nil
				default:
					fmt.Fprintln(w, "---")
					return printRaw(w, raw, output)
				}
			}
			tail := map[string]any{
				"filters":           filters,
				"includeAttributes": attributes,
			}
			for _, raw := range result.Logs {
				if err := print(raw); err != nil {
					return err
				}
			}
			if n := len(result.Logs); n > 0 {
				var last logEntry
				_ = json.Unmarshal(result.Logs[n-1], &last)
				tail["cursor"] = last.CompletedAt + "|" + last.ID
			}
			return c.tailLogs(cmd.Context(), tail, print)
		},
	}
	cmd.Flags().StringVarP(&output, "output", "o", "table", "Output format: table, yaml, or json")
	cmd.Flags().BoolVarP(&follow, "follow", "f", false, "Stream new requests as they complete")
	cmd.Flags().Int64Var(&limit, "limit", 20, "Number of recent requests to show (max 500)")
	cmd.Flags().DurationVar(&since, "since", 0, "Only show requests newer than a relative duration, like 5m or 1h")
	cmd.Flags().BoolVar(&attributes, "attributes", false, "Include all logged attributes")
	filters.addFlags(cmd)
	_ = cmd.RegisterFlagCompletionFunc("output", cobra.FixedCompletions([]string{"table", "yaml", "json"}, cobra.ShellCompDirectiveNoFileComp))
	return cmd
}

// tailLogs streams the server-sent events from /api/logs/tail until the context is cancelled or the server errors.
func (c *client) tailLogs(ctx context.Context, body any, onLog func(json.RawMessage) error) error {
	data, err := json.Marshal(body)
	if err != nil {
		return err
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, c.address+"/api/logs/tail", bytes.NewReader(data))
	if err != nil {
		return err
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Accept", "text/event-stream")
	// Streams must outlive the shared client's request timeout.
	stream := *c.http
	stream.Timeout = 0
	resp, err := stream.Do(req)
	if err != nil {
		return fmt.Errorf("request failed: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		data, _ := io.ReadAll(resp.Body)
		return responseError(resp, data)
	}

	scanner := bufio.NewScanner(resp.Body)
	scanner.Buffer(make([]byte, 64*1024), 16*1024*1024)
	var event string
	var payload strings.Builder
	for scanner.Scan() {
		line := scanner.Text()
		switch {
		case line == "":
			switch event {
			case "log":
				var tailEvent struct {
					Entry json.RawMessage `json:"entry"`
				}
				if err := json.Unmarshal([]byte(payload.String()), &tailEvent); err != nil {
					return fmt.Errorf("decode log event: %w", err)
				}
				if err := onLog(tailEvent.Entry); err != nil {
					return err
				}
			case "error":
				var errorEvent struct {
					Message string `json:"message"`
				}
				_ = json.Unmarshal([]byte(payload.String()), &errorEvent)
				return fmt.Errorf("log stream failed: %s", errorEvent.Message)
			}
			event = ""
			payload.Reset()
		case strings.HasPrefix(line, "event:"):
			event = strings.TrimSpace(strings.TrimPrefix(line, "event:"))
		case strings.HasPrefix(line, "data:"):
			if payload.Len() > 0 {
				payload.WriteByte('\n')
			}
			payload.WriteString(strings.TrimPrefix(strings.TrimPrefix(line, "data:"), " "))
		}
	}
	if ctx.Err() != nil {
		return nil
	}
	if err := scanner.Err(); err != nil {
		return err
	}
	return fmt.Errorf("log stream closed by server")
}

var logHeader = []string{"TIME", "STATUS", "DURATION", "PROVIDER", "MODEL", "TOKENS(IN/OUT)", "COST", "ID"}

// followRowFormat uses fixed widths, since rows are printed as they arrive and cannot be aligned as a batch.
const followRowFormat = "%-19s  %-6s  %-8s  %-12.12s  %-40.40s  %-14s  %-10s  %s\n"

func printFollowRow(w io.Writer, row []string) {
	args := make([]any, len(row))
	for i, v := range row {
		args[i] = v
	}
	fmt.Fprintf(w, followRowFormat, args...)
}

func logRow(raw json.RawMessage) []string {
	var e logEntry
	_ = json.Unmarshal(raw, &e)
	ts := e.CompletedAt
	if t, err := time.Parse(time.RFC3339Nano, e.CompletedAt); err == nil {
		ts = t.Local().Format("2006-01-02 15:04:05")
	}
	status := "-"
	if e.HTTPStatus != nil {
		status = fmt.Sprint(*e.HTTPStatus)
	} else if e.Error != nil {
		status = "error"
	}
	model := deref(e.GenAI.ResponseModel)
	if model == "-" {
		model = deref(e.GenAI.RequestModel)
	}
	tokens := "-"
	if e.Usage.InputTokens != nil || e.Usage.OutputTokens != nil {
		tokens = fmt.Sprintf("%s/%s", deref(e.Usage.InputTokens), deref(e.Usage.OutputTokens))
	}
	cost := "-"
	if e.Cost != nil {
		cost = fmt.Sprintf("$%.6f", *e.Cost)
	}
	return []string{ts, status, fmt.Sprintf("%dms", e.DurationMs), deref(e.GenAI.ProviderName), model, tokens, cost, e.ID}
}

func deref[T any](v *T) string {
	if v == nil {
		return "-"
	}
	return fmt.Sprint(*v)
}

func printRaw(w io.Writer, raw json.RawMessage, output string) error {
	switch output {
	case "json":
		var indented bytes.Buffer
		if err := json.Indent(&indented, raw, "", "  "); err != nil {
			return err
		}
		fmt.Fprintf(w, "%s\n", indented.Bytes())
		return nil
	case "yaml":
		data, err := yaml.JSONToYAML(raw)
		if err != nil {
			return err
		}
		fmt.Fprint(w, string(data))
		return nil
	default:
		return fmt.Errorf("output format %q not supported", output)
	}
}
