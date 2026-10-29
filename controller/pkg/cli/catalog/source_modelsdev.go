package catalog

import (
	"cmp"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"maps"
	"net/http"
	"slices"
	"strings"
	"time"

	"github.com/shopspring/decimal"
)

var modelsDevSourceName = "models.dev"

func init() {
	importSources[modelsDevSourceName] = func(ctx context.Context, opts importOptions) (*ModelCatalog, []string, error) {
		api, err := modelsDevFetchAPI(ctx)
		if err != nil {
			return nil, nil, err
		}
		return modelsDevTransform(api, modelsDevSelectProviders(api, opts.providers, opts.excludeProviders), opts.legacy)
	}
}

var modelsDevProviderIDs = map[string]string{
	// Native providers, each backed by a dedicated gateway provider implementation.
	"openai":         "openai",
	"anthropic":      "anthropic",
	"amazon-bedrock": "aws.bedrock",
	"google":         "gcp.gemini",
	"google-vertex":  "gcp.vertex_ai",
	"azure":          "azure",
	"github-copilot": "copilot",
	// TODO: not yet exposed at cost time — proxy resolves these custom-backed providers as "custom".
	"cohere":           "cohere",
	"baseten":          "baseten",
	"cerebras":         "cerebras",
	"deepinfra":        "deepinfra",
	"deepseek":         "deepseek",
	"groq":             "groq",
	"huggingface":      "huggingface",
	"mistral":          "mistral",
	"openrouter":       "openrouter",
	"togetherai":       "togetherai",
	"xai":              "xai",
	"fireworks-ai":     "fireworks",
	"meta":             "meta",
	"perplexity-agent": "perplexity",
	"nebius":           "nebius",
}

type modelsDevProvider struct {
	ID     string                    `json:"id"`
	Name   string                    `json:"name"`
	Models map[string]modelsDevModel `json:"models"`
}

type modelsDevModel struct {
	ID               string                     `json:"id"`
	Name             string                     `json:"name"`
	Family           string                     `json:"family"`
	Status           string                     `json:"status"`
	ReleaseDate      string                     `json:"release_date"`
	Cost             *modelsDevCost             `json:"cost"`
	Reasoning        *bool                      `json:"reasoning"`
	ReasoningOptions []modelsDevReasoningOption `json:"reasoning_options"`
	ToolCall         *bool                      `json:"tool_call"`
	StructuredOutput *bool                      `json:"structured_output"`
	Temperature      *bool                      `json:"temperature"`
	Limit            Limits                     `json:"limit"`
	Modalities       Modalities                 `json:"modalities"`
	Experimental     struct {
		Modes map[string]modelsDevMode `json:"modes"`
	} `json:"experimental"`
}

type modelsDevReasoningOption struct {
	Type   string    `json:"type"`
	Values []*string `json:"values"`
	Min    *int64    `json:"min"`
	Max    *int64    `json:"max"`
}

type modelsDevMode struct {
	Cost     *modelsDevRates `json:"cost"`
	Provider struct {
		Body struct {
			ServiceTier string `json:"service_tier"`
		} `json:"body"`
	} `json:"provider"`
}

type modelsDevRates struct {
	Input       json.Number `json:"input"`
	Output      json.Number `json:"output"`
	CacheRead   json.Number `json:"cache_read"`
	CacheWrite  json.Number `json:"cache_write"`
	Reasoning   json.Number `json:"reasoning"`
	InputAudio  json.Number `json:"input_audio"`
	OutputAudio json.Number `json:"output_audio"`
}

type modelsDevCost struct {
	modelsDevRates
	Tiers []modelsDevTier `json:"tiers"`
}

type modelsDevTier struct {
	modelsDevRates
	Tier modelsDevTierKind `json:"tier"`
}

type modelsDevTierKind struct {
	Type string `json:"type"`
	Size uint64 `json:"size"`
}

func modelsDevFetchAPI(ctx context.Context) (map[string]modelsDevProvider, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, "https://models.dev/api.json?type=all", nil)
	if err != nil {
		return nil, fmt.Errorf("build request for models.dev api.json: %w", err)
	}
	client := &http.Client{Timeout: 30 * time.Second}
	resp, err := client.Do(req)
	if err != nil {
		return nil, fmt.Errorf("fetch models.dev api.json: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("fetch models.dev api.json: unexpected status %d", resp.StatusCode)
	}
	return modelsDevDecodeAPI(io.LimitReader(resp.Body, 64<<20))
}

