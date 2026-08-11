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

	hrefs, warns := awsMDParseAvailability(strings.NewReader(page))
	if len(warns) != 0 {
		t.Fatalf("unexpected warnings: %v", warns)
	}
	want := []string{"model-card-ai21-labs-jamba-1-5-large.md", "model-card-ai21-labs-jamba-1-5-mini.md"}
	if !slices.Equal(hrefs, want) {
		t.Fatalf("hrefs = %v, want %v", hrefs, want)
	}
}

func TestParseMDAvailabilitySkipsHeaderAndSeparator(t *testing.T) {
	// Nova Pro: runtime=no, mantle=yes -> Mantle-only -> selected
	page := `| **Model name** | **bedrock-runtime** | **bedrock-mantle** |
| --- | --- | --- |
| [Nova Pro](model-card-amazon-nova-pro.md) | ![](icon-no.png) | ![](icon-yes.png) | `

	hrefs, _ := awsMDParseAvailability(strings.NewReader(page))
	if len(hrefs) != 1 || hrefs[0] != "model-card-amazon-nova-pro.md" {
		t.Fatalf("hrefs = %v, want [model-card-amazon-nova-pro.md]", hrefs)
	}
}

func TestParseMDAvailabilitySkipsUnservedModels(t *testing.T) {
	// A model served on either endpoint is included; one served on neither is skipped.
	page := `| Model name | ` + "`bedrock-runtime`" + ` | ` + "`bedrock-mantle`" + ` |
| --- | --- | --- |
| [Sonnet](model-card-anthropic-claude-3-5-sonnet.md) | ![](icon-yes.png) | ![](icon-yes.png) |
| [Titan](model-card-amazon-titan-text.md) | ![](icon-yes.png) | ![](icon-no.png) |
| [Retired](model-card-retired.md) | ![](icon-no.png) | ![](icon-no.png) | `

	hrefs, _ := awsMDParseAvailability(strings.NewReader(page))
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

	got := awsMDParseModelCard(strings.NewReader(page))
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

	got := awsMDParseModelCard(strings.NewReader(page))
	if len(got) != 1 || !slices.Equal(got["amazon.nova-pro-v1:0"], []string{mantleTag}) {
		t.Fatalf("got = %v, want {amazon.nova-pro-v1:0: [mantle]}", got)
	}
}

func TestParseMDModelCardSkipsInvalidIDs(t *testing.T) {
	page := `| bedrock-mantle | N/A | https://example.com |
| bedrock-mantle | --- | N/A |
| bedrock-mantle | valid.model-id | N/A | `

	got := awsMDParseModelCard(strings.NewReader(page))
	if len(got) != 1 || !slices.Equal(got["valid.model-id"], []string{mantleTag}) {
		t.Fatalf("got = %v, want {valid.model-id: [mantle]}", got)
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
	for id, m := range models {
		if !slices.Contains(m.Tags, mantleTag) && !slices.Contains(m.Tags, runtimeTag) {
			t.Errorf("model %q has no endpoint tag", id)
		}
		if !modelIDRe.MatchString(id) || !strings.Contains(id, ".") {
			t.Errorf("model ID %q is not a valid base model ID", id)
		}
	}
}
