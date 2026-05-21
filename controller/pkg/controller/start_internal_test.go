package controller

import (
	"testing"
)

func TestNormalizeProxyImageTag(t *testing.T) {
	tests := []struct {
		name string
		tag  string
		want string
	}{
		{
			name: "adds v prefix",
			tag:  "1.2.3",
			want: "v1.2.3",
		},
		{
			name: "preserves v-prefixed tag",
			tag:  "v1.2.3",
			want: "v1.2.3",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := normalizeProxyImageTag(tt.tag)
			if got != tt.want {
				t.Fatalf("expected tag %q, got %q", tt.want, got)
			}
		})
	}
}
