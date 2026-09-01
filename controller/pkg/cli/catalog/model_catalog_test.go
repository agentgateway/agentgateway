package catalog

import (
	"reflect"
	"testing"

	"sigs.k8s.io/yaml"
)

func TestOverlayCatalog(t *testing.T) {
	base := ModelCatalog{Providers: map[string]Provider{
		"openai": {Models: map[string]Model{
			"existing": {Rates: Rates{Input: "1", Output: "2"}},
		}},
	}}
	var overlay ModelCatalog
	if err := yaml.UnmarshalStrict([]byte(`
providers:
  openai:
    models:
      existing:
        rates:
          output: "3"
      added:
        rates:
          input: "4"
`), &overlay); err != nil {
		t.Fatal(err)
	}

	base.overlayWith(&overlay)

	want := ModelCatalog{Providers: map[string]Provider{
		"openai": {Models: map[string]Model{
			"existing": {Rates: Rates{Input: "1", Output: "3"}},
			"added":    {Rates: Rates{Input: "4"}},
		}},
	}}
	if !reflect.DeepEqual(base, want) {
		t.Fatalf("merged catalog = %#v, want %#v", base, want)
	}
}
