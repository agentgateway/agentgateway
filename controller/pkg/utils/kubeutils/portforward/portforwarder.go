package portforward

import (
	"context"

	"github.com/avast/retry-go/v4"
)

// Inspired by: https://github.com/istio/istio/blob/master/pkg/kube/portforwarder.go

// PortForwarder manages the forwarding of a single port.
// Implementations are NOT thread-safe, as the goroutine that Starts the PortForward
// should also be the one that Closes it
type PortForwarder interface {
	// Start runs this PortForwarder.
	Start(ctx context.Context, options ...retry.Option) error

	// Address returns the local forwarded address. Only valid while the apiPortForwarder is running.
	Address() string

	// Close this apiPortForwarder and release any resources.
	Close()

	// ErrChan returns a channel that returns an error when one is encountered. While Start() may return an initial error,
	// the port-forward connection may be lost at anytime. The ErrChan can be read to determine if/when the port-forwarding terminates.
	// This can return nil if the port forwarding stops gracefully.
	ErrChan() <-chan error

	// WaitForStop blocks until connection closed (e.g. control-C interrupt)
	WaitForStop()
}
