package catalog

import (
	"context"
	"fmt"
	"io"
	"maps"
	"net/http"
	"reflect"
	"regexp"
	"slices"
	"strconv"
	"strings"
	"time"

	"github.com/shopspring/decimal"
	"golang.org/x/net/html"
)

const vertexSourceName = "vertex-pricing"
const vertexProviderID = "gcp.vertex_ai"
const vertexPricingURL = "https://cloud.google.com/gemini-enterprise-agent-platform/generative-ai/pricing?hl=en"

func init() {
	importSources[vertexSourceName] = func(ctx context.Context, opts importOptions) (*ModelCatalog, []string, error) {
		matches := func(p string) bool {
			id, ok := modelsDevMapProviderID(p)
			return ok && id == vertexProviderID
		}
		if (len(opts.providers) > 0 && !slices.ContainsFunc(opts.providers, matches)) || slices.ContainsFunc(opts.excludeProviders, matches) {
			return &ModelCatalog{}, nil, nil
		}
		req, err := http.NewRequestWithContext(ctx, http.MethodGet, vertexPricingURL, nil)
		if err != nil {
			return nil, nil, err
		}
		// Google serves an empty page to Go's default User-Agent.
		req.Header.Set("User-Agent", "agctl")
		client := &http.Client{Timeout: 30 * time.Second}
		resp, err := client.Do(req)
		if err != nil {
			return nil, nil, err
		}
		defer resp.Body.Close()
		if resp.StatusCode != http.StatusOK {
			return nil, nil, fmt.Errorf("fetch Vertex pricing: HTTP %d", resp.StatusCode)
		}
		cat, warnings, err := vertexParsePricing(resp.Body, time.Now())
		if err != nil {
			return nil, nil, err
		}
		// Only augment models defined by earlier sources.
		known := opts.merged.Providers[vertexProviderID].Models
		maps.DeleteFunc(cat.Providers[vertexProviderID].Models, func(id string, _ Model) bool {
			_, ok := known[id]
			return !ok
		})
		claude, err := vertexClaudeRegionalTiers(known)
		if err != nil {
			return nil, nil, err
		}
		maps.Copy(cat.Providers[vertexProviderID].Models, claude)
		return cat, warnings, nil
	}
}

// vertexClaudeVersion matches Claude model IDs on Vertex, such as claude-sonnet-4-5@20250929.
var vertexClaudeVersion = regexp.MustCompile(`^claude-[a-z]+-(\d+)(?:-(\d+))?@`)

// vertexClaudeRegionalPremium is the regional and multi-region premium over global endpoints for
// Claude 4.5 and later models.
var vertexClaudeRegionalPremium = decimal.RequireFromString("1.1")

// vertexClaudeRegionalTiers returns regional tiers for Claude 4.5 and later models, derived from
// their global standard rates and long-context tiers.
func vertexClaudeRegionalTiers(known map[string]Model) (map[string]Model, error) {
	out := map[string]Model{}
	for id, model := range known {
		m := vertexClaudeVersion.FindStringSubmatch(id)
		if m == nil {
			continue
		}
		major, _ := strconv.Atoi(m[1])
		minor, _ := strconv.Atoi(m[2])
		if major < 4 || (major == 4 && minor < 5) {
			continue
		}
		var tiers []Tier
		if !model.Rates.IsZero() {
			rates, err := scaleRates(model.Rates, vertexClaudeRegionalPremium)
			if err != nil {
				return nil, fmt.Errorf("%s: %w", id, err)
			}
			tiers = append(tiers, Tier{Region: "regional", Rates: rates})
		}
		for _, tier := range model.Tiers {
			if tier.ServiceTier != "" || tier.Region != "" {
				continue
			}
			rates, err := scaleRates(tier.Rates, vertexClaudeRegionalPremium)
			if err != nil {
				return nil, fmt.Errorf("%s: %w", id, err)
			}
			tiers = append(tiers, Tier{ContextOver: tier.ContextOver, Region: "regional", Rates: rates})
		}
		if len(tiers) > 0 {
			out[id] = Model{Tiers: tiers}
		}
	}
	return out, nil
}

// scaleRates multiplies every set rate by factor.
func scaleRates(r Rates, factor decimal.Decimal) (Rates, error) {
	scale := func(m Money) (Money, error) {
		if m == "" {
			return m, nil
		}
		d, err := m.Decimal()
		if err != nil {
			return "", err
		}
		return Money(d.Mul(factor).Round(maxFractionalDigits).String()), nil
	}
	out := r
	// Walk every Money field, so rates added to Rates later are scaled too.
	fields := reflect.ValueOf(&out).Elem()
	for _, field := range fields.Fields() {
		m, ok := reflect.TypeAssert[*Money](field.Addr())
		if !ok {
			continue
		}
		scaled, err := scale(*m)
		if err != nil {
			return Rates{}, err
		}
		*m = scaled
	}
	out.CacheWriteDetails = maps.Clone(r.CacheWriteDetails)
	for ttl, m := range out.CacheWriteDetails {
		scaled, err := scale(m)
		if err != nil {
			return Rates{}, err
		}
		out.CacheWriteDetails[ttl] = scaled
	}
	return out, nil
}

