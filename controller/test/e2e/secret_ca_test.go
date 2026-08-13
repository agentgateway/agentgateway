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
