package catalog

import (
	"context"
	"fmt"
	"io"
	"net/http"
	"slices"
	"strings"
	"time"

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
		return vertexParsePricing(resp.Body, time.Now())
	}
}

// vertexParsePricing imports Global prices. The catalog cannot select by region yet.
// Flex and Batch share a table; Batch-only models get a harmless flex tier.
func vertexParsePricing(r io.Reader, now time.Time) (*ModelCatalog, []string, error) {
	doc, err := html.Parse(r)
	if err != nil {
		return nil, nil, err
	}
	models := map[string]Model{}
	var warnings []string
	services := map[string]bool{}
	for table := range doc.Descendants() {
		if table.Type != html.ElementNode || table.Data != "table" {
			continue
		}
		rows := vertexTableRows(table)
		if len(rows) == 0 {
			continue
		}
		header := rows[0]
		service := ""
		switch joined := strings.Join(header, " "); {
		case strings.Contains(joined, "with Priority"):
			service = "priority"
		case strings.Contains(joined, "with Flex/Batch"):
			service = "flex"
		default:
			continue
		}
		services[service] = true
		regionCol := slices.Index(header, "Region")
		// Column index per [context tier] for input and cached input prices; -1 if absent.
		inputCols, cacheCols := [2]int{-1, -1}, [2]int{-1, -1}
		for i, h := range header {
			ctx := 0
			if strings.Contains(h, "> 200K") {
				ctx = 1
			} else if !strings.Contains(h, "<= 200K") {
				continue
			}
			if strings.Contains(h, "cached input") {
				cacheCols[ctx] = i
			} else {
				inputCols[ctx] = i
			}
		}
		if inputCols[0] < 0 || inputCols[1] < 0 {
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
				if active {
					name, _, _ = strings.Cut(name, "(")
					id = strings.Join(strings.Fields(strings.Trim(name, " *")), "-")
				}
			}
			if row[1] != "" {
				usage = strings.ToLower(row[1])
			}
			if id == "" || (regionCol >= 0 && row[regionCol] != "Global" && row[regionCol] != "Global (Flex)") {
				continue
			}
			if row[inputCols[0]] == "N/A" && row[inputCols[1]] == "N/A" {
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
				warnings = append(warnings, fmt.Sprintf("%s %s: image-output pricing is not represented by the catalog", id, service))
				continue
			default:
				return nil, nil, fmt.Errorf("unrecognized Vertex usage %q for %s %s", usage, id, service)
			}
			model := models[id]
			for i, threshold := range []uint64{0, 200000} {
				value := row[inputCols[i]]
				if value == "N/A" {
					continue
				}
				rate, err := vertexMoney(value)
				if err != nil {
					return nil, nil, fmt.Errorf("%s %s: %w", id, service, err)
				}
				idx := slices.IndexFunc(model.Tiers, func(t Tier) bool { return t.ServiceTier == service && t.ContextOver == threshold })
				if idx < 0 {
					model.Tiers = append(model.Tiers, Tier{ServiceTier: service, ContextOver: threshold})
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
					return nil, nil, fmt.Errorf("duplicate Vertex price for %s %s %s context %d", id, service, field, threshold)
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
	for _, service := range []string{"priority", "flex"} {
		if !services[service] {
			return nil, nil, fmt.Errorf("no Vertex %s pricing tables found", service)
		}
	}
	for id, model := range models {
		for _, tier := range model.Tiers {
			if tier.Rates.Input == "" || tier.Rates.Output == "" {
				return nil, nil, fmt.Errorf("incomplete Vertex %s prices for %s context %d", tier.ServiceTier, id, tier.ContextOver)
			}
		}
		// Equal long-context prices need no additional tier.
		model.Tiers = slices.DeleteFunc(model.Tiers, func(t Tier) bool {
			return t.ContextOver != 0 && slices.ContainsFunc(model.Tiers, func(base Tier) bool {
				return base.ContextOver == 0 && base.ServiceTier == t.ServiceTier && base.Rates == t.Rates
			})
		})
		models[id] = model
	}
	return &ModelCatalog{Providers: map[string]Provider{vertexProviderID: {Models: models}}}, warnings, nil
}

func vertexMoney(value string) (Money, error) {
	rate := Money(strings.TrimPrefix(value, "$"))
	if !strings.HasPrefix(value, "$") || rate.validate() != nil {
		return "", fmt.Errorf("invalid Vertex price %q", value)
	}
	return rate, nil
}

// Tables use explicit empty cells for repeated model and usage labels.
func vertexTableRows(table *html.Node) [][]string {
	var rows [][]string
	var text func(*html.Node) string
	text = func(n *html.Node) string {
		if n.Type == html.TextNode {
			return n.Data
		}
		var s strings.Builder
		for c := n.FirstChild; c != nil; c = c.NextSibling {
			s.WriteString(text(c))
		}
		return s.String()
	}
	var walk func(*html.Node)
	walk = func(n *html.Node) {
		if n.Type == html.ElementNode && n.Data == "tr" {
			var row []string
			for c := n.FirstChild; c != nil; c = c.NextSibling {
				if c.Type == html.ElementNode && (c.Data == "td" || c.Data == "th") {
					row = append(row, strings.Join(strings.Fields(text(c)), " "))
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
