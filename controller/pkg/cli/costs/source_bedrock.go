package costs

import (
	"bufio"
	"context"
	"fmt"
	"io"
	"net/http"
	"regexp"
	"strings"
	"time"
)

const bedrockMantleSourceName = "aws-bedrock-mantle"

// bedrockProviderID is the cost-catalog provider key for AWS Bedrock models (matches modelsDevProviderIDs).
const bedrockProviderID = "aws.bedrock"

const awsMDBaseURL = "https://docs.aws.amazon.com/bedrock/latest/userguide/"
const awsMDAvailURL = awsMDBaseURL + "models-endpoint-availability.md"

var modelIDRe = regexp.MustCompile(`^[a-z0-9][a-z0-9-]*\.[a-z0-9]`)
var mdLinkRe = regexp.MustCompile(`\[.*?\]\(([^)]+)\)`)

func init() {
	importSources[bedrockMantleSourceName] = func(ctx context.Context, _ importOptions) (*ModelCatalog, []string, error) {
		return awsBedrockMantleFetch(ctx)
	}
}

// awsBedrockMantleFetch scrapes the AWS availability page for Mantle-only models
// (bedrock-mantle=yes, bedrock-runtime=no) and returns them flagged in the catalog.
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

	models := make(map[string]Model)
	for _, href := range unique {
		cardBody, err := awsMDGetBody(ctx, client, awsMDBaseURL+href)
		if err != nil {
			warns = append(warns, fmt.Sprintf("fetch %s: %v", href, err))
			continue
		}
		for _, id := range awsMDParseModelCard(cardBody) {
			models[id] = Model{Mantle: true}
		}
		cardBody.Close()
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

// awsMDParseAvailability returns model-card hrefs for Mantle-only models
// (bedrock-mantle=yes, bedrock-runtime=no). Columns: name(1), runtime(2), mantle(3).
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
		// Mantle-only: present on Mantle, absent from Runtime.
		if !strings.Contains(mantleCell, "icon-yes.png") {
			continue
		}
		if strings.Contains(runtimeCell, "icon-yes.png") {
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

// awsMDParseModelCard returns bedrock-mantle model IDs from a model card's
// Programmatic Access table (rows: | bedrock-mantle | <model-id> | ... |).
func awsMDParseModelCard(r io.Reader) []string {
	var ids []string
	seen := make(map[string]bool)
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
		endpoint := strings.TrimSpace(fields[1])
		if endpoint != "bedrock-mantle" {
			continue
		}
		id := strings.TrimSpace(fields[2])
		if id == "" || strings.Contains(id, "---") || strings.Contains(id, "**") {
			continue
		}
		if !modelIDRe.MatchString(id) {
			continue
		}
		if !seen[id] {
			seen[id] = true
			ids = append(ids, id)
		}
	}
	return ids
}