func modelsDevSelectProviders(api map[string]modelsDevProvider, requested, excluded []string) []string {
	selected := requested
	if len(selected) == 0 {
		selected = make([]string, 0, len(modelsDevProviderIDs))
		for id := range modelsDevProviderIDs {
			if _, ok := api[id]; ok {
				selected = append(selected, id)
			}
		}
	}
	omit := make(map[string]struct{}, len(excluded))
	for _, id := range excluded {
		omit[id] = struct{}{}
	}
	ids := make([]string, 0, len(selected))
	for _, id := range selected {
		if _, ok := omit[id]; !ok {
			ids = append(ids, id)
		}
	}
	slices.Sort(ids)
	return ids
}

func modelsDevDecodeAPI(r io.Reader) (map[string]modelsDevProvider, error) {
	var api map[string]modelsDevProvider
	if err := json.NewDecoder(r).Decode(&api); err != nil {
		return nil, fmt.Errorf("decode api.json: %w", err)
	}
	return api, nil
}

func modelsDevMapProviderID(sourceID string) (string, bool) {
	gatewayID, ok := modelsDevProviderIDs[sourceID]
	return gatewayID, ok
}

func modelsDevTransform(api map[string]modelsDevProvider, providers []string, legacy bool) (*ModelCatalog, []string, error) {
	cat := &ModelCatalog{Providers: map[string]Provider{}}
	warns := []string{}
	warn := func(format string, args ...any) { warns = append(warns, fmt.Sprintf(format, args...)) }
	matchedProvider := false

	for _, srcID := range providers {
		gwID, supported := modelsDevMapProviderID(srcID)
		if !supported {
			warn("provider %q is not supported by the proxy; skipping", srcID)
			continue
		}
		src, ok := api[srcID]
		if !ok {
			warn("provider %q not found in source", srcID)
			continue
		}
		matchedProvider = true
		models := map[string]Model{}
		for modelID, m := range src.Models {
			if m.Status == "deprecated" && !legacy {
				continue
			}
			entry, err := modelsDevBuildModel(gwID, modelID, m, warn)
			if err != nil {
				return nil, warns, fmt.Errorf("%s/%s: %w", gwID, modelID, err)
			}
			// Metadata alone does not earn a catalog entry; require pricing or tags.
			if entry.Rates.IsZero() && len(entry.Tiers) == 0 && len(entry.Tags) == 0 {
				continue
			}
			models[modelID] = entry
		}
		if len(models) == 0 {
			continue
		}
		// Union rather than overwrite: a gateway id may be the target of more than one source id.
		if existing, ok := cat.Providers[gwID]; ok {
			maps.Copy(existing.Models, models)
		} else {
			cat.Providers[gwID] = Provider{Models: models}
		}
	}

	if len(cat.Providers) == 0 {
		if matchedProvider {
			return nil, warns, fmt.Errorf("no importable models matched %v", providers)
		}
		return nil, warns, fmt.Errorf("no providers matched %v", providers)
	}
	return cat, warns, nil
}

func modelsDevBuildModel(provider, model string, m modelsDevModel, warn func(format string, args ...any)) (Model, error) {
	label := provider + "/" + model
	entry := Model{
		Name:     m.Name,
		Status:   m.Status,
		Released: m.ReleaseDate,
		Limits:   m.Limit,
		Capabilities: Capabilities{
			Reasoning:        m.Reasoning,
			StructuredOutput: m.StructuredOutput,
			ToolCall:         m.ToolCall,
			Temperature:      m.Temperature,
		},
		Modalities: m.Modalities,
	}
	for _, option := range m.ReasoningOptions {
		switch option.Type {
		case "toggle":
			entry.Reasoning.Toggle = new(true)
		case "effort":
			for _, v := range option.Values {
				if v != nil {
					entry.Reasoning.Effort = append(entry.Reasoning.Effort, *v)
				}
			}
		case "budget_tokens":
			budget := &BudgetTokens{}
			// Some sources use -1 as a sentinel for a dynamic budget; treat it as unset.
			if option.Min != nil && *option.Min >= 0 {
				budget.Min = new(uint64(*option.Min))
			}
			if option.Max != nil && *option.Max >= 0 {
				budget.Max = new(uint64(*option.Max))
			}
			entry.Reasoning.BudgetTokens = budget
		default:
			warn("%s: skipping unsupported reasoning option %q", label, option.Type)
		}
	}
	claude := strings.HasPrefix(m.Family, "claude-")
	if claude {
		var effort, budgetTokens bool
		for _, option := range m.ReasoningOptions {
			switch option.Type {
			case "effort":
				effort = true
			case "budget_tokens":
				budgetTokens = true
			}
		}
		if budgetTokens {
			entry.Tags = append(entry.Tags, "legacy_thinking")
		} else if effort {
			entry.Tags = append(entry.Tags, "adaptive_thinking")
		}
	}

	if m.Cost != nil {
		rates, err := modelsDevBuildRates(&m.Cost.modelsDevRates, label)
		if err != nil {
			return Model{}, err
		}
		entry.Rates = rates

		tiers, err := modelsDevBuildTiers(m.Cost.Tiers, label, warn)
		if err != nil {
			return Model{}, err
		}
		entry.Tiers = tiers
	}

	// Only modes selected by an OpenAI-style service_tier map onto our service tiers.
	for _, name := range slices.Sorted(maps.Keys(m.Experimental.Modes)) {
		mode := m.Experimental.Modes[name]
		serviceTier := mode.Provider.Body.ServiceTier
		if mode.Cost == nil || (serviceTier != "priority" && serviceTier != "flex") {
			continue
		}
		rates, err := modelsDevBuildRates(mode.Cost, label+" "+name+" mode")
		if err != nil {
			return Model{}, err
		}
		entry.Tiers = append(entry.Tiers, Tier{ServiceTier: serviceTier, Rates: rates})
	}

	// models.dev does not publish extended-TTL cache write rates, so derive them from input.
	derive := func(r *Rates) {
		ttl, multiplier := "", ""
		switch {
		case claude:
			// Anthropic bills 1h writes at 2x input.
			ttl, multiplier = "1h", "2"
		case provider == "openai":
			// OpenAI bills 12h writes at 2.5x input.
			ttl, multiplier = "12h", "2.5"
		default:
			return
		}
		if rate := deriveCacheWrite(r, multiplier); rate != "" {
			r.CacheWriteDetails = map[string]Money{ttl: rate}
		}
	}
	derive(&entry.Rates)
	for i := range entry.Tiers {
		derive(&entry.Tiers[i].Rates)
	}

	return entry, nil
}

