package costs

import (
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/agentgateway/agentgateway/controller/test/testutils"
)

func TestSelectModelsDevProviders(t *testing.T) {
	api := sampleAPI()

	t.Run("explicit", func(t *testing.T) {
		got := modelsDevSelectProviders(api, []string{"google", "openai"})
		want := []string{"google", "openai"}
		if !reflect.DeepEqual(got, want) {
			t.Fatalf("modelsDevSelectProviders(..., explicit) = %v, want %v", got, want)
		}
	})

	t.Run("supported only by default", func(t *testing.T) {
		got := modelsDevSelectProviders(api, nil)
		want := []string{"anthropic", "google", "openai"}
		if !reflect.DeepEqual(got, want) {
			t.Fatalf("modelsDevSelectProviders(..., nil) = %v, want %v", got, want)
		}
	})
}

func sampleAPI() map[string]modelsDevProvider {
	return map[string]modelsDevProvider{
		"openai": {ID: "openai", Models: map[string]modelsDevModel{
			"chatgpt-image-latest": {},
			"gpt-4": {
				Status: "deprecated",
				Cost:   &modelsDevCost{modelsDevRates: modelsDevRates{Input: "30", Output: "60"}},
			},
			"gpt-4o-mini": {
				Cost: &modelsDevCost{modelsDevRates: modelsDevRates{Input: "0.15", Output: "0.6", CacheRead: "0.075"}},
			},
			"gpt-4o-mini-audio-preview": {
				Cost: &modelsDevCost{modelsDevRates: modelsDevRates{Input: "0.15", Output: "0.6", InputAudio: "10", OutputAudio: "20"}},
			},
		}},
		"anthropic": {ID: "anthropic", Models: map[string]modelsDevModel{
			"claude-sonnet-4-5": {
				Cost: &modelsDevCost{modelsDevRates: modelsDevRates{Input: "3", Output: "15", CacheRead: "0.3", CacheWrite: "3.75"}},
			},
		}},
		"google": {ID: "google", Models: map[string]modelsDevModel{
			"gemini-2.5-pro": {
				Cost: &modelsDevCost{
					modelsDevRates: modelsDevRates{Input: "1.25", Output: "10", CacheRead: "0.125"},
					Tiers: []modelsDevTier{{
						modelsDevRates: modelsDevRates{Input: "2.5", Output: "15", CacheRead: "0.25"},
						Tier:           modelsDevTierKind{Type: "context", Size: 200000},
					}},
				},
			},
		}},
		"alibaba-cn": {ID: "alibaba-cn", Models: map[string]modelsDevModel{
			"qwen3-omni-flash": {
				Cost: &modelsDevCost{modelsDevRates: modelsDevRates{Input: "0.058", Output: "0.23", InputAudio: "3.584", OutputAudio: "7.168"}},
			},
		}},
		"freebie": {ID: "freebie", Models: map[string]modelsDevModel{
			"identity": {},
		}},
	}
}

func TestTransformIncludesLegacyModelsWhenRequested(t *testing.T) {
	cat, warns, err := modelsDevTransform(sampleAPI(), []string{"openai"}, true)
	if err != nil {
		t.Fatal(err)
	}
	if len(warns) != 0 {
		t.Fatalf("unexpected warnings: %v", warns)
	}
	if _, ok := cat.Providers["openai"].Models["gpt-4"]; !ok {
		t.Fatal("expected deprecated model when legacy is true")
	}
}

func TestTransformOmitsModelsWithoutCost(t *testing.T) {
	api := map[string]modelsDevProvider{
		"openai": {Models: map[string]modelsDevModel{"chatgpt-image-latest": {}}},
	}
	_, _, err := modelsDevTransform(api, []string{"openai"}, false)
	if err == nil || !strings.Contains(err.Error(), "no importable models") {
		t.Fatalf("expected no importable models error, got %v", err)
	}
}

func TestTransformSkipsUnsupportedProvider(t *testing.T) {
	_, warns, err := modelsDevTransform(sampleAPI(), []string{"alibaba-cn"}, false)
	if err == nil || !strings.Contains(err.Error(), "no providers matched") {
		t.Fatalf("expected no providers matched error, got %v", err)
	}
	if len(warns) != 1 || !strings.Contains(warns[0], "not supported") {
		t.Fatalf("expected not-supported warning, got %v", warns)
	}
}

func TestTransformMatchesGoldenCatalog(t *testing.T) {
	cat, warns, err := modelsDevTransform(sampleAPI(), []string{"anthropic", "google", "openai"}, false)
	if err != nil {
		t.Fatal(err)
	}
	if len(warns) != 0 {
		t.Fatalf("unexpected warnings: %v", warns)
	}
	if err := cat.Validate(); err != nil {
		t.Fatalf("catalog invalid: %v", err)
	}
	got, err := marshalCatalog(cat, true)
	if err != nil {
		t.Fatal(err)
	}
	want, err := os.ReadFile(filepath.Join(
		testutils.GitRootDirectory(),
		"crates",
		"agentgateway",
		"src",
		"llm",
		"cost",
		"testdata",
		"model_catalog.golden.json",
	))
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != string(want) {
		t.Fatalf("golden catalog mismatch\n--- got ---\n%s\n--- want ---\n%s", got, want)
	}
}

func TestTransformRoundsOverPreciseRate(t *testing.T) {
	api := map[string]modelsDevProvider{
		"openai": {Models: map[string]modelsDevModel{
			"m": {Cost: &modelsDevCost{modelsDevRates: modelsDevRates{Input: json.Number("0.049999999999999996")}}},
		}},
	}
	cat, warns, err := modelsDevTransform(api, []string{"openai"}, false)
	if err != nil {
		t.Fatal(err)
	}
	if got := cat.Providers["openai"].Models["m"].Rates.Input; got != "0.05" {
		t.Fatalf("rounded rate = %q, want 0.05", got)
	}
	if len(warns) != 0 {
		t.Fatalf("unexpected warnings: %v", warns)
	}
}

func TestTransformRejectsNegativeRate(t *testing.T) {
	api := map[string]modelsDevProvider{
		"openai": {Models: map[string]modelsDevModel{
			"m": {Cost: &modelsDevCost{modelsDevRates: modelsDevRates{Input: json.Number("-1")}}},
		}},
	}
	if _, _, err := modelsDevTransform(api, []string{"openai"}, false); err == nil {
		t.Fatal("expected error for negative rate")
	}
}

func TestTransformMissingProviderWarnsAndEmptyErrors(t *testing.T) {
	// azure is supported by the proxy but absent from this source snapshot.
	_, warns, err := modelsDevTransform(sampleAPI(), []string{"azure"}, false)
	if err == nil {
		t.Fatal("expected error when no providers match")
	}
	if len(warns) != 1 || !strings.Contains(warns[0], "not found") {
		t.Fatalf("expected not-found warning, got %v", warns)
	}
}
