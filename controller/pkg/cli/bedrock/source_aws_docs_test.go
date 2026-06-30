package bedrock

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
)

func TestParseAvailabilityReturnsCardHrefs(t *testing.T) {
	page := `<html><body>
<table>
  <tr><th>Model</th><th>bedrock-runtime</th></tr>
  <tr>
    <td><a href="./model-card-amazon-nova-pro.html">Nova Pro</a></td>
    <td><img src="icon-yes.png"/></td>
  </tr>
  <tr>
    <td><a href="./model-card-amazon-nova-lite.html">Nova Lite</a></td>
    <td><img src="icon-no.png"/></td>
  </tr>
</table>
</body></html>`

	hrefs, warns, err := awsDocsParseAvailability(strings.NewReader(page))
	if err != nil {
		t.Fatal(err)
	}
	if len(warns) != 0 {
		t.Fatalf("unexpected warnings: %v", warns)
	}
	if len(hrefs) != 1 || hrefs[0] != "model-card-amazon-nova-pro.html" {
		t.Fatalf("hrefs = %v, want [model-card-amazon-nova-pro.html]", hrefs)
	}
}

func TestParseAvailabilityIgnoresNonRuntimeTables(t *testing.T) {
	page := `<html><body>
<table>
  <tr><th>Model</th><th>batch-inference</th></tr>
  <tr>
    <td><a href="./model-card-some-model.html">Some Model</a></td>
    <td><img src="icon-yes.png"/></td>
  </tr>
</table>
</body></html>`

	hrefs, _, err := awsDocsParseAvailability(strings.NewReader(page))
	if err != nil {
		t.Fatal(err)
	}
	if len(hrefs) != 0 {
		t.Fatalf("expected no hrefs from non-bedrock-runtime table, got %v", hrefs)
	}
}

func TestParseModelCardExtractsIDs(t *testing.T) {
	page := `<html><body>
<p>Base ID: <code>amazon.nova-pro-v1:0</code></p>
<p>CRIS ID: <code>us.amazon.nova-pro-v1:0</code></p>
<p>EU CRIS: <code>eu.amazon.nova-pro-v1:0</code></p>
<p>Ignored: <code>not a model id</code></p>
<p>Ignored short: <code>abc</code></p>
</body></html>`

	ids, err := awsDocsParseModelCard(strings.NewReader(page))
	if err != nil {
		t.Fatal(err)
	}
	// CRIS variants should be stripped to the same base; deduplication leaves one entry.
	if len(ids) != 1 || ids[0] != "amazon.nova-pro-v1:0" {
		t.Fatalf("ids = %v, want [amazon.nova-pro-v1:0]", ids)
	}
}

func TestStripScrapedCRIS(t *testing.T) {
	cases := []struct{ in, want string }{
		{"us.amazon.nova-pro-v1:0", "amazon.nova-pro-v1:0"},
		{"eu.anthropic.claude-3-5-sonnet", "anthropic.claude-3-5-sonnet"},
		{"apac.amazon.titan-text-express-v1", "amazon.titan-text-express-v1"},
		{"amazon.nova-pro-v1:0", "amazon.nova-pro-v1:0"}, // no prefix → unchanged
	}
	for _, c := range cases {
		if got := stripScrapedCRIS(c.in); got != c.want {
			t.Errorf("stripScrapedCRIS(%q) = %q, want %q", c.in, got, c.want)
		}
	}
}

// TestAwsDocsFetchLive calls the live AWS docs page. Run with go test (no -short).
func TestAwsDocsFetchLive(t *testing.T) {
	if testing.Short() {
		t.Skip("skipping live AWS docs fetch")
	}

	table, warns, err := awsDocsFetch(context.Background())
	if err != nil {
		t.Fatalf("awsDocsFetch: %v", err)
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

	// Every ID must match the expected format and contain at least one ".".
	for _, id := range table.Models {
		if !modelIDRe.MatchString(id) {
			t.Errorf("model ID %q does not match modelIDRe", id)
		}
		if !strings.Contains(id, ".") {
			t.Errorf("model ID %q has no '.'", id)
		}
		// No CRIS geo prefix should survive into the output.
		for _, prefix := range scrapeCRISPrefixes {
			if strings.HasPrefix(id, prefix) {
				t.Errorf("model ID %q still has CRIS prefix %q", id, prefix)
			}
		}
	}

	// Output must round-trip as valid JSON matching RuntimeTable.
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
