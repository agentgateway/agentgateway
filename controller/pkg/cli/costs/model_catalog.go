package costs

import (
	"encoding/json"
	"fmt"
	"reflect"
	"strings"

	"github.com/shopspring/decimal"
)

type ModelCatalog struct {
	Providers map[string]Provider `json:"providers"`
}

func (c *ModelCatalog) Validate() error {
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
	Rates Rates  `json:"rates,omitzero"`
	Tiers []Tier `json:"tiers,omitempty"`
}

func (m Model) IsZero() bool {
	return m.Rates.IsZero() && len(m.Tiers) == 0
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
		m, ok := v.Field(i).Interface().(Money)
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
