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

// Chat-format tags emitted for Mantle-served models. These must stay in sync with the proxy's
// crates/llm/src/model_catalog.rs tags and crate::ChatFormat::tag.
const (
	openaiCompletionsTag = "openai_completions"
	openaiResponsesTag   = "openai_responses"
	anthropicMessagesTag = "anthropic_messages"
)

// mantleAPITags maps an AWS model-card API label to the chat-format tag the proxy routes on.
// APIs per endpoint: https://docs.aws.amazon.com/bedrock/latest/userguide/apis.html
// Mantle surface: https://docs.aws.amazon.com/bedrock/latest/userguide/bedrock-mantle.html
var mantleAPITags = map[string]string{
	"Messages":         anthropicMessagesTag,
	"Responses":        openaiResponsesTag,
	"Chat Completions": openaiCompletionsTag,
}

// mantleAPIColumns is the fixed column order of a model card's per-endpoint "APIs supported" table,
// e.g. https://docs.aws.amazon.com/bedrock/latest/userguide/model-card-openai-gpt-oss-120b.html
var mantleAPIColumns = []string{"Messages", "Responses", "Chat Completions", "Converse", "Invoke"}

// mantleAPISectionRe matches the "APIs supported on `bedrock-mantle`" heading that precedes a
// model card's per-endpoint API table (some cards append " endpoint", some don't).
var mantleAPISectionRe = regexp.MustCompile("(?i)APIs supported on `bedrock-mantle`")

// mdImageRe strips markdown image tags (the support/no-support icons) from a table cell.
var mdImageRe = regexp.MustCompile(`!\[[^\]]*\]\([^)]*\)`)

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

