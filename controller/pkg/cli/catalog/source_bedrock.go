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
	"sync"
	"time"

	"golang.org/x/sync/errgroup"
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

// maxConcurrentCardFetches bounds how many model-card pages we scrape in parallel.
const maxConcurrentCardFetches = 12

var modelIDRe = regexp.MustCompile(`^[a-z0-9][a-z0-9-]*\.[a-z0-9]`)
var mdLinkRe = regexp.MustCompile(`\[.*?\]\(([^)]+)\)`)

func init() {
	importSources[bedrockMantleSourceName] = func(ctx context.Context, opts importOptions) (*ModelCatalog, []string, error) {
		// If --providers narrows to providers that exclude Bedrock, contribute nothing.
		if len(opts.providers) > 0 && !slices.ContainsFunc(opts.providers, func(p string) bool {
			gw, ok := modelsDevMapProviderID(p)
			return ok && gw == bedrockProviderID
		}) {
			return &ModelCatalog{Providers: map[string]Provider{}}, nil, nil
		}
		return awsBedrockMantleFetch(ctx)
	}
}

// availRow is one served-model row from the endpoint-availability page.
type availRow struct {
	href    string // "model-card-*.md" link
	runtime bool
	mantle  bool
}

func (r availRow) tags() []string {
	var t []string
	if r.runtime {
		t = append(t, runtimeTag)
	}
	if r.mantle {
		t = append(t, mantleTag)
	}
	return t
}

