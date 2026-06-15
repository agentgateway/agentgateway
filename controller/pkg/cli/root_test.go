package cli

import "testing"

func TestDeprecatedTopLevelProxyAliases(t *testing.T) {
	rootCmd := NewRootCmd()

	tests := []struct {
		name      string
		canonical string
	}{
		{
			name:      "config",
			canonical: "agctl proxy config",
		},
		{
			name:      "trace",
			canonical: "agctl proxy trace",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			cmd, _, err := rootCmd.Find([]string{tt.name})
			if err != nil {
				t.Fatal(err)
			}
			if cmd == nil {
				t.Fatalf("expected to find %q command", tt.name)
			}
			if !cmd.Hidden {
				t.Fatalf("%q command should be hidden", tt.name)
			}

			wantDeprecated := `use "` + tt.canonical + `" instead`
			if cmd.Deprecated != wantDeprecated {
				t.Fatalf("deprecated message = %q, want %q", cmd.Deprecated, wantDeprecated)
			}
		})
	}
}
