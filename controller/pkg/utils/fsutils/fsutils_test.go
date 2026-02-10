package fsutils_test

import (
	"os"
	"path/filepath"

	. "github.com/onsi/ginkgo/v2"
	. "github.com/onsi/gomega"

	. "github.com/kgateway-dev/kgateway/v2/pkg/utils/fsutils"
)

var _ = Describe("FSUtils", func() {
	Describe("IsDirectory", func() {
		It("correctly detects directories and files", func() {
			// Test with a Temporary Directory
			tempDir, err := os.MkdirTemp("", "test")
			Expect(err).NotTo(HaveOccurred())
			defer os.RemoveAll(tempDir)

			Expect(IsDirectory(tempDir)).To(BeTrue())

			// Test with a non existent directory
			Expect(IsDirectory("/testDir")).To(BeFalse())

			// Test with file instead of directory
			f, err := os.CreateTemp("", "test")
			Expect(err).NotTo(HaveOccurred())
			defer os.Remove(f.Name())
			Expect(IsDirectory(f.Name())).To(BeFalse())
		})
	})

	Describe("MustGetThisDir", func() {
		It("returns a valid directory path", func() {
			dir := MustGetThisDir()
			Expect(dir).NotTo(BeEmpty())
			Expect(IsDirectory(dir)).To(BeTrue())
		})
	})

	Describe("GoModPath", func() {
		It("returns a valid go.mod file path", func() {
			path := GoModPath()
			Expect(path).NotTo(BeEmpty())
			Expect(filepath.Base(path)).To(Equal("go.mod"))
		})
	})

	Describe("GetModuleRoot", func() {
		It("returns a valid module root containing go.mod", func() {
			root := GetModuleRoot()
			Expect(root).NotTo(BeEmpty())
			Expect(IsDirectory(root)).To(BeTrue())

			// Verify go.mod exists in root
			_, err := os.Stat(filepath.Join(root, "go.mod"))
			Expect(err).NotTo(HaveOccurred())
		})
	})
})