// awsBedrockMantleFetch tags every served Bedrock model by endpoint, mapping availability-page
// card slugs to models.dev IDs to skip fetching most cards and scraping only the unmapped ones.
// TODO: also emit per-model chat-format tags once the docs expose supported inference APIs.
func awsBedrockMantleFetch(ctx context.Context) (*ModelCatalog, []string, error) {
	client := &http.Client{Timeout: 30 * time.Second}

	body, err := awsMDGetBody(ctx, client, awsMDAvailURL)
	if err != nil {
		return nil, nil, fmt.Errorf("fetch availability page: %w", err)
	}
	rows, warns := awsMDParseAvailability(body)
	body.Close()

	seen := make(map[string]bool, len(rows))
	unique := make([]availRow, 0, len(rows))
	for _, r := range rows {
		if !seen[r.href] {
			seen[r.href] = true
			unique = append(unique, r)
		}
	}

	// Fast path: resolve card slugs via models.dev to skip fetching those cards (best-effort).
	index, idxWarns := bedrockModelsDevIndex(ctx)
	warns = append(warns, idxWarns...)

	tagSets := make(map[string]map[string]bool)
	addTags := func(id string, tags []string) {
		set := tagSets[id]
		if set == nil {
			set = make(map[string]bool)
			tagSets[id] = set
		}
		for _, t := range tags {
			set[t] = true
		}
	}

	var fallback []availRow
	for _, r := range unique {
		if mapped := index[bedrockSlugKey(cardSlug(r.href))]; len(mapped) > 0 {
			for _, id := range mapped {
				addTags(id, r.tags())
			}
			continue
		}
		fallback = append(fallback, r)
	}

	cardTags, cardWarns := awsFetchModelCards(ctx, client, fallback)
	warns = append(warns, cardWarns...)
	for id, tags := range cardTags {
		addTags(id, tags)
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

// cardSlug turns a "model-card-foo.md" href into its "foo" slug.
func cardSlug(href string) string {
	return strings.TrimPrefix(strings.TrimSuffix(href, ".md"), "model-card-")
}

// bedrockModelsDevIndex indexes models.dev Bedrock IDs by normalized key (see bedrockModelKey).
// Returns an empty index plus a warning if models.dev is unavailable, so all cards then fall back to scraping.
func bedrockModelsDevIndex(ctx context.Context) (map[string][]string, []string) {
	api, err := modelsDevFetchAPI(ctx)
	if err != nil {
		return nil, []string{fmt.Sprintf("models.dev mapping unavailable, scraping all cards: %v", err)}
	}
	index := map[string][]string{}
	for srcID, prov := range api {
		if gw, ok := modelsDevMapProviderID(srcID); !ok || gw != bedrockProviderID {
			continue
		}
		for id := range prov.Models {
			if key := bedrockModelKey(id); key != "" {
				index[key] = append(index[key], id)
			}
		}
	}
	for k := range index {
		slices.Sort(index[k])
	}
	return index, nil
}

// awsFetchModelCards scrapes the given cards in parallel (bounded), returning endpoint tags per
// model ID. Per-card failures become warnings rather than aborting the import.
func awsFetchModelCards(ctx context.Context, client *http.Client, rows []availRow) (map[string][]string, []string) {
	var (
		mu      sync.Mutex
		tagSets = make(map[string]map[string]bool)
		warns   []string
	)

	g, gctx := errgroup.WithContext(ctx)
	g.SetLimit(maxConcurrentCardFetches)
	for _, r := range rows {
		g.Go(func() error {
			cardBody, err := awsMDGetBody(gctx, client, awsMDBaseURL+r.href)
			if err != nil {
				mu.Lock()
				warns = append(warns, fmt.Sprintf("fetch %s: %v", r.href, err))
				mu.Unlock()
				return nil
			}
			cardTags, cardWarns := awsMDParseModelCard(cardBody)
			cardBody.Close()

			mu.Lock()
			for _, w := range cardWarns {
				warns = append(warns, fmt.Sprintf("%s: %s", r.href, w))
			}
			for id, tags := range cardTags {
				set := tagSets[id]
				if set == nil {
					set = make(map[string]bool)
					tagSets[id] = set
				}
				for _, t := range tags {
					set[t] = true
				}
			}
			mu.Unlock()
			return nil
		})
	}
	// g.Go always returns nil (failures are collected as warnings), so Wait cannot error.
	_ = g.Wait()

	out := make(map[string][]string, len(tagSets))
	for id, set := range tagSets {
		tags := make([]string, 0, len(set))
		for t := range set {
			tags = append(tags, t)
		}
		out[id] = tags
	}
	return out, warns
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

// awsMDParseAvailability returns one row per served model. Columns: name(1) with card link,
// bedrock-runtime(2), bedrock-mantle(3). Models served on neither endpoint are skipped.
func awsMDParseAvailability(r io.Reader) ([]availRow, []string) {
	var rows []availRow
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
		runtimeServed := strings.Contains(fields[2], "icon-yes.png")
		mantleServed := strings.Contains(fields[3], "icon-yes.png")
		// Skip header rows (**bold**) and separator rows (---)
		if strings.Contains(nameCell, "---") || strings.Contains(nameCell, "**") {
			continue
		}
		// Skip models not served on any endpoint.
		if !runtimeServed && !mantleServed {
			continue
		}
		m := mdLinkRe.FindStringSubmatch(nameCell)
		if m == nil {
			warns = append(warns, fmt.Sprintf("no model card link in row: %s", nameCell))
			continue
		}
		href := m[1]
		if strings.HasPrefix(href, "model-card-") {
			rows = append(rows, availRow{href: href, runtime: runtimeServed, mantle: mantleServed})
		}
	}
	if err := scanner.Err(); err != nil {
		warns = append(warns, fmt.Sprintf("scan availability page: %v", err))
	}
	return rows, warns
}

// awsMDParseModelCard maps each model ID to its (unique, unsorted) endpoint tags from a card's
// access table, plus any warnings (e.g. a scan error that may have truncated the results).
func awsMDParseModelCard(r io.Reader) (map[string][]string, []string) {
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
	var warns []string
	if err := scanner.Err(); err != nil {
		warns = append(warns, fmt.Sprintf("scan model card: %v", err))
	}
	out := make(map[string][]string, len(sets))
	for id, set := range sets {
		tags := make([]string, 0, len(set))
		for t := range set {
			tags = append(tags, t)
		}
		out[id] = tags
	}
	return out, warns
}

var bedrockRegionPrefixes = []string{
	"us.", "eu.", "au.", "apac.", "global.", "ca.", "sa.", "jp.", "in.",
}

var (
	bedrockVerColonRe = regexp.MustCompile(`:[0-9]+$`)  // trailing ":0"
	bedrockVerVRe     = regexp.MustCompile(`-v[0-9]+$`) // trailing "-v1"
	bedrockDateRe     = regexp.MustCompile(`-[0-9]{8}(-|$)`)
	bedrockNonAlnumRe = regexp.MustCompile(`[^a-z0-9]`)
)

// bedrockModelKey normalizes a models.dev Bedrock ID to a provider+name key, dropping the region
// prefix, version/date suffixes, and separators (e.g. "us.amazon.nova-pro-v1:0" -> "amazonnovapro").
func bedrockModelKey(id string) string {
	id = strings.ToLower(id)
	for _, p := range bedrockRegionPrefixes {
		if strings.HasPrefix(id, p) {
			id = id[len(p):]
			break
		}
	}
	provider, rest, found := strings.Cut(id, ".")
	if !found {
		provider, rest = id, ""
	}
	rest = bedrockVerColonRe.ReplaceAllString(rest, "")
	rest = bedrockVerVRe.ReplaceAllString(rest, "")
	rest = bedrockDateRe.ReplaceAllString(rest, "$1")
	return bedrockNonAlnumRe.ReplaceAllString(provider+rest, "")
}

// bedrockSlugKey normalizes a card slug into the same key space as bedrockModelKey, dropping
// provider-suffix noise ("labs", "ai") and doubled provider tokens (amazon-amazon, deepseek-deepseek).
func bedrockSlugKey(slug string) string {
	var toks []string
	for t := range strings.SplitSeq(strings.ToLower(slug), "-") {
		if t == "" || t == "labs" || t == "ai" {
			continue
		}
		if len(toks) > 0 && toks[len(toks)-1] == t {
			continue
		}
		toks = append(toks, t)
	}
	return bedrockNonAlnumRe.ReplaceAllString(strings.Join(toks, ""), "")
}