// awsBedrockMantleFetch tags every served Bedrock model by endpoint, and every Mantle-served model
// by the chat formats Mantle accepts for it.
// (https://docs.aws.amazon.com/bedrock/latest/userguide/models-endpoint-availability.html), mapping
// card slugs to models.dev IDs to skip fetching most cards.
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

	type mappedMantle struct {
		href string
		ids  []string
	}
	var fetchHrefs []string
	var unmapped []string
	var mantleMapped []mappedMantle
	for _, r := range unique {
		mapped := index[bedrockSlugKey(cardSlug(r.href))]
		if len(mapped) == 0 {
			unmapped = append(unmapped, r.href)
			fetchHrefs = append(fetchHrefs, r.href)
			continue
		}
		for _, id := range mapped {
			addTags(id, r.tags())
		}
		if r.mantle {
			mantleMapped = append(mantleMapped, mappedMantle{href: r.href, ids: mapped})
			fetchHrefs = append(fetchHrefs, r.href)
		}
	}

	cards, cardWarns := awsFetchCards(ctx, client, fetchHrefs)
	warns = append(warns, cardWarns...)
	// Unmapped rows: take endpoint (and Mantle format) tags discovered from the card itself.
	for _, href := range unmapped {
		for id, tags := range cards[href].idTags {
			addTags(id, tags)
		}
	}
	// Mapped Mantle rows: attach the card's format tags to their existing models.dev IDs.
	for _, mm := range mantleMapped {
		for _, id := range mm.ids {
			addTags(id, cards[mm.href].formats)
		}
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

// parsedCard is one model card's contribution: endpoint (+Mantle format) tags per model ID
type parsedCard struct {
	idTags  map[string][]string
	formats []string
}

// awsFetchCards scrapes the given cards in parallel (bounded), returning each parsed card keyed by
// href. Per-card failures become warnings rather than aborting the import.
func awsFetchCards(ctx context.Context, client *http.Client, hrefs []string) (map[string]parsedCard, []string) {
	var (
		mu     sync.Mutex
		result = make(map[string]parsedCard, len(hrefs))
		warns  []string
	)

	g, gctx := errgroup.WithContext(ctx)
	g.SetLimit(maxConcurrentCardFetches)
	for _, href := range hrefs {
		g.Go(func() error {
			cardBody, err := awsMDGetBody(gctx, client, awsMDBaseURL+href)
			if err != nil {
				mu.Lock()
				warns = append(warns, fmt.Sprintf("fetch %s: %v", href, err))
				mu.Unlock()
				return nil
			}
			lines, lineWarns := readCardLines(cardBody)
			cardBody.Close()
			formats := awsMDParseMantleFormats(lines)
			idTags := cardIDTags(parseCardEndpointIDs(lines), formats)

			mu.Lock()
			for _, w := range lineWarns {
				warns = append(warns, fmt.Sprintf("%s: %s", href, w))
			}
			result[href] = parsedCard{idTags: idTags, formats: formats}
			mu.Unlock()
			return nil
		})
	}
	// g.Go always returns nil (failures are collected as warnings), so Wait cannot error.
	_ = g.Wait()

	return result, warns
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

func readCardLines(r io.Reader) ([]string, []string) {
	scanner := docScanner(r)
	var lines []string
	for scanner.Scan() {
		lines = append(lines, scanner.Text())
	}
	var warns []string
	if err := scanner.Err(); err != nil {
		warns = append(warns, fmt.Sprintf("scan model card: %v", err))
	}
	return lines, warns
}

func awsMDParseModelCard(r io.Reader) (map[string][]string, []string) {
	lines, warns := readCardLines(r)
	return cardIDTags(parseCardEndpointIDs(lines), awsMDParseMantleFormats(lines)), warns
}

func parseCardEndpointIDs(lines []string) map[string]map[string]bool {
	sets := make(map[string]map[string]bool)
	for _, line := range lines {
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
	return sets
}

func cardIDTags(sets map[string]map[string]bool, mantleFormats []string) map[string][]string {
	out := make(map[string][]string, len(sets))
	for id, set := range sets {
		if set[mantleTag] {
			for _, t := range mantleFormats {
				set[t] = true
			}
		}
		tags := make([]string, 0, len(set))
		for t := range set {
			tags = append(tags, t)
		}
		out[id] = tags
	}
	return out
}

// awsMDParseMantleFormats returns the chat-format tags Mantle serves for a model card.
// https://docs.aws.amazon.com/bedrock/latest/userguide/apis.html
func awsMDParseMantleFormats(lines []string) []string {
	if tags, ok := parseMantleAPITable(lines); ok {
		return tags
	}
	return parseSummaryAPIFormats(lines)
}

// parseMantleAPITable reads the per-endpoint "APIs supported on `bedrock-mantle`" table. The bool
// reports whether that table was present, so a card without it can fall back to the summary table.
func parseMantleAPITable(lines []string) ([]string, bool) {
	for i, line := range lines {
		if !mantleAPISectionRe.MatchString(line) {
			continue
		}
		end := min(i+8, len(lines))
		for _, row := range lines[i+1 : end] {
			if !strings.Contains(row, "icon-yes") && !strings.Contains(row, "icon-no") {
				continue
			}
			fields := strings.Split(row, "|")
			var tags []string
			for idx, col := range mantleAPIColumns {
				fi := idx + 1 // leading "|" makes fields[0] empty
				if fi < len(fields) && strings.Contains(fields[fi], "icon-yes") {
					if tag, ok := mantleAPITags[col]; ok {
						tags = append(tags, tag)
					}
				}
			}
			return tags, true
		}
		return nil, true
	}
	return nil, false
}

// parseSummaryAPIFormats reads the older summary table's "APIs supported" column
func parseSummaryAPIFormats(lines []string) []string {
	for i, line := range lines {
		if !strings.HasPrefix(strings.TrimSpace(line), "|") {
			continue
		}
		fields := strings.Split(line, "|")
		apiCol := -1
		for j, f := range fields {
			if strings.Contains(f, "APIs supported") {
				apiCol = j
				break
			}
		}
		if apiCol == -1 {
			continue
		}
		var tags []string
		seen := make(map[string]bool)
		for _, row := range lines[i+1:] {
			if !strings.HasPrefix(strings.TrimSpace(row), "|") {
				break // end of the table
			}
			rf := strings.Split(row, "|")
			if apiCol >= len(rf) || !strings.Contains(rf[apiCol], "icon-yes") {
				continue
			}
			label := strings.TrimSpace(mdImageRe.ReplaceAllString(rf[apiCol], ""))
			if tag, ok := mantleAPITags[label]; ok && !seen[tag] {
				seen[tag] = true
				tags = append(tags, tag)
			}
		}
		return tags
	}
	return nil
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
