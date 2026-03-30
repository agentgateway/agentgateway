package remotehttp

import (
	"crypto/sha256"
	"encoding/hex"
	"strings"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
)

type FetchKey string

func (k FetchKey) String() string {
	return string(k)
}

func (r FetchTarget) Key() FetchKey {
	transport := r.Transport

	hash := sha256.New()
	writeHashPart := func(value string) {
		_, _ = hash.Write([]byte(value))
		_, _ = hash.Write([]byte{0})
	}

	writeHashPart(r.URL)
	writeHashPart(transportVerificationFingerprint(r.URL, transport.Verification))
	writeHashPart(transport.ServerName)
	writeHashPart(transport.CABundleHash)
	for _, nextProto := range transport.NextProtos {
		writeHashPart(nextProto)
	}

	sum := hash.Sum(nil)
	return FetchKey(hex.EncodeToString(sum[:]))
}

func transportVerificationFingerprint(url string, mode agentgateway.InsecureTLSMode) string {
	switch mode {
	case agentgateway.InsecureTLSModeAll:
		return "insecure"
	case agentgateway.InsecureTLSModeHostname:
		return "hostname"
	default:
		if strings.HasPrefix(url, "http://") {
			return ""
		}
		return "strict"
	}
}
