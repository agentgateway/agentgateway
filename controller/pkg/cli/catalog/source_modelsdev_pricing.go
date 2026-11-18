package catalog

import (
	"bytes"
	"encoding/json"
	"fmt"
	"maps"
	"reflect"
	"slices"
	"strings"
	"time"

	"github.com/shopspring/decimal"
)

type modelsDevCost struct {
	Text        modelsDevModalityCost `json:"text"`
	Audio       modelsDevModalityCost `json:"audio"`
	Multipliers []modelsDevMultiplier `json:"multipliers"`
}

type modelsDevModalityCost struct {
	Input  modelsDevPrice `json:"input"`
	Output modelsDevPrice `json:"output"`
}

type modelsDevPrice struct {
	Unit string `json:"unit"`
	modelsDevPriceValues
	Overrides []modelsDevPriceOverride `json:"overrides"`
}

type modelsDevPriceValues struct {
	Price     json.Number `json:"price"`
	Reasoning json.Number `json:"reasoning"`
	Cache     struct {
		modelsDevCachePrices
		Explicit modelsDevCachePrices `json:"explicit"`
	} `json:"cache"`
	replacedTTL bool
}

func (p modelsDevPriceValues) cacheWrite() (modelsDevCachePrices, string) {
	if len(p.Cache.Write) != 0 {
		return p.Cache.modelsDevCachePrices, "text.input.cache.write"
	}
	return p.Cache.Explicit, "text.input.cache.explicit.write"
}

type modelsDevCachePrices struct {
	Read  json.Number         `json:"read"`
	Write modelsDevCacheWrite `json:"write"`
}

func (c *modelsDevCachePrices) overlay(next modelsDevCachePrices) {
	if next.Read != "" {
		c.Read = next.Read
	}
	if len(next.Write) == 0 {
		return
	}
	if next.Write[""] != "" || c.Write[""] != "" {
		c.Write = next.Write
		return
	}
	merged := make(modelsDevCacheWrite, len(c.Write)+len(next.Write))
	maps.Copy(merged, c.Write)
	maps.Copy(merged, next.Write)
	c.Write = merged
}

type modelsDevCacheWrite map[string]json.Number

func (w *modelsDevCacheWrite) UnmarshalJSON(data []byte) error {
	if bytes.HasPrefix(data, []byte("{")) {
		return json.Unmarshal(data, (*map[string]json.Number)(w))
	}
	var price json.Number
	if err := json.Unmarshal(data, &price); err != nil {
		return err
	}
	*w = modelsDevCacheWrite{"": price}
	return nil
}

type modelsDevPriceOverride struct {
	When modelsDevCondition `json:"when"`
	modelsDevPriceValues
}

type modelsDevMultiplier struct {
	When   modelsDevCondition `json:"when"`
	Factor json.RawMessage    `json:"factor"`
}

type modelsDevCondition map[string]json.RawMessage

// Catalog tiers can express context thresholds and the service tier reported by the provider.
func (c modelsDevCondition) tier() (uint64, []string, bool) {
	var over uint64
	var services []string
	for key, raw := range c {
		switch key {
		case "context":
			var threshold map[string]uint64
			if json.Unmarshal(raw, &threshold) != nil || len(threshold) != 1 {
				return 0, nil, false
			}
			if gt, ok := threshold["gt"]; ok {
				over = gt
			} else if gte, ok := threshold["gte"]; ok {
				over = max(gte, 1) - 1
			} else {
				return 0, nil, false
			}
		case "service_tier":
			var tiers struct {
				Equals string   `json:"equals"`
				In     []string `json:"in"`
			}
			if json.Unmarshal(raw, &tiers) != nil {
				return 0, nil, false
			}
			services = tiers.In
			if tiers.Equals != "" {
				services = []string{tiers.Equals}
			}
			if len(services) == 0 {
				return 0, nil, false
			}
		default:
			return 0, nil, false
		}
	}
	return over, services, c != nil
}

