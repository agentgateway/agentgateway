package costs

import (
	"context"
	"os"
	"strings"
	"testing"
)

func TestParseMDAvailabilityReturnsMantleOnlyCardHrefs(t *testing.T) {
	// Column layout: name | bedrock-runtime | bedrock-mantle
	// Jamba Large: runtime=no, mantle=yes  -> Mantle-only  -> selected
	// Jamba Mini:  runtime=yes, mantle=yes -> on both      -> skipped
	page := `| Model name | ` + "`bedrock-runtime`" + ` | ` + "`bedrock-mantle`" + ` |
| --- | --- | --- |
| [Jamba 1.5 Large](model-card-ai21-labs-jamba-1-5-large.md) | ![](http://docs.aws.amazon.com/bedrock/latest/userguide/images/icons/icon-no.png) | ![](http://docs.aws.amazon.com/bedrock/latest/userguide/images/icons/icon-yes.png) |
| [Jamba 1.5 Mini](model-card-ai21-labs-jamba-1-5-mini.md) | ![](http://docs.aws.amazon.com/bedrock/latest/userguide/images/icons/icon-yes.png) | ![](http://docs.aws.amazon.com/bedrock/latest/userguide/images/icons/icon-yes.png) | `

	hrefs, warns := awsMDParseAvailability(strings.NewReader(page))
	if len(warns) != 0 {
		t.Fatalf("unexpected warnings: %v", warns)
	}
	if len(hrefs) != 1 || hrefs[0] != "model-card-ai21-labs-jamba-1-5-large.md" {
		t.Fatalf("hrefs = %v, want [model-card-ai21-labs-jamba-1-5-large.md]", hrefs)
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

func TestParseMDAvailabilitySkipsRuntimeCapableModels(t *testing.T) {
	// A model available on Runtime (regardless of Mantle) must not appear in the
	// Mantle-only allow-list.
	page := `| Model name | ` + "`bedrock-runtime`" + ` | ` + "`bedrock-mantle`" + ` |
| --- | --- | --- |
| [Sonnet](model-card-anthropic-claude-3-5-sonnet.md) | ![](icon-yes.png) | ![](icon-yes.png) |
| [Titan](model-card-amazon-titan-text.md) | ![](icon-yes.png) | ![](icon-no.png) | `

	hrefs, _ := awsMDParseAvailability(strings.NewReader(page))
	if len(hrefs) != 0 {
		t.Fatalf("hrefs = %v, want [] (no Mantle-only models)", hrefs)
	}
}

func TestParseMDModelCardExtractsMantleID(t *testing.T) {
	page := `| **Endpoint** | **Model ID** | **In-Region endpoint URL** |
| --- | --- | --- |
| bedrock-runtime | anthropic.claude-opus-4-8 | N/A |
| bedrock-mantle | anthropic.claude-opus-4-8 | https://bedrock-mantle.{region}.api.aws | `

	ids := awsMDParseModelCard(strings.NewReader(page))
	if len(ids) != 1 || ids[0] != "anthropic.claude-opus-4-8" {
		t.Fatalf("ids = %v, want [anthropic.claude-opus-4-8]", ids)
	}
}

func TestParseMDModelCardDeduplicates(t *testing.T) {
	// Some model cards repeat the same model ID across multiple bedrock-mantle rows.
	page := `| bedrock-mantle | amazon.nova-pro-v1:0 | N/A |
| bedrock-mantle | amazon.nova-pro-v1:0 | https://example.com | `

	ids := awsMDParseModelCard(strings.NewReader(page))
	if len(ids) != 1 || ids[0] != "amazon.nova-pro-v1:0" {
		t.Fatalf("ids = %v, want [amazon.nova-pro-v1:0]", ids)
	}
}

func TestParseMDModelCardSkipsInvalidIDs(t *testing.T) {
	page := `| bedrock-mantle | N/A | https://example.com |
| bedrock-mantle | --- | N/A |
| bedrock-mantle | valid.model-id | N/A | `

	ids := awsMDParseModelCard(strings.NewReader(page))
	if len(ids) != 1 || ids[0] != "valid.model-id" {
		t.Fatalf("ids = %v, want [valid.model-id]", ids)
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

	// The Mantle-only set is legitimately allowed to be small or empty, so we only
	// validate the shape of whatever is returned.
	models := cat.Providers[bedrockProviderID].Models
	t.Logf("fetched %d Mantle-only models", len(models))
	for id, m := range models {
		if !m.Mantle {
			t.Errorf("model %q not flagged mantle", id)
		}
		if !modelIDRe.MatchString(id) || !strings.Contains(id, ".") {
			t.Errorf("model ID %q is not a valid base model ID", id)
		}
	}
}
