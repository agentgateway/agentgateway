package catalog

import (
	"bufio"
	"context"
	"fmt"
	"io"
	"net/http"
	"regexp"
	"slices"
	"strings"
	"time"
)

const bedrockMantleSourceName = "aws-bedrock-mantle"

// bedrockProviderID is the catalog provider key for AWS Bedrock models (matches modelsDevProviderIDs).
const bedrockProviderID = "aws.bedrock"

// Endpoint tags marking where a Bedrock model is served (must match the proxy's model_catalog::tags).
const (
	runtimeTag = "runtime"
	mantleTag  = "mantle"
)

// endpointTags maps a model card's programmatic-access endpoint to its catalog tag.
var endpointTags = map[string]string{
	"bedrock-runtime": runtimeTag,
	"bedrock-mantle":  mantleTag,
}

const awsMDBaseURL = "https://docs.aws.amazon.com/bedrock/latest/userguide/"
const awsMDAvailURL = awsMDBaseURL + "models-endpoint-availability.md"

var modelIDRe = regexp.MustCompile(`^[a-z0-9][a-z0-9-]*\.[a-z0-9]`)
var mdLinkRe = regexp.MustCompile(`\[.*?\]\(([^)]+)\)`)

func init() {
	importSources[bedrockMantleSourceName] = func(ctx context.Context, _ importOptions) (*ModelCatalog, []string, error) {
		return awsBedrockMantleFetch(ctx)
	}
}

// awsBedrockMantleFetch tags every served Bedrock model with the endpoints its model card lists it under.
// TODO: also emit per-model chat-format tags once the docs expose supported inference APIs.
func awsBedrockMantleFetch(ctx context.Context) (*ModelCatalog, []string, error) {
	client := &http.Client{Timeout: 30 * time.Second}

	body, err := awsMDGetBody(ctx, client, awsMDAvailURL)
	if err != nil {
		return nil, nil, fmt.Errorf("fetch availability page: %w", err)
	}
	cardHrefs, warns := awsMDParseAvailability(body)
	body.Close()

	// Deduplicate hrefs while preserving order.
	seen := make(map[string]bool, len(cardHrefs))
	unique := make([]string, 0, len(cardHrefs))
	for _, h := range cardHrefs {
		if !seen[h] {
			seen[h] = true
			unique = append(unique, h)
		}
	}

	// Accumulate endpoint tags per model ID across all cards before flattening.
	tagSets := make(map[string]map[string]bool)
	for _, href := range unique {
		cardBody, err := awsMDGetBody(ctx, client, awsMDBaseURL+href)
		if err != nil {
			warns = append(warns, fmt.Sprintf("fetch %s: %v", href, err))
			continue
		}
		for id, tags := range awsMDParseModelCard(cardBody) {
			set := tagSets[id]
			if set == nil {
				set = make(map[string]bool)
				tagSets[id] = set
			}
			for _, t := range tags {
				set[t] = true
			}
		}
		cardBody.Close()
	}

	models := make(map[string]Model, len(tagSets))
	for id, set := range tagSets {
		tags := make([]string, 0, len(set))
		for t := range set {
			tags = append(tags, t)
		}
		slices.Sort(tags)
		models[id] = Model{Tags: tags}
	}

	return &ModelCatalog{
		Providers: map[string]Provider{bedrockProviderID: {Models: models}},
	}, warns, nil
}

func awsMDGetBody(ctx context.Context, client *http.Client, url string) (io.ReadCloser, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return nil, err
	}
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

// docScanner allows up to 1MB lines; bufio's 64K default can truncate long markdown rows.
func docScanner(r io.Reader) *bufio.Scanner {
	s := bufio.NewScanner(r)
	s.Buffer(make([]byte, 0, 64*1024), 1024*1024)
	return s
}

// awsMDParseAvailability returns model-card hrefs for every served model. Columns: name(1), runtime(2), mantle(3).
func awsMDParseAvailability(r io.Reader) ([]string, []string) {
	var hrefs []string
	var warns []string
	scanner := docScanner(r)
	for scanner.Scan() {
		line := scanner.Text()
		if !strings.HasPrefix(line, "|") {
			continue
		}
		fields := strings.Split(line, "|")
		// Need at least: | name | bedrock-runtime | bedrock-mantle |
		if len(fields) < 4 {
			continue
		}
		nameCell := strings.TrimSpace(fields[1])
		runtimeCell := strings.TrimSpace(fields[2])
		mantleCell := strings.TrimSpace(fields[3])
		// Skip header rows (**bold**) and separator rows (---)
		if strings.Contains(nameCell, "---") || strings.Contains(nameCell, "**") {
			continue
		}
		// Skip models not served on any endpoint; the card supplies the actual endpoint tags.
		if !strings.Contains(runtimeCell, "icon-yes.png") && !strings.Contains(mantleCell, "icon-yes.png") {
			continue
		}
		m := mdLinkRe.FindStringSubmatch(nameCell)
		if m == nil {
			warns = append(warns, fmt.Sprintf("no model card link in row: %s", nameCell))
			continue
		}
		href := m[1]
		if strings.HasPrefix(href, "model-card-") {
			hrefs = append(hrefs, href)
		}
	}
	return hrefs, warns
}

// awsMDParseModelCard maps each model ID to its (unique, unsorted) endpoint tags from a card's access table.
func awsMDParseModelCard(r io.Reader) map[string][]string {
	sets := make(map[string]map[string]bool)
	scanner := docScanner(r)
	for scanner.Scan() {
		line := scanner.Text()
		if !strings.HasPrefix(line, "|") {
			continue
		}
		fields := strings.Split(line, "|")
		if len(fields) < 3 {
			continue
		}
		tag, ok := endpointTags[strings.TrimSpace(fields[1])]
		if !ok {
			continue
		}
		id := strings.TrimSpace(fields[2])
		if id == "" || strings.Contains(id, "---") || strings.Contains(id, "**") {
			continue
		}
		if !modelIDRe.MatchString(id) {
			continue
		}
		set := sets[id]
		if set == nil {
			set = make(map[string]bool)
			sets[id] = set
		}
		set[tag] = true
	}
	out := make(map[string][]string, len(sets))
	for id, set := range sets {
		tags := make([]string, 0, len(set))
		for t := range set {
			tags = append(tags, t)
		}
		out[id] = tags
	}
	return out
}
