package standalone

import (
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math/big"
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
		models  []string
		budgets []string
		action  string
	)
	cmd := &cobra.Command{
		Use:   "create NAME",
		Short: "Create an API key and print it",
		Long: `Create an API key and print it. The key is stored as a SHA-256 hash and
cannot be retrieved again.

Budgets are written as AMOUNT[/WINDOW], where AMOUNT is in USD (10usd or $10)
or tokens (1000000tokens), and WINDOW defaults to 30d.`,
		Example: `  agctl standalone keys create alice
  agctl standalone keys create ci --model 'openai/*' --budget 5usd/24h --budget-action block`,
		Args:         cobra.ExactArgs(1),
		SilenceUsage: true,
		RunE: func(cmd *cobra.Command, args []string) error {
			name := args[0]
			var existing resourceList
			if err := c.do(cmd.Context(), http.MethodGet, "/api/config/resources/llm.apiKey", nil, &existing); err != nil {
				return err
			}
			for _, r := range existing.Resources {
				if strings.EqualFold(resourceName(r), name) {
					return fmt.Errorf("API key %q already exists", name)
				}
			}
			raw, err := generateKey()
			if err != nil {
				return err
			}
			hash := sha256.Sum256([]byte(raw))
			key := apiKey{
				KeyHash: "sha256:" + hex.EncodeToString(hash[:]),
				Metadata: map[string]any{
					"name":                     name,
					"agentgateway.dev/keyHint": raw[:7] + "..." + raw[len(raw)-4:],
				},
				AllowedModels: models,
			}
			for _, spec := range budgets {
				b, err := parseBudget(spec, action)
				if err != nil {
					return err
				}
				key.Budgets = append(key.Budgets, b)
			}
			body := map[string]any{"resources": []map[string]any{{"value": key}}}
			if err := c.do(cmd.Context(), http.MethodPut, "/api/config/resources/llm.apiKey", body, nil); err != nil {
				return err
			}
			fmt.Fprintln(cmd.OutOrStdout(), raw)
			fmt.Fprintf(cmd.ErrOrStderr(), "API key %q created. Save it now; it cannot be shown again.\n", name)
			return nil
		},
	}
	cmd.Flags().StringSliceVar(&models, "model", nil, "Model pattern the key may use (repeatable; default all)")
	cmd.Flags().StringArrayVar(&budgets, "budget", nil, "Budget as AMOUNT[/WINDOW], for example 10usd/30d or 1000000tokens/24h (repeatable)")
	cmd.Flags().StringVar(&action, "budget-action", "audit", "Action when a budget is exceeded: audit or block")
	_ = cmd.RegisterFlagCompletionFunc("budget-action", cobra.FixedCompletions([]string{"audit", "block"}, cobra.ShellCompDirectiveNoFileComp))
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
						End int64 `json:"end"`
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
				resets := time.UnixMilli(b.Window.End).Local().Format("2006-01-02 15:04")
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

func generateKey() (string, error) {
	const alphabet = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
	var sb strings.Builder
	sb.WriteString("agw_sk_")
	for range 32 {
		n, err := rand.Int(rand.Reader, big.NewInt(int64(len(alphabet))))
		if err != nil {
			return "", err
		}
		sb.WriteByte(alphabet[n.Int64()])
	}
	return sb.String(), nil
}

func parseBudget(spec, action string) (budget, error) {
	var b budget
	amount, window, _ := strings.Cut(spec, "/")
	if window == "" {
		window = "30d"
	}
	lower := strings.ToLower(strings.TrimSpace(amount))
	switch {
	case strings.HasPrefix(lower, "$"):
		b.Limit.Unit, lower = "USD", strings.TrimPrefix(lower, "$")
	case strings.HasSuffix(lower, "usd"):
		b.Limit.Unit, lower = "USD", strings.TrimSuffix(lower, "usd")
	case strings.HasSuffix(lower, "tokens"):
		b.Limit.Unit, lower = "Tokens", strings.TrimSuffix(lower, "tokens")
	default:
		return b, fmt.Errorf("budget %q: amount must end in usd or tokens, or start with $", spec)
	}
	value, err := strconv.ParseFloat(strings.TrimSpace(lower), 64)
	if err != nil || value < 0 || (b.Limit.Unit == "Tokens" && value != float64(int64(value))) {
		return b, fmt.Errorf("budget %q: invalid amount", spec)
	}
	switch strings.ToLower(action) {
	case "audit":
		b.OnBudgetExceeded = "Audit"
	case "block":
		b.OnBudgetExceeded = "Block"
	default:
		return b, fmt.Errorf("--budget-action must be audit or block")
	}
	b.Limit.Amount = value
	b.Window.Rolling = window
	b.Name = strings.ToLower(b.Limit.Unit) + "-" + window
	return b, nil
}

func formatBudgetAmount(unit, amount string) string {
	if unit == "USD" {
		return "$" + amount
	}
	return amount + " tokens"
}
