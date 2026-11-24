package standalone

import (
	"encoding/json"
	"fmt"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"text/tabwriter"
	"time"

	"github.com/spf13/cobra"
)

type apiKey struct {
	Key           string         `json:"key,omitempty"`
	KeyHash       string         `json:"keyHash,omitempty"`
	Metadata      map[string]any `json:"metadata,omitempty"`
	AllowedModels []string       `json:"allowedModels,omitempty"`
	Budgets       []budget       `json:"budgets,omitempty"`
}

type budget struct {
	Name  string `json:"name"`
	Limit struct {
		Unit   string  `json:"unit"`
		Amount float64 `json:"amount"`
	} `json:"limit"`
	Window struct {
		Rolling string `json:"rolling"`
	} `json:"window"`
	OnBudgetExceeded string `json:"onBudgetExceeded"`
}

func keysCommand(c *client) *cobra.Command {
	cmd := &cobra.Command{
		Use:          "keys",
		Short:        "List and create LLM API keys",
		Args:         cobra.NoArgs,
		SilenceUsage: true,
		RunE: func(cmd *cobra.Command, _ []string) error {
			var result resourceList
			if err := c.do(cmd.Context(), http.MethodGet, "/api/config/resources/llm.apiKey", nil, &result); err != nil {
				return err
			}
			tw := tabwriter.NewWriter(cmd.OutOrStdout(), 0, 4, 2, ' ', 0)
			fmt.Fprintln(tw, "NAME\tKEY\tMODELS\tBUDGETS\tCREATED\tID")
			for _, r := range result.Resources {
				var key apiKey
				_ = json.Unmarshal(r.Value, &key)
				hint := "****"
				if key.Key != "" {
					hint = key.Key[:min(7, len(key.Key))] + "..." + key.Key[max(0, len(key.Key)-4):]
				} else if h, ok := key.Metadata["agentgateway.dev/keyHint"].(string); ok && h != "" {
					hint = h
				}
				models := "*"
				if key.AllowedModels != nil {
					models = strings.Join(key.AllowedModels, ",")
				}
				budgets := "-"
				if len(key.Budgets) > 0 {
					var parts []string
					for _, b := range key.Budgets {
						parts = append(parts, formatBudgetAmount(b.Limit.Unit, strconv.FormatFloat(b.Limit.Amount, 'f', -1, 64))+"/"+b.Window.Rolling)
					}
					budgets = strings.Join(parts, ",")
				}
				created := "-"
				if ts, ok := key.Metadata["agentgateway.dev/createdAt"].(float64); ok {
					created = time.Unix(int64(ts), 0).Local().Format("2006-01-02 15:04")
				}
				fmt.Fprintf(tw, "%s\t%s\t%s\t%s\t%s\t%s\n", resourceName(r), hint, models, budgets, created, r.ID)
			}
			return tw.Flush()
		},
	}
	cmd.AddCommand(keysCreateCommand(c))
	return cmd
}

func keysCreateCommand(c *client) *cobra.Command {
	var (
		models   []string
		metadata string
	)
	cmd := &cobra.Command{
		Use:   "create NAME",
		Short: "Create an API key and print it",
		Long: `Create an API key and print it. The key is stored as a SHA-256 hash and
cannot be retrieved again.`,
		Example: `  agctl standalone keys create alice
  agctl standalone keys create ci --model 'openai/*' --metadata '{"team": "platform"}'`,
		Args:         cobra.ExactArgs(1),
		SilenceUsage: true,
		RunE: func(cmd *cobra.Command, args []string) error {
			name := args[0]
			// The gateway generates the secret for a key sent without one.
			key := apiKey{
				Metadata:      map[string]any{},
				AllowedModels: models,
			}
			if metadata != "" {
				if err := json.Unmarshal([]byte(metadata), &key.Metadata); err != nil || key.Metadata == nil {
					return fmt.Errorf("--metadata must be a JSON object")
				}
			}
			key.Metadata["name"] = name
			body := map[string]any{"resources": []map[string]any{{"value": key}}}
			var result resourceList
			if err := c.do(cmd.Context(), http.MethodPut, "/api/config/resources/llm.apiKey", body, &result); err != nil {
				return err
			}
			var created apiKey
			if len(result.Resources) == 1 {
				_ = json.Unmarshal(result.Resources[0].Value, &created)
			}
			if created.Key == "" {
				return fmt.Errorf("the gateway did not return a generated key")
			}
			fmt.Fprintln(cmd.OutOrStdout(), created.Key)
			fmt.Fprintf(cmd.ErrOrStderr(), "API key %q created. Save it now; it cannot be shown again.\n", name)
			return nil
		},
	}
	cmd.Flags().StringSliceVar(&models, "model", nil, "Model pattern the key may use (repeatable; default all)")
	cmd.Flags().StringVar(&metadata, "metadata", "", `Metadata to attach to the key, as a JSON object, for example '{"team": "platform"}'`)
	return cmd
}

func budgetsCommand(c *client) *cobra.Command {
	var keyName string
	cmd := &cobra.Command{
		Use:          "budgets",
		Short:        "Show API key budget usage",
		Args:         cobra.NoArgs,
		SilenceUsage: true,
		RunE: func(cmd *cobra.Command, _ []string) error {
			path := "/api/budgets/status"
			if keyName != "" {
				path += "?apiKeyName=" + url.QueryEscape(keyName)
			}
			var result struct {
				Budgets []struct {
					APIKeyName string `json:"apiKeyName"`
					Name       string `json:"name"`
					Limit      struct {
						Unit   string `json:"unit"`
						Amount string `json:"amount"`
					} `json:"limit"`
					Usage struct {
						Used      string `json:"used"`
						Remaining string `json:"remaining"`
						Exceeded  bool   `json:"exceeded"`
					} `json:"usage"`
					Window struct {
						// End is null for lifetime budgets, which never reset.
						End *int64 `json:"end"`
					} `json:"window"`
					OnBudgetExceeded string `json:"onBudgetExceeded"`
				} `json:"budgets"`
			}
			if err := c.do(cmd.Context(), http.MethodGet, path, nil, &result); err != nil {
				return err
			}
			tw := tabwriter.NewWriter(cmd.OutOrStdout(), 0, 4, 2, ' ', 0)
			fmt.Fprintln(tw, "KEY\tBUDGET\tUSED\tLIMIT\tREMAINING\tRESETS\tSTATUS\tACTION")
			for _, b := range result.Budgets {
				status := "ok"
				if b.Usage.Exceeded {
					status = "exceeded"
				}
				resets := "never"
				if b.Window.End != nil {
					resets = time.UnixMilli(*b.Window.End).Local().Format("2006-01-02 15:04")
				}
				fmt.Fprintf(tw, "%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n",
					b.APIKeyName, b.Name,
					formatBudgetAmount(b.Limit.Unit, b.Usage.Used),
					formatBudgetAmount(b.Limit.Unit, b.Limit.Amount),
					formatBudgetAmount(b.Limit.Unit, b.Usage.Remaining),
					resets, status, strings.ToLower(b.OnBudgetExceeded))
			}
			return tw.Flush()
		},
	}
	cmd.Flags().StringVar(&keyName, "key", "", "Only show budgets for this API key name")
	return cmd
}

func formatBudgetAmount(unit, amount string) string {
	if unit == "USD" {
		return "$" + amount
	}
	return amount + " tokens"
}
