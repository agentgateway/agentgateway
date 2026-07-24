package bedrock

import (
	"context"
	"encoding/json"
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

// TestAwsMDFetchLive calls the live AWS docs page. Run with go test (no -short).
func TestAwsMDFetchLive(t *testing.T) {
	if testing.Short() || os.Getenv("AGENTGATEWAY_E2E") == "" {
		t.Skip("set AGENTGATEWAY_E2E=true to run the live AWS docs scrape")
	}

	table, warns, err := awsMDFetch(context.Background())
	if err != nil {
		t.Fatalf("awsMDFetch: %v", err)
	}
	for _, w := range warns {
		t.Logf("warning: %s", w)
	}

	// The Mantle-only set is legitimately allowed to be small or empty, so we do
	// not assert a lower bound; we only validate the shape of whatever is returned.
	t.Logf("fetched %d Mantle-only models", len(table.Models))
	if table.Source == "" {
		t.Fatal("ModelTable.Source is empty")
	}

	for _, id := range table.Models {
		if !modelIDRe.MatchString(id) {
			t.Errorf("model ID %q does not match modelIDRe", id)
		}
		if !strings.Contains(id, ".") {
			t.Errorf("model ID %q has no '.'", id)
		}
	}

	data, err := marshalTable(table, false)
	if err != nil {
		t.Fatalf("marshalTable: %v", err)
	}
	var roundTrip ModelTable
	if err := json.Unmarshal(data, &roundTrip); err != nil {
		t.Fatalf("unmarshal round-trip: %v", err)
	}
	if len(roundTrip.Models) != len(table.Models) {
		t.Fatalf("round-trip model count %d != original %d", len(roundTrip.Models), len(table.Models))
	}
}
