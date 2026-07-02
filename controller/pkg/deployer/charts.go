package deployer

import (
	"embed"
	"fmt"
	"io/fs"
	"path/filepath"

	"helm.sh/helm/v4/pkg/chart/loader/archive"
	chartv2 "helm.sh/helm/v4/pkg/chart/v2"
	"helm.sh/helm/v4/pkg/chart/v2/loader"

	"github.com/agentgateway/agentgateway/controller/pkg/version"
)

func loadChart(fs embed.FS) (*chartv2.Chart, error) {
	c, err := loadFs(fs)
	if err != nil {
		return nil, err
	}
	// simulate what `helm package` in the Makefile does
	if version.Version != version.UndefinedVersion {
		c.Metadata.AppVersion = version.Version
		c.Metadata.Version = version.Version
	}

	return c, nil
}

func loadFs(filesystem fs.FS) (*chartv2.Chart, error) {
	var bufferedFiles []*archive.BufferedFile
	entries, err := fs.ReadDir(filesystem, ".")
	if err != nil {
		return nil, err
	}
	if len(entries) != 1 {
		return nil, fmt.Errorf("expected exactly one entry in the chart folder, got %v", entries)
	}

	root := entries[0].Name()
	err = fs.WalkDir(filesystem, root, func(path string, d fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if d.IsDir() {
			return nil
		}

		data, readErr := fs.ReadFile(filesystem, path)
		if readErr != nil {
			return readErr
		}

		relativePath, relErr := filepath.Rel(root, path)
		if relErr != nil {
			return relErr
		}

		bufferedFile := &archive.BufferedFile{
			Name: relativePath,
			Data: data,
		}

		bufferedFiles = append(bufferedFiles, bufferedFile)
		return nil
	})
	if err != nil {
		return nil, err
	}

	return loader.LoadFiles(bufferedFiles)
}