// vertexParsePricing imports service tier prices, and Non-global prices as regional tiers.
// Standard Global prices are the base rates from models.dev.
// Flex and Batch share a table; Batch-only models get a harmless flex tier.
func vertexParsePricing(r io.Reader, now time.Time) (*ModelCatalog, []string, error) {
	doc, err := html.Parse(r)
	if err != nil {
		return nil, nil, err
	}
	models := map[string]Model{}
	services := map[string]bool{}
	var warns []string
	for table := range doc.Descendants() {
		if table.Type != html.ElementNode || table.Data != "table" {
			continue
		}
		rows := vertexTableRows(table)
		if len(rows) == 0 {
			continue
		}
		header := rows[0]
		service := vertexTableService(table, header)
		regionCol := slices.Index(header, "Region")
		if service == "" || (service == "standard" && regionCol < 0) {
			continue
		}
		services[service] = true
		// Standard prices are the base rates, so they only contribute region tiers.
		tierService := service
		if service == "standard" {
			tierService = ""
		}
		// Column index per [context tier] for input and cached input prices; -1 if absent.
		inputCols, cacheCols := [2]int{-1, -1}, [2]int{-1, -1}
		// Columns without a context threshold apply to all context sizes.
		for i, h := range header {
			if !strings.HasPrefix(h, "Price") {
				continue
			}
			ctx := 0
			if strings.Contains(h, "> 200K") {
				ctx = 1
			}
			switch lower := strings.ToLower(h); {
			case strings.Contains(lower, "cache write"):
				// Cache write pricing is announced ahead of its effective date, with no
				// machine-readable schedule, so it is not imported.
				continue
			case strings.Contains(lower, "cached input"), strings.Contains(lower, "cache read"):
				cacheCols[ctx] = i
			default:
				inputCols[ctx] = i
			}
		}
		if inputCols[0] < 0 {
			return nil, nil, fmt.Errorf("unrecognized Vertex %s pricing columns: %v", service, header)
		}
		id, usage := "", ""
		for _, row := range rows[1:] {
			if len(row) != len(header) {
				return nil, nil, fmt.Errorf("Vertex %s row has %d columns, expected %d", service, len(row), len(header))
			}
			if row[0] != "" {
				id, usage = "", ""
				// Scheduled price changes are listed as "<model> through <date>" and "<model> starting <date>".
				name := strings.ToLower(row[0])
				active := true
				for _, kw := range []string{"through", "starting"} {
					before, date, ok := strings.Cut(name, kw)
					if !ok {
						continue
					}
					date, rest, _ := strings.Cut(date, ")")
					// A cell can name further models after the date, which the row format cannot attribute.
					if extra := strings.Trim(rest, " ,"); extra != "" {
						warn := fmt.Sprintf("Vertex: skipped %q listed after the date in %q", extra, row[0])
						if !slices.Contains(warns, warn) {
							warns = append(warns, warn)
						}
					}
					d, err := time.Parse("January 2, 2006", strings.TrimSpace(date))
					if err != nil {
						return nil, nil, fmt.Errorf("Vertex %s model %q: %w", service, row[0], err)
					}
					if kw == "through" {
						active = now.Before(d.AddDate(0, 0, 1))
					} else {
						active = !now.Before(d)
					}
					name = before
					break
				}
				// Live API models price each modality separately, which the catalog cannot express.
				if active && !strings.Contains(name, "live api") {
					name, _, _ = strings.Cut(name, "(")
					id = strings.Join(strings.Fields(strings.Trim(name, " *")), "-")
				}
			}
			if row[1] != "" {
				usage = strings.ToLower(row[1])
			}
			if id == "" {
				continue
			}
			region := ""
			if regionCol >= 0 {
				switch strings.TrimSuffix(row[regionCol], "*") {
				case "Global", "Global (Flex)":
				case "Non-global":
					region = "regional"
				default:
					continue
				}
			}
			if service == "standard" && region == "" {
				continue
			}
			if row[inputCols[0]] == "N/A" && (inputCols[1] < 0 || row[inputCols[1]] == "N/A") {
				continue
			}
			field := ""
			switch {
			case strings.Contains(usage, "input") && strings.Contains(usage, "audio") && !strings.Contains(usage, "text"):
				field = "audio"
			case strings.HasPrefix(usage, "input"):
				field = "input"
			case strings.HasPrefix(usage, "text output"):
				field = "output"
			case strings.HasPrefix(usage, "image output"):
				continue
			default:
				return nil, nil, fmt.Errorf("unrecognized Vertex usage %q for %s %s", usage, id, service)
			}
			model := models[id]
			for i, threshold := range []uint64{0, 200000} {
				if inputCols[i] < 0 {
					continue
				}
				value := row[inputCols[i]]
				if value == "N/A" {
					continue
				}
				rate, err := vertexMoney(value)
				if err != nil {
					return nil, nil, fmt.Errorf("%s %s: %w", id, service, err)
				}
				idx := slices.IndexFunc(model.Tiers, func(t Tier) bool {
					return t.ServiceTier == tierService && t.Region == region && t.ContextOver == threshold
				})
				if idx < 0 {
					model.Tiers = append(model.Tiers, Tier{ServiceTier: tierService, Region: region, ContextOver: threshold})
					idx = len(model.Tiers) - 1
				}
				rates := &model.Tiers[idx].Rates
				var dst *Money
				switch field {
				case "input":
					dst = &rates.Input
				case "audio":
					dst = &rates.InputAudio
				case "output":
					dst = &rates.Output
				}
				if *dst != "" {
					return nil, nil, fmt.Errorf("duplicate Vertex price for %s %s %s %s context %d", id, service, region, field, threshold)
				}
				*dst = rate
				switch field {
				case "output":
					rates.Reasoning = rate
				case "input":
					if strings.Contains(usage, "audio") {
						rates.InputAudio = rate
					}
					if cacheCols[i] >= 0 && row[cacheCols[i]] != "N/A" {
						if rates.CacheRead, err = vertexMoney(row[cacheCols[i]]); err != nil {
							return nil, nil, fmt.Errorf("%s %s: %w", id, service, err)
						}
					}
				}
			}
			models[id] = model
		}
	}
	for _, service := range []string{"standard", "priority", "flex"} {
		if !services[service] {
			return nil, nil, fmt.Errorf("no Vertex %s pricing tables found", service)
		}
	}
	for id, model := range models {
		for _, tier := range model.Tiers {
			if tier.Rates.Input == "" || tier.Rates.Output == "" {
				return nil, nil, fmt.Errorf("incomplete Vertex %s %s prices for %s context %d", tier.ServiceTier, tier.Region, id, tier.ContextOver)
			}
		}
		// Equal long-context prices need no additional tier.
		model.Tiers = slices.DeleteFunc(model.Tiers, func(t Tier) bool {
			return t.ContextOver != 0 && slices.ContainsFunc(model.Tiers, func(base Tier) bool {
				return base.ContextOver == 0 && base.ServiceTier == t.ServiceTier && base.Region == t.Region && reflect.DeepEqual(base.Rates, t.Rates)
			})
		})
		models[id] = model
	}
	return &ModelCatalog{Providers: map[string]Provider{vertexProviderID: {Models: models}}}, warns, nil
}

