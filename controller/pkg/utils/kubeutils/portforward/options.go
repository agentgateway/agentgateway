package portforward

import (
	"io"
	"os"
)

type Option func(*properties)

type properties struct {
	kubeConfig        string
	kubeContext       string
	resourceType      string // deployment, service, pod
	resourceName      string
	resourceNamespace string
	localPort         int
	remotePort        int
	localAddress      string
	stdout            io.Writer
	stderr            io.Writer
}

func WithKubeContext(kubeContext string) Option {
	return func(config *properties) {
		config.kubeContext = kubeContext
	}
}

func WithService(name, namespace string) Option {
	return WithResource(name, namespace, "service")
}

func WithResource(name, namespace, resourceType string) Option {
	return func(config *properties) {
		config.resourceName = name
		config.resourceNamespace = namespace
		config.resourceType = resourceType
	}
}

// WithRemotePort sets the remote port for the port-forwarding
// This overrides the local port and makes it set as random.
// Retrieve the allocated port with the Address() method on the PortForwarder.
func WithRemotePort(remotePort int) Option {
	// 0 is special value for the local port, it will result in a port being chosen at random
	return WithPorts(0, remotePort)
}

// WithPorts sets the local and remote ports for the port-forwarding
// Prefer WithRemotePort for local tests to prevent collisions.
func WithPorts(localPort, remotePort int) Option {
	return func(config *properties) {
		config.localPort = localPort
		config.remotePort = remotePort
	}
}

func WithWriters(out, err io.Writer) Option {
	return func(config *properties) {
		config.stdout = out
		config.stderr = err
	}
}

func buildPortForwardProperties(options ...Option) *properties {
	//default
	cfg := &properties{
		kubeConfig:        "",
		kubeContext:       "",
		resourceName:      "",
		resourceNamespace: "",
		localPort:         0,
		remotePort:        0,
		localAddress:      "localhost",
		stdout:            os.Stdout,
		stderr:            os.Stderr,
	}

	//apply opts
	for _, opt := range options {
		opt(cfg)
	}

	return cfg
}
