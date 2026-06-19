//go:build e2e

package base

import (
	"github.com/Masterminds/semver/v3"
	istioassert "istio.io/istio/pkg/test/util/assert"
	apiextensionsv1 "k8s.io/apiextensions-apiserver/pkg/apis/apiextensions/v1"
	"sigs.k8s.io/controller-runtime/pkg/client"
)

type GwApiChannel string

const (
	GwApiChannelStandard     GwApiChannel = "standard"
	GwApiChannelExperimental GwApiChannel = "experimental"
)

type GwApiVersion struct {
	semver.Version
}

func GwApiVersionMustParse(version string) GwApiVersion {
	return GwApiVersion{Version: *semver.MustParse(version)}
}

var (
	// HTTPRoutes.spec.rules[].name was added in 1.2.0 experimental and 1.4.0 standard.
	GwApiV1_2_0 = GwApiVersionMustParse("1.2.0")
	// BackendTLSPolicy moved to standard/v1 in 1.4.0.
	GwApiV1_4_0 = GwApiVersionMustParse("1.4.0")

	GwApiRequireRouteNames = map[GwApiChannel]*GwApiVersion{
		GwApiChannelExperimental: &GwApiV1_2_0,
		GwApiChannelStandard:     &GwApiV1_4_0,
	}

	GwApiRequireBackendTLSPolicy = map[GwApiChannel]*GwApiVersion{
		GwApiChannelExperimental: &GwApiV1_4_0,
		GwApiChannelStandard:     &GwApiV1_4_0,
	}
)

func gatewayAPIMinVersionMatches(requirements map[GwApiChannel]*GwApiVersion, channel GwApiChannel, current GwApiVersion) bool {
	switch channel {
	case GwApiChannelExperimental:
		if requiredVersion, exists := requirements[GwApiChannelExperimental]; exists {
			return current.GreaterThan(&requiredVersion.Version) || current.Equal(&requiredVersion.Version)
		}
		return true
	case GwApiChannelStandard:
		if requiredVersion, exists := requirements[GwApiChannelStandard]; exists {
			return current.GreaterThan(&requiredVersion.Version) || current.Equal(&requiredVersion.Version)
		}
		if _, hasExperimental := requirements[GwApiChannelExperimental]; hasExperimental {
			return false
		}
		return true
	default:
		return false
	}
}

func (s *Test) detectAndCacheGwApiInfo() {
	if s.gwApiChannel != "" {
		return
	}
	crd := &apiextensionsv1.CustomResourceDefinition{}
	err := s.TestInstallation.ClusterContext.ControllerClient.Get(s.Ctx, client.ObjectKey{Name: "gateways.gateway.networking.k8s.io"}, crd)
	istioassert.NoError(s, err)

	channel, hasChannel := crd.Annotations["gateway.networking.k8s.io/channel"]
	if !hasChannel {
		s.Fatal("Gateway CRD missing 'gateway.networking.k8s.io/channel' annotation")
	}
	s.gwApiChannel = GwApiChannel(channel)

	versionStr, hasVersion := crd.Annotations["gateway.networking.k8s.io/bundle-version"]
	if !hasVersion {
		s.Fatal("Gateway CRD missing 'gateway.networking.k8s.io/bundle-version' annotation")
	}

	version, err := semver.NewVersion(versionStr)
	if err != nil {
		s.Fatalf("failed to parse Gateway API version %q: %v", versionStr, err)
	}
	s.gwApiVersion = version
}

func (s *Test) getCurrentGwApiChannel() GwApiChannel {
	return s.gwApiChannel
}

func (s *Test) getCurrentGwApiVersion() GwApiVersion {
	return GwApiVersion{Version: *s.gwApiVersion}
}

func (s *Test) ShouldSkip() bool {
	if len(s.MinGwApiVersion) == 0 {
		return false
	}

	currentVersion := s.getCurrentGwApiVersion()
	currentChannel := s.getCurrentGwApiChannel()

	if currentVersion.Version.String() == "" {
		s.Fatal("cannot determine Gateway API version")
	}

	return !gatewayAPIMinVersionMatches(s.MinGwApiVersion, currentChannel, currentVersion)
}
