package catalog

import (
	"os"
	"reflect"
	"strings"
	"testing"
	"time"

	"istio.io/istio/pilot/test/util"
)

func TestVertexPricing(t *testing.T) {
	// Relevant tables from Google's pricing page, with presentation attributes removed.
	f, err := os.Open("testdata/vertex-pricing.html")
	if err != nil {
		t.Fatal(err)
	}
	defer f.Close()
	cat, _, err := vertexParsePricing(f, time.Date(2026, time.September, 24, 0, 0, 0, 0, time.UTC))
	if err != nil {
		t.Fatal(err)
	}
	if err := cat.Validate(); err != nil {
		t.Fatal(err)
	}
	got, err := marshalCatalog(cat, true)
	if err != nil {
		t.Fatal(err)
	}
	util.CompareContent(t, got, "testdata/vertex-pricing.golden.json")
}

func TestVertexPricingSwitchesScheduledPrices(t *testing.T) {
	f, err := os.Open("testdata/vertex-pricing.html")
	if err != nil {
		t.Fatal(err)
	}
	defer f.Close()
	cat, _, err := vertexParsePricing(f, time.Date(2026, time.December, 1, 0, 0, 0, 0, time.UTC))
	if err != nil {
		t.Fatal(err)
	}
	got := map[[2]string]Money{}
	for _, tier := range cat.Providers[vertexProviderID].Models["gemini-4-argon"].Tiers {
		got[[2]string{tier.ServiceTier, tier.Region}] = tier.Rates.Input
	}
	want := map[[2]string]Money{
		{"", "regional"}:         "4.40",
		{"priority", ""}:         "7.20",
		{"priority", "regional"}: "7.92",
		{"flex", ""}:             "2.00",
		{"flex", "regional"}:     "2.20",
	}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("gemini-4-argon input rates = %v, want %v", got, want)
	}
}

func TestVertexPricingWarnsAboutModelsAfterScheduledDate(t *testing.T) {
	f, err := os.Open("testdata/vertex-pricing.html")
	if err != nil {
		t.Fatal(err)
	}
	defer f.Close()
	// The page lists "Gemini 4 Argon (Starting December 1, 2026), Gemini 4 Argon Cyber Permissive" in one cell.
	cat, warns, err := vertexParsePricing(f, time.Date(2026, time.December, 1, 0, 0, 0, 0, time.UTC))
	if err != nil {
		t.Fatal(err)
	}
	models := cat.Providers[vertexProviderID].Models
	if _, ok := models["gemini-4-argon"]; !ok {
		t.Error("gemini-4-argon not imported")
	}
	if _, ok := models["gemini-4-argon-cyber-permissive"]; ok {
		t.Error("gemini-4-argon-cyber-permissive imported from a cell that names it after the date")
	}
	if !strings.Contains(strings.Join(warns, "\n"), `"gemini 4 argon cyber permissive"`) {
		t.Errorf("warnings = %q, want one naming the skipped gemini 4 argon cyber permissive", warns)
	}
}

func TestVertexPricingClassifiesTables(t *testing.T) {
	row := func(cells ...string) string {
		return "<tr><td>" + strings.Join(cells, "</td><td>") + "</td></tr>"
	}
	table := func(price string, rows ...string) string {
		return "<table>" + row("Model", "Type", "Region", price) + strings.Join(rows, "") + "</table>"
	}
	tab := func(id, label, content string) string {
		return `<button role="tab" id="` + id + `">` + label + `</button><div role="tabpanel" aria-labelledby="` + id + `">` + content + "</div>"
	}
	rows := func(input, output string) []string {
		return []string{
			row("Gemini X", "Input (text)", "Global", "$"+input),
			row("", "", "Non-global", "$1"+input),
			row("", "Text output", "Global", "$"+output),
			row("", "", "Non-global", "$1"+output),
		}
	}
	priority := table("Price (/1M tokens) with Priority", rows("1", "2")...)
	flex := table("Price (/1M tokens) with Flex/Batch", rows("3", "4")...)
	standard := tab("t0", "Standard Model", table("Price (/1M tokens)", rows("5", "6")...))
	untabbed := table("Price (/1M tokens)", rows("9", "9")...)
	now := time.Date(2026, time.October, 1, 0, 0, 0, 0, time.UTC)

	if _, _, err := vertexParsePricing(strings.NewReader(priority+flex), now); err == nil || !strings.Contains(err.Error(), "standard") {
		t.Errorf("missing standard table: error = %v", err)
	}

	cat, _, err := vertexParsePricing(strings.NewReader(priority+flex+standard+untabbed), now)
	if err != nil {
		t.Fatal(err)
	}
	rates := func(input, output Money) Rates {
		return Rates{Input: input, Output: output, Reasoning: output}
	}
	want := []Tier{
		{ServiceTier: "priority", Rates: rates("1", "2")},
		{ServiceTier: "priority", Region: "regional", Rates: rates("11", "12")},
		{ServiceTier: "flex", Rates: rates("3", "4")},
		{ServiceTier: "flex", Region: "regional", Rates: rates("13", "14")},
		{Region: "regional", Rates: rates("15", "16")},
	}
	if got := cat.Providers[vertexProviderID].Models["gemini-x"].Tiers; !reflect.DeepEqual(got, want) {
		t.Errorf("tiers = %+v\nwant %+v", got, want)
	}
}

