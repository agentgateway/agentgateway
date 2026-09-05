package catalog

import (
	"encoding/json"
	"fmt"
	"reflect"
	"slices"
	"strings"
	"time"

	"github.com/shopspring/decimal"
)

type ModelCatalog struct {
	Metadata  *CatalogMetadata    `json:"metadata,omitempty"`
	Providers map[string]Provider `json:"providers"`
}

type CatalogMetadata struct {
	// Source is accepted for compatibility with older catalogs.
	Source      string    `json:"source,omitempty"`
	GeneratedAt time.Time `json:"generatedAt"`
}

func (c *ModelCatalog) overlayWith(overlay *ModelCatalog) {
	if c.Providers == nil {
		c.Providers = map[string]Provider{}
	}
	if global, ok := overlay.Providers["*"]; ok {
		providerIDs := make([]string, 0, len(c.Providers))
		for providerID := range c.Providers {
			providerIDs = append(providerIDs, providerID)
		}
		slices.Sort(providerIDs)
		for _, providerID := range providerIDs {
			provider := c.Providers[providerID]
			provider.overlayWith(global, false)
			c.Providers[providerID] = provider
		}
	}

	providerIDs := make([]string, 0, len(overlay.Providers))
	for providerID := range overlay.Providers {
		if providerID != "*" {
			providerIDs = append(providerIDs, providerID)
		}
	}
	slices.Sort(providerIDs)
	for _, providerID := range providerIDs {
		overlayProvider := overlay.Providers[providerID]
		provider := c.Providers[providerID]
		provider.overlayWith(overlayProvider, true)
		c.Providers[providerID] = provider
	}
}

func (p *Provider) overlayWith(overlay Provider, addExactModels bool) {
	if p.Models == nil {
		p.Models = map[string]Model{}
	}
	modelPatterns := make([]string, 0, len(overlay.Models))
	exactModels := make([]string, 0, len(overlay.Models))
	for modelID := range overlay.Models {
		if strings.Contains(modelID, "*") {
			modelPatterns = append(modelPatterns, modelID)
		} else {
			exactModels = append(exactModels, modelID)
		}
	}
	slices.Sort(modelPatterns)
	slices.Sort(exactModels)

	baseModelIDs := make([]string, 0, len(p.Models))
	for modelID := range p.Models {
		baseModelIDs = append(baseModelIDs, modelID)
	}
	slices.Sort(baseModelIDs)
	for _, pattern := range modelPatterns {
		for _, modelID := range baseModelIDs {
			if starMatch(pattern, modelID) {
				model := p.Models[modelID]
				model.overlayWith(overlay.Models[pattern])
				p.Models[modelID] = model
			}
		}
	}
	for _, modelID := range exactModels {
		model, exists := p.Models[modelID]
		if !exists && !addExactModels {
			continue
		}
		model.overlayWith(overlay.Models[modelID])
		p.Models[modelID] = model
	}
}

func (m *Model) overlayWith(overlay Model) {
	m.Rates.overlayWith(overlay.Rates)
	if len(overlay.Tiers) > 0 {
		m.Tiers = overlay.Tiers
	}
	for _, tag := range overlay.Tags {
		if !slices.Contains(m.Tags, tag) {
			m.Tags = append(m.Tags, tag)
		}
	}
}

// starMatch matches a string pattern where '*' represents any sequence of bytes, including '/'.
func starMatch(pattern, value string) bool {
	patternIndex, valueIndex := 0, 0
	starIndex, retryValueIndex := -1, 0
	for valueIndex < len(value) {
		switch {
		case patternIndex < len(pattern) && pattern[patternIndex] == value[valueIndex]:
			patternIndex++
			valueIndex++
		case patternIndex < len(pattern) && pattern[patternIndex] == '*':
			starIndex = patternIndex
			patternIndex++
			retryValueIndex = valueIndex
		case starIndex >= 0:
			patternIndex = starIndex + 1
			retryValueIndex++
			valueIndex = retryValueIndex
		default:
			return false
		}
	}
	for patternIndex < len(pattern) && pattern[patternIndex] == '*' {
		patternIndex++
	}
	return patternIndex == len(pattern)
}

