//go:build conformance

package conformance_test

import (
	"testing"

	"github.com/stretchr/testify/require"
)

func TestChooseMetallbAddressPrefersStaticOnlyPool(t *testing.T) {
	got, err := chooseMetallbAddress([]metalLBAddressPool{
		{
			name:       "default",
			autoAssign: true,
			addresses:  []string{"192.0.2.10"},
		},
		{
			name:       "static",
			autoAssign: false,
			addresses:  []string{"192.0.2.20"},
		},
	}, nil)

	require.NoError(t, err)
	require.Equal(t, "192.0.2.20", got)
}

func TestChooseMetallbAddressSkipsUsedAndIPv6Addresses(t *testing.T) {
	got, err := chooseMetallbAddress([]metalLBAddressPool{
		{
			name:       "static",
			autoAssign: false,
			addresses:  []string{"2001:db8::1", "192.0.2.10", "192.0.2.11"},
		},
	}, map[string]struct{}{
		"192.0.2.10": {},
	})

	require.NoError(t, err)
	require.Equal(t, "192.0.2.11", got)
}

func TestChooseMetallbAddressRejectsStaticAutoAssignedOverlap(t *testing.T) {
	_, err := chooseMetallbAddress([]metalLBAddressPool{
		{
			name:       "default",
			autoAssign: true,
			addresses:  []string{"192.0.2.10", "192.0.2.11"},
		},
		{
			name:       "static",
			autoAssign: false,
			addresses:  []string{"192.0.2.10"},
		},
	}, nil)

	require.ErrorContains(t, err, "non-auto-assigned")
}

func TestChooseMetallbAddressFallsBackToAutoAssignedPool(t *testing.T) {
	got, err := chooseMetallbAddress([]metalLBAddressPool{
		{
			name:       "default",
			autoAssign: true,
			addresses:  []string{"192.0.2.10"},
		},
	}, nil)

	require.NoError(t, err)
	require.Equal(t, "192.0.2.10", got)
}

func TestCandidateIPv4Addresses(t *testing.T) {
	require.Equal(t, []string{"192.0.2.9", "192.0.2.1"}, candidateIPv4Addresses("192.0.2.1-192.0.2.9"))
	require.Equal(t, []string{"192.0.2.7"}, candidateIPv4Addresses("192.0.2.4/30"))
	require.Nil(t, candidateIPv4Addresses("2001:db8::1"))
}
