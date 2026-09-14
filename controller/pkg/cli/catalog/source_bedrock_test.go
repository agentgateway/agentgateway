package catalog

import (
	"context"
	"os"
	"slices"
	"strings"
	"testing"
)

func TestParseMDAvailabilityReturnsServedCardHrefs(t *testing.T) {
	// Column layout: name | bedrock-runtime | bedrock-mantle
	// Both are served (Large=mantle-only, Mini=both) so both are selected; endpoint tags come from the card.
	page := `| Model name | ` + "`bedrock-runtime`" + ` | ` + "`bedrock-mantle`" + ` |
| --- | --- | --- |
| [Jamba 1.5 Large](model-card-ai21-labs-jamba-1-5-large.md) | ![](http://docs.aws.amazon.com/bedrock/latest/userguide/images/icons/icon-no.png) | ![](http://docs.aws.amazon.com/bedrock/latest/userguide/images/icons/icon-yes.png) |
| [Jamba 1.5 Mini](model-card-ai21-labs-jamba-1-5-mini.md) | ![](http://docs.aws.amazon.com/bedrock/latest/userguide/images/icons/icon-yes.png) | ![](http://docs.aws.amazon.com/bedrock/latest/userguide/images/icons/icon-yes.png) | `

	rows, warns := awsMDParseAvailability(strings.NewReader(page))
	if len(warns) != 0 {
		t.Fatalf("unexpected warnings: %v", warns)
	}
	want := []availRow{
		{href: "model-card-ai21-labs-jamba-1-5-large.md", runtime: false, mantle: true},
		{href: "model-card-ai21-labs-jamba-1-5-mini.md", runtime: true, mantle: true},
	}
	if !slices.Equal(rows, want) {
		t.Fatalf("rows = %v, want %v", rows, want)
	}
}

func TestParseMDAvailabilitySkipsHeaderAndSeparator(t *testing.T) {
	// Nova Pro: runtime=no, mantle=yes -> Mantle-only -> selected
	page := `| **Model name** | **bedrock-runtime** | **bedrock-mantle** |
| --- | --- | --- |
| [Nova Pro](model-card-amazon-nova-pro.md) | ![](icon-no.png) | ![](icon-yes.png) | `

	rows, _ := awsMDParseAvailability(strings.NewReader(page))
	if len(rows) != 1 || rows[0].href != "model-card-amazon-nova-pro.md" || rows[0].runtime || !rows[0].mantle {
		t.Fatalf("rows = %v, want [{model-card-amazon-nova-pro.md runtime=false mantle=true}]", rows)
	}
}

func TestParseMDAvailabilitySkipsUnservedModels(t *testing.T) {
	// A model served on either endpoint is included; one served on neither is skipped.
	page := `| Model name | ` + "`bedrock-runtime`" + ` | ` + "`bedrock-mantle`" + ` |
| --- | --- | --- |
| [Sonnet](model-card-anthropic-claude-3-5-sonnet.md) | ![](icon-yes.png) | ![](icon-yes.png) |
| [Titan](model-card-amazon-titan-text.md) | ![](icon-yes.png) | ![](icon-no.png) |
| [Retired](model-card-retired.md) | ![](icon-no.png) | ![](icon-no.png) | `

	rows, _ := awsMDParseAvailability(strings.NewReader(page))
	var hrefs []string
	for _, r := range rows {
		hrefs = append(hrefs, r.href)
	}
	want := []string{"model-card-anthropic-claude-3-5-sonnet.md", "model-card-amazon-titan-text.md"}
	if !slices.Equal(hrefs, want) {
		t.Fatalf("hrefs = %v, want %v", hrefs, want)
	}
}

func TestParseMDModelCardTagsPerEndpoint(t *testing.T) {
	// A model listed under both endpoints is tagged with both.
	page := `| **Endpoint** | **Model ID** | **In-Region endpoint URL** |
| --- | --- | --- |
| bedrock-runtime | anthropic.claude-opus-4-8 | N/A |
| bedrock-mantle | anthropic.claude-opus-4-8 | https://bedrock-mantle.{region}.api.aws | `

	got, warns := awsMDParseModelCard(strings.NewReader(page))
	if len(warns) != 0 {
		t.Fatalf("unexpected warnings: %v", warns)
	}
	tags := got["anthropic.claude-opus-4-8"]
	slices.Sort(tags)
	want := []string{mantleTag, runtimeTag}
	if len(got) != 1 || !slices.Equal(tags, want) {
		t.Fatalf("got = %v, want {anthropic.claude-opus-4-8: %v}", got, want)
	}
}

func TestParseMDModelCardDeduplicates(t *testing.T) {
	// Some model cards repeat the same model ID across multiple bedrock-mantle rows.
	page := `| bedrock-mantle | amazon.nova-pro-v1:0 | N/A |
| bedrock-mantle | amazon.nova-pro-v1:0 | https://example.com | `

	got, _ := awsMDParseModelCard(strings.NewReader(page))
	if len(got) != 1 || !slices.Equal(got["amazon.nova-pro-v1:0"], []string{mantleTag}) {
		t.Fatalf("got = %v, want {amazon.nova-pro-v1:0: [mantle]}", got)
	}
}

