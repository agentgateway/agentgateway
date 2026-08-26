package trace

import (
	"fmt"
	"time"

	"github.com/spf13/cobra"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/flag"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

const defaultFollowMaxDuration = 5 * time.Second

type traceFlags struct {
	namespace      string
	proxyAdminPort int
	traceFile      string
	expression     string
	followDuration time.Duration
	raw            bool
	port           int
	local          bool
}

func Command() flag.Command {
	flags := &traceFlags{
		proxyAdminPort: wellknown.ProxyAdminPort,
	}

	return flag.Command{
		Use:   "trace [resource] [-- <curl args...>]",
		Short: "Trace requests handled by an agentgateway pod or local instance.",
		Long:  "Start an agentgateway debug trace, render it in a TUI or JSONL, and optionally trigger the traced request against a pod or a local instance.",
		Example: `  agctl proxy trace
  
  # Watch for the next request on a pod and trace it, displaying the result in a TUI
  agctl proxy trace gateway/my-gateway
  
  # Watch for the next request on a pod and trace it, displaying the result in a JSONL format
  agctl proxy trace --raw
  
  # Enable tracing and send a request to the gateway. The <host> part of the request is only used for setting the Hostname of the request,
  # and is not used for DNS resolution.
  agctl proxy trace --port 80 -- http://host/some/path
  
  # Enable tracing and send a request to the gateway running locally.
  agctl proxy trace --local --port 8080 -- http://host/some/path
  
  # Render trace JSONL from a file or stdin instead of opening a live trace.
  agctl proxy trace --file trace.jsonl
  agctl proxy trace --file - --raw
  
  # Watch for the next request matching a CEL expression.
  agctl proxy trace --expression 'request.path == "/healthz"'

  # Follow matching requests for up to 10 minutes (JSONL includes requestId).
  agctl proxy trace --follow=10m --raw --expression 'request.path == "/v1/responses"'
  
  # Enable tracing and send a request to the gateway, with some curl arguments.
  agctl proxy trace gateway/my-gateway --raw --port 80 -- http://host/some/path -H "Authorization: Bearer sk-123"`,
		Args: func(cmd *cobra.Command, args []string) error {
			_, _, err := parseArgs(cmd, args, flags)
			return err
		},
		AddFlags: func(cmd *cobra.Command) {
			flags.attach(cmd)
		},
		RunE: func(cmd *cobra.Command, args []string) error {
			resourceArg, requestURL, err := parseArgs(cmd, args, flags)
			if err != nil {
				return err
			}
			return run(cmd, flags, resourceArg, requestURL)
		},
	}
}

func (f *traceFlags) attach(cmd *cobra.Command) {
	cmd.Flags().StringVarP(&f.namespace, "namespace", "n", "", "Namespace to use when resolving resources")
	cmd.Flags().IntVar(&f.proxyAdminPort, "proxy-admin-port", f.proxyAdminPort, "Agentgateway admin port")
	cmd.Flags().StringVarP(&f.traceFile, "file", "f", "", "Trace JSONL file to render, or - for stdin")
	cmd.Flags().StringVar(&f.expression, "expression", "", "CEL expression for selecting which request to trace")
	cmd.Flags().DurationVar(&f.followDuration, "follow", defaultFollowMaxDuration, "Continue tracing matching requests for this duration (maximum 1h)")
	cmd.Flags().Lookup("follow").NoOptDefVal = defaultFollowMaxDuration.String()
	cmd.Flags().BoolVar(&f.raw, "raw", false, "Print trace events as JSONL instead of opening the TUI")
	cmd.Flags().IntVar(&f.port, "port", 0, "Gateway listener port to use when triggering a request")
	cmd.Flags().BoolVar(&f.local, "local", false, "Trace against a local agentgateway instance on 127.0.0.1")
}

func parseArgs(cmd *cobra.Command, args []string, flags *traceFlags) (string, []string, error) {
	resourceArgs, requestArgs := splitArgsAtDash(args, cmd.ArgsLenAtDash())
	if len(resourceArgs) > 1 {
		return "", nil, fmt.Errorf("accepts at most 1 resource arg, received %d", len(resourceArgs))
	}
	if flags.proxyAdminPort < 1 || flags.proxyAdminPort > 65535 {
		return "", nil, fmt.Errorf("invalid --proxy-admin-port %d", flags.proxyAdminPort)
	}
	if flags.local && len(resourceArgs) > 0 {
		return "", nil, fmt.Errorf("--local does not accept a resource argument")
	}
	follow := cmd.Flags().Changed("follow")
	if follow && flags.followDuration < time.Millisecond {
		return "", nil, fmt.Errorf("--follow duration must be at least 1ms")
	}
	if follow && flags.followDuration > time.Hour {
		return "", nil, fmt.Errorf("--follow duration must not exceed 1h")
	}
	if flags.traceFile != "" {
		if len(resourceArgs) > 0 {
			return "", nil, fmt.Errorf("--file does not accept a resource argument")
		}
		if flags.local {
			return "", nil, fmt.Errorf("at most one of --file or --local may be passed")
		}
		if flags.expression != "" {
			return "", nil, fmt.Errorf("--file does not accept --expression")
		}
		if follow {
			return "", nil, fmt.Errorf("--file does not accept --follow")
		}
		if len(requestArgs) > 0 {
			return "", nil, fmt.Errorf("--file does not accept a request URL after --")
		}
	}
	if flags.port < 0 || flags.port > 65535 {
		return "", nil, fmt.Errorf("invalid --port %d", flags.port)
	}
	if flags.port == 0 && len(requestArgs) > 0 {
		return "", nil, fmt.Errorf("a request URL requires --port")
	}
	if flags.port != 0 && len(requestArgs) == 0 {
		return "", nil, fmt.Errorf("--port requires a request URL after --")
	}

	var resourceArg string
	if len(resourceArgs) == 1 {
		resourceArg = resourceArgs[0]
	}
	return resourceArg, requestArgs, nil
}

func splitArgsAtDash(args []string, dash int) ([]string, []string) {
	if dash < 0 {
		return args, nil
	}
	return args[:dash], args[dash:]
}
