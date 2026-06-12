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

// HashKey returns the SHA-256 FetchKey over parts. Each part is
// null-terminated so the concatenation is unambiguous, making the key a stable
// identity for (target, transport) tuples — callers that change the part list
// or its order change every derived cache key.
func HashKey(parts ...string) FetchKey {
	hash := sha256.New()
	for _, part := range parts {
		_, _ = hash.Write([]byte(part))
		_, _ = hash.Write([]byte{0})
	}
	return FetchKey(hex.EncodeToString(hash.Sum(nil)))
}

func (r FetchTarget) Key() FetchKey {
	transport := r.Transport

	parts := []string{r.URL}
	if r.ProxyURL != "" {
		parts = append(parts, r.ProxyURL)
	}
	parts = append(parts,
		transportVerificationFingerprint(r.URL, transport.Verification),
		transport.ServerName,
		transport.CABundleHash,
	)
	parts = append(parts, transport.NextProtos...)

	pt := r.ProxyTransport
	if pt.ServerName != "" || pt.CABundleHash != "" || pt.Verification != "" || len(pt.NextProtos) > 0 {
		parts = append(parts,
			transportVerificationFingerprint(r.ProxyURL, pt.Verification),
			pt.ServerName,
			pt.CABundleHash,
		)
		parts = append(parts, pt.NextProtos...)
	}

	return HashKey(parts...)
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
