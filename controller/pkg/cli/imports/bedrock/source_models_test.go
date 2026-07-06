package bedrock

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
)

func TestParseMDAvailabilityReturnsCardHrefs(t *testing.T) {
	page := `| Model name | ` + "`bedrock-runtime`" + ` | ` + "`bedrock-mantle`" + ` |
| --- | --- | --- |
| [Jamba 1.5 Large](model-card-ai21-labs-jamba-1-5-large.md) | ![](http://docs.aws.amazon.com/bedrock/latest/userguide/images/icons/icon-yes.png) | ![](http://docs.aws.amazon.com/bedrock/latest/userguide/images/icons/icon-no.png) |
| [Jamba 1.5 Mini](model-card-ai21-labs-jamba-1-5-mini.md) | ![](http://docs.aws.amazon.com/bedrock/latest/userguide/images/icons/icon-no.png) | ![](http://docs.aws.amazon.com/bedrock/latest/userguide/images/icons/icon-no.png) | `

	hrefs, warns := awsMDParseAvailability(strings.NewReader(page))
	if len(warns) != 0 {
		t.Fatalf("unexpected warnings: %v", warns)
	}
	if len(hrefs) != 1 || hrefs[0] != "model-card-ai21-labs-jamba-1-5-large.md" {
		t.Fatalf("hrefs = %v, want [model-card-ai21-labs-jamba-1-5-large.md]", hrefs)
	}
}

func TestParseMDAvailabilitySkipsHeaderAndSeparator(t *testing.T) {
	page := `| **Model name** | **bedrock-runtime** | **bedrock-mantle** |
| --- | --- | --- |
| [Nova Pro](model-card-amazon-nova-pro.md) | ![](icon-yes.png) | ![](icon-no.png) | `

	hrefs, _ := awsMDParseAvailability(strings.NewReader(page))
	if len(hrefs) != 1 || hrefs[0] != "model-card-amazon-nova-pro.md" {
		t.Fatalf("hrefs = %v, want [model-card-amazon-nova-pro.md]", hrefs)
	}
}

func TestParseMDModelCardExtractsID(t *testing.T) {
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
	// Some model cards repeat the same model ID across multiple bedrock-runtime rows.
	page := `| bedrock-runtime | amazon.nova-pro-v1:0 | N/A |
| bedrock-runtime | amazon.nova-pro-v1:0 | https://example.com | `

	ids := awsMDParseModelCard(strings.NewReader(page))
	if len(ids) != 1 || ids[0] != "amazon.nova-pro-v1:0" {
		t.Fatalf("ids = %v, want [amazon.nova-pro-v1:0]", ids)
	}
}

func TestParseMDModelCardSkipsInvalidIDs(t *testing.T) {
	page := `| bedrock-runtime | N/A | https://example.com |
| bedrock-runtime | --- | N/A |
| bedrock-runtime | valid.model-id | N/A | `

	ids := awsMDParseModelCard(strings.NewReader(page))
	if len(ids) != 1 || ids[0] != "valid.model-id" {
		t.Fatalf("ids = %v, want [valid.model-id]", ids)
	}
}

// TestAwsMDFetchLive calls the live AWS docs page. Run with go test (no -short).
func TestAwsMDFetchLive(t *testing.T) {
	if testing.Short() {
		t.Skip("skipping live AWS docs fetch")
	}

	table, warns, err := awsMDFetch(context.Background())
	if err != nil {
		t.Fatalf("awsMDFetch: %v", err)
	}
	for _, w := range warns {
		t.Logf("warning: %s", w)
	}

	const minModels = 10
	if len(table.Models) < minModels {
		t.Fatalf("got %d models, want at least %d", len(table.Models), minModels)
	}
	if table.Source == "" {
		t.Fatal("RuntimeTable.Source is empty")
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
	var roundTrip RuntimeTable
	if err := json.Unmarshal(data, &roundTrip); err != nil {
		t.Fatalf("unmarshal round-trip: %v", err)
	}
	if len(roundTrip.Models) != len(table.Models) {
		t.Fatalf("round-trip model count %d != original %d", len(roundTrip.Models), len(table.Models))
	}
}
