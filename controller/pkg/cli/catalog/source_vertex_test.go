package catalog

import (
	"os"
	"testing"
	"time"

	"istio.io/istio/pilot/test/util"
)

func TestVertexPricing(t *testing.T) {
	// Relevant tables from Google's pricing page, with presentation attributes removed.
	f, err := os.Open("testdata/vertex-pricing.html")
	if err != nil {
		t.Fatal(err)
	}
	defer f.Close()
	cat, _, err := vertexParsePricing(f, time.Date(2026, time.September, 24, 0, 0, 0, 0, time.UTC))
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
	util.CompareContent(t, got, "testdata/vertex-pricing.golden.json")
}
