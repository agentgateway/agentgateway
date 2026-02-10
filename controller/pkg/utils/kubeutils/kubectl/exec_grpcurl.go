package kubectl

import (
	"context"
	"fmt"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/requestutils/grpcurl"
)

// GrpcurlResponse holds the structured output from a grpcurl command execution.
// This typically includes Stdout, Stderr. The exit code is part of the error.
type GrpcurlResponse struct {
	StdOut string
	StdErr string
}

// GrpcurlFromPod executes a grpcurl command in the specified pod using the cli's ExecWithOptions method.
// It constructs the grpcurl command from the given options, executes it, and returns the response.
// The error returned by ExecWithOptions (if any) is passed through. If err is nil, the command
// had an exit code of 0. If err is of type *ExecError, it contains the non-zero exit code.
func (c *Cli) GrpcurlFromPod(
	ctx context.Context,
	podOpts PodExecOptions,
	grpcurlOptions ...grpcurl.Option,
) (*GrpcurlResponse, error) {
	// Create a new grpcurl command based on the provided options
	grpcCmd := grpcurl.NewCommand(grpcurlOptions...)
	args := grpcCmd.ToArgs()

	// The command to execute in the pod is "grpcurl" followed by its arguments
	fullCommand := append([]string{
		"exec",
		"--container=grpcurl",
		podOpts.Name,
		"-n",
		podOpts.Namespace,
		"--",
		"grpcurl",
	}, args...)

	fmt.Printf("Executing grpcurl command: %s\n", fullCommand)

	// Execute the command in the pod
	stdout, stderr, err := c.ExecuteOn(ctx, c.kubeContext, fullCommand...)

	return &GrpcurlResponse{StdOut: stdout, StdErr: stderr}, err
}
