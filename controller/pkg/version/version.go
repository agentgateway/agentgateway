package version

import (
	"encoding/json"
	"fmt"
	"runtime"
	"runtime/debug"
)

var (
	// UndefinedVersion is the version of the kgateway controller
	// if the version is not set.
	UndefinedVersion = "undefined"
	// Version is the version of the kgateway controller.
	// This is set by the linker during build.
	Version string
	// ref is the version of the kgateway controller.
	// Constructed from the build info during init
	ref *version
)

type version struct {
	Controller string `json:"version"`
	Commit     string `json:"commit"`
	Date       string `json:"buildDate"`
	OS         string `json:"runtimeOS"`
	Arch       string `json:"runtimeArch"`
}

func String() string {
	data, err := json.Marshal(ref)
	if err != nil {
		return fmt.Sprintf("unable to generate version string: %v", err)
	}
	return string(data)
}

func init() {
	info, ok := debug.ReadBuildInfo()
	if !ok {
		Version = UndefinedVersion
		return
	}
	v := Version
	if v == "" {
		// TODO(tim): use info.Main.Version instead of UndefinedVersion.
		v = UndefinedVersion
	}
	ref = &version{
		Controller: v,
		OS:         runtime.GOOS,
		Arch:       runtime.GOARCH,
	}
	for _, setting := range info.Settings {
		switch setting.Key {
		case "vcs.revision":
			ref.Commit = setting.Value
		case "vcs.time":
			ref.Date = setting.Value
		}
	}
}
