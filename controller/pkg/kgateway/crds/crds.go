package crds

import (
	"path"
	"path/filepath"
	"runtime"

	_ "embed"
)

//go:embed gateway-crds.yaml
var GatewayCrds []byte

func getDirectory() string {
	_, filename, _, _ := runtime.Caller(0)
	return path.Dir(filename)
}

// directory is the absolute path to the directory containing the crd files
// It can't change at runtime, so we can cache it
var directory = getDirectory()

// AbsPathToCrd returns the absolute path to directory in which crds are stored (currently the same directory as this file)
// Used for tests to find the crd files if needed
func AbsPathToCrd(crdFile string) string {
	return filepath.Join(directory, crdFile)
}