func (c modelsDevCondition) matches(over uint64, service string) bool {
	threshold, services, ok := c.tier()
	return ok && over >= threshold && (len(services) == 0 || slices.Contains(services, service))
}

// Resolve date predicates at refresh time. Regional adjustments are omitted from global default rates.
func (c modelsDevCondition) forDate(asOf string, skipped map[string]bool) modelsDevCondition {
	if len(c) == 0 {
		skipped["missing condition"] = true
		return nil
	}
	result := modelsDevCondition{}
	active := true
	for key, raw := range c {
		switch key {
		case "context", "service_tier":
			if _, _, ok := (modelsDevCondition{key: raw}).tier(); ok {
				result[key] = raw
				continue
			}
		case "date":
			if matches, ok := modelsDevDateMatches(raw, asOf); ok {
				active = active && matches
				continue
			}
		case "region":
			active = false
			continue
		case "not":
			var inner map[string]json.RawMessage
			if json.Unmarshal(raw, &inner) == nil && len(inner) == 1 && inner["region"] != nil {
				active = false
				continue
			}
		}
		skipped[key] = true
		active = false
	}
	if !active {
		return nil
	}
	return result
}

func modelsDevDateMatches(raw json.RawMessage, asOf string) (bool, bool) {
	var condition map[string]json.RawMessage
	if json.Unmarshal(raw, &condition) != nil || len(condition) != 1 {
		return false, false
	}
	for op, value := range condition {
		if op == "in" {
			var dates []string
			if json.Unmarshal(value, &dates) != nil || dates == nil {
				return false, false
			}
			for _, date := range dates {
				if _, err := time.Parse(time.DateOnly, date); err != nil {
					return false, false
				}
			}
			return slices.Contains(dates, asOf), true
		}
		var date string
		if json.Unmarshal(value, &date) != nil {
			return false, false
		}
		if _, err := time.Parse(time.DateOnly, date); err != nil {
			return false, false
		}
		switch op {
		case "equals":
			return asOf == date, true
		case "gt":
			return asOf > date, true
		case "gte":
			return asOf >= date, true
		case "lt":
			return asOf < date, true
		case "lte":
			return asOf <= date, true
		}
	}
	return false, false
}

func (c *modelsDevCost) forDate(asOf string, skipped map[string]bool) *modelsDevCost {
	if c == nil {
		return &modelsDevCost{}
	}
	cost := *c
	for _, price := range []*modelsDevPrice{&cost.Text.Input, &cost.Text.Output, &cost.Audio.Input, &cost.Audio.Output} {
		var overrides []modelsDevPriceOverride
		for _, override := range price.Overrides {
			if override.When = override.When.forDate(asOf, skipped); override.When != nil {
				overrides = append(overrides, override)
			}
		}
		price.Overrides = overrides
	}
	cost.Multipliers = nil
	for _, multiplier := range c.Multipliers {
		if multiplier.When = multiplier.When.forDate(asOf, skipped); multiplier.When != nil {
			cost.Multipliers = append(cost.Multipliers, multiplier)
		}
	}
	return &cost
}

func (c *modelsDevCost) prices() []modelsDevPrice {
	return []modelsDevPrice{c.Text.Input, c.Text.Output, c.Audio.Input, c.Audio.Output}
}