// deriveCacheWrite scales input by multiplier. Anthropic and OpenAI both price default-TTL writes at
// 1.25x input; only derive when cacheWrite follows that rule, so a source with a different pricing
// scheme is left to fall back to cacheWrite.
func deriveCacheWrite(r *Rates, multiplier string) Money {
	if r.CacheWrite == "" {
		return ""
	}
	input, err := r.Input.Decimal()
	if err != nil {
		return ""
	}
	cacheWrite, err := r.CacheWrite.Decimal()
	if err != nil || !cacheWrite.Equal(input.Mul(decimal.RequireFromString("1.25"))) {
		return ""
	}
	return Money(input.Mul(decimal.RequireFromString(multiplier)).String())
}

func modelsDevBuildTiers(src []modelsDevTier, label string, warn func(format string, args ...any)) ([]Tier, error) {
	var tiers []Tier
	for _, t := range src {
		if t.Tier.Type != "context" {
			warn("%s: skipping unsupported tier type %q", label, t.Tier.Type)
			continue
		}
		if t.Tier.Size == 0 {
			warn("%s: skipping context tier with zero threshold", label)
			continue
		}
		rates, err := modelsDevBuildRates(&t.modelsDevRates, label+" tier")
		if err != nil {
			return nil, err
		}
		tiers = append(tiers, Tier{ContextOver: t.Tier.Size, Rates: rates})
	}
	slices.SortFunc(tiers, func(a, b Tier) int { return cmp.Compare(a.ContextOver, b.ContextOver) })
	return tiers, nil
}

func modelsDevBuildRates(src *modelsDevRates, label string) (Rates, error) {
	var r Rates
	for _, f := range []struct {
		dst  *Money
		src  json.Number
		name string
	}{
		{&r.Input, src.Input, "input"},
		{&r.Output, src.Output, "output"},
		{&r.CacheRead, src.CacheRead, "cacheRead"},
		{&r.CacheWrite, src.CacheWrite, "cacheWrite"},
		{&r.Reasoning, src.Reasoning, "reasoning"},
		{&r.InputAudio, src.InputAudio, "inputAudio"},
		{&r.OutputAudio, src.OutputAudio, "outputAudio"},
	} {
		m, err := modelsDevNormalizeMoney(f.src, label+" "+f.name)
		if err != nil {
			return Rates{}, err
		}
		*f.dst = m
	}
	return r, nil
}

func modelsDevNormalizeMoney(n json.Number, label string) (Money, error) {
	s := n.String()
	if s == "" {
		return "", nil
	}
	d, err := decimal.NewFromString(s)
	if err != nil {
		return "", fmt.Errorf("rate %s %q: %w", label, s, err)
	}
	if d.IsNegative() {
		return "", fmt.Errorf("rate %s is negative: %q", label, s)
	}
	if d.Exponent() < -maxFractionalDigits {
		d = d.Round(maxFractionalDigits)
	}
	return Money(d.String()), nil
}
