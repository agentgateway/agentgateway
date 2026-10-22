package standalone

import (
	"cmp"
	"encoding/json"
	"fmt"
	"net/http"
	"slices"
	"strings"
	"text/tabwriter"
	"time"

	"github.com/spf13/cobra"
)

type analyticsRow struct {
	Start            string         `json:"start"`
	Group            map[string]any `json:"group"`
	Requests         int64          `json:"requests"`
	Errors           int64          `json:"errors"`
	TotalTokens      int64          `json:"totalTokens"`
	Cost             float64        `json:"cost"`
	InputTokens      int64          `json:"inputTokens"`
	CacheReadTokens  int64          `json:"cacheReadTokens"`
	TTFTRequests     int64          `json:"ttftRequests"`
	TTFTMs           float64        `json:"ttftMs"`
	GenerationTokens int64          `json:"generationTokens"`
	GenerationMs     float64        `json:"generationMs"`
}

func (r *analyticsRow) add(o analyticsRow) {
	r.Requests += o.Requests
	r.Errors += o.Errors
	r.TotalTokens += o.TotalTokens
	r.Cost += o.Cost
	r.InputTokens += o.InputTokens
	r.CacheReadTokens += o.CacheReadTokens
	r.TTFTRequests += o.TTFTRequests
	r.TTFTMs += o.TTFTMs
	r.GenerationTokens += o.GenerationTokens
	r.GenerationMs += o.GenerationMs
}

func (r analyticsRow) cells() []string {
	ttft, tps, cache := "-", "-", "-"
	if r.TTFTRequests > 0 {
		ttft = time.Duration(r.TTFTMs / float64(r.TTFTRequests) * float64(time.Millisecond)).Round(time.Millisecond).String()
	}
	if r.GenerationMs > 0 {
		tps = fmt.Sprintf("%.0f", float64(r.GenerationTokens)/r.GenerationMs*1000)
	}
	if r.InputTokens > 0 {
		cache = fmt.Sprintf("%.1f%%", float64(r.CacheReadTokens)/float64(r.InputTokens)*100)
	}
	return []string{
		fmt.Sprint(r.Requests), fmt.Sprint(r.Errors), fmt.Sprint(r.TotalTokens), fmt.Sprintf("$%.6f", r.Cost),
		ttft, tps, cache,
	}
}

// groupByFields maps --by names to the server's groupBy fields. Any other name groups by that log attribute.
var groupByFields = map[string]string{
	"provider":          "provider",
	"model":             "requestModel",
	"response-model":    "responseModel",
	"status":            "httpStatus",
	"ttft":              "ttft",
	"tokens-per-second": "outputTokensPerSecond",
}

// histogramUnits labels histogram groups, whose values are bucket lower bounds.
var histogramUnits = map[string]string{
	"ttft":                  "ms+",
	"outputTokensPerSecond": " tok/s+",
}

