package costs

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"maps"
	"net/http"
	"sort"
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
		return modelsDevTransform(api, modelsDevSelectProviders(api, opts.providers), opts.legacy)
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
	"cohere":       "cohere",
	"baseten":      "baseten",
	"cerebras":     "cerebras",
	"deepinfra":    "deepinfra",
	"deepseek":     "deepseek",
	"groq":         "groq",
	"huggingface":  "huggingface",
	"mistral":      "mistral",
	"openrouter":   "openrouter",
	"togetherai":   "togetherai",
	"xai":          "xai",
	"fireworks-ai": "fireworks",
}

type modelsDevProvider struct {
	ID     string                    `json:"id"`
	Name   string                    `json:"name"`
	Models map[string]modelsDevModel `json:"models"`
}

type modelsDevModel struct {
	ID     string         `json:"id"`
	Name   string         `json:"name"`
	Status string         `json:"status"`
	Cost   *modelsDevCost `json:"cost"`
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
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, "https://models.dev/api.json", nil)
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

func modelsDevSelectProviders(api map[string]modelsDevProvider, requested []string) []string {
	if len(requested) > 0 {
		return requested
	}
	ids := make([]string, 0, len(modelsDevProviderIDs))
	for id := range modelsDevProviderIDs {
		if _, ok := api[id]; ok {
			ids = append(ids, id)
		}
	}
	sort.Strings(ids)
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
			if entry.IsZero() {
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
	var entry Model

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

	return entry, nil
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
	sort.Slice(tiers, func(i, j int) bool { return tiers[i].ContextOver < tiers[j].ContextOver })
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
