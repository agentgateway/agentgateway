package fsutils_test

import (
	"os"
	"path/filepath"
	"testing"

	"istio.io/istio/pkg/test/util/assert"

	. "github.com/agentgateway/agentgateway/controller/pkg/utils/fsutils"
)

func TestIsDirectory(t *testing.T) {
	// Test with a Temporary Directory
	tempDir, err := os.MkdirTemp("", "test")
	assert.NoError(t, err)
	defer os.RemoveAll(tempDir)

	assert.Equal(t, true, IsDirectory(tempDir))

	// Test with a non existent directory
	assert.Equal(t, false, IsDirectory("/testDir"))

	// Test with file instead of directory
	f, err := os.CreateTemp("", "test")
	assert.NoError(t, err)
	defer os.Remove(f.Name())
	assert.Equal(t, false, IsDirectory(f.Name()))
}

func TestMustGetThisDir(t *testing.T) {
	dir := MustGetThisDir()
	assert.Equal(t, true, dir != "")
	assert.Equal(t, true, IsDirectory(dir))
}

func TestGoModPath(t *testing.T) {
	path := GoModPath()
	assert.Equal(t, true, path != "")
	assert.Equal(t, "go.mod", filepath.Base(path))
}

func TestGetModuleRoot(t *testing.T) {
	root := GetModuleRoot()
	assert.Equal(t, true, root != "")
	assert.Equal(t, true, IsDirectory(root))

	// Verify go.mod exists in root
	_, err := os.Stat(filepath.Join(root, "go.mod"))
	assert.NoError(t, err)
}
