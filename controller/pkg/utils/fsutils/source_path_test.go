package fsutils

import (
	"os"
	"path/filepath"
	"testing"
)

func TestSourceDirectory(t *testing.T) {
	want, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	if got := MustGetThisDir(); got != want {
		t.Fatalf("source directory = %q, want %q", got, want)
	}
	file := filepath.Join(want, "source_path_test.go")
	if got := ResolveSourceFile(file); got != file {
		t.Fatalf("absolute source file = %q, want %q", got, file)
	}
}