func TestParseMDModelCardSkipsInvalidIDs(t *testing.T) {
	page := `| bedrock-mantle | N/A | https://example.com |
| bedrock-mantle | --- | N/A |
| bedrock-mantle | valid.model-id | N/A | `

	got, _ := awsMDParseModelCard(strings.NewReader(page))
	if len(got) != 1 || !slices.Equal(got["valid.model-id"], []string{mantleTag}) {
		t.Fatalf("got = %v, want {valid.model-id: [mantle]}", got)
	}
}

// Newer cards carry a per-endpoint API table; only the bedrock-mantle one drives format tags, and
// only for the Mantle-served model ID. Modeled on model-card-openai-gpt-oss-120b.
func TestParseMDModelCardFormatTagsPerEndpointTable(t *testing.T) {
	card := `**APIs supported on ` + "`bedrock-runtime`" + ` endpoint**

| **Messages** | **Responses** | **Chat Completions** | **Converse** | **Invoke** |
| --- | --- | --- | --- | --- |
| ![](icon-no.png) | ![](icon-yes.png) | ![](icon-yes.png) | ![](icon-yes.png) | ![](icon-yes.png) |

**APIs supported on ` + "`bedrock-mantle`" + ` endpoint**

| **Messages** | **Responses** | **Chat Completions** | **Converse** | **Invoke** |
| --- | --- | --- | --- | --- |
| ![](icon-no.png) | ![](icon-yes.png) | ![](icon-yes.png) | ![](icon-no.png) | ![](icon-no.png) |

| **Endpoint** | **Model ID** | **In-Region endpoint URL** |
| --- | --- | --- |
| bedrock-runtime | openai.gpt-oss-120b-1:0 | https://bedrock-runtime.{region}.amazonaws.com |
| bedrock-mantle | openai.gpt-oss-120b | https://bedrock-mantle.{region}.api.aws | `

	got, _ := awsMDParseModelCard(strings.NewReader(card))
	mantle := got["openai.gpt-oss-120b"]
	slices.Sort(mantle)
	wantMantle := []string{mantleTag, openaiCompletionsTag, openaiResponsesTag}
	if !slices.Equal(mantle, wantMantle) {
		t.Fatalf("mantle id tags = %v, want %v", mantle, wantMantle)
	}
	// The Runtime-only ID must not pick up the Mantle format tags (or the runtime API table's).
	if runtime := got["openai.gpt-oss-120b-1:0"]; !slices.Equal(runtime, []string{runtimeTag}) {
		t.Fatalf("runtime id tags = %v, want [runtime]", runtime)
	}
}

// Older cards only have a summary "APIs supported" column with no endpoint split. We keep the APIs
// Mantle serves (Chat Completions) and drop Runtime-only ones (Invoke/Converse). Modeled on
// model-card-deepseek-deepseek-v3-1.
func TestParseMDModelCardFormatTagsSummaryTable(t *testing.T) {
	card := `| **Input Modalities** | **Output Modalities** | **[APIs supported](apis.html)** | **[Endpoints supported](endpoints.html)** |
| --- | --- | --- | --- |
| ![](icon-no.png) Image | ![](icon-no.png) Image | ![](icon-yes.png) Chat Completions | ![](icon-yes.png) bedrock-mantle |
| ![](icon-yes.png) Text | ![](icon-yes.png) Text | ![](icon-yes.png) Invoke | ![](icon-yes.png) bedrock-runtime |
| ![](icon-no.png) Video | ![](icon-no.png) Video | ![](icon-yes.png) Converse |  |

| **Endpoint** | **Model ID** | **In-Region endpoint URL** |
| --- | --- | --- |
| bedrock-mantle | deepseek.v3.1 | https://bedrock-mantle.{region}.api.aws | `

	got, _ := awsMDParseModelCard(strings.NewReader(card))
	mantle := got["deepseek.v3.1"]
	slices.Sort(mantle)
	want := []string{mantleTag, openaiCompletionsTag}
	if !slices.Equal(mantle, want) {
		t.Fatalf("mantle id tags = %v, want %v", mantle, want)
	}
}

// The per-endpoint table wins over the summary table when both are present, and Anthropic models
// resolve to the Messages format. Modeled on model-card-anthropic-claude-sonnet-5.
func TestParseMantleFormatsAnthropicMessages(t *testing.T) {
	card := `**APIs supported on ` + "`bedrock-mantle`" + ` endpoint**

| **Messages** | **Responses** | **Chat Completions** | **Converse** | **Invoke** |
| --- | --- | --- | --- | --- |
| ![](icon-yes.png) | ![](icon-no.png) | ![](icon-no.png) | ![](icon-no.png) | ![](icon-no.png) |`

	if got := awsMDParseMantleFormats(strings.Split(card, "\n")); !slices.Equal(got, []string{anthropicMessagesTag}) {
		t.Fatalf("formats = %v, want [%s]", got, anthropicMessagesTag)
	}
}

