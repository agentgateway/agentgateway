package catalog

import (
	"context"
	"fmt"
	"io"
	"net/http"
	"slices"
	"strings"
	"time"

	"github.com/shopspring/decimal"
)

const openaiPricingSourceName = "openai-pricing"
const openaiProviderID = "openai"

// Markdown rendering of https://developers.openai.com/api/docs/pricing.
const openaiPricingURL = "https://developers.openai.com/api/docs/pricing.md"

// openaiPricingSections are the pricing page sections imported; models.dev covers the rest.
var openaiPricingSections = []string{"Realtime and audio generation models", "Transcription models"}

// openaiPricingModelIDs maps display names in the pricing tables to API model ids.
var openaiPricingModelIDs = map[string]string{
	"Whisper": "whisper-1",
}

func init() {
	importSources[openaiPricingSourceName] = func(ctx context.Context, opts importOptions) (*ModelCatalog, []string, error) {
		matches := func(p string) bool {
			id, ok := modelsDevMapProviderID(p)
			return ok && id == openaiProviderID
		}
		if (len(opts.providers) > 0 && !slices.ContainsFunc(opts.providers, matches)) || slices.ContainsFunc(opts.excludeProviders, matches) {
			return &ModelCatalog{}, nil, nil
		}
		req, err := http.NewRequestWithContext(ctx, http.MethodGet, openaiPricingURL, nil)
		if err != nil {
			return nil, nil, err
		}
		client := &http.Client{Timeout: 30 * time.Second}
		resp, err := client.Do(req)
		if err != nil {
			return nil, nil, err
		}
		defer resp.Body.Close()
		if resp.StatusCode != http.StatusOK {
			return nil, nil, fmt.Errorf("fetch OpenAI pricing: HTTP %d", resp.StatusCode)
		}
		return openaiParsePricing(resp.Body)
	}
}

// openaiParsePricing imports audio model prices. Each imported section is a heading line followed by
// a single table. Audio rows map to the audio rates, text rows to the base rates; image rows have no
// matching rate.
func openaiParsePricing(r io.Reader) (*ModelCatalog, []string, error) {
	models := map[string]Model{}
	found := map[string]bool{}
	section := ""
	var header []string
	scanner := docScanner(r)
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if slices.Contains(openaiPricingSections, line) {
			section, header = line, nil
			continue
		}
		if section == "" {
			continue
		}
		if !strings.HasPrefix(line, "|") {
			// The section ends with its table.
			if header != nil {
				section = ""
			}
			continue
		}
		cells := strings.Split(strings.Trim(line, "|"), "|")
		for i := range cells {
			cells[i] = strings.TrimSpace(cells[i])
		}
		if header == nil {
			header = cells
			found[section] = true
			continue
		}
		if strings.HasPrefix(cells[0], "---") {
			continue
		}
		if len(cells) != len(header) {
			return nil, nil, fmt.Errorf("OpenAI %q row has %d columns, expected %d", section, len(cells), len(header))
		}
		id := cells[0]
		if mapped, ok := openaiPricingModelIDs[id]; ok {
			id = mapped
		}
		modality := ""
		if i := slices.Index(header, "Modality"); i >= 0 {
			modality = cells[i]
		}
		model := models[id]
		rates := &model.Rates
		for i, h := range header {
			var dst *Money
			switch {
			case h == "Input" && modality == "Audio":
				dst = &rates.InputAudio
			case h == "Input":
				dst = &rates.Input
			case strings.HasPrefix(h, "Output") && modality == "Audio":
				dst = &rates.OutputAudio
			case strings.HasPrefix(h, "Output"):
				dst = &rates.Output
			// Audio and text share a single cache read rate; keep the text one.
			case h == "Cached input" && modality != "Audio":
				dst = &rates.CacheRead
			// Token-billed models list an estimated per-minute cost; only bill by the minute without token rates.
			case h == "Estimated cost" && rates.Input == "" && rates.Output == "":
				dst = &rates.PerAudioMinute
			default:
				continue
			}
			value := cells[i]
			if value == "-" || modality == "Image" {
				continue
			}
			if amount, ok := strings.CutSuffix(value, " / minute"); ok {
				if dst != &rates.PerAudioMinute {
					return nil, nil, fmt.Errorf("OpenAI %s: unexpected per-minute %s price %q", id, h, value)
				}
				value = amount
			}
			// Character-billed models (tts-1) have no matching rate.
			if strings.HasSuffix(value, " / 1M characters") {
				continue
			}
			rate, ok := strings.CutPrefix(value, "$")
			d, err := decimal.NewFromString(rate)
			if !ok || err != nil || d.IsNegative() {
				return nil, nil, fmt.Errorf("OpenAI %s: invalid %s price %q", id, h, value)
			}
			if *dst != "" {
				return nil, nil, fmt.Errorf("duplicate OpenAI %s price for %s", h, id)
			}
			*dst = Money(d.String())
		}
		if !model.Rates.IsZero() {
			models[id] = model
		}
	}
	if err := scanner.Err(); err != nil {
		return nil, nil, err
	}
	for _, s := range openaiPricingSections {
		if !found[s] {
			return nil, nil, fmt.Errorf("no OpenAI %q pricing table found", s)
		}
	}
	return &ModelCatalog{Providers: map[string]Provider{openaiProviderID: {Models: models}}}, nil, nil
}
