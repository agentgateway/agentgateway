package controller

import (
	"testing"

	"istio.io/istio/pkg/ptr"

	apisettings "github.com/agentgateway/agentgateway/controller/api/settings"
	"github.com/agentgateway/agentgateway/controller/pkg/version"
)

func TestDefaultProxyImageTag(t *testing.T) {
	origVersion := version.Version
	origGitVersion := version.GitVersion
	t.Cleanup(func() {
		version.Version = origVersion
		version.GitVersion = origGitVersion
	})

	t.Run("uses explicit setting", func(t *testing.T) {
		version.Version = "1.0.1-dev"
		version.GitVersion = "v1.0.1-dev"

		tag := defaultProxyImageTag(&apisettings.Settings{
			ProxyImageTag: ptr.Of("custom-tag"),
		})

		if tag == nil || *tag != "custom-tag" {
			t.Fatalf("expected explicit tag, got %v", tag)
		}
	})

	t.Run("uses build version without adding prefix", func(t *testing.T) {
		version.Version = "1.0.1-dev"
		version.GitVersion = "v1.0.1-dev"

		tag := defaultProxyImageTag(&apisettings.Settings{})

		if tag == nil || *tag != "1.0.1-dev" {
			t.Fatalf("expected unprefixed build version, got %v", tag)
		}
	})

	t.Run("falls back to git version", func(t *testing.T) {
		version.Version = ""
		version.GitVersion = "v1.0.1-dev"

		tag := defaultProxyImageTag(&apisettings.Settings{})

		if tag == nil || *tag != "v1.0.1-dev" {
			t.Fatalf("expected git version fallback, got %v", tag)
		}
	})
}
