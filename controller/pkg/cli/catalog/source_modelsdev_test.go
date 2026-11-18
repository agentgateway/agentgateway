package catalog

import (
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
		got := modelsDevSelectProviders(api, []string{"google", "openai", "openrouter"}, nil)
		want := []string{"google", "openai", "openrouter"}
		if !reflect.DeepEqual(got, want) {
			t.Fatalf("modelsDevSelectProviders(..., explicit) = %v, want %v", got, want)
		}
	})

	t.Run("supported only by default", func(t *testing.T) {
		got := modelsDevSelectProviders(api, nil, nil)
		want := []string{"anthropic", "google", "openai", "openrouter"}
		if !reflect.DeepEqual(got, want) {
			t.Fatalf("modelsDevSelectProviders(..., nil) = %v, want %v", got, want)
		}
	})

	t.Run("excluded", func(t *testing.T) {
		got := modelsDevSelectProviders(api, nil, []string{"openrouter"})
		want := []string{"anthropic", "google", "openai"}
		if !reflect.DeepEqual(got, want) {
			t.Fatalf("modelsDevSelectProviders(..., excluded) = %v, want %v", got, want)
		}
	})
}

func sampleAPI() map[string]modelsDevProvider {
	api, err := modelsDevDecodeAPI(strings.NewReader(`{
		"openai": {"models": {
			"chatgpt-image-latest": {},
			"gpt-4": {"status": "deprecated", "cost": {"text": {"input": {"price": 30}, "output": {"price": 60}}}},
			"gpt-5.6": {
				"cost": {"text": {
					"input": {"price": 1, "cache": {"read": 0.1, "write": 1.25}, "overrides": [
						{"when": {"context": {"gt": 200000}}, "price": 2, "cache": {"write": 2.5}}
					]},
					"output": {"price": 5, "overrides": [{"when": {"context": {"gt": 200000}}, "price": 10}]}
				}},
				"experimental": {"modes": {"fast": {"service_tier": "priority", "cost": {"text": {
					"input": {"price": 2, "cache": {"write": 2.5}}, "output": {"price": 10}
				}}}}}
			},
			"gpt-4o-mini": {
				"name": "GPT-4o mini",
				"release_date": "2024-07-18",
				"cost": {"text": {"input": {"price": 0.15, "cache": {"read": 0.075, "write": 0.1875}}, "output": {"price": 0.6}}},
				"capabilities": {"reasoning": {"supported": false}, "tools": {"supported": true}, "temperature": true},
				"limit": {"context": 128000, "output": 16384},
				"modalities": {"input": ["text", "image/png", "image/jpeg"], "output": ["text"]},
				"experimental": {"modes": {
					"fast": {"body": {"service_tier": "priority"}, "cost": {"text": {"input": {"price": 0.25}, "output": {"price": 1}}}},
					"ultrafast": {"cost": {"text": {"input": {"price": 1}, "output": {"price": 4}}}}
				}}
			},
			"gpt-4o-mini-audio-preview": {"cost": {
				"text": {"input": {"price": 0.15}, "output": {"price": 0.6}},
				"audio": {"input": {"price": 10}, "output": {"price": 20}}
			}}
		}},
		"anthropic": {"models": {
			"claude-opus-4-7": {
				"family": "claude-opus", "cost": {"text": {"input": {"price": 5}, "output": {"price": 25}}},
				"capabilities": {"reasoning": {"effort": ["low", "high"]}}
			},
			"claude-opus-5-5": {
				"family": "claude-opus",
				"cost": {
					"text": {"input": {"price": 4}, "output": {"price": 20}},
					"multipliers": [{"when": {"service_tier": {"equals": "fast"}}, "factor": {"text": 2}}]
				}
			},
			"claude-opus-4-6": {
				"family": "claude-opus", "cost": {"text": {"input": {"price": 5}, "output": {"price": 25}}},
				"capabilities": {"reasoning": {"toggle": true, "effort": ["low", "high"], "budget": {"min": 1024, "max": -1}}}
			},
			"claude-sonnet-4-5": {
				"family": "claude-sonnet", "cost": {"text": {
					"input": {"price": 3, "cache": {"read": 0.3, "write": {"5m": 3.75, "1h": 6}}}, "output": {"price": 15}
				}},
				"capabilities": {"reasoning": {"budget": {}}}
			}
		}},
		"google": {"models": {"gemini-2.5-pro": {"cost": {"text": {
			"input": {"price": 1.25, "cache": {"read": 0.125}, "overrides": [
				{"when": {"context": {"gt": 200000}}, "price": 2.5, "cache": {"read": 0.25}}
			]},
			"output": {"price": 10, "overrides": [{"when": {"context": {"gt": 200000}}, "price": 15}]}
		}}}}},
		"openrouter": {"models": {}},
		"alibaba-cn": {"models": {}},
		"freebie": {"models": {}}
	}`))
	if err != nil {
		panic(err)
	}
	return api
}

