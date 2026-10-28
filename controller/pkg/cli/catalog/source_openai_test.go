package catalog

import (
	"os"
	"testing"

	"istio.io/istio/pilot/test/util"
)

func TestOpenAIPricing(t *testing.T) {
	// Relevant sections from OpenAI's markdown pricing page.
	f, err := os.Open("testdata/openai-pricing.md")
	if err != nil {
		t.Fatal(err)
	}
	defer f.Close()
	cat, _, err := openaiParsePricing(f)
	if err != nil {
		t.Fatal(err)
	}
	if err := cat.Validate(); err != nil {
		t.Fatal(err)
	}
	got, err := marshalCatalog(cat, true)
	if err != nil {
		t.Fatal(err)
	}
	util.CompareContent(t, got, "testdata/openai-pricing.golden.json")
}
