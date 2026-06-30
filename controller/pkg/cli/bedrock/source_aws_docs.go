package bedrock

import (
	"context"
	"fmt"
	"io"
	"net/http"
	"regexp"
	"sort"
	"strings"
	"time"

	"golang.org/x/net/html"
)

const awsDocsSourceName = "aws-docs"
const awsDocsBaseURL = "https://docs.aws.amazon.com/bedrock/latest/userguide/"
const awsDocsAvailURL = awsDocsBaseURL + "models-endpoint-availability.html"

// CRIS inference-profile geo prefixes to strip when normalizing scraped IDs.
var scrapeCRISPrefixes = []string{"us-gov.", "us.", "eu.", "apac.", "au.", "global.", "ca.", "sa.", "jp.", "il."}

// modelIDRe matches strings that look like Bedrock model IDs (with or without a CRIS prefix).
var modelIDRe = regexp.MustCompile(`^([a-z]{2,6}\.)?[a-z0-9][a-z0-9-]*\.[a-z0-9]`)

func init() {
	importSources[awsDocsSourceName] = func(ctx context.Context) (*RuntimeTable, []string, error) {
		return awsDocsFetch(ctx)
	}
}

func awsDocsFetch(ctx context.Context) (*RuntimeTable, []string, error) {
	client := &http.Client{Timeout: 30 * time.Second}

	avail, err := awsDocsGetBody(ctx, client, awsDocsAvailURL)
	if err != nil {
		return nil, nil, fmt.Errorf("fetch availability page: %w", err)
	}
	cardHrefs, warns, err := awsDocsParseAvailability(avail)
	avail.Close()
	if err != nil {
		return nil, warns, fmt.Errorf("parse availability page: %w", err)
	}

	// Deduplicate hrefs while preserving order.
	seen := make(map[string]bool, len(cardHrefs))
	uniqueHrefs := cardHrefs[:0]
	for _, h := range cardHrefs {
		if !seen[h] {
			seen[h] = true
			uniqueHrefs = append(uniqueHrefs, h)
		}
	}

	ids := make(map[string]struct{})
	for _, href := range uniqueHrefs {
		cardBody, err := awsDocsGetBody(ctx, client, awsDocsBaseURL+href)
		if err != nil {
			warns = append(warns, fmt.Sprintf("fetch %s: %v", href, err))
			continue
		}
		cardIDs, parseErr := awsDocsParseModelCard(cardBody)
		cardBody.Close()
		if parseErr != nil {
			warns = append(warns, fmt.Sprintf("parse %s: %v", href, parseErr))
			continue
		}
		for _, id := range cardIDs {
			ids[id] = struct{}{}
		}
		time.Sleep(50 * time.Millisecond)
	}

	models := make([]string, 0, len(ids))
	for id := range ids {
		models = append(models, id)
	}
	sort.Strings(models)

	return &RuntimeTable{
		Source: awsDocsAvailURL,
		Models: models,
	}, warns, nil
}

func awsDocsGetBody(ctx context.Context, client *http.Client, url string) (io.ReadCloser, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("User-Agent", "Mozilla/5.0 (bedrock-runtime-table-generator)")
	resp, err := client.Do(req)
	if err != nil {
		return nil, err
	}
	if resp.StatusCode != http.StatusOK {
		resp.Body.Close()
		return nil, fmt.Errorf("HTTP %d", resp.StatusCode)
	}
	return resp.Body, nil
}

// awsDocsParseAvailability parses the endpoint-availability page and returns
// the card hrefs (e.g. "model-card-amazon-nova-pro.html") for all models that
// have bedrock-runtime support.
func awsDocsParseAvailability(r io.Reader) ([]string, []string, error) {
	doc, err := html.Parse(r)
	if err != nil {
		return nil, nil, err
	}

	var cardHrefs []string
	var warns []string
	for _, table := range collectNodes(doc, "table") {
		hrefs, ws := awsDocsParseTable(table)
		cardHrefs = append(cardHrefs, hrefs...)
		warns = append(warns, ws...)
	}
	return cardHrefs, warns, nil
}