func TestCardSlug(t *testing.T) {
	if got := cardSlug("model-card-anthropic-claude-opus-4-8.md"); got != "anthropic-claude-opus-4-8" {
		t.Fatalf("cardSlug = %q", got)
	}
}

func TestBedrockModelKeyNormalizesRegionAndVersion(t *testing.T) {
	// Region prefix, date, and version suffixes are stripped; regional variants share one key.
	cases := map[string]string{
		"amazon.nova-pro-v1:0":                        "amazonnovapro",
		"anthropic.claude-opus-4-8":                   "anthropicclaudeopus48",
		"us.anthropic.claude-opus-4-8":                "anthropicclaudeopus48",
		"anthropic.claude-haiku-4-5-20251001-v1:0":    "anthropicclaudehaiku45",
		"eu.anthropic.claude-haiku-4-5-20251001-v1:0": "anthropicclaudehaiku45",
		"deepseek.r1-v1:0":                            "deepseekr1",
		"deepseek.v3.2":                               "deepseekv32",
		"meta.llama3-1-8b-instruct-v1:0":              "metallama318binstruct",
	}
	for id, want := range cases {
		if got := bedrockModelKey(id); got != want {
			t.Errorf("bedrockModelKey(%q) = %q, want %q", id, got, want)
		}
	}
}

func TestBedrockSlugKeyMatchesModelKeyForCleanNames(t *testing.T) {
	// A card slug and its models.dev ID must normalize to the same key so the card can be skipped.
	cases := map[string]string{ // slug -> models.dev id
		"amazon-nova-pro":            "amazon.nova-pro-v1:0",
		"anthropic-claude-opus-4-8":  "us.anthropic.claude-opus-4-8",
		"anthropic-claude-haiku-4-5": "anthropic.claude-haiku-4-5-20251001-v1:0",
		"deepseek-deepseek-r1":       "deepseek.r1-v1:0",               // doubled provider collapses
		"ai21-labs-jamba-1-5-large":  "ai21.jamba-1-5-large-v1:0",      // "labs" dropped
		"meta-llama-3-1-8b-instruct": "meta.llama3-1-8b-instruct-v1:0", // dash placement differs
	}
	for slug, id := range cases {
		if bedrockSlugKey(slug) != bedrockModelKey(id) {
			t.Errorf("slug %q (%q) != id %q (%q)", slug, bedrockSlugKey(slug), id, bedrockModelKey(id))
		}
	}
}

func TestBedrockSlugKeyDoesNotMisMapDistinctModels(t *testing.T) {
	// Distinct models with similar labels must NOT collide (else they'd get the wrong tags).
	pairs := [][2]string{
		{"anthropic-claude-3-haiku", "anthropic.claude-haiku-4-5-20251001-v1:0"},
		{"amazon-nova-canvas", "amazon.nova-2-lite-v1:0"},
		{"meta-llama-3-2-11b-instruct", "meta.llama3-3-70b-instruct-v1:0"},
		{"mistral-ai-mistral-7b-instruct", "mistral.ministral-3-14b-instruct"},
	}
	for _, p := range pairs {
		if bedrockSlugKey(p[0]) == bedrockModelKey(p[1]) {
			t.Errorf("slug %q must not map to distinct model %q", p[0], p[1])
		}
	}
}

// TestAwsBedrockMantleFetchLive calls the live AWS docs page.
func TestAwsBedrockMantleFetchLive(t *testing.T) {
	if testing.Short() || os.Getenv("AGENTGATEWAY_E2E") == "" {
		t.Skip("set AGENTGATEWAY_E2E=true to run the live AWS docs scrape")
	}

	cat, warns, err := awsBedrockMantleFetch(context.Background())
	if err != nil {
		t.Fatalf("awsBedrockMantleFetch: %v", err)
	}
	for _, w := range warns {
		t.Logf("warning: %s", w)
	}
	if err := cat.Validate(); err != nil {
		t.Fatalf("invalid catalog: %v", err)
	}

	// We only validate the shape of whatever is returned.
	models := cat.Providers[bedrockProviderID].Models
	t.Logf("fetched %d served Bedrock models", len(models))
	formatTags := map[string]bool{openaiCompletionsTag: true, openaiResponsesTag: true, anthropicMessagesTag: true}
	var mantleModels, mantleWithFormat int
	for id, m := range models {
		if !slices.Contains(m.Tags, mantleTag) && !slices.Contains(m.Tags, runtimeTag) {
			t.Errorf("model %q has no endpoint tag", id)
		}
		if !modelIDRe.MatchString(id) || !strings.Contains(id, ".") {
			t.Errorf("model ID %q is not a valid base model ID", id)
		}
		if slices.Contains(m.Tags, mantleTag) {
			mantleModels++
			if slices.ContainsFunc(m.Tags, func(t string) bool { return formatTags[t] }) {
				mantleWithFormat++
				t.Logf("mantle model %q: %v", id, m.Tags)
			}
		}
	}
	// The whole point of fetching Mantle cards is the format tags; most Mantle models should carry one.
	if mantleModels > 0 && mantleWithFormat == 0 {
		t.Errorf("no Mantle model carries a chat-format tag (%d Mantle models); format import is broken", mantleModels)
	}
}