func TestTransformIncludesLegacyModelsWhenRequested(t *testing.T) {
	cat, warns, err := modelsDevTransform(sampleAPI(), []string{"openai"}, true, "2026-11-18")
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
	_, _, err := modelsDevTransform(api, []string{"openai"}, false, "2026-11-18")
	if err == nil || !strings.Contains(err.Error(), "no importable models") {
		t.Fatalf("expected no importable models error, got %v", err)
	}
}

func TestTransformSkipsUnsupportedProvider(t *testing.T) {
	_, warns, err := modelsDevTransform(sampleAPI(), []string{"alibaba-cn"}, false, "2026-11-18")
	if err == nil || !strings.Contains(err.Error(), "no providers matched") {
		t.Fatalf("expected no providers matched error, got %v", err)
	}
	if len(warns) != 1 || !strings.Contains(warns[0], "not supported") {
		t.Fatalf("expected not-supported warning, got %v", warns)
	}
}

func TestTransformMatchesGoldenCatalog(t *testing.T) {
	cat, warns, err := modelsDevTransform(sampleAPI(), []string{"anthropic", "google", "openai"}, false, "2026-11-18")
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
		"catalog",
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
			"m": {Cost: &modelsDevCost{Text: modelsDevModalityCost{Input: modelsDevPrice{modelsDevPriceValues: modelsDevPriceValues{Price: "0.049999999999999996"}}}}},
		}},
	}
	cat, warns, err := modelsDevTransform(api, []string{"openai"}, false, "2026-11-18")
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
			"m": {Cost: &modelsDevCost{Text: modelsDevModalityCost{Input: modelsDevPrice{modelsDevPriceValues: modelsDevPriceValues{Price: "-1"}}}}},
		}},
	}
	if _, _, err := modelsDevTransform(api, []string{"openai"}, false, "2026-11-18"); err == nil {
		t.Fatal("expected error for negative rate")
	}
}

func TestTransformMissingProviderWarnsAndEmptyErrors(t *testing.T) {
	// azure is supported by the proxy but absent from this source snapshot.
	_, warns, err := modelsDevTransform(sampleAPI(), []string{"azure"}, false, "2026-11-18")
	if err == nil {
		t.Fatal("expected error when no providers match")
	}
	if len(warns) != 1 || !strings.Contains(warns[0], "not found") {
		t.Fatalf("expected not-found warning, got %v", warns)
	}
}

func TestModelsDevV2PricingAndDecisionEndpoints(t *testing.T) {
	api, err := modelsDevDecodeAPI(strings.NewReader(`{
		"typesafe": {"models": {"jev-latest": {
			"type": "decision",
			"api": {"systemone": {}},
			"modalities": {"input": ["text", "image/png", "image/jpeg"], "output": ["decision"]},
			"capabilities": {"structured_output": true},
			"cost": {"text": {"input": {"price": 0.042}, "output": {"price": 0}}}
		}}},
		"openai": {"models": {
			"tiered": {"cost": {
				"text": {
					"input": {
						"price": 1, "cache": {"read": 0.1, "explicit": {"write": {"5m": 1.25, "1h": 3}}},
						"overrides": [{"when": {"context": {"gte": 200001}}, "price": 2}]
					},
					"output": {"price": 5}
				},
				"multipliers": [
					{"when": {"service_tier": {"equals": "priority"}}, "factor": {"text": {"input": {"price": 2, "cache": {"explicit": 2}}, "output": 2}}},
					{"when": {"service_tier": {"equals": "priority"}, "region": {"equals": "us"}}, "factor": 3}
				]
			}},
			"duration": {"cost": {"audio": {"input": {"unit": "second", "price": 0.002}}}},
			"characters": {"cost": {"text": {"input": {"unit": "character", "price": 22}}}}
		}}
	}`))
	if err != nil {
		t.Fatal(err)
	}
	cat, warns, err := modelsDevTransform(api, []string{"openai", "typesafe"}, false, "2026-11-18")
	if err != nil || len(warns) != 0 {
		t.Fatalf("import failed: %v (%v)", err, warns)
	}
	if err := cat.Validate(); err != nil {
		t.Fatal(err)
	}
	jev := cat.Providers["typesafe"].Models["jev-latest"]
	if !reflect.DeepEqual(jev.Endpoints, []string{"/systemone"}) ||
		!reflect.DeepEqual(jev.Modalities.Input, []string{"text", "image"}) ||
		jev.Capabilities.StructuredOutput == nil || !*jev.Capabilities.StructuredOutput ||
		jev.Rates.Input != "0.042" || jev.Rates.Output != "0" {
		t.Fatalf("unexpected Jev entry: %+v", jev)
	}
	models := cat.Providers["openai"].Models
	if _, ok := models["characters"]; ok {
		t.Fatal("character pricing must not become token pricing")
	}
	if got := models["duration"].Rates.PerAudioMinute; got != "0.12" {
		t.Fatalf("per-minute rate = %q, want 0.12", got)
	}
	tiered := models["tiered"]
	want := []Tier{
		{ContextOver: 200000, Rates: Rates{Input: "2"}},
		{ServiceTier: "priority", Rates: Rates{Input: "2", Output: "10", CacheWrite: "2.5", CacheWriteDetails: map[string]Money{"1h": "6"}}},
		{ServiceTier: "priority", ContextOver: 200000, Rates: Rates{Input: "4", Output: "10", CacheWrite: "2.5", CacheWriteDetails: map[string]Money{"1h": "6"}}},
	}
	if !reflect.DeepEqual(tiered.Tiers, want) {
		t.Fatalf("tiers = %+v, want %+v", tiered.Tiers, want)
	}
}

