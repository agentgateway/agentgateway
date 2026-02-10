package fsutils

import (
	"log"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
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
	return filepath.Dir(thisFile)
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
