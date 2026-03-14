package remotehttp

import (
	"crypto/sha256"
	"encoding/hex"
)

type FetchKey string

func (r Request) Key() FetchKey {
	transport := r.Transport

	hash := sha256.New()
	writeHashPart := func(value string) {
		_, _ = hash.Write([]byte(value))
		_, _ = hash.Write([]byte{0})
	}

	writeHashPart(r.URL)
	writeHashPart(string(transport.Verification))
	writeHashPart(transport.ServerName)
	writeHashPart(transport.CABundleHash)
	for _, nextProto := range transport.NextProtos {
		writeHashPart(nextProto)
	}

	sum := hash.Sum(nil)
	return FetchKey(hex.EncodeToString(sum[:]))
}