func modelsDevPricing(cost *modelsDevCost, modes map[string]modelsDevMode, provider string, claude bool, label, asOf string, warn func(string, ...any)) (Rates, []Tier, error) {
	skipped := map[string]bool{}
	cost = cost.forDate(asOf, skipped)
	modeCosts := map[string]*modelsDevCost{}
	for _, name := range slices.Sorted(maps.Keys(modes)) {
		mode := modes[name]
		if mode.Cost == nil {
			continue
		}
		service := mode.ServiceTier
		if service == "" {
			service = mode.Body.ServiceTier
		}
		if mode.Body.Speed == "fast" {
			service = "fast"
		}
		if service == "" {
			service = name
		}
		modeCosts[service] = mode.Cost.forDate(asOf, skipped)
	}

	if len(skipped) != 0 {
		warn("%s: skipping unsupported models.dev pricing conditions: %s", label, strings.Join(slices.Sorted(maps.Keys(skipped)), ", "))
	}

	thresholds := map[string]map[uint64]bool{"": {0: true}}
	add := func(condition modelsDevCondition, mode string) {
		over, services, ok := condition.tier()
		if !ok {
			return
		}
		if len(services) == 0 {
			services = []string{mode}
		}
		for _, service := range services {
			if mode != "" && mode != service {
				continue
			}
			switch service {
			case "", "standard", "flex", "priority", "reserved", "ultrafast", "fast":
				if thresholds[service] == nil {
					thresholds[service] = map[uint64]bool{0: true}
				}
				thresholds[service][over] = true
			}
		}
	}
	collect := func(c *modelsDevCost, mode string) {
		for _, price := range c.prices() {
			for _, override := range price.Overrides {
				add(override.When, mode)
			}
		}
		for _, multiplier := range c.Multipliers {
			add(multiplier.When, mode)
		}
	}
	collect(cost, "")
	for service, mode := range modeCosts {
		switch service {
		case "standard", "flex", "priority", "reserved", "ultrafast", "fast":
			if thresholds[service] == nil {
				thresholds[service] = map[uint64]bool{0: true}
			}
			collect(mode, service)
		}
	}

	base, err := cost.rates(0, "", provider, claude, label)
	if err != nil {
		return Rates{}, nil, err
	}
	var tiers []Tier
	for _, service := range slices.Sorted(maps.Keys(thresholds)) {
		points := thresholds[service]
		if service != "" {
			maps.Copy(points, thresholds[""])
		}
		var previous Rates
		for _, over := range slices.Sorted(maps.Keys(points)) {
			if service == "" && over == 0 {
				continue
			}
			rates, err := cost.rates(over, service, provider, claude, label)
			if err != nil {
				return Rates{}, nil, err
			}
			regularInput := cost.Text.Input.at(over, service)
			replacedTTL := regularInput.replacedTTL
			if mode := modeCosts[service]; mode != nil {
				modeRates, err := mode.rates(over, service, provider, claude, label+" "+service)
				if err != nil {
					return Rates{}, nil, err
				}
				if modeRates.CacheWrite != "" {
					modeWrite, _ := mode.Text.Input.at(over, service).cacheWrite()
					regularWrite, _ := regularInput.cacheWrite()
					if modeWrite.Write[""] != "" && (regularInput.replacedTTL || (len(regularWrite.Write) != 0 && regularWrite.Write[""] == "")) {
						replacedTTL = true
						// The mode's scalar replaces the regular TTL schedule.
						rates.CacheWriteDetails = nil
						modeRates.CacheWriteDetails = nil
					}
				}
				rates.overlayWith(modeRates)
			}

			standard := base
			if service != "" {
				standard, err = cost.rates(over, "", provider, claude, label)
				if err != nil {
					return Rates{}, nil, err
				}
				if cost.Text.Input.at(over, "").replacedTTL {
					standard = modelsDevCacheWriteFallback(standard, base)
				}
			}
			if replacedTTL {
				rates = modelsDevCacheWriteFallback(rates, standard)
			}
			effective := standard
			effective.overlayWith(previous)
			if !reflect.DeepEqual(rates, effective) {
				previous = modelsDevRateDelta(rates, standard)
				tiers = append(tiers, Tier{ServiceTier: service, ContextOver: over, Rates: previous})
			}
		}
	}
	return base, tiers, nil
}

// The runtime merges TTL maps. Explicitly price inherited TTLs that now use the default rate.
func modelsDevCacheWriteFallback(rates, inherited Rates) Rates {
	if rates.CacheWrite == "" {
		return rates
	}
	var details map[string]Money
	for ttl := range inherited.CacheWriteDetails {
		if _, ok := rates.CacheWriteDetails[ttl]; ok {
			continue
		}
		if details == nil {
			details = make(map[string]Money, len(inherited.CacheWriteDetails))
			maps.Copy(details, rates.CacheWriteDetails)
		}
		details[ttl] = rates.CacheWrite
	}
	if details != nil {
		rates.CacheWriteDetails = details
	}
	return rates
}

