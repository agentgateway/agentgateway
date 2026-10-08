package fsutils

import (
	"log"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
)

// IsDirectory checks the provided path is a directory by first checking something exists at that path
// and then checking that it is a directory.
func IsDirectory(dir string) bool {
	stat, err := os.Stat(dir)
	if err != nil {
		return false
	}
	return stat.IsDir()
}

// MustGetThisDir returns the absolute path to the diretory containing the .go file containing the calling function
func MustGetThisDir() string {
	_, thisFile, _, ok := runtime.Caller(1)
	if !ok {
		log.Fatalf("Failed to get runtime.Caller")
	}
	return filepath.Dir(ResolveSourceFile(thisFile))
}

var sourceModule = sync.OnceValue(func() []string {
	out, err := exec.Command("go", "list", "-m", "-f", "{{.Path}}\n{{.Dir}}").Output()
	if err != nil {
		panic(err)
	}
	return strings.SplitN(strings.TrimSpace(string(out)), "\n", 2)
})

// ResolveSourceFile maps a runtime.Caller filename back to the checkout when
// -trimpath replaces the source directory with the module import path.
func ResolveSourceFile(file string) string {
	if filepath.IsAbs(file) {
		return file
	}
	module := sourceModule()
	rel, ok := strings.CutPrefix(filepath.ToSlash(file), module[0]+"/")
	if !ok || len(module) != 2 {
		panic("source file is outside the current module: " + file)
	}
	return filepath.Join(module[1], filepath.FromSlash(rel))
}

// GoModPath returns the absolute path to the go.mod file for the current dir
func GoModPath() string {
	out, err := exec.Command("go", "env", "GOMOD").CombinedOutput()
	if err != nil {
		log.Fatal(err)
	}
	return strings.TrimSpace(string(out))
}

// GetModuleRoot returns the project root dir (based on gomod location)
func GetModuleRoot() string {
	return filepath.Dir(GoModPath())
}
