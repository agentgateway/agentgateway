package helmutils

import (
	"context"
	"io"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/cmdutils"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/threadsafe"
)

// Client is a utility for executing `helm` commands
type Client struct {
	// receiver is the default destination for the helm stdout and stderr
	receiver io.Writer

	namespace string
	helmPath  string
}

// NewClient returns an implementation of the helmutils.Client
func NewClient() *Client {
	return &Client{
		receiver: io.Discard,
		helmPath: "helm",
	}
}

// WithReceiver sets the io.Writer that will be used by default for the stdout and stderr
// of cmdutils.Cmd created by the Client
// This modifies the value in place, so affects shared references to the Client and future commands run by the Client.
// Wrap this in a threadsafe struct to avoid data races when wrapped in io.MultiWriter in cmdutils.
func (c *Client) WithReceiver(receiver io.Writer) *Client {
	c.receiver = &threadsafe.WriterWrapper{W: receiver}
	return c
}

// WithNamespace sets the namespace that all commands will be invoked against
func (c *Client) WithNamespace(ns string) *Client {
	c.namespace = ns
	return c
}

// WithHelmPath sets the path to the helm executable
func (c *Client) WithHelmPath(path string) *Client {
	c.helmPath = path
	return c
}

// Command returns a Cmd that executes kubectl command, including the --context if it is defined
// The Cmd sets the Stdout and Stderr to the receiver of the Cli
func (c *Client) Command(ctx context.Context, args ...string) cmdutils.Cmd {
	if c.namespace != "" {
		args = append([]string{"--namespace", c.namespace}, args...)
	}

	return cmdutils.Command(ctx, c.helmPath, args...).
		// For convenience, we set the stdout and stderr to the receiver
		// This can still be overwritten by consumers who use the commands
		WithStdout(c.receiver).
		WithStderr(c.receiver)
}

// RunCommand creates a Cmd and then runs it
func (c *Client) RunCommand(ctx context.Context, args ...string) error {
	return c.Command(ctx, args...).Run().Cause()
}

func (c *Client) Install(ctx context.Context, opts InstallOpts) error {
	args := append([]string{"install"}, opts.all()...)
	return c.RunCommand(ctx, args...)
}

func (c *Client) Upgrade(ctx context.Context, opts InstallOpts) error {
	args := append([]string{"upgrade", "--install"}, opts.all()...)
	return c.RunCommand(ctx, args...)
}

func (c *Client) Uninstall(ctx context.Context, opts UninstallOpts) error {
	args := append([]string{"uninstall"}, opts.all()...)
	return c.RunCommand(ctx, args...)
}

func (c *Client) Delete(ctx context.Context, extraArgs ...string) error {
	args := append([]string{
		"delete",
	}, extraArgs...)

	return c.RunCommand(ctx, args...)
}

func (c *Client) AddRepository(ctx context.Context, chartName string, chartUrl string, extraArgs ...string) error {
	args := append([]string{
		"repo",
		"add",
		chartName,
		chartUrl,
	}, extraArgs...)
	return c.RunCommand(ctx, args...)
}

// ReleaseExists checks if a helm release exists in the given namespace.
// It returns true if the release exists, false otherwise.
func (c *Client) ReleaseExists(ctx context.Context, releaseName, namespace string) bool {
	args := []string{"status", releaseName}
	if namespace != "" {
		args = append(args, "--namespace", namespace)
	}

	err := c.Command(ctx, args...).Run().Cause()
	return err == nil
}