func vertexMoney(value string) (Money, error) {
	rate := Money(strings.TrimPrefix(value, "$"))
	if !strings.HasPrefix(value, "$") || rate.validate() != nil {
		return "", fmt.Errorf("invalid Vertex price %q", value)
	}
	return rate, nil
}

var vertexFootnote = regexp.MustCompile(`\[\d+\]`)

// vertexTableService reads the service tier from the header, or else from the table's tab label.
func vertexTableService(table *html.Node, header []string) string {
	switch joined := strings.Join(header, " "); {
	case strings.Contains(joined, "with Priority"):
		return "priority"
	case strings.Contains(joined, "with Flex/Batch"):
		return "flex"
	}
	switch label := vertexTabLabel(table); {
	case strings.HasPrefix(label, "Standard"):
		return "standard"
	case label == "Priority":
		return "priority"
	case strings.Contains(label, "Flex") || strings.Contains(label, "Batch"):
		return "flex"
	}
	return ""
}

func vertexTabLabel(n *html.Node) string {
	for panel := n.Parent; panel != nil; panel = panel.Parent {
		if vertexAttr(panel, "role") != "tabpanel" {
			continue
		}
		id := vertexAttr(panel, "aria-labelledby")
		root := panel
		for root.Parent != nil {
			root = root.Parent
		}
		for tab := range root.Descendants() {
			if id != "" && tab.Type == html.ElementNode && vertexAttr(tab, "id") == id {
				return strings.Join(strings.Fields(vertexText(tab)), " ")
			}
		}
		return ""
	}
	return ""
}

func vertexAttr(n *html.Node, key string) string {
	for _, a := range n.Attr {
		if a.Key == key {
			return a.Val
		}
	}
	return ""
}

func vertexText(n *html.Node) string {
	if n.Type == html.TextNode {
		return n.Data
	}
	var s strings.Builder
	for c := n.FirstChild; c != nil; c = c.NextSibling {
		s.WriteString(vertexText(c))
	}
	return s.String()
}

// Tables use explicit empty cells for repeated model and usage labels.
func vertexTableRows(table *html.Node) [][]string {
	var rows [][]string
	var walk func(*html.Node)
	walk = func(n *html.Node) {
		if n.Type == html.ElementNode && n.Data == "tr" {
			var row []string
			for c := n.FirstChild; c != nil; c = c.NextSibling {
				if c.Type == html.ElementNode && (c.Data == "td" || c.Data == "th") {
					row = append(row, strings.Join(strings.Fields(vertexFootnote.ReplaceAllString(vertexText(c), "")), " "))
				}
			}
			if len(row) > 0 {
				rows = append(rows, row)
			}
			return
		}
		for c := n.FirstChild; c != nil; c = c.NextSibling {
			walk(c)
		}
	}
	walk(table)
	return rows
}
