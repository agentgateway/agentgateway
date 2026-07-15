package annotations

import (
	"testing"

	"k8s.io/apimachinery/pkg/util/sets"
)

func TestParseInternalPorts(t *testing.T) {
	listeners := sets.New[int32](80, 8080, 443)
	isListenerPort := func(p int32) bool { return listeners.Has(p) }

	tests := []struct {
		name    string
		value   string
		want    []int32
		wantErr bool
	}{
		{name: "empty", value: "", want: nil},
		{name: "single", value: "8080", want: []int32{8080}},
		{name: "multiple with spaces", value: "80, 8080 ,443", want: []int32{80, 443, 8080}},
		{name: "trailing comma ignored", value: "8080,", want: []int32{8080}},
		{name: "non-numeric rejects all", value: "http", wantErr: true},
		{name: "zero out of range", value: "0", wantErr: true},
		{name: "above range", value: "70000", wantErr: true},
		{name: "unmatched port", value: "9999", wantErr: true},
		{name: "any error rejects whole annotation", value: "8080,9999", wantErr: true},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, errs := ParseInternalPorts(tt.value, isListenerPort)
			if tt.wantErr {
				if len(errs) == 0 {
					t.Fatalf("expected errors, got none (set=%v)", got.UnsortedList())
				}
				if got.Len() != 0 {
					t.Fatalf("on error the set must be empty, got %v", got.UnsortedList())
				}
				return
			}
			if len(errs) != 0 {
				t.Fatalf("unexpected errors: %v", errs)
			}
			want := sets.New(tt.want...)
			if !got.Equal(want) {
				t.Fatalf("got %v, want %v", got.UnsortedList(), tt.want)
			}
		})
	}
}
