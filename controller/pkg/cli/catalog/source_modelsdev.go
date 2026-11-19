package catalog

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"maps"
	"net/http"
	"slices"
	"strings"
	"time"
)

var modelsDevSourceName = "models.dev"

func init() {
	importSources[modelsDevSourceName] = func(ctx context.Context, opts importOptions) (*ModelCatalog, []string, error) {
		api, err := modelsDevFetchAPI(ctx)
		if err != nil {
			return nil, nil, err
		}
		return modelsDevTransform(api, modelsDevSelectProviders(api, opts.providers, opts.excludeProviders), opts.legacy, time.Now().UTC().Format(time.DateOnly))
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
	"typesafe":       "typesafe",
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
	"wandb":            "coreweave",
}

// modelsDevTypeEndpoints maps a models.dev model type to the client endpoints serving it.
var modelsDevTypeEndpoints = map[string][]string{
	"embedding":     {"/embeddings"},
	"reranking":     {"/rerank"},
	"image":         {"/images/generations", "/images/edits"},
	"speech":        {"/audio/speech"},
	"transcription": {"/audio/transcriptions"},
}

type modelsDevProvider struct {
	Models map[string]modelsDevModel `json:"models"`
}

type modelsDevModel struct {
	Name         string                     `json:"name"`
	Family       string                     `json:"family"`
	Type         string                     `json:"type"`
	Status       string                     `json:"status"`
	ReleaseDate  string                     `json:"release_date"`
	Cost         *modelsDevCost             `json:"cost"`
	Limit        Limits                     `json:"limit"`
	Modalities   Modalities                 `json:"modalities"`
	API          map[string]json.RawMessage `json:"api"`
	Capabilities struct {
		Tools struct {
			Supported *bool `json:"supported"`
		} `json:"tools"`
		Reasoning struct {
			Supported *bool    `json:"supported"`
			Toggle    *bool    `json:"toggle"`
			Effort    []string `json:"effort"`
			Budget    *struct {
				Min *int64 `json:"min"`
				Max *int64 `json:"max"`
			} `json:"budget"`
		} `json:"reasoning"`
		StructuredOutput *bool `json:"structured_output"`
		Temperature      *bool `json:"temperature"`
	} `json:"capabilities"`
	Experimental struct {
		Modes map[string]modelsDevMode `json:"modes"`
	} `json:"experimental"`
}

type modelsDevMode struct {
	Cost        *modelsDevCost `json:"cost"`
	ServiceTier string         `json:"service_tier"`
	Body        struct {
		ServiceTier string `json:"service_tier"`
		Speed       string `json:"speed"`
	} `json:"body"`
}

func modelsDevFetchAPI(ctx context.Context) (map[string]modelsDevProvider, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, "https://models.dev/experimental/v2.0/api.json", nil)
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

func modelsDevTransform(api map[string]modelsDevProvider, providers []string, legacy bool, asOf string) (*ModelCatalog, []string, error) {
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
		for _, modelID := range slices.Sorted(maps.Keys(src.Models)) {
			m := src.Models[modelID]
			if m.Status == "deprecated" && !legacy {
				continue
			}
			entry, err := modelsDevBuildModel(gwID, modelID, m, asOf, warn)
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

func modelsDevBuildModel(provider, model string, m modelsDevModel, asOf string, warn func(string, ...any)) (Model, error) {
	reasoning := m.Capabilities.Reasoning
	entry := Model{
		Name:     m.Name,
		Status:   m.Status,
		Released: m.ReleaseDate,
		Limits:   m.Limit,
		Capabilities: Capabilities{
			Reasoning:        reasoning.Supported,
			StructuredOutput: m.Capabilities.StructuredOutput,
			ToolCall:         m.Capabilities.Tools.Supported,
			Temperature:      m.Capabilities.Temperature,
		},
		Modalities: Modalities{
			Input:  modelsDevModalities(m.Modalities.Input),
			Output: modelsDevModalities(m.Modalities.Output),
		},
		Reasoning: Reasoning{Toggle: reasoning.Toggle, Effort: reasoning.Effort},
	}
	if m.Type != "" && m.Type != "chat" {
		entry.Endpoints = modelsDevTypeEndpoints[m.Type]
		if m.Type == "decision" {
			if _, ok := m.API["systemone"]; ok {
				entry.Endpoints = []string{"/systemone"}
			}
		}
		if _, realtime := m.API["realtime"]; m.Type == "transcription" && realtime {
			if _, rest := m.API["transcriptions"]; !rest {
				entry.Endpoints = nil
			}
		}
		if entry.Endpoints == nil {
			entry.Endpoints = []string{}
		}
	}
	if budget := reasoning.Budget; budget != nil {
		entry.Reasoning.BudgetTokens = &BudgetTokens{}
		if budget.Min != nil && *budget.Min >= 0 {
			entry.Reasoning.BudgetTokens.Min = new(uint64(*budget.Min))
		}
		if budget.Max != nil && *budget.Max >= 0 {
			entry.Reasoning.BudgetTokens.Max = new(uint64(*budget.Max))
		}
	}
	claude := strings.HasPrefix(m.Family, "claude-")
	if claude {
		if reasoning.Budget != nil {
			entry.Tags = append(entry.Tags, "legacy_thinking")
		} else if len(reasoning.Effort) != 0 {
			entry.Tags = append(entry.Tags, "adaptive_thinking")
		}
	}
	rates, tiers, err := modelsDevPricing(m.Cost, m.Experimental.Modes, provider, claude, provider+"/"+model, asOf, warn)
	entry.Rates, entry.Tiers = rates, tiers
	return entry, err
}

func modelsDevModalities(src []string) []string {
	var out []string
	for _, modality := range src {
		switch {
		case modality == "application/pdf":
			modality = "pdf"
		case strings.HasPrefix(modality, "image/"), strings.HasPrefix(modality, "audio/"), strings.HasPrefix(modality, "video/"), strings.HasPrefix(modality, "text/"):
			modality, _, _ = strings.Cut(modality, "/")
		}
		if !slices.Contains(out, modality) {
			out = append(out, modality)
		}
	}
	return out
}