func awsDocsParseTable(table *html.Node) ([]string, []string) {
	rows := collectNodes(table, "tr")
	if len(rows) == 0 {
		return nil, nil
	}

	// Only process tables whose header mentions bedrock-runtime.
	if !strings.Contains(nodeText(rows[0]), "bedrock-runtime") {
		return nil, nil
	}

	var cardHrefs []string
	var warns []string

	for _, row := range rows[1:] {
		tds := directChildren(row, "td")
		if len(tds) < 2 {
			continue
		}
		// Second column: bedrock-runtime support icon.
		if !cellHasIcon(tds[1], "icon-yes.png") {
			continue
		}
		href := findHref(tds[0])
		if href == "" {
			warns = append(warns, fmt.Sprintf("no model card link for %q", nodeText(tds[0])))
			continue
		}
		href = strings.TrimPrefix(href, "./")
		if strings.HasPrefix(href, "model-card-") {
			cardHrefs = append(cardHrefs, href)
		}
	}
	return cardHrefs, warns
}

// awsDocsParseModelCard extracts base model IDs from a model card page.
// IDs are found in <code> elements, CRIS geo prefixes are stripped.
func awsDocsParseModelCard(r io.Reader) ([]string, error) {
	doc, err := html.Parse(r)
	if err != nil {
		return nil, err
	}

	seen := make(map[string]bool)
	var ids []string
	for _, code := range collectNodes(doc, "code") {
		text := strings.TrimSpace(nodeText(code))
		if strings.Contains(text, " ") {
			continue
		}
		if !modelIDRe.MatchString(text) {
			continue
		}
		base := stripScrapedCRIS(text)
		if strings.Count(base, ".") < 1 {
			continue
		}
		if !seen[base] {
			seen[base] = true
			ids = append(ids, base)
		}
	}
	return ids, nil
}

func stripScrapedCRIS(id string) string {
	for _, p := range scrapeCRISPrefixes {
		if strings.HasPrefix(id, p) {
			return id[len(p):]
		}
	}
	return id
}

// collectNodes returns all descendant nodes with the given element tag.
func collectNodes(n *html.Node, tag string) []*html.Node {
	var result []*html.Node
	var walk func(*html.Node)
	walk = func(node *html.Node) {
		if node.Type == html.ElementNode && node.Data == tag {
			result = append(result, node)
		}
		for c := node.FirstChild; c != nil; c = c.NextSibling {
			walk(c)
		}
	}
	walk(n)
	return result
}

// directChildren returns the immediate element children of n with the given tag.
func directChildren(n *html.Node, tag string) []*html.Node {
	var children []*html.Node
	for c := n.FirstChild; c != nil; c = c.NextSibling {
		if c.Type == html.ElementNode && c.Data == tag {
			children = append(children, c)
		}
	}
	return children
}

// nodeText returns the concatenated text content of n and all its descendants.
func nodeText(n *html.Node) string {
	var sb strings.Builder
	var walk func(*html.Node)
	walk = func(node *html.Node) {
		if node.Type == html.TextNode {
			sb.WriteString(node.Data)
		}
		for c := node.FirstChild; c != nil; c = c.NextSibling {
			walk(c)
		}
	}
	walk(n)
	return strings.TrimSpace(sb.String())
}

// cellHasIcon reports whether a <td> node contains an <img> whose src includes iconName.
func cellHasIcon(td *html.Node, iconName string) bool {
	for _, img := range collectNodes(td, "img") {
		for _, a := range img.Attr {
			if a.Key == "src" && strings.Contains(a.Val, iconName) {
				return true
			}
		}
	}
	return false
}

// findHref returns the href of the first <a> element found in n.
func findHref(n *html.Node) string {
	if n.Type == html.ElementNode && n.Data == "a" {
		for _, a := range n.Attr {
			if a.Key == "href" {
				return a.Val
			}
		}
	}
	for c := n.FirstChild; c != nil; c = c.NextSibling {
		if href := findHref(c); href != "" {
			return href
		}
	}
	return ""
}
