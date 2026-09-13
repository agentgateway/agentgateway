//go:build e2e

package e2e_test

import (
	"testing"

	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
)

func TestBackendTLSSecretCA(tt *testing.T) {
	t := New(tt)
	t.Apply(
		manifest("secret-ca", "source.yaml"),
		manifest("secret-ca", "route.yaml"),
	)
	t.Send("secret-ca.example.com", base.ExpectOK())
}

// TestBackendTLSCustomCAKey covers reading the CA bundle from a key other than ca.crt. Both
// sources in the manifest hold garbage under ca.crt, so the handshake only succeeds if the
// `key` selector is honoured.
func TestBackendTLSCustomCAKey(tt *testing.T) {
	t := New(tt)
	t.Apply(
		manifest("secret-ca", "custom-key.yaml"),
		manifest("secret-ca", "custom-key-route.yaml"),
	)
	t.Send("custom-key-ca.example.com", base.ExpectOK())
}
