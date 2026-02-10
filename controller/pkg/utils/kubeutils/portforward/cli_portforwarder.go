package portforward

import (
	"bufio"
	"context"
	"fmt"
	"net"
	"os"
	"os/exec"
	"strconv"
	"strings"

	"github.com/avast/retry-go/v4"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/cmdutils"
)

var _ PortForwarder = &cliPortForwarder{}

// NewCliPortForwarder returns an implementation of a PortForwarder that relies on the Kubernetes CLI to perform port-forwarding
// This implementation is NOT thread-safe
func NewCliPortForwarder(options ...Option) PortForwarder {
	return &cliPortForwarder{
		properties: buildPortForwardProperties(options...),

		// The following are populated when Start is invoked
		errCh:     nil,
		cmd:       nil,
		cmdCancel: nil,
	}
}

type cliPortForwarder struct {
	// properties represents the set of user-defined values to configure the apiPortForwarder
	properties *properties

	errCh chan error

	cmd       *exec.Cmd
	cmdCancel context.CancelFunc
}

func (c *cliPortForwarder) Start(ctx context.Context, options ...retry.Option) error {
	return retry.Do(func() error {
		return c.startOnce(ctx)
	}, options...)
}

func (c *cliPortForwarder) startOnce(ctx context.Context) error {
	if c.properties.localPort == 0 {
		// 0 is a special value, which means "choose for me a free port"
		freePort, err := getFreePort()
		if err != nil {
			return err
		}
		c.properties.localPort = freePort
	}

	cmdCtx, cmdCancel := context.WithCancel(ctx)
	c.cmd = exec.CommandContext( //nolint:gosec // G204: kubectl port-forward with controlled parameters from port forwarder config
		cmdCtx,
		"kubectl",
		"port-forward",
		"-n",
		c.properties.resourceNamespace,
		fmt.Sprintf("%s/%s", c.properties.resourceType, c.properties.resourceName),
		fmt.Sprintf("%d:%d", c.properties.localPort, c.properties.remotePort),
	)

	// Print command being executed when enabled
	if printPortForwardCommands {
		args := []string{
			"port-forward",
			"-n",
			c.properties.resourceNamespace,
			fmt.Sprintf("%s/%s", c.properties.resourceType, c.properties.resourceName),
			fmt.Sprintf("%d:%d", c.properties.localPort, c.properties.remotePort),
		}
		fmt.Fprintf(os.Stderr, "+ %s\n", cmdutils.PrettyCommand(false, "kubectl", args...))
	}

	// Errors should not happen here unless some other thing has futzed
	// with this cmd's stdout/err.
	fwdOut, err := c.cmd.StdoutPipe()
	if err != nil {
		return err
	}
	fwdErr, err := c.cmd.StderrPipe()
	if err != nil {
		return err
	}

	c.cmdCancel = cmdCancel

	c.errCh = make(chan error, 1)

	// short circuit error return if we can't even start
	if err := c.cmd.Start(); err != nil {
		return err
	}

	// TODO Because we are not using a real Go-only kube client but are spawning a long-running
	// subprocess, wait until the subprocess actually writes a success msg to stdout before
	// trying to query the endpoint or we will get spurious failures because this func
	// will return even though the port-forward hasn't happened yet.
	outScan := bufio.NewScanner(fwdOut)
	for outScan.Scan() {
		if strings.Contains(outScan.Text(), "Forwarding from") {
			// We are good, port-forward is ready.
			return nil
		}
	}

	// If we get here, we didn't get any stdout, so grab stderr and return it as error
	stdErr := bufio.NewScanner(fwdErr)
	stdErr.Scan()
	return fmt.Errorf("failed to start port-forward: %s", stdErr.Text())
}

func (c *cliPortForwarder) Address() string {
	return net.JoinHostPort(c.properties.localAddress, strconv.Itoa(c.properties.localPort))
}

func (c *cliPortForwarder) Close() {
	if c.cmdCancel != nil {
		c.cmdCancel()
	}
}

func (c *cliPortForwarder) ErrChan() <-chan error {
	// This channel is not functional in the cliPortForwarder implementation
	return c.errCh
}

func (c *cliPortForwarder) WaitForStop() {
	if c.cmd.Process != nil {
		c.errCh <- c.cmd.Wait()
	}
}

func getFreePort() (int, error) {
	l, err := net.Listen("tcp", ":0") //nolint:gosec // G102: Binding to all interfaces is intentional for finding free port
	if err != nil {
		return 0, err
	}
	defer l.Close()
	tcpAddr, ok := l.Addr().(*net.TCPAddr)
	if !ok {
		return 0, fmt.Errorf("Error occurred looking for an open tcp port")
	}
	return tcpAddr.Port, nil
}