// Keep only prices which differ from the rates the runtime will overlay this tier onto.
func modelsDevRateDelta(rates, base Rates) Rates {
	for _, field := range []struct {
		rate *Money
		base Money
	}{
		{&rates.Input, base.Input},
		{&rates.Output, base.Output},
		{&rates.CacheRead, base.CacheRead},
		{&rates.CacheWrite, base.CacheWrite},
		{&rates.Reasoning, base.Reasoning},
		{&rates.InputAudio, base.InputAudio},
		{&rates.OutputAudio, base.OutputAudio},
		{&rates.PerAudioMinute, base.PerAudioMinute},
	} {
		if *field.rate == field.base {
			*field.rate = ""
		}
	}
	cacheWrites := maps.Clone(rates.CacheWriteDetails)
	for ttl, price := range cacheWrites {
		if base.CacheWriteDetails[ttl] == price {
			delete(cacheWrites, ttl)
		}
	}
	rates.CacheWriteDetails = nil
	if len(cacheWrites) != 0 {
		rates.CacheWriteDetails = cacheWrites
	}
	return rates
}

func (p modelsDevPrice) at(over uint64, service string) modelsDevPriceValues {
	values := p.modelsDevPriceValues
	for _, override := range p.Overrides {
		if !override.When.matches(over, service) {
			continue
		}
		if override.Price != "" {
			values.Price = override.Price
		}
		if override.Reasoning != "" {
			values.Reasoning = override.Reasoning
		}
		previous, _ := values.cacheWrite()
		values.Cache.modelsDevCachePrices.overlay(override.Cache.modelsDevCachePrices)
		values.Cache.Explicit.overlay(override.Cache.Explicit)
		current, _ := values.cacheWrite()
		values.replacedTTL = current.Write[""] != "" && (values.replacedTTL || (len(previous.Write) != 0 && previous.Write[""] == ""))
	}
	return values
}

func (c *modelsDevCost) rates(over uint64, service, provider string, claude bool, label string) (Rates, error) {
	textIn := c.Text.Input.at(over, service)
	textOut := c.Text.Output.at(over, service)
	audioIn := c.Audio.Input.at(over, service)
	audioOut := c.Audio.Output.at(over, service)
	var rates Rates
	var deriveCacheTTL bool
	type price struct {
		target *Money
		value  json.Number
		path   string
	}
	var prices []price
	if unit := c.Text.Input.Unit; unit == "" || unit == "token" {
		read, readPath := textIn.Cache.Read, "text.input.cache.read"
		if read == "" {
			read, readPath = textIn.Cache.Explicit.Read, "text.input.cache.explicit.read"
		}
		prices = append(prices, price{&rates.Input, textIn.Price, "text.input.price"},
			price{&rates.CacheRead, read, readPath})
		cacheWrite, writePath := textIn.cacheWrite()
		writes := cacheWrite.Write
		deriveCacheTTL = writes[""] != "" && !textIn.replacedTTL
		defaultTTL := modelsDevDefaultCacheTTL(writes)
		if value := writes[defaultTTL]; value != "" {
			path := writePath
			if defaultTTL != "" {
				path += "." + defaultTTL
			}
			prices = append(prices, price{&rates.CacheWrite, value, path})
		}
		for _, ttl := range slices.Sorted(maps.Keys(writes)) {
			if ttl == defaultTTL && ttl != "1h" && ttl != "12h" {
				continue
			}
			value := writes[ttl]
			rate, err := c.rate(value, writePath+"."+ttl, over, service, label)
			if err != nil {
				return Rates{}, err
			}
			if rates.CacheWriteDetails == nil {
				rates.CacheWriteDetails = map[string]Money{}
			}
			rates.CacheWriteDetails[ttl] = rate
		}
	}
	if unit := c.Text.Output.Unit; unit == "" || unit == "token" {
		prices = append(prices, price{&rates.Output, textOut.Price, "text.output.price"},
			price{&rates.Reasoning, textOut.Reasoning, "text.output.reasoning"})
	}
	switch c.Audio.Input.Unit {
	case "", "token":
		prices = append(prices, price{&rates.InputAudio, audioIn.Price, "audio.input.price"})
	case "minute", "second":
		prices = append(prices, price{&rates.PerAudioMinute, audioIn.Price, "audio.input.price"})
	}
	if unit := c.Audio.Output.Unit; unit == "" || unit == "token" {
		prices = append(prices, price{&rates.OutputAudio, audioOut.Price, "audio.output.price"})
	}
	for _, p := range prices {
		rate, err := c.rate(p.value, p.path, over, service, label)
		if err != nil {
			return Rates{}, err
		}
		*p.target = rate
	}
	if deriveCacheTTL {
		addDerivedCacheWrite(&rates, provider, claude)
	}
	return rates, nil
}