func analyticsCommand(c *client) *cobra.Command {
	var (
		output   string
		by       []string
		since    time.Duration
		interval time.Duration
		filters  logFilters
	)
	cmd := &cobra.Command{
		Use:   "analytics",
		Short: "Summarize request counts, tokens, cost, and latency",
		Long: `Summarize request counts, tokens, cost, and latency from the request log database.

--by groups results by provider, model, response-model, status, ttft,
tokens-per-second, or any logged attribute such as agentgateway.user.
ttft and tokens-per-second group into histogram buckets labelled by their lower bound.
--interval breaks results down over time.`,
		Example: `  agctl standalone analytics
  agctl standalone analytics --by provider,model --since 168h
  agctl standalone analytics --by agentgateway.user --provider openai
  agctl standalone analytics --by model --since 6h --interval 1h
  agctl standalone analytics --by model,ttft`,
		Args:         cobra.NoArgs,
		SilenceUsage: true,
		RunE: func(cmd *cobra.Command, _ []string) error {
			from := time.Now().Add(-since)
			body := map[string]any{
				"timeRange": map[string]any{"from": from.UTC().Format(time.RFC3339)},
				"filters":   filters,
			}
			if interval > 0 {
				body["bucketSeconds"] = int64(interval.Seconds())
			} else {
				body["bucketCount"] = 1
			}
			groupBy := []map[string]string{}
			var keys []string
			for _, name := range by {
				if field, found := groupByFields[name]; found {
					groupBy = append(groupBy, map[string]string{"field": field})
					keys = append(keys, field)
				} else {
					groupBy = append(groupBy, map[string]string{"field": "attributes", "key": name})
					keys = append(keys, name)
				}
			}
			body["groupBy"] = groupBy

			var raw json.RawMessage
			if err := c.do(cmd.Context(), http.MethodPost, "/api/logs/analytics/summary", body, &raw); err != nil {
				return err
			}
			if output != "table" {
				return printRaw(cmd.OutOrStdout(), raw, output)
			}
			var result struct {
				Buckets []analyticsRow `json:"buckets"`
				Groups  []analyticsRow `json:"groups"`
			}
			if err := json.Unmarshal(raw, &result); err != nil {
				return fmt.Errorf("decode response: %w", err)
			}

			rows := result.Buckets
			if interval == 0 {
				rows = result.Groups
				slices.SortStableFunc(rows, func(a, b analyticsRow) int {
					return cmp.Or(cmp.Compare(b.Cost, a.Cost), cmp.Compare(b.Requests, a.Requests))
				})
			}
			var header []string
			if interval > 0 {
				header = append(header, "TIME")
			}
			for _, name := range by {
				header = append(header, strings.ToUpper(name))
			}
			metrics := []string{"REQUESTS", "ERRORS", "TOKENS", "COST", "AVG TTFT", "TOK/S", "CACHE HIT"}
			header = append(header, metrics...)

			tw := tabwriter.NewWriter(cmd.OutOrStdout(), 0, 4, 2, ' ', 0)
			fmt.Fprintln(tw, strings.Join(header, "\t"))
			var total analyticsRow
			for _, r := range rows {
				total.add(r)
				if len(by) == 0 && interval == 0 {
					continue
				}
				var cells []string
				if interval > 0 {
					start := r.Start
					if t, err := time.Parse(time.RFC3339Nano, r.Start); err == nil {
						start = t.Local().Format("2006-01-02 15:04")
					}
					cells = append(cells, start)
				}
				for _, key := range keys {
					value := "-"
					if v := r.Group[key]; v != nil {
						value = fmt.Sprint(v) + histogramUnits[key]
					}
					cells = append(cells, value)
				}
				cells = append(cells, r.cells()...)
				fmt.Fprintln(tw, strings.Join(cells, "\t"))
			}
			if len(rows) != 1 || len(by) == 0 {
				cells := make([]string, len(header)-len(metrics), len(header))
				if len(cells) > 0 {
					cells[0] = "TOTAL"
				}
				cells = append(cells, total.cells()...)
				fmt.Fprintln(tw, strings.Join(cells, "\t"))
			}
			return tw.Flush()
		},
	}
	cmd.Flags().StringVarP(&output, "output", "o", "table", "Output format: table, yaml, or json")
	cmd.Flags().StringSliceVar(&by, "by", nil, "Group by provider, model, response-model, status, ttft, tokens-per-second, or a log attribute (repeatable)")
	cmd.Flags().DurationVar(&since, "since", 24*time.Hour, "Summarize requests newer than a relative duration")
	cmd.Flags().DurationVar(&interval, "interval", 0, "Break results down into buckets of this duration")
	filters.addFlags(cmd)
	_ = cmd.RegisterFlagCompletionFunc("output", cobra.FixedCompletions([]string{"table", "yaml", "json"}, cobra.ShellCompDirectiveNoFileComp))
	_ = cmd.RegisterFlagCompletionFunc("by", cobra.FixedCompletions([]string{
		"provider", "model", "response-model", "status", "ttft", "tokens-per-second", "agentgateway.user", "agentgateway.group", "user_agent.name",
	}, cobra.ShellCompDirectiveNoFileComp))
	return cmd
}
