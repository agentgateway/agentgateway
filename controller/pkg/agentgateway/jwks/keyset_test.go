// Run this package's tests under strict FIPS 140-3 enforcement.
//go:debug fips140=only

package jwks

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"testing"

	"github.com/stretchr/testify/require"
)

// Key sets carrying x5c and x5t must round-trip under fips140=only, where
// go-jose's SHA-1 thumbprint check would otherwise panic.
func TestKeySetWithCertificateChainRoundTrips(t *testing.T) {
	in, err := ParseKeySet([]byte(sampleJWKS))
	require.NoError(t, err)
	require.Len(t, in.Keys, 2)
	for _, key := range in.Keys {
		require.Len(t, key.Certificates, 1)
		require.Equal(t, key.Certificates[0].PublicKey, key.Key)
		require.Len(t, key.CertificateThumbprintSHA1, 20)
	}

	data, err := MarshalKeySet(in)
	require.NoError(t, err)

	out, err := ParseKeySet(data)
	require.NoError(t, err)
	require.Equal(t, in, out)

	decoded, err := DecodeKeySet(bytes.NewReader(data))
	require.NoError(t, err)
	require.Equal(t, in, decoded)

	// The exemption must preserve go-jose's certificate/thumbprint consistency
	// checks on both parsing paths and on serialization.
	badThumbprint := bytes.Repeat([]byte{0xff}, 20)
	var document map[string]any
	require.NoError(t, json.Unmarshal(data, &document))
	document["keys"].([]any)[0].(map[string]any)["x5t"] = base64.RawURLEncoding.EncodeToString(badThumbprint)
	badData, err := json.Marshal(document)
	require.NoError(t, err)
	_, err = ParseKeySet(badData)
	require.ErrorContains(t, err, "x5c thumbprint does not match x5t")
	_, err = DecodeKeySet(bytes.NewReader(badData))
	require.ErrorContains(t, err, "x5c thumbprint does not match x5t")
	in.Keys[0].CertificateThumbprintSHA1 = badThumbprint
	_, err = MarshalKeySet(in)
	require.ErrorContains(t, err, "thumbprint, does not match cert chain")
}