func TestModelsDevPricingAsOf(t *testing.T) {
	api, err := modelsDevDecodeAPI(strings.NewReader(`{
		"openai": {"models": {"dated": {"cost": {
			"text": {"input": {"price": 2, "overrides": [
				{"when": {"date": {"gte": "2026-12-01"}}, "price": 4},
				{"when": {"websocket": {"equals": true}}, "price": 1}
			]}, "output": {"price": 10}},
			"multipliers": [
				{"when": {"service_tier": {"equals": "ultrafast"}, "date": {"lt": "2027-03-01"}}, "factor": 3},
				{"when": {"service_tier": {"equals": "ultrafast"}, "date": {"gte": "2027-03-01"}}, "factor": 6},
				{"when": {"date": {"gte": "2027-03-01"}}, "factor": {"text": {"output": 2}}},
				{"when": {"region": {"equals": "us"}}, "factor": 1.1},
				{"when": {"not": {"region": {"equals": "global"}}}, "factor": 1.1},
				{"when": {"utc_hours": {"lt": "04:00"}}, "factor": 0.5}
			]
		}}}}
	}`))
	if err != nil {
		t.Fatal(err)
	}
	for _, tt := range []struct {
		date      string
		base      Rates
		ultrafast Rates
	}{
		{"2026-11-30", Rates{Input: "2", Output: "10"}, Rates{Input: "6", Output: "30"}},
		{"2026-12-01", Rates{Input: "4", Output: "10"}, Rates{Input: "12", Output: "30"}},
		{"2027-03-01", Rates{Input: "4", Output: "20"}, Rates{Input: "24", Output: "120"}},
	} {
		t.Run(tt.date, func(t *testing.T) {
			cat, warns, err := modelsDevTransform(api, []string{"openai"}, false, tt.date)
			if err != nil {
				t.Fatal(err)
			}
			wantWarnings := []string{"openai/dated: skipping unsupported models.dev pricing conditions: utc_hours, websocket"}
			if !reflect.DeepEqual(warns, wantWarnings) {
				t.Fatalf("warnings = %v, want %v", warns, wantWarnings)
			}
			model := cat.Providers["openai"].Models["dated"]
			wantTiers := []Tier{{ServiceTier: "ultrafast", Rates: tt.ultrafast}}
			if !reflect.DeepEqual(model.Rates, tt.base) || !reflect.DeepEqual(model.Tiers, wantTiers) {
				t.Fatalf("rates = %+v and tiers = %+v, want %+v and %+v", model.Rates, model.Tiers, tt.base, wantTiers)
			}
		})
	}
}

func TestModelsDevScalarCacheWriteReplacesTTLRates(t *testing.T) {
	api, err := modelsDevDecodeAPI(strings.NewReader(`{
		"anthropic": {"models": {
			"conditional": {"family": "claude-opus", "cost": {"text": {"input": {
				"price": 1, "cache": {"explicit": {"write": {"5m": 1, "1h": 2}}},
				"overrides": [
					{"when": {"context": {"gt": 100000}}, "cache": {"write": 1.25}},
					{"when": {"service_tier": {"equals": "priority"}}, "cache": {"write": 3}}
				]
			}}}},
			"mode": {"family": "claude-opus",
				"cost": {"text": {"input": {"price": 1, "cache": {"write": {"5m": 1, "1h": 2}}}}},
				"experimental": {"modes": {"fast": {"cost": {"text": {"input": {"price": 1, "cache": {"write": 1.25}}}}}}}
			}
		}}
	}`))
	if err != nil {
		t.Fatal(err)
	}
	cat, _, err := modelsDevTransform(api, []string{"anthropic"}, false, "2026-11-18")
	if err != nil {
		t.Fatal(err)
	}
	models := cat.Providers["anthropic"].Models
	want := map[string][]Tier{
		"conditional": {
			{ContextOver: 100000, Rates: Rates{CacheWrite: "1.25", CacheWriteDetails: map[string]Money{"1h": "1.25"}}},
			{ServiceTier: "priority", Rates: Rates{CacheWrite: "3", CacheWriteDetails: map[string]Money{"1h": "3"}}},
		},
		"mode": {
			{ServiceTier: "fast", Rates: Rates{CacheWrite: "1.25", CacheWriteDetails: map[string]Money{"1h": "1.25"}}},
		},
	}
	for model, tiers := range want {
		if got := models[model].Tiers; !reflect.DeepEqual(got, tiers) {
			t.Errorf("%s tiers = %+v, want %+v", model, got, tiers)
		}
	}
}