func (c *modelsDevCost) rate(value json.Number, path string, over uint64, service, label string) (Money, error) {
	if value == "" {
		return "", nil
	}
	rate, err := decimal.NewFromString(value.String())
	if err != nil || rate.IsNegative() {
		return "", fmt.Errorf("%s: invalid %s price %q", label, path, value)
	}
	if path == "audio.input.price" && c.Audio.Input.Unit == "second" {
		rate = rate.Mul(decimal.NewFromInt(60))
	}
	for _, multiplier := range c.Multipliers {
		if !multiplier.When.matches(over, service) {
			continue
		}
		factor, err := modelsDevFactor(multiplier.Factor, strings.Split(path, "."))
		if err != nil {
			return "", fmt.Errorf("%s: invalid %s multiplier: %w", label, path, err)
		}
		rate = rate.Mul(factor)
	}
	return Money(rate.Round(maxFractionalDigits).String()), nil
}

func modelsDevFactor(raw json.RawMessage, path []string) (decimal.Decimal, error) {
	for len(raw) != 0 && raw[0] == '{' && len(path) != 0 {
		var fields map[string]json.RawMessage
		if err := json.Unmarshal(raw, &fields); err != nil {
			return decimal.Zero, err
		}
		raw, path = fields[path[0]], path[1:]
	}
	if len(raw) == 0 || raw[0] == '{' {
		return decimal.NewFromInt(1), nil
	}
	factor, err := decimal.NewFromString(string(raw))
	if err != nil || factor.IsNegative() {
		return decimal.Zero, fmt.Errorf("invalid factor %s", raw)
	}
	return factor, nil
}

func modelsDevDefaultCacheTTL(writes modelsDevCacheWrite) string {
	if _, ok := writes[""]; ok {
		return ""
	}
	var shortest time.Duration
	var selected string
	for _, ttl := range slices.Sorted(maps.Keys(writes)) {
		duration, err := time.ParseDuration(ttl)
		if err == nil && (shortest == 0 || duration < shortest) {
			shortest, selected = duration, ttl
		}
	}
	return selected
}

func addDerivedCacheWrite(r *Rates, provider string, claude bool) {
	var ttl string
	var multiplier decimal.Decimal
	switch {
	case claude:
		ttl, multiplier = "1h", decimal.NewFromInt(2)
	case provider == "openai":
		ttl, multiplier = "12h", decimal.RequireFromString("2.5")
	default:
		return
	}
	if r.CacheWrite == "" || r.CacheWriteDetails[ttl] != "" {
		return
	}
	input, _ := r.Input.Decimal()
	write, _ := r.CacheWrite.Decimal()
	if write.Equal(input.Mul(decimal.RequireFromString("1.25"))) {
		if r.CacheWriteDetails == nil {
			r.CacheWriteDetails = map[string]Money{}
		}
		r.CacheWriteDetails[ttl] = Money(input.Mul(multiplier).Round(maxFractionalDigits).String())
	}
}
