package jwks

import (
	"crypto/fips140"
	"encoding/json"
	"io"

	"github.com/go-jose/go-jose/v4"
)

// go-jose hashes a key's x5c leaf certificate with SHA-1 to check it against x5t
// whenever a certificate chain is present. The thumbprint is an identifier, not
// a security function, so key set (de)serialization runs outside strict FIPS
// 140-3 enforcement (GODEBUG=fips140=only), where SHA-1 would panic.

// ParseKeySet parses a JSON Web Key Set.
func ParseKeySet(data []byte) (ks jose.JSONWebKeySet, err error) {
	fips140.WithoutEnforcement(func() { err = json.Unmarshal(data, &ks) })
	return ks, err
}

// DecodeKeySet reads one JSON Web Key Set from r.
func DecodeKeySet(r io.Reader) (ks jose.JSONWebKeySet, err error) {
	fips140.WithoutEnforcement(func() { err = json.NewDecoder(r).Decode(&ks) })
	return ks, err
}

// MarshalKeySet serializes a JSON Web Key Set.
func MarshalKeySet(ks jose.JSONWebKeySet) (data []byte, err error) {
	fips140.WithoutEnforcement(func() { data, err = json.Marshal(ks) })
	return data, err
}