func TestVertexPricingCacheReadAndWriteColumns(t *testing.T) {
	row := func(cells ...string) string {
		return "<tr><td>" + strings.Join(cells, "</td><td>") + "</td></tr>"
	}
	// Gemini 4 tables label the cached input column "Cache read" and add a "Cache write" column
	// whose prices carry a footnote marker.
	table := func(id, label, input, cacheRead, cacheWrite, output string) string {
		return `<button role="tab" id="` + id + `">` + label + `</button><div role="tabpanel" aria-labelledby="` + id + `"><table>` +
			row("Model", "Type", "Region", "Price (/1M tokens)", "Price (/1M tokens) *Cache read", "Price (/1M tokens)Cache write") +
			row("Gemini Y", "Input (text)", "Global", "$"+input, "$"+cacheRead, "$"+cacheWrite+" ***") +
			row("", "", "Non-global", "$1"+input, "$1"+cacheRead, "$1"+cacheWrite+" ***") +
			row("", "Text output", "Global", "$"+output, "$"+output, "$"+output) +
			row("", "", "Non-global", "$1"+output, "$1"+output, "$1"+output) +
			"</table></div>"
	}
	page := table("t0", "Standard Model", "1", "0.1", "9", "2") +
		table("t1", "Priority", "3", "0.3", "9", "4") +
		table("t2", "Flex/Batch", "5", "0.5", "9", "6")
	cat, _, err := vertexParsePricing(strings.NewReader(page), time.Date(2026, time.October, 1, 0, 0, 0, 0, time.UTC))
	if err != nil {
		t.Fatal(err)
	}
	rates := func(input, cacheRead, output Money) Rates {
		return Rates{Input: input, CacheRead: cacheRead, Output: output, Reasoning: output}
	}
	want := []Tier{
		{Region: "regional", Rates: rates("11", "10.1", "12")},
		{ServiceTier: "priority", Rates: rates("3", "0.3", "4")},
		{ServiceTier: "priority", Region: "regional", Rates: rates("13", "10.3", "14")},
		{ServiceTier: "flex", Rates: rates("5", "0.5", "6")},
		{ServiceTier: "flex", Region: "regional", Rates: rates("15", "10.5", "16")},
	}
	if got := cat.Providers[vertexProviderID].Models["gemini-y"].Tiers; !reflect.DeepEqual(got, want) {
		t.Errorf("tiers = %+v\nwant %+v", got, want)
	}
}

func TestVertexClaudeRegionalTiers(t *testing.T) {
	rates := Rates{Input: "3", Output: "15", CacheRead: "0.3", CacheWrite: "3.75", CacheWriteDetails: map[string]Money{"1h": "6"}}
	got, err := vertexClaudeRegionalTiers(map[string]Model{
		"claude-sonnet-4-5@20250929": {Rates: rates},
		"claude-opus-5@default":      {Rates: Rates{Input: "5"}},
		"claude-haiku-5-5@default": {Rates: Rates{Input: "0.1"}, Tiers: []Tier{
			{ContextOver: 100000, Rates: Rates{Input: "0.5"}},
			{ServiceTier: "priority", Rates: Rates{Input: "0.2"}},
		}},
		// Older Claude models have no regional premium.
		"claude-opus-4-1@20250805":      {Rates: rates},
		"claude-sonnet-4@20250514":      {Rates: rates},
		"claude-3-5-sonnet-v2@20241022": {Rates: rates},
		"gemini-3.1-pro":                {Rates: rates},
	})
	if err != nil {
		t.Fatal(err)
	}
	want := map[string]Model{
		"claude-sonnet-4-5@20250929": {Tiers: []Tier{{Region: "regional", Rates: Rates{
			Input: "3.3", Output: "16.5", CacheRead: "0.33", CacheWrite: "4.125", CacheWriteDetails: map[string]Money{"1h": "6.6"},
		}}}},
		"claude-opus-5@default": {Tiers: []Tier{{Region: "regional", Rates: Rates{Input: "5.5"}}}},
		"claude-haiku-5-5@default": {Tiers: []Tier{
			{Region: "regional", Rates: Rates{Input: "0.11"}},
			{ContextOver: 100000, Region: "regional", Rates: Rates{Input: "0.55"}},
		}},
	}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("tiers = %+v, want %+v", got, want)
	}
	for id, m := range got {
		if err := m.validate(); err != nil {
			t.Errorf("%s: %v", id, err)
		}
	}
}