func (c *ModelCatalog) Validate() error {
	if c.Metadata != nil {
		if c.Metadata.GeneratedAt.IsZero() {
			return fmt.Errorf("metadata generatedAt is required")
		}
	}
	for provider, p := range c.Providers {
		for model, m := range p.Models {
			if err := m.validate(); err != nil {
				return fmt.Errorf("%s/%s: %w", provider, model, err)
			}
		}
	}
	return nil
}

type Provider struct {
	Models map[string]Model `json:"models"`
}

type Model struct {
	Rates Rates    `json:"rates,omitzero"`
	Tiers []Tier   `json:"tiers,omitempty"`
	Tags  []string `json:"tags,omitempty"`
}

func (m Model) IsZero() bool {
	return m.Rates.IsZero() && len(m.Tiers) == 0 && len(m.Tags) == 0
}

type Rates struct {
	Input       Money `json:"input,omitempty"`
	Output      Money `json:"output,omitempty"`
	CacheRead   Money `json:"cacheRead,omitempty"`
	CacheWrite  Money `json:"cacheWrite,omitempty"`
	Reasoning   Money `json:"reasoning,omitempty"`
	InputAudio  Money `json:"inputAudio,omitempty"`
	OutputAudio Money `json:"outputAudio,omitempty"`
}

type Tier struct {
	ContextOver uint64 `json:"contextOver"`
	Rates       Rates  `json:"rates,omitzero"`
}

type Money string

func (r Rates) IsZero() bool {
	return r == Rates{}
}

func (r *Rates) overlayWith(overlay Rates) {
	dst := reflect.ValueOf(r).Elem()
	src := reflect.ValueOf(overlay)
	for i := range src.NumField() {
		if !src.Field(i).IsZero() {
			dst.Field(i).Set(src.Field(i))
		}
	}
}

func (m Money) Decimal() (decimal.Decimal, error) {
	if m == "" {
		return decimal.Zero, nil
	}
	d, err := decimal.NewFromString(string(m))
	if err != nil {
		return decimal.Zero, fmt.Errorf("invalid money %q: %w", string(m), err)
	}
	return d, nil
}

// maxFractionalDigits bounds rate precision. Money is exact decimal, never float;
// rates are USD per 1,000,000 tokens and never need more than micro-dollar precision.
const maxFractionalDigits = 6

func (m Money) validate() error {
	if m == "" {
		return nil
	}
	d, err := m.Decimal()
	if err != nil {
		return err
	}
	if d.IsNegative() {
		return fmt.Errorf("money %q is negative", string(m))
	}
	if d.Exponent() < -maxFractionalDigits {
		return fmt.Errorf("money %q exceeds %d fractional digits", string(m), maxFractionalDigits)
	}
	for _, r := range string(m) {
		if r == 'e' || r == 'E' {
			return fmt.Errorf("money %q uses scientific notation", string(m))
		}
	}
	return nil
}

func (m *Model) validate() error {
	if err := m.Rates.validate(); err != nil {
		return err
	}
	var prev uint64
	for i, t := range m.Tiers {
		if i > 0 && t.ContextOver <= prev {
			return fmt.Errorf("tier %d threshold %d not strictly greater than previous %d", i, t.ContextOver, prev)
		}
		prev = t.ContextOver
		if err := t.Rates.validate(); err != nil {
			return fmt.Errorf("tier %d: %w", i, err)
		}
	}
	return nil
}

func (r *Rates) validate() error {
	v := reflect.ValueOf(*r)
	t := v.Type()
	for i := 0; i < v.NumField(); i++ {
		m, ok := reflect.TypeAssert[Money](v.Field(i))
		if !ok {
			continue
		}
		if err := m.validate(); err != nil {
			return fmt.Errorf("rate %s: %w", jsonFieldName(t.Field(i)), err)
		}
	}
	return nil
}

func jsonFieldName(field reflect.StructField) string {
	name, _, _ := strings.Cut(field.Tag.Get("json"), ",")
	if name == "" {
		return field.Name
	}
	return name
}

func marshalCatalog(cat *ModelCatalog, pretty bool) ([]byte, error) {
	marshal := json.Marshal
	if pretty {
		marshal = func(v any) ([]byte, error) { return json.MarshalIndent(v, "", "  ") }
	}
	data, err := marshal(cat)
	if err != nil {
		return nil, fmt.Errorf("marshal catalog: %w", err)
	}
	return append(data, '\n'), nil
}
